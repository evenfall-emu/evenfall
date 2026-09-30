# Memory map and bus

Everything the GBA's CPU and DMA do goes through one bus: fetching an instruction, reading a variable, writing a pixel to video memory, setting a hardware register.
The bus decides which chip answers an address, and how long the answer takes.

That second part matters more than it sounds.
Games run directly on the hardware, and much of their timing depends on how long their own code takes.
A music driver must refill its buffer before it runs dry, a raster effect must change a register before the next line is drawn, and a busy-wait loop counts iterations.
All of that relies on each access costing exactly what it costs on the console.
Many bugs that look like CPU or graphics bugs in a GBA emulator turn out to be bus bugs: a wrong wait state, a missing mirror, or the wrong value for a read of nothing.

In Evenfall, the bus is a model of behavior more than of storage.
For every access, [`Bus`](../../../systems/gba/src/bus/mod.rs) works out which region the address falls in, returns the value and the number of cycles the access took, applies any side effect (a register write that starts a DMA, say), and brings every other device up to that cycle.
Each access carries its address, its width, whether it continues the previous access or starts a new one, and who made it: a CPU instruction fetch, a CPU data access, or DMA.

## A few terms

- **Cycle.** One tick of the GBA's 16.78 MHz master clock.
  Every cost on this page is in cycles.
- **Wait states.** Extra cycles a slow chip adds to an access.
  The cartridge is the slowest part of the machine, and a register (`WAITCNT`) lets the game choose how many wait states it uses.
- **N and S accesses.** A non-sequential (N) access starts at a new address.
  A sequential (S) access continues right after the previous one and is usually cheaper, because the chip already knows where to read next.
- **Open bus.** What a read returns when no chip answers.
  On the GBA it is the last value that crossed the bus, not zero.
- **Mirror.** A region smaller than its address window repeats to fill it, so several addresses reach the same byte.

Addresses are hexadecimal, written `04000204h`.
The CPU's side of an access (how instructions turn into fetches and data accesses) is described in [`cpu.md`](cpu.md#cycle-classes-and-timed-accesses).

## Memory map

| Range                   | Region                     | Bus    | Cycles (byte / half / word) |
| ----------------------- | -------------------------- | ------ | --------------------------- |
| `00000000h`–`00003FFFh` | BIOS ROM, 16 KiB           | 32-bit | 1 / 1 / 1                   |
| `00004000h`–`01FFFFFFh` | unmapped                   | —      | open bus, 1                 |
| `02000000h`–`0203FFFFh` | EWRAM, 256 KiB             | 16-bit | 3 / 3 / 6                   |
| `03000000h`–`03007FFFh` | IWRAM, 32 KiB              | 32-bit | 1 / 1 / 1                   |
| `04000000h`–`040003FEh` | I/O registers              | 32-bit | 1 / 1 / 1                   |
| `05000000h`–`050003FFh` | palette RAM, 1 KiB         | 16-bit | 1 / 1 / 2                   |
| `06000000h`–`06017FFFh` | VRAM, 96 KiB               | 16-bit | 1 / 1 / 2                   |
| `07000000h`–`070003FFh` | OAM, 1 KiB                 | 32-bit | 1 / 1 / 1                   |
| `08000000h`–`09FFFFFFh` | Game Pak ROM, wait state 0 | 16-bit | `WAITCNT`                   |
| `0A000000h`–`0BFFFFFFh` | Game Pak ROM, wait state 1 | 16-bit | `WAITCNT`                   |
| `0C000000h`–`0DFFFFFFh` | Game Pak ROM, wait state 2 | 16-bit | `WAITCNT`                   |
| `0E000000h`–`0E00FFFFh` | Game Pak SRAM/Flash window | 8-bit  | `WAITCNT`                   |
| `10000000h`–`FFFFFFFFh` | unmapped                   | —      | open bus, 1                 |

Source: [GBATEK, GBA Memory Map][gbatek-map].
The cycle column is the whole cost of one access.

