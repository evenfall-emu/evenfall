# Memory map and bus

Many visible GBA compatibility failures come from the bus rather than from
the CPU or the video chip: bus widths, wait states, prefetch, mirrors, and
what a read of nothing returns. Evenfall, like ZiGBA, models the bus as
behavior, not as storage. `Bus` ([`bus/mod.rs`](../../../systems/gba/src/bus/mod.rs)) decodes every
access, returns its value and its cost, applies its side effects, and advances
every device to the cycle of the access. Each access carries an address, a
width, an N/S kind and a source (CPU code fetch, CPU data, or DMA); the cycle
classes are defined in [`cpu.md`](cpu.md#cycle-classes-and-timed-accesses).

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

Source: [GBATEK, GBA Memory Map][gbatek-map]. The cycle column is the total
cost of one access, including the base cycle. Game Pak costs depend on
`WAITCNT` ([below](#waitcnt-and-game-pak-wait-states)). EEPROM, GPIO and
cartridge sensors sit inside the Game Pak windows and are described in
[`cartridge.md`](cartridge.md). Video-memory writes and their visibility
during a scanline belong to [`video.md`](video.md).

## Region behavior

**Mirroring.** EWRAM repeats every 256 KiB through `02FFFFFFh`, IWRAM every
32 KiB through `03FFFFFFh`, palette RAM and OAM every 1 KiB through their
16 MiB pages, and the SRAM window every 64 KiB through `0FFFFFFFh` (how a
smaller save chip repeats inside that window is described in
[`cartridge.md`](cartridge.md)). The three Game Pak windows show the same
cartridge image of up to 32 MiB. BIOS and I/O are not mirrored: I/O
registers decode only at their own addresses in the `04xxxxxxh` page.

**VRAM.** The 96 KiB of VRAM repeat in a 128 KiB pattern, not by
`address % 96 KiB` ([GBATEK, Unpredictable Things][gbatek-unpredictable]).
`vram_index` maps an offset within the 128 KiB page as follows:

| Offset in 128 KiB page | Tiled modes (0–2)      | Bitmap modes (3–5)                               |
| ---------------------- | ---------------------- | ------------------------------------------------ |
| `00000h`–`17FFFh`      | VRAM `00000h`–`17FFFh` | same                                             |
| `18000h`–`1BFFFh`      | VRAM `10000h`–`13FFFh` | inaccessible: reads return 0, writes are ignored |
| `1C000h`–`1FFFFh`      | VRAM `14000h`–`17FFFh` | same                                             |

OBJ tiles begin at VRAM `10000h` in tiled modes and `14000h` in bitmap modes.
In bitmap modes the mirror of `10000h`–`13FFFh` would alias below the OBJ
boundary, so it is cut off. Pinned by "bitmap modes leave the lower half of
mirrored VRAM unmapped" ([`bus/mod.rs`](../../../systems/gba/src/bus/mod.rs)), and
cross-checked against mGBA 0.10.5 [`memory.c`][mgba-memory] and
NanoBoyAdvance [`ppu.hh`][nba-ppu].

**Byte writes to video memory.** Palette RAM and the BG part of VRAM sit on a
16-bit bus with no byte strobes: a byte write stores the byte into both
halves of the aligned halfword. Byte writes to OBJ VRAM (at or above the OBJ
boundary for the current mode) and to OAM are ignored. Word writes to palette
RAM and VRAM are two halfword beats, each visible after its own cycle.

**Save memory.** The SRAM/Flash window is 8 bits wide and every access costs
the SRAM wait states; how halfword and word accesses map onto its single
byte lane is described in [`cartridge.md`](cartridge.md). The CPU passes
unaligned addresses in this region to the bus unchanged, unlike every other
region (`is_byte_wide_save_address`).

**Unused Game Pak ROM.** A read past the end of the cartridge image returns
the halfword the multiplexed address/data bus leaves behind: address bits
16–1 for each halfword, so a word read at `08001234h` returns `091B091Ah`.
Pinned by "unused Game Pak ROM reads expose the multiplexed halfword address".

**Writes that change nothing.** Stores to Game Pak ROM are discarded unless
they hit an attached GPIO or EEPROM port. They still cost the selected Game
Pak wait states and stop the prefetch buffer, as in NanoBoyAdvance's
cartridge-write path and mGBA's timed no-op fallback. Writes to the BIOS, to
unmapped addresses, and to unused or read-only I/O registers are discarded at
one cycle; none of them stops emulation. Pinned by "ordinary Game Pak writes
consume wait states without changing ROM" and "unused and read-only MMIO
writes are ignored".

**Unaligned accesses.** The CPU aligns its own requests and rotates loaded
data ([`cpu.md`](cpu.md#single-halfword-and-signed-transfers)), so the bus
receives aligned halfwords and words everywhere except the save window. An
I/O word access must be word-aligned; the bus treats a misaligned one as an
internal invariant violation, and the HLE BIOS checks guest pointers against
it before use ([`bios.md`](bios.md#fault-containment)).

## Open bus and unused I/O

A read that reaches no device returns **open bus**: whatever value the bus
last carried, not zero. ZiGBA keeps one `open_bus_value`, the value of the
most recent bus read by any master: a CPU instruction fetch, a CPU data read,
or a DMA read. Reads of unmapped regions return it, with the byte or halfword
lane selected by the address, and cost one cycle. A Thumb fetch leaves only
its halfword there, with the upper half zero.

DMA has its own latch for sources it never reads; see
[`timers-dma-interrupts.md`](timers-dma-interrupts.md).

I/O reads follow GBATEK's distinction between unused and write-only ports
([GBATEK, Unpredictable Things][gbatek-unpredictable]):

| Read                                                                                                      | Result                        |
| --------------------------------------------------------------------------------------------------------- | ----------------------------- |
| a word whose two halfwords are both unused                                                                | open bus                      |
| an unused halfword whose partner in the same word is readable                                             | zero                          |
| an unused halfword whose partner is also unused                                                           | the matching half of open bus |
| a write-only register (BG scroll offsets, affine reference points, DMA addresses and counts, `JOY_TRANS`) | zero                          |

Pinned by "unmapped and unused MMIO reads distinguish open-bus words from
paired zero halves" in
[ZiGBA's `bus_test.zig`](../../../refs/zigba/test/bus_test.zig).

## I/O byte lanes

The I/O registers are halfword registers on a 32-bit bus. A word access is two
halfword accesses, low then high, in one cycle. Byte accesses need rules of
their own, cross-checked against mGBA 0.10.5's `GBAIOWrite8` and
NanoBoyAdvance's byte-register dispatch:

- A byte read returns the addressed byte of the same modeled halfword.
- A byte write merges with the other lane taken from the device's **write**
  state, not its read value. This matters for registers that do not read back:
  DMA source, destination and count, timer reload values (the counter reads
  differently), BG scroll offsets, affine reference points and `JOY_TRANS`.
- Sound register bytes go to per-channel byte handlers
  ([`audio.md`](audio.md)).
- A byte write to `IF` clears flags only in the written lane.
- Byte writes to read-only `VCOUNT` and `KEYINPUT` are ignored.
- `POSTFLG` (`04000300h`) and `HALTCNT` (`04000301h`) are byte registers
  sharing one halfword ([`timers-dma-interrupts.md`](timers-dma-interrupts.md)).
- A register with no modeled write state ignores byte writes rather than
  acquiring invented storage.

Pinned by "implemented MMIO registers expose their individual byte lanes",
"MMIO byte writes preserve the other lane for video timers and DMA" and
"MMIO byte writes keep interrupt flag clearing lane-local".

## BIOS read protection

On hardware, a read from the BIOS region while the PC is outside the BIOS does
not return BIOS data. It returns the most recent opcode the CPU fetched from
the BIOS, a latch that changes as the BIOS boots, runs services and
dispatches interrupts ([GBATEK, Unpredictable Things: Reading from BIOS
Memory][gbatek-unpredictable]).

ZiGBA models this latch in HLE mode, where no BIOS bytes exist at all: every
read of `00000000h`–`00003FFFh` returns `Bus.bios_latch`, which the HLE BIOS
sets at the points where the retail BIOS would have fetched those opcodes.

| Moment                                      | Latch value |
| ------------------------------------------- | ----------- |
| after boot                                  | `E129F000h` |
| after an SWI returns                        | `E3A02004h` |
| during an IRQ handler                       | `E25EF004h` |
| after an IRQ returns                        | `E55EC002h` |
| after an HLE sound jump-list helper returns | `E12FFF1Eh` |

The jsmolka BIOS test ROM checks the latch around startup, `Sqrt` and the IRQ
trampoline and passes under HLE boot.

With an external BIOS image, the bus returns the image's bytes to every read,
including reads by code running outside the BIOS; the protection latch is not
applied on that path. The same jsmolka ROM therefore fails its first check
under external boot ([ZiGBA compatibility](../../../refs/zigba/docs/compatibility.md)).

## WAITCNT and Game Pak wait states

`WAITCNT` (`04000204h`) sets the SRAM wait states, first-access (N) and
second-access (S) wait states for each of the three Game Pak ROM windows, and
the prefetch buffer ([GBATEK, GBA System Control][gbatek-system]).

GBATEK's numbers are **waitstates**, not totals: every 16-bit Game Pak unit or
8-bit SRAM access costs one base cycle plus the programmed waitstates, as
GBATEK's `WAITCNT` notes state. Three sources agree once this is accounted
for: GBATEK's memory map lists a default
ROM access as 5/5/8 cycles; mGBA's waitstate tables hold waitstates and its
ARM core adds one; NanoBoyAdvance's tables hold totals. ZiGBA's tables
(`refresh_game_pak_timing`, `sram_cycles`) hold totals:

| Bits  | Field                  | Setting → total cycles per unit    |
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

At the power-on value `WAITCNT = 0`, a wait-state-0 ROM halfword costs 5
cycles non-sequential and 3 sequential, a word 8 and 6, and an SRAM access 5.
A 32-bit Game Pak access is two 16-bit units: the first priced N or S as
requested, the second always S. A unit that starts on a 128 KiB boundary is
non-sequential whatever the CPU requested; GBATEK's example is an `LDMIA`
from `0801FFF8h` whose beat at `08020000h` takes N timing. Pinned by
"default Game Pak, SRAM, and WRAM access costs match the GBATEK totals" and
"Game Pak timing follows every wait field and forces N at 128 KiB
boundaries".

EWRAM costs are fixed at 3/3/6 cycles.

## Sequential and non-sequential accesses

The bus decides whether the CPU's next code fetch is N or S from what the CPU
last did, rather than assuming instruction fetches are always sequential:

- A code fetch leaves the next fetch sequential.
- A CPU data access to memory (a load, a store, or a block-transfer beat)
  breaks the sequence: the next code fetch is non-sequential.
- A CPU data access to I/O leaves the next fetch sequential.
- A pipeline refill is an explicit N fetch at the target followed by an S
  fetch.
- Within a block transfer, the first beat is N and every later beat is S.

The pipeline's fetch-ahead honors this state. The rules are pinned by
hardware measurement rather than code reading: in the mGBA suite's load and
store rows, each result exceeds fetch plus data plus internal cycles by
exactly N − S, the cost of the broken sequence, while I/O data accesses show
no such excess.

## Game Pak prefetch buffer

The Game Pak has a prefetch buffer that, when enabled by `WAITCNT` bit 14,
keeps reading sequential ROM while the CPU is busy elsewhere, so later opcode
fetches can complete in one cycle
([GBATEK, GBA GamePak Prefetch][gbatek-prefetch]). It serves only opcodes
fetched from Game Pak ROM.
ZiGBA models it in `Bus.game_pak_prefetch` (`GamePakPrefetch`), cross-checked
against mGBA 0.10.5 and NanoBoyAdvance with GBATEK as the behavioral source:

- **Start.** A Game Pak opcode fetch the buffer cannot serve discards the
  buffer, pays its ordinary wait states, and restarts the buffer at the next
  unit: halfwords for Thumb code, words for ARM code.
- **Capacity.** Eight halfwords, or four words.
- **Fill.** Each unit takes its duty: the S cost of the window (twice that for
  an ARM word), or the N cost for a unit that starts on a 128 KiB boundary.
  The buffer fills during internal cycles and during accesses to other
  regions, and stops when full.
- **Hits.** An opcode fetch that finds its unit buffered takes one cycle. A
  fetch of the unit still in flight waits for the rest of that unit's duty.
- **Stops.** Any Game Pak data access (a ROM read by the CPU or DMA, SRAM,
  EEPROM, GPIO, or any Game Pak write) discards the buffer before paying its
  own wait states. It costs one extra cycle when a unit is one cycle from
  completing, or when an ARM word fill is at its halfway point.
- **Leaving ROM.** An opcode fetch from any other region discards the buffer.
- **`WAITCNT` changes.** The unit in flight keeps its duty; the next unit is
  priced with the new settings.
- **Disabled-prefetch quirk.** GBATEK's "prefetch disable bug": while the
  buffer is off, a Game Pak opcode with internal cycles that does not change
  r15 has its opcode fetch time changed from 1S to 1N. ZiGBA makes the first
  Game Pak opcode fetch after any internal cycle during Game Pak execution N.
  Turning the buffer off also forces the next Game Pak opcode fetch to N.

Pinned by the prefetch tests in
[ZiGBA's `bus_test.zig`](../../../refs/zigba/test/bus_test.zig),
among them "Thumb Game Pak prefetch fills during internal cycles and serves
buffered opcodes", "ARM Game Pak prefetch buffers complete words and
completes an in-flight word", "Game Pak data reads and writes cancel
prefetched opcodes", "WAITCNT changes preserve the in flight fetch and reprice
the next unit" and "disabling prefetch, or an internal cycle while disabled,
forces the next opcode fetch N". No reference source code was copied.

## mGBA suite timing tables

The [mGBA test suite](https://github.com/mgba-emu/suite) records hardware
cycle counts for short instruction sequences in `src/timing.c`. ZiGBA cannot
build the suite ROM on its development host (it needs the devkitARM
toolchain), so
[`tests/mgba_suite_timing.rs`](../../../systems/gba/tests/mgba_suite_timing.rs)
reproduces the suite's measurement instead. Each case builds a ROM of the
suite's shape (a branch, literal loads, an I/O store starting the timer, the
code under test, an I/O store ending it), runs it through `Machine`, and
subtracts a calibration run with no code, as the suite's `doResult` does. Only
the instruction sequences, re-encoded by hand, and the recorded hardware
numbers are taken from the suite.

ROM rows run under the suite's eight `WAITCNT` columns: `0000h`, `4000h`,
`0004h`, `4004h`, `0010h`, `4010h`, `0014h` and `4014h` (prefetch on or off,
WS0 N of 5 or 4, WS0 S of 3 or 2). EWRAM and IWRAM rows run the same code
from RAM through a ROM trampoline at `WAITCNT = 0`.

| Group               | Rows                                                                                                                        | What it pins                                                                                          |
| ------------------- | --------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| ARM basics          | `nop`, `nop / nop`, `b pc`, `ldrh`/`ldr`/`strh` via the stack, `ldrh`/`ldr` from ROM                                        | fetch costs, the broken-sequence rule, the internal load cycle, ROM data cancelling prefetch          |
| Trivial loops       | ARM and Thumb, 16 iterations, ROM, EWRAM, IWRAM                                                                             | steady state when every backward branch refills the pipeline and prefetch never serves the loop top   |
| Block transfers     | `ldmia`/`stmia` of 1, 2 and 6 registers, single and doubled                                                                 | first-beat N, later beats S, the trailing internal cycle                                              |
| OAM-to-ROM overflow | `ldmia` with writeback from `07FFFFFCh` down to `07FFFFECh`, ARM and Thumb                                                  | per-word region decode and writeback across the OAM/ROM boundary                                      |
| Multiplies          | `mul`, `mla`, `smull`, `smlal`, `umull`, `umlal`, Thumb `mul`, ten operands each                                            | signed and unsigned multiplier sizing, Rd = Rs aliasing, Thumb `mul` sized from Rd, accumulate cost   |
| Interworking        | ARM `bx` call to a Thumb stub and back                                                                                      | ARM/Thumb refill costs                                                                                |
| Immediate DMA3      | 16 transfer shapes (16/32-bit, IWRAM or ROM source and destination, 1 or 16 units) from ARM and Thumb, read through timer 0 | DMA start delay, Game Pak N/S across a transfer and for the CPU access it interrupts, completion cost |

The DMA rules the last group pins are described in
[`timers-dma-interrupts.md`](timers-dma-interrupts.md), along with
[`tests/mgba_suite_source.rs`](../../../systems/gba/tests/mgba_suite_source.rs),
which reproduces the suite's DMA register-protocol table.

Every row passes in every column except one DMA residual: the suite's
IWRAM-to-ROM rows read one cycle more than ZiGBA with ROM code, prefetch on
and `WAITCNT` `4000h` or `4004h`. The fixture asserts ZiGBA's value there so
that any change is visible.

## Open questions

- **External-BIOS read protection.** With a BIOS image, ZiGBA returns raw
  BIOS bytes to reads made from outside the BIOS; GBATEK's protection latch
  is modeled only in HLE mode.
- **Open-bus composition.** ZiGBA returns the last value read, and a Thumb
  fetch stores only its halfword. How the prefetched opcodes compose the
  open-bus word in Thumb state, per region (GBATEK, Unpredictable Things), is
  not modeled.
- **Video-memory contention.** GBATEK's memory map adds one cycle when the
  CPU accesses video memory while the LCD does; ZiGBA charges palette, VRAM
  and OAM accesses fixed cycles.
- **Affine parameter reads.** `BG2PA`–`BG3PD` read back their written values,
  although GBATEK lists them as write-only; the other write-only video
  registers read as zero.
- **EWRAM wait control.** The undocumented internal memory control register
  at `04000800h` is not modeled; writes to it are ignored and EWRAM always
  costs 3/3/6 cycles.
- **DMA timing residual.** The IWRAM-to-ROM offset above has no identified
  mechanism.
- **`nop ; b pc`.** The suite's `nop ; b pc` row reuses the `b pc` test
  function, which has no leading `nop`, so the real two-instruction shape is
  unmeasured and no oracle is pinned for it.
- **Prefetch collision cycle.** The extra cycle when data traffic stops a
  unit about to complete follows NanoBoyAdvance's model; it has not been
  isolated by a hardware measurement here.
- **Prefetch disable transition.** The forced N fetch after turning the
  buffer off is part of the cross-checked emulator model, not a GBATEK
  statement.

[gbatek-map]: https://rust-console.github.io/gbatek-gbaonly/#gbamemorymap
[gbatek-unpredictable]: https://rust-console.github.io/gbatek-gbaonly/#gbaunpredictablethings
[gbatek-system]: https://rust-console.github.io/gbatek-gbaonly/#gbasystemcontrol
[gbatek-prefetch]: https://rust-console.github.io/gbatek-gbaonly/#gbagamepakprefetch
[mgba-memory]: https://github.com/mgba-emu/mgba/blob/0.10.5/src/gba/memory.c
[nba-ppu]: https://github.com/nba-emu/NanoBoyAdvance/blob/55b5cf0ae3d929582ac5bfd486558173502b8354/src/nba/src/hw/ppu/ppu.hh#L59-L115
