# Execution engine

A plain interpreter spends most of its time on bookkeeping.
For every instruction it fetches the word through the bus, decodes it, dispatches to a handler, and brings every device up to date.
Games run the same few loops millions of times, so almost all of that work repeats.

The execution engine removes it.
It decodes each instruction once into a cache, then runs straight from the cache until the next point where something outside the CPU could notice: a device event, an I/O access it can't handle, a store into code.
It charges every instruction exactly the cycles the interpreter would, so the rest of the machine can't tell the difference.

The engine lives in [`cpu/engine/`](../../systems/gba/src/cpu/engine/mod.rs):

- [`arm.rs`](../../systems/gba/src/cpu/engine/arm.rs) runs ARM code from IWRAM and EWRAM,
- [`thumb.rs`](../../systems/gba/src/cpu/engine/thumb.rs) runs Thumb code from IWRAM, EWRAM and cartridge ROM,
- [`mem.rs`](../../systems/gba/src/cpu/engine/mem.rs) holds the memory access paths both share.

The interpreter (`Cpu::step` over `cpu-arm`'s `Core`) stays the reference.
It runs everything the engine refuses, and randomized tests compare the two on registers, flags, memory, clocks and bus state.
[`performance.md`](../performance.md#execution-engine) summarizes the engine alongside the other fast paths.

## Staying exact

A run leaves the machine exactly where the interpreter would after the same instructions, however execution is sliced.

**Clock.** Each instruction costs what the interpreter charges: its fetch, data and internal cycles, and a branch's two refill fetches.
A run keeps a budget, the cycles left before `min(yield point, device deadline − 1)`, and stops before any instruction that wouldn't fit.
So no device event can fall inside a run, and devices need no attention until it ends.

**Refusals.** A run also stops before anything it doesn't model, and the interpreter runs that instruction:

- `SWI`, PSR writes, mode and instruction-set changes, and HLE BIOS code,
- I/O accesses the device batch can't take,
- a pending interrupt, or DMA,
- stores that reach decoded code ([below](#op-caches)).

A refused run changes nothing.
When the op at the entry address will certainly be refused with the current registers (an op the engine never runs, a `BX` to ARM, or in ARM code a block transfer whose base points at I/O), the run is refused before any setup.

**I/O.** A single I/O load or store runs at the instruction's cycle.

- IE, IF, IME, POSTFLG, HALTCNT and the DMA source, destination and count registers are read and written by the run itself.
  The run only stops when a store halts the CPU or raises an interrupt it takes.
- DISPSTAT and VCOUNT loads read the display position the run started at, advanced to the access's cycle.
- Stores to the affine parameters and reference points are made by the run itself while no pixel is being drawn, where the bus would record no raster change anyway.
- Other registers the device batch can take go through the bus between passes of the same run.
  A store that can't schedule a device event keeps the run's device deadline.

So code that writes several I/O registers in a row, and interrupt handlers that start by reading them, stay in one run.

**BIOS.** ARM code in the BIOS runs in the IWRAM loop, with the BIOS's fetch timing and read protection.
`BX`, loads into `pc` and data processing into `pc` switch between the BIOS and IWRAM op arrays without leaving the run.

**Registers.** Registers live in a local array, and the CPSR or the Thumb flags in host registers.
The Thumb array has 256 entries, so any byte of an op can index it without a mask.
The ARM loop, entered again after each I/O access, keeps just the 16 registers, with register numbers typed so that they need no bounds check.
The program counter, the two-instruction pipeline, the next instruction address, the open-bus value, the next fetch's access kind and the cartridge prefetch buffer are only written back when a run stops.

**Entry.** A run starts only if the pipeline holds what memory holds at the next two instruction addresses.
Otherwise the interpreter carries on.

## Op caches

An [`Op`](../../systems/gba/src/cpu/engine/mod.rs) is 8 bytes: a handler kind, an inner kind or block length, two register fields, and a 32-bit immediate, packed operand or raw word.
`CodeCache` holds one array per RAM region and instruction set, with a slot per instruction address, and Thumb pages of 32 KiB of ROM, allocated when first used.

A slot decodes itself the first time it runs.
Debug builds check on every executed op that it equals a fresh decode.
ARM arrays carry a second half of sentinel slots, so a masked index never needs a bounds check, and running off the end of a region continues in its next mirror.

Caches aren't part of the machine's state.
A RAM write through the bus forgets the slots it covers, once their 256-byte page has held decoded code or the prefetched instructions of running code.
Inside a run, a store to such a page is checked exactly against decoded ops of both instruction sets and the running code's prefetched instructions.
If it hits one, the run stops and the interpreter makes the store and forgets the slots.
ROM never changes, so ROM pages are never forgotten.
Loading a state or resetting clears everything.

## Timing

- **RAM.** A fetch costs the same whether sequential or not: one cycle from IWRAM, three (Thumb) or six (ARM) from EWRAM.
- **Cartridge ROM with prefetch off.** A fetch is sequential unless the previous instruction accessed data or spent internal cycles, so a run carries the cost of its next fetch.
- **Cartridge ROM with prefetch on.** The buffer follows a closed form expressed in the budget.
  The head halfword is ready at a known budget, and each later one a sequential wait after it, at most eight ahead.
  A fetch waits for the head.
  A branch whose target is the halfword being read keeps it, and otherwise the buffer restarts.
  A ROM data load that meets a fetch in progress waits a cycle and then empties the buffer.
  [`Bus::thumb_prefetch_entry` and `thumb_prefetch_finish`](../../systems/gba/src/bus/kernel.rs) translate between this form and `GamePakPrefetch`, and refuse buffer states the form doesn't describe.
  Code within 22 bytes of the end of a 128 KiB block, where the buffer's next access is non-sequential, is left to the interpreter.

## Memory accesses

IWRAM and EWRAM loads and stores run inline at their bus cost.
Cartridge ROM loads run when the address holds plain ROM bytes, not an EEPROM, the GPIO port or the end of the image.
Video memory goes through [`VideoMemory`](../../systems/gba/src/bus/kernel.rs), the same code the bus uses, so every write marks the scanline memo and the OAM caches exactly as a bus write would:

- Loads from palette RAM, VRAM and OAM always run.
- OAM stores always run, since the bus records nothing about their timing.
- VRAM and palette stores run only if they end before the next visible pixel.
  At entry, a run works out how long the display stays between visible pixels: to the end of the frame in VBlank, to the next line in HBlank.
  There the bus records no raster history, and the device flush it would do before such a store only brings devices up to the clock, which the end of the batch does too.

## ARM blocks

A data-processing op that starts a straight run of single-fetch ALU ops (any operand form but a register-specified shift, conditional ones included) records the run's length in its `inner` byte the first time it runs.
When the budget covers the whole run, an out-of-line loop runs it with a single charge: one table dispatch per op, two ops per turn.
A `B`, or a taken conditional `B`, that ends a run continues into the run at its target while the budget covers it, so a loop body runs without returning to the dispatch loop.
A slot that changed since its run was measured ends the run there, and the dispatch loop takes over and measures again.

Thumb code has no blocks.
They were tried: they sped up a long diagnostic loop, but Thumb game code has short runs of ALU ops, and the block check in every ALU op cost more than blocks saved.

## How it is checked

Tests in [`engine/tests.rs`](../../systems/gba/src/cpu/engine/tests.rs) compare the engine with stepping the interpreter:

- `arm_ram_engine_matches_interpreter_for_random_code` and `thumb_engine_matches_interpreter_for_random_code` run random code in IWRAM, EWRAM and ROM (ROM page edges included), with random registers pointing at RAM, ROM and video memory, random flags, wait states, display modes, buffer states and horizons, trials starting in HDraw and VBlank, and self-modifying stores.
  After every run they compare registers, CPSR, clock, open bus, fetch kind, prefetch state, RAM, video memory, OAM caches, scanline memo marks and raster history, and at the end of each trial the display position.
- `thumb_rom_engine_matches_stepping_from_every_buffer_state` covers every entry state of the prefetch buffer, internal cycles, ROM loads and branches to every distance from the buffer head.
- `video_stores_stop_before_the_first_visible_dot` sweeps VRAM stores across the edge of HBlank.
- `arm_blocks_follow_slots_that_change_after_measuring` changes a measured block's slots at every position.

The Thumb ROM, ARM IWRAM and runner tests in [`cpu/mod.rs`](../../systems/gba/src/cpu/mod.rs) and [`audit_tests.rs`](../../systems/gba/src/cpu/audit_tests.rs) run through the engine too.

## Open questions

- A Thumb block design that costs nothing per op when no block starts, such as a separate block kind chosen at decode, might get the gain without the dispatch cost.
- Block transfers to video memory or I/O, I/O accesses the device batch can't take, and Thumb code in the BIOS still go to the interpreter.
- Games with self-modifying code in hot loops pay for a run exit at each store into code.