The table explains a lot about how games are written.
IWRAM is small but fast and 32 bits wide, so games copy their hottest ARM code there.
EWRAM is eight times larger but 16 bits wide, so a 32-bit access takes two trips.
Cartridge ROM is the slowest, and its cost depends on `WAITCNT` ([below](#waitcnt-and-game-pak-wait-states)).
EEPROM, the real-time clock and cartridge sensors also sit inside the cartridge windows and are described in [`cartridge.md`](cartridge.md).
When a write to video memory becomes visible on screen is described in [`video.md`](video.md).

## Region behavior

**Mirroring.** EWRAM repeats every 256 KiB up to `02FFFFFFh`, IWRAM every 32 KiB up to `03FFFFFFh`, and palette RAM and OAM every 1 KiB through their 16 MiB windows.
The save window repeats every 64 KiB up to `0FFFFFFFh`, and [`cartridge.md`](cartridge.md) explains how a smaller save chip repeats inside it.
The three ROM windows all show the same cartridge image of up to 32 MiB, each with its own wait states.
The BIOS and I/O registers don't repeat.
An I/O register answers only at its own address.

**VRAM.** The 96 KiB of VRAM repeat in a 128 KiB pattern, which leaves a 32 KiB hole to fill ([GBATEK, Unpredictable Things][gbatek-unpredictable]).
The hole mirrors the last 32 KiB, except in bitmap modes:

| Offset in 128 KiB window | Tiled modes (0–2)      | Bitmap modes (3–5)                       |
| ------------------------ | ---------------------- | ---------------------------------------- |
| `00000h`–`17FFFh`        | VRAM `00000h`–`17FFFh` | same                                     |
| `18000h`–`1BFFFh`        | VRAM `10000h`–`13FFFh` | nothing: reads return 0, writes are lost |
| `1C000h`–`1FFFFh`        | VRAM `14000h`–`17FFFh` | same                                     |

In bitmap modes, sprite graphics start at `14000h` instead of `10000h`, and the part of the mirror that would land below that boundary reads as nothing. mGBA 0.10.5 ([`memory.c`][mgba-memory]) and NanoBoyAdvance ([`ppu.hh`][nba-ppu]) agree.

**Byte writes to video memory.** Palette RAM and the background part of VRAM can only be written 16 bits at a time.
A byte write there stores the same byte into both halves of the halfword.
Byte writes to sprite VRAM and to OAM are ignored.
A word write to palette RAM or VRAM is two halfword writes, and each one becomes visible after its own cycle.

**Save memory.** The save window is 8 bits wide, and every access pays the SRAM wait states.
How 16- and 32-bit accesses map onto its single byte is described in [`cartridge.md`](cartridge.md).
This is the only region where the CPU passes a misaligned address to the bus unchanged.

**Reading past the end of the ROM.** The cartridge shares the same wires for addresses and data.
When the ROM has nothing at an address, the wires still hold the address the console just sent, so the read returns address bits 16–1.
A word read at `08001234h` returns `091B091Ah`.

**Writes that change nothing.** A write to cartridge ROM is dropped unless it reaches a save chip or the GPIO port.
It still costs the cartridge wait states and stops the prefetch buffer, as in NanoBoyAdvance and mGBA.
Writes to the BIOS, to unmapped addresses and to unused or read-only registers cost one cycle and are dropped.
None of them stops emulation.

**Misaligned accesses.** The CPU aligns its own requests and rotates the data it loads ([`cpu.md`](cpu.md#single-halfword-and-signed-transfers)), so apart from the save window the bus only sees aligned accesses.
A misaligned 32-bit I/O access can't happen from guest code, so the bus treats one as a bug in Evenfall.
The HLE BIOS checks the pointers a game passes it for the same reason ([`bios.md`](bios.md#fault-containment)).

## Open bus and unused I/O

A read that reaches no chip returns **open bus**: whatever value the bus last carried.
Buggy games read unmapped memory, typically through a stale pointer, and their behavior then depends on that leftover value.

Evenfall keeps the value of the most recent read by anyone, whether a CPU instruction fetch, a CPU data read or a DMA read.
A read of an unmapped address returns the byte or halfword of that value the address selects, and costs one cycle.
A Thumb instruction fetch is only 16 bits, so it leaves its halfword with the upper half zero.
DMA keeps its own value for sources it never reads, described in [`timers-dma-interrupts.md`](timers-dma-interrupts.md).

I/O registers are a mix of readable, write-only and unused addresses, and each reads differently ([GBATEK, Unpredictable Things][gbatek-unpredictable]):

| Read                                                                                                      | Result                        |
| --------------------------------------------------------------------------------------------------------- | ----------------------------- |
| a word whose two halfwords are both unused                                                                | open bus                      |
| an unused halfword whose partner in the same word is readable                                             | zero                          |
| an unused halfword whose partner is also unused                                                           | the matching half of open bus |
| a write-only register (background scroll, affine reference points, DMA addresses and counts, `JOY_TRANS`) | zero                          |

## I/O byte lanes

I/O registers are 16 bits wide on a 32-bit bus.
A word access touches two registers, low then high, in one cycle.
Byte accesses need their own rules, which Evenfall takes from mGBA 0.10.5 (`GBAIOWrite8`) and NanoBoyAdvance:

- A byte read returns the addressed byte of the register.
- A byte write keeps the other byte from the register's **written** value, not from what it reads back.
  The difference shows for registers that read back something else or nothing: DMA addresses and counts, timer reload values (a timer reads its counter instead), background scroll, affine reference points and `JOY_TRANS`.
- Sound register bytes go to per-channel handlers ([`audio.md`](audio.md)).
- A byte write to `IF`, the interrupt flags, clears flags only in that byte.
- Byte writes to the read-only `VCOUNT` and `KEYINPUT` are ignored.
- `POSTFLG` (`04000300h`) and `HALTCNT` (`04000301h`) are two byte registers sharing one halfword ([`timers-dma-interrupts.md`](timers-dma-interrupts.md)).
- A register with no stored write value ignores byte writes, instead of gaining storage it doesn't have on hardware.

## BIOS read protection

The BIOS protects itself from being read by games.
A read from the BIOS region by code running outside the BIOS doesn't return BIOS data.
It returns the last instruction the CPU fetched from the BIOS, which changes as the BIOS boots, runs services and dispatches interrupts ([GBATEK, Unpredictable Things: Reading from BIOS Memory][gbatek-unpredictable]).

Evenfall models this in HLE mode, where it runs its own BIOS code and has no BIOS image at all.
Every read of `00000000h`–`00003FFFh` returns `Bus.bios_latch`, which the HLE BIOS sets wherever the real BIOS would have fetched those instructions:

| Moment                                      | Value       |
| ------------------------------------------- | ----------- |
| after boot                                  | `E129F000h` |
| after an SWI returns                        | `E3A02004h` |
| during an IRQ handler                       | `E25EF004h` |
| after an IRQ returns                        | `E55EC002h` |
| after an HLE sound jump-list helper returns | `E12FFF1Eh` |

The jsmolka BIOS test ROM checks these values around startup, `Sqrt` and the interrupt handler, and passes under HLE boot.

With a real BIOS image, the protection isn't modeled yet.
Every read returns the image's bytes, including reads from code outside the BIOS, so the same test ROM fails its first check ([compatibility](../../compatibility.md#known-divergences)).

## WAITCNT and Game Pak wait states

`WAITCNT` (`04000204h`) sets how many wait states the cartridge uses: one setting for the save chip, and two for each of the three ROM windows (one for N accesses and one for S accesses).
It also turns the prefetch buffer on and off ([GBATEK, GBA System Control][gbatek-system]).
Games usually set it once at startup to the fastest values their cartridge supports.

GBATEK's numbers are wait states, added to one base cycle.
Evenfall's tables (`refresh_game_pak_timing`, `sram_cycles`) hold totals, as NanoBoyAdvance's do:

| Bits  | Field                  | Setting → total cycles per access  |
| ----- | ---------------------- | ---------------------------------- |
| 1–0   | SRAM                   | 0–3 → 5, 4, 3, 9                   |
| 3–2   | WS0 first (N)          | 0–3 → 5, 4, 3, 9                   |
| 4     | WS0 second (S)         | 0–1 → 3, 2                         |
| 6–5   | WS1 first (N)          | 0–3 → 5, 4, 3, 9                   |
| 7     | WS1 second (S)         | 0–1 → 5, 2                         |
| 9–8   | WS2 first (N)          | 0–3 → 5, 4, 3, 9                   |
| 10    | WS2 second (S)         | 0–1 → 9, 2                         |
| 12–11 | PHI terminal output    | not modeled                        |
| 14    | prefetch buffer enable | [below](#game-pak-prefetch-buffer) |
| 15    | Game Pak type          | read-only, reads 0                 |

At power-on (`WAITCNT = 0`), a 16-bit ROM read costs 5 cycles as an N access and 3 as an S access.
A 32-bit read is two 16-bit reads, the second always S, so it costs 8 or 6.
A save chip access costs 5.

The cartridge counts addresses in 128 KiB blocks, so an access that starts a new block is always N, whatever the CPU asked for.
GBATEK's example is an `LDMIA` from `0801FFF8h`, whose read at `08020000h` pays N timing.

EWRAM always costs 3 cycles for a byte or halfword and 6 for a word.

## Sequential and non-sequential accesses

Whether the CPU's next instruction fetch counts as N or S depends on what the CPU did just before.
Evenfall doesn't assume instruction fetches are always sequential:

- An instruction fetch leaves the next fetch sequential.
- A data access to memory (a load, a store, or one step of a block transfer) breaks the sequence, so the next instruction fetch is N.
- A data access to an I/O register doesn't break the sequence.
- A jump refills the pipeline with an N fetch at the target, then an S fetch.
- In a block transfer, the first access is N and the rest are S.

These rules come from measurements, not from reading other emulators.
In the mGBA test suite's load and store rows, each result is exactly N − S cycles longer than fetch plus data plus internal cycles, which is the cost of the broken sequence.
I/O accesses show no such difference.

## Game Pak prefetch buffer

Code running from the cartridge is slow, because every instruction fetch pays ROM wait states.
The prefetch buffer hides some of that.
While the CPU is busy with something else (an internal cycle, or an access to another region), the cartridge keeps reading the next instructions into a small buffer.
A later fetch that finds its instruction there takes one cycle instead of several ([GBATEK, GBA GamePak Prefetch][gbatek-prefetch]).
Games turn it on with `WAITCNT` bit 14, and it only ever holds instructions.

Evenfall models the buffer in `Bus.game_pak_prefetch` (`GamePakPrefetch`).
GBATEK describes the behavior, and mGBA 0.10.5 and NanoBoyAdvance serve as cross-checks:

- **Start.** When the buffer can't serve an instruction fetch from ROM, it is emptied, the fetch pays its normal wait states, and the buffer starts reading from the next instruction: halfwords for Thumb code, words for ARM code.
- **Size.** Eight halfwords, or four words.
- **Filling.** Each entry takes the window's S cost to read (twice that for an ARM word), or its N cost when it starts a 128 KiB block.
  The buffer only fills while the CPU isn't using the cartridge, and stops when full.
- **Hits.** A fetch whose instruction is already buffered takes one cycle.
  A fetch of the entry still being read waits for that read to finish.
- **Interruptions.** Any other cartridge access (a ROM read by the CPU or DMA, the save chip, EEPROM, GPIO, or any cartridge write) empties the buffer before paying its own wait states.
  It costs one extra cycle when an entry was one cycle from done, or when an ARM word was half read.
- **Leaving ROM.** An instruction fetch from any other region empties the buffer.
- **`WAITCNT` changes.** The entry being read keeps its cost.
  The next one uses the new settings.
- **Turned off.** GBATEK describes a hardware bug with the buffer disabled: a ROM instruction that has internal cycles and doesn't write the program counter pays an N fetch instead of an S fetch.
  Evenfall makes the first ROM instruction fetch after any internal cycle N.
  Turning the buffer off also makes the next ROM instruction fetch N.

## How it is checked

Each rule on this page has a test in [`tests/bus.rs`](../../../systems/gba/tests/bus.rs), named after the rule it checks, for example `unused_game_pak_rom_reads_expose_the_multiplexed_halfword_address` or `game_pak_timing_follows_every_wait_field_and_forces_n_at_128_kib_boundaries`.

### mGBA suite timing tables

The [mGBA test suite](https://github.com/mgba-emu/suite) records how many cycles short instruction sequences take on real hardware, in `src/timing.c`.
Building the suite's ROM needs the devkitARM toolchain, so [`tests/mgba_suite_timing.rs`](../../../systems/gba/tests/mgba_suite_timing.rs) reproduces the measurement instead.
Each case builds a small ROM shaped like the suite's (start a timer, run the code under test, stop the timer), runs it in Evenfall, and subtracts a calibration run with no code, as the suite does.
Only the instruction sequences and the recorded hardware numbers come from the suite.

Cartridge rows run under the suite's eight `WAITCNT` settings: prefetch on or off, and two choices each of N and S wait states.
RAM rows run the same code from EWRAM and IWRAM.

| Group               | What it checks                                                                                       |
| ------------------- | ---------------------------------------------------------------------------------------------------- |
| ARM basics          | fetch costs, the broken-sequence rule, the extra load cycle, ROM data reads interrupting prefetch    |
| Trivial loops       | ARM and Thumb loops in ROM, EWRAM and IWRAM, where every backward jump refills the pipeline          |
| Block transfers     | `ldmia` and `stmia` of 1, 2 and 6 registers: first access N, the rest S, the trailing internal cycle |
| OAM-to-ROM overflow | a block load that runs off the end of OAM into ROM, decoded one word at a time                       |
| Multiplies          | every multiply instruction on ten operands, including how operand size sets the cost                 |
| Interworking        | the cost of switching between ARM and Thumb                                                          |
| Immediate DMA3      | 16 DMA shapes started from ARM and Thumb: start delay, cartridge timing and completion cost          |

The DMA rules are described in [`timers-dma-interrupts.md`](timers-dma-interrupts.md), along with [`tests/mgba_suite_source.rs`](../../../systems/gba/tests/mgba_suite_source.rs), which reproduces the suite's DMA register table.

Every row passes in every setting except one.
With code in ROM, prefetch on and `WAITCNT` `4000h` or `4004h`, the hardware's IWRAM-to-ROM DMA rows read one cycle more than Evenfall.
The test asserts Evenfall's value there, so any change shows up.

## Open questions

- **Real-BIOS read protection.** With a BIOS image, reads from outside the BIOS return the image's bytes instead of the protected value.
- **How open bus is composed.** Evenfall returns the last value read, and a Thumb fetch stores only its halfword.
  On hardware the value in Thumb state depends on the region and on the prefetched instructions (GBATEK, Unpredictable Things).
  A game that reads unmapped memory by mistake can depend on it, for instance a runaway string copy that only stops when open bus happens to read `FFh`.
- **Video-memory contention.** GBATEK adds one cycle when the CPU accesses video memory while the display is reading it.
  Evenfall charges fixed costs.
- **Affine parameter reads.** `BG2PA`–`BG3PD` read back their written values, although GBATEK lists them as write-only.
  The other write-only video registers read as zero.
- **EWRAM wait control.** The undocumented register at `04000800h` that changes EWRAM's speed isn't modeled.
  Writes to it are ignored.
- **DMA timing difference.** The one-cycle IWRAM-to-ROM difference above has no known cause.
- **`nop ; b pc`.** The suite's row with this label reuses the `b pc` test code, so the two-instruction version has never been measured.
- **Prefetch collision cycle.** The extra cycle when a data access interrupts an almost-finished prefetch follows NanoBoyAdvance.
  No hardware measurement isolates it.
- **Turning prefetch off.** The forced N fetch after disabling the buffer comes from other emulators, not from GBATEK.

[gbatek-map]: https://rust-console.github.io/gbatek-gbaonly/#gbamemorymap
[gbatek-unpredictable]: https://rust-console.github.io/gbatek-gbaonly/#gbaunpredictablethings
[gbatek-system]: https://rust-console.github.io/gbatek-gbaonly/#gbasystemcontrol
[gbatek-prefetch]: https://rust-console.github.io/gbatek-gbaonly/#gbagamepakprefetch
[mgba-memory]: https://github.com/mgba-emu/mgba/blob/0.10.5/src/gba/memory.c
[nba-ppu]: https://github.com/nba-emu/NanoBoyAdvance/blob/55b5cf0ae3d929582ac5bfd486558173502b8354/src/nba/src/hw/ppu/ppu.hh#L59-L115
