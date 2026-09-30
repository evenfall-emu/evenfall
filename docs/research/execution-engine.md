# Execution engine

The execution engine runs ARM and Thumb code from caches of pre-decoded
instructions instead of fetching, decoding and dispatching each word through
the interpreter. It lives in
[`cpu/engine/`](../../systems/gba/src/cpu/engine/mod.rs):
[`arm.rs`](../../systems/gba/src/cpu/engine/arm.rs) for ARM code in IWRAM
and EWRAM, [`thumb.rs`](../../systems/gba/src/cpu/engine/thumb.rs) for Thumb
code in IWRAM, EWRAM and Game Pak ROM, and
[`mem.rs`](../../systems/gba/src/cpu/engine/mem.rs) for the data paths both
share. The ordinary interpreter (`Cpu::step` over `cpu-arm`'s `Core`) stays
the semantic and timing reference: it executes everything the engine
refuses, and randomized differentials compare the two on registers, flags,
memory, clocks and bus state.

## Exactness

A run leaves the machine exactly where the interpreter would after the same
instructions, however execution is sliced:

- **Clock.** Each instruction costs what the interpreter charges: its fetch,
  data and internal cycles, and a branch's two refill fetches. A run keeps a
  budget, the cycles left before `min(yield point, device deadline − 1)`,
  and stops _before_ any instruction that would not fit. No device event
  can fall inside a run, so devices need no attention until it ends.
- **Refusals.** A run also stops before anything it does not model: SWI,
  PSR writes, mode and instruction-set changes, HLE code, MMIO accesses the
  device batch cannot take, a pending interrupt, DMA, and stores that reach
  code (below). The interpreter then executes that instruction. A refused
  run changes nothing. When the cached op at the entry address is certainly
  refused with the current registers (an op the engine never runs, BX to
  ARM, or in ARM code a block transfer through a base register pointing at
  MMIO), the run is refused before any setup.
- **MMIO.** A single MMIO load or store runs at the instruction's cycle.
  IE, IF, IME, POSTFLG, HALTCNT and the DMA source, destination and count
  latches are read and written by the run itself, which stops only when a
  store halts the CPU or raises an IRQ it takes. DISPSTAT and VCOUNT loads
  read the PPU position the run started at, advanced to the access's data
  cycle; halfword and word stores to the affine parameters and reference
  points are made by the run where no dot is visible, where the bus's raster
  segments would record nothing. Other registers the device
  batch can take go through the bus between passes of the same run; a store
  that can schedule no device event keeps the run's device deadline. Code
  that writes several I/O registers in a row, and interrupt handlers that
  start by reading them, stay in one run.
- **BIOS.** ARM code in the BIOS runs in the IWRAM loop with BIOS fetch
  timing and read protection, and BX, loads of pc and data processing into
  pc switch between the BIOS and IWRAM op arrays without leaving the run.
- **Exit state.** Registers live in a local array and the CPSR or the Thumb
  flags in host registers. The Thumb array has 256 entries, so any byte of
  an op indexes it without a mask; the ARM loop, entered again after each
  MMIO access, keeps the 16 registers alone, with register numbers typed so
  that they need no bounds check. r15, the two-instruction pipeline, the next
  instruction address, the open-bus latch, the next fetch's access kind and
  the Game Pak prefetch buffer are materialized only when a run stops.
- **Entry state.** A run starts only if the pipeline holds what memory holds
  at the next two instruction addresses; otherwise the interpreter goes on.

## Op caches

An [`Op`](../../systems/gba/src/cpu/engine/mod.rs) is 8 bytes: a handler
kind, an inner kind or block length, two register fields and a 32-bit
immediate, packed operand or raw word. `CodeCache` holds one array per RAM
region and instruction set, with a slot per instruction address, and
lazily allocated Thumb pages of 32 KiB of ROM. A slot decodes itself on
first execution; debug builds assert on every executed op that it equals a
fresh decode. ARM arrays carry a second half of sentinels, so a masked index
never needs a bounds check and running off the region's end continues in
the next mirror.

Caches are transient. RAM writes through the bus forget the slots they cover
once their 256-byte page has held decoded code or the prefetch window of an
instruction. Inside a run, a store to such a page is checked exactly against
decoded ops of both instruction sets and the running code's prefetched
instructions; if it reaches one, the run stops and the interpreter performs
the store (and forgets the slots). ROM never changes, so ROM pages are never
forgotten. State loads and resets clear everything.

## Timing

- **RAM.** A fetch costs the same whether sequential or not: one cycle from
  IWRAM, three (Thumb) or six (ARM) from EWRAM.
- **Game Pak ROM without prefetch.** A fetch is sequential unless the
  previous instruction accessed data or spent internal cycles, so a run
  carries the cost of its next fetch.
- **Game Pak ROM with prefetch.** The buffer follows a closed form expressed
  in the budget: the head halfword is ready at a known budget, each later
  one a duty (the S wait) after it, at most eight ahead. A fetch waits for
  the head; a branch whose target is the halfword in flight keeps it,
  otherwise the buffer restarts; a ROM data load that meets a buffer fetch
  in flight waits a cycle and then discards the buffer.
  [`Bus::thumb_prefetch_entry` and `thumb_prefetch_finish`](../../systems/gba/src/bus/kernel.rs)
  translate between that form and `GamePakPrefetch`, and refuse buffer
  states the form does not describe. Code within 22 bytes of a 128 KiB
  block's end, where the buffer's next access is nonsequential, is left to
  the interpreter.

## Data paths

IWRAM and EWRAM loads and stores run inline at their bus cost. Game Pak ROM
loads run when the address holds plain image bytes (no EEPROM, GPIO port or
end of image). Video memory goes through
[`VideoMemory`](../../systems/gba/src/bus/kernel.rs), the one implementation
the bus also uses, so every write marks the scanline memo and the OAM caches
exactly as a bus write does:

- loads from palette RAM, VRAM and OAM always run;
- OAM stores always run: the bus records nothing about their timing;
- VRAM and palette stores run only if they end before the next visible dot.
  A run computes at entry how long the PPU stays outside visible dots: to
  the end of the frame in VBlank, to the next line in HBlank. There the bus
  records no raster history, and the device flush it performs before such a
  store only brings lazily advanced devices up to the clock, which the end
  of the batch does equally.

## ARM blocks

A data processing op that leads a straight run of one-fetch ALU ops (any
operand form but a register-specified shift, conditional ones included)
records the run's length in its `inner` byte the first time it executes.
When the budget covers the whole run, an out-of-line loop runs it with one
charge: a single table dispatch per op, two ops per turn. A B or taken
conditional B that ends a run continues into the run at its target while
the budget covers it, so a loop body runs without returning to the dispatch
loop. A slot that changed since its run was measured ends the run there; the
dispatch loop takes over and measures again.

## Evidence

Tests in [`engine/tests.rs`](../../systems/gba/src/cpu/engine/tests.rs)
compare the engine with stepping:

- `arm_ram_engine_matches_interpreter_for_random_code` and
  `thumb_engine_matches_interpreter_for_random_code`: random code in IWRAM,
  EWRAM and ROM (ROM page edges included), random registers pointing at
  RAM, ROM and video memory, flags, wait states, display modes, buffer
  states and horizons, trials starting in HDraw and VBlank, and
  self-modifying stores. After every run they compare registers, CPSR,
  clock, open bus, fetch kind, prefetch state, RAM, video memory, OAM
  caches, scanline memo marks and raster history, and at the end of each
  trial the PPU position.
- `thumb_rom_engine_matches_stepping_from_every_buffer_state`: every entry
  state of the prefetch buffer, internal cycles, ROM loads and branches to
  every distance from the buffer head.
- `video_stores_stop_before_the_first_visible_dot`: VRAM stores swept across
  the HBlank edge.
- `arm_blocks_follow_slots_that_change_after_measuring`: a measured block
  whose slots change at every position.

The Thumb ROM, ARM IWRAM and runner tests in
[`cpu/mod.rs`](../../systems/gba/src/cpu/mod.rs) and
[`audit_tests.rs`](../../systems/gba/src/cpu/audit_tests.rs), ported from
ZiGBA's kernel tests, drive the engine too, and the parity harness runs
every scenario through it under several slice schedules.

## Open questions

Thumb code runs without blocks (see above); a design that costs nothing per
op when no block leads, such as a separate block kind chosen at decode,
might recover the diagnostic loop's gain without the dispatch cost. Block
transfers to video memory or MMIO, MMIO accesses the device batch cannot
take, and Thumb code in the BIOS still go to the interpreter. Games with
self-modifying code in hot loops pay for exits at each store into code.
