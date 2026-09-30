# Compatibility

Evenfall checks GBA behavior with headless, deterministic oracles, not a game
compatibility list. The Rust core ports ZiGBA, which remains the executable
parity oracle ([`methodology.md`](methodology.md)); hardware claims still
require independent evidence. Run instructions are in
[`testing.md`](testing.md). External test ROMs live at the pinned, gitignored
checkouts described in [`refs/README.md`](../refs/README.md); project probes
are built from [`probe/`](../probe/README.md). No commercial ROM or Nintendo
BIOS is bundled.

## ROM validators

[`systems/gba/tests/validator/main.rs`](../systems/gba/tests/validator/main.rs) runs
every available case and checks its reported registers and any source-derived
frame, audio, save or host-device oracle. Register and result-screen cases
require a stable terminal self-loop; still-image cases capture the completed
frame after boot. A guest fault or an incorrect output fails the case. HLE
boot passes the jsmolka and probe cases below. `--require-roms` turns missing
inputs into failures for CI; without it the runner prints the skipped case
and its missing ROM path.

### jsmolka/gba-tests

Every jsmolka ROM is checked against the SHA-256 recorded in
[`jsmolka_cases.rs`](../systems/gba/tests/validator/jsmolka_cases.rs) before execution.
Register cases read the failure number at a fixed evaluation address and
require a stable terminal self-loop; screen cases compare every pixel of the
completed frame against the suite's glyphs or an analytic pattern. The
external column uses the pinned Cult-of-GBA open replacement BIOS, not a
Nintendo BIOS.

| Case                    | Behavior checked                                                                     | Oracle          | HLE  | External     |
| ----------------------- | ------------------------------------------------------------------------------------ | --------------- | ---- | ------------ |
| `jsmolka-arm`           | ARM instructions, flags, modes, memory and branches                                  | register report | pass | pass         |
| `jsmolka-thumb`         | Thumb arithmetic, shifts, memory and control flow                                    | register report | pass | pass         |
| `jsmolka-memory`        | EWRAM, IWRAM, palette, VRAM, OAM, ROM mirrors and video-memory byte stores           | register report | pass | pass         |
| `jsmolka-bios`          | protected BIOS reads, `Sqrt` and IRQ trampoline                                      | register report | pass | fails test 1 |
| `jsmolka-save-none`     | reads without a save medium                                                          | register report | pass | pass         |
| `jsmolka-save-sram`     | SRAM mirroring and byte-wide save bus under wider accesses                           | register report | pass | pass         |
| `jsmolka-save-flash64`  | flash commands and erase/write sequences                                             | register report | pass | pass         |
| `jsmolka-save-flash128` | flash commands with bank switching                                                   | register report | pass | pass         |
| `jsmolka-unsafe`        | SRAM mirroring and unused Game Pak reads                                             | result screen   | pass | pass         |
| `jsmolka-nes`           | execution from VRAM, self-modifying code and decrementing `STM` observed through DMA | result screen   | pass | pass         |
| `jsmolka-ppu-hello`     | text from the suite's font                                                           | still image     | pass | pass         |
| `jsmolka-ppu-shades`    | tiled blue shades                                                                    | still image     | pass | pass         |
| `jsmolka-ppu-stripes`   | alternating two-color tiled columns                                                  | still image     | pass | pass         |

The suite's `unsafe` expectations are emulator-oriented: its upstream README
calls its results on flash-cartridge hardware inconclusive. The HLE results
therefore establish agreement with the pinned ROM, not a hardware claim.

### Project probes

The bare-metal probes boot through HLE, regardless of the validator's
`--bios` option. They stop at the fixed idle address `0x08000304` and publish
their result in `r12`; linked probes check every peer's `r12`. The harness
supplies keys, serial peers, a JoyBus master, a deterministic RTC epoch and
sensor inputs as specified by each case. Register oracles come from GBATEK,
probe source or arithmetic. Frame cases compare the whole completed frame;
split cases require exact rows away from line 40 and a genuine mid-line span
where a `VCOUNT` polling phase makes the exact dot unstable. Audio cases
inspect the complete stereo DAC stream, including level and run-length
patterns, rather than accepting a truncated queue. Every case below passes.
The source-derived expectations are in
[`probe_cases.rs`](../systems/gba/tests/validator/probe_cases.rs) and the probe intent
in [`probe/README.md`](../probe/README.md).

| Probe                | Behavior checked                                                                | Oracle                    |
| -------------------- | ------------------------------------------------------------------------------- | ------------------------- |
| `probe-timer`        | timer0 IRQs and cascaded timer1/timer2 counts bracketed by an independent event | register                  |
| `probe-display-irq`  | VBlank, HBlank and VCount-match IRQs                                            | register                  |
| `probe-keypad`       | selected key presses raise edge-triggered keypad IRQs                           | register                  |
| `probe-halt-stop`    | HALT timer wake, then STOP key wake                                             | register                  |
| `probe-dma`          | immediate DMA3 ROM→EWRAM checksum                                               | register                  |
| `probe-hdma`         | repeating HBlank DMA2 feeds background scroll                                   | frame                     |
| `probe-vdma`         | one-shot VBlank DMA3 uploads a tile even without a VBlank IRQ                   | frame                     |
| `probe-eeprom`       | DMA3 serial EEPROM detection, read-back and saved block                         | register, save type/bytes |
| `probe-rtc`          | RTC date/time from an injected epoch over GPIO                                  | register                  |
| `probe-rtc-write`    | RTC write and immediate read-back                                               | register                  |
| `probe-gyro`         | gyro ADC sampled over GPIO                                                      | register                  |
| `probe-solar`        | injected light level trips at the predicted pulse                               | register                  |
| `probe-tilt`         | two-axis tilt sample without a save medium                                      | register                  |
| `probe-rumble`       | GPIO motor state reaches the host                                               | register, device output   |
| `probe-link`         | normal two-player full-duplex SIO                                               | linked registers          |
| `probe-multi4`       | multiplayer exchange among four peers                                           | linked registers          |
| `probe-stop-sio`     | serial completion wakes a stopped peer with IME off                             | linked registers          |
| `probe-uart`         | UART transmit pacing and receive polling                                        | linked registers          |
| `probe-joybus`       | JoyBus slave handles a master WRITE                                             | register                  |
| `probe-joybustx`     | JoyBus slave READ response bytes and status                                     | register, master response |
| `probe-mb2slave`     | HLE MultiBoot to two verifying multiplayer clients                              | linked registers          |
| `probe-mb2mhz`       | HLE MultiBoot over Normal-32 at 2 MHz                                           | linked registers          |
| `probe-psg`          | square PSG duty and alternating sample runs                                     | samples                   |
| `probe-fifo`         | DMA1/timer0 FIFO stream and run lengths                                         | samples                   |
| `probe-noise`        | narrow then wide LFSR tap recurrence                                            | samples                   |
| `probe-wave`         | wave-channel nibble ramp                                                        | samples                   |
| `probe-wavechain`    | two-bank wave chaining                                                          | samples                   |
| `probe-wavereadback` | wave RAM shift-register read-back in both bank modes                            | register                  |
| `probe-sweep`        | upward channel-1 sweep shortens periods                                         | samples                   |
| `probe-sweepdown`    | downward channel-1 sweep lengthens periods                                      | samples                   |
| `probe-envelope`     | decreasing channel-2 envelope levels                                            | samples                   |
| `probe-obj`          | overlapping sprites and OBJ priority                                            | frame                     |
| `probe-objrot`       | affine rotated sprite                                                           | frame                     |
| `probe-objdouble`    | double-size affine sprite                                                       | frame                     |
| `probe-objflip`      | horizontal and vertical tile flips                                              | frame                     |
| `probe-objmosaic`    | mosaic alignment, clamping and right-edge extension                             | frame                     |
| `probe-objsemi`      | semi-transparent OBJ forces itself into the blend                               | frame                     |
| `probe-objwin`       | OBJ-window mask enables a background                                            | frame                     |
| `probe-affinerot`    | rotated affine background and transparent edge                                  | frame                     |
| `probe-affinewrap`   | affine background wraparound                                                    | frame                     |
| `probe-mode1`        | text over affine layer with transparent overflow                                | frame                     |
| `probe-blend`        | alpha blending                                                                  | frame                     |
| `probe-brighten`     | brightness increase                                                             | frame                     |
| `probe-darken`       | brightness decrease                                                             | frame                     |
| `probe-window`       | WIN0 rectangle edges                                                            | frame                     |
| `probe-mosaic`       | background mosaic with transparent colour zero                                  | frame                     |
| `probe-text8`        | 8bpp text map ignores palette-bank bits                                         | frame                     |
| `probe-scrollwrap`   | background scroll wraps a tile map                                              | frame                     |
| `probe-bitmap3`      | direct-colour Mode 3                                                            | frame                     |
| `probe-bitmap4`      | paletted Mode 4 page selection                                                  | frame                     |
| `probe-bitmap5`      | Mode 5 bounds and backdrop                                                      | frame                     |
| `probe-greenswap`    | green swap of adjacent pixels                                                   | frame                     |
| `probe-forceblank`   | forced blank produces white                                                     | frame                     |
| `probe-midcombo`     | mid-line scroll and palette writes under a window                               | split                     |
| `probe-midwin`       | changing WIN0 bounds mid-line                                                   | split                     |
| `probe-midbldy`      | changing brightness mid-line                                                    | split                     |
| `probe-midaffine`    | changing affine matrix mid-line                                                 | split                     |
| `probe-midmosaic`    | changing mosaic size mid-line                                                   | split                     |

## mGBA suite ROM

The pinned [mGBA suite](https://github.com/mgba-emu/suite) is one
menu-driven ROM whose test suites compare what the console does with tables
of results recorded on hardware.
[`tools/mgba-suite/build.sh`](../tools/mgba-suite/build.sh) builds it from
the pinned commit with a pinned devkitARM container image; the ROM stays a
gitignored build product. One parity scenario per suite and boot mode
(`tools/parity/scenarios/mgba-suite-*.json`) boots the ROM, selects the
suite, runs it and pages through its result list. `evenfall-parity
mgba-suite report` reads every page from the text tilemap the ROM draws and
recovers each test's pass or fail mark, the list a player sees on screen.
Each suite runs under HLE boot and under the pinned Cult-of-GBA replacement
BIOS. Commands are in
[`tools/mgba-suite/README.md`](../tools/mgba-suite/README.md).

ZiGBA and Evenfall produce byte-identical traces for every scenario: frames,
audio, SRAM and complete save states at every checkpoint, under three host
slicing schedules. Every result below therefore holds for both cores; a
failing test is a failed hardware comparison, not a parity difference.

The ROM draws only the first 29 characters of a test name. "Video tests",
the last menu entry, is interactive and records no results. In "SIO timing
tests" the multiplayer rows time out without a link partner, which the
suite's table expects; they are not counted as results. Tests not named below
pass.

| Suite                  | Fails under HLE                                                                                                                                                                                                                                       | Fails under Cult-of-GBA BIOS                                                                    | Causes                                                                                                                                                                                                      |
| ---------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Memory tests           | ROM, ROM out-of-bounds, IWRAM, EWRAM, palette, VRAM, VRAM OBJ and OAM loads and their mirrors; SRAM loads and stores and their mirrors; BIOS load                                                                                                     | SRAM loads and stores and their mirrors; BIOS load; BIOS out-of-bounds load; out-of-bounds load | [HLE `CpuSet` halfwords](#hle-cpuset-halfword-copies), [SRAM block transfers](#block-transfers-on-sram), [HLE `IntrWait` latch](#hle-intrwait-latch), [replacement BIOS](#hle-and-replacement-bios-effects) |
| I/O read tests         | `BG0CNT`, `BG1CNT`; scroll, affine, window, `MOSAIC` and `BLDY` registers; `DMA0`–`DMA3` `SAD`/`DAD`; `DMA0CNT_HI`–`DMA2CNT_HI`; `FIFO_A_HI`, `FIFO_B_HI`; the unused upper halves at `4Eh`, `56h`, `5Ah`, `5Eh`, `8Eh`, `AAh`, `AEh` and `E2h`–`FEh` | same                                                                                            | [write-only and unused I/O](#write-only-and-unused-io-reads), [Thumb open bus](#thumb-open-bus)                                                                                                             |
| Timing tests           | Calibration; BIOS Division, Division 2, Sqrt, Sqrt 2, Sqrt 3, ArcTan; CpuSet; Trivial and Short DMA (16 and 32/to ROM)                                                                                                                                | same, except BIOS ArcTan passes                                                                 | [timer and IRQ latency](#timer-and-irq-latency), [HLE and BIOS timing](#hle-and-replacement-bios-effects), [DMA residual](#known-residuals)                                                                 |
| Timer count-up tests   | every test                                                                                                                                                                                                                                            | every test                                                                                      | [timer and IRQ latency](#timer-and-irq-latency)                                                                                                                                                             |
| Timer IRQ tests        | every test                                                                                                                                                                                                                                            | every test                                                                                      | [timer and IRQ latency](#timer-and-irq-latency)                                                                                                                                                             |
| Shifter tests          | pass                                                                                                                                                                                                                                                  | pass                                                                                            |                                                                                                                                                                                                             |
| Carry tests            | pass                                                                                                                                                                                                                                                  | pass                                                                                            |                                                                                                                                                                                                             |
| Multiply long tests    | −1 × −1, `$7F` × −1, −1 × `$7F`, `$80` × `$7F`, `$81` × `$7F`, and every product with `$80` or `$81` as second operand                                                                                                                                | same                                                                                            | [long-multiply carry](#long-multiply-carry)                                                                                                                                                                 |
| BIOS math tests        | pass                                                                                                                                                                                                                                                  | `ArcTan2` with a zero first operand and a nonzero second; `Div` by zero                         | [replacement BIOS](#hle-and-replacement-bios-effects)                                                                                                                                                       |
| DMA tests              | pass                                                                                                                                                                                                                                                  | pass                                                                                            |                                                                                                                                                                                                             |
| SIO register R/W tests | pass                                                                                                                                                                                                                                                  | pass                                                                                            |                                                                                                                                                                                                             |
| SIO timing tests       | Normal8/256k, Normal8/2M, Normal32/256k, Normal32/2M                                                                                                                                                                                                  | same                                                                                            | [timer and IRQ latency](#timer-and-irq-latency)                                                                                                                                                             |
| Misc. edge case tests  | DMA Prefetch, H-blank bit start, DMA count latching                                                                                                                                                                                                   | same                                                                                            | [Thumb open bus](#thumb-open-bus), [HBlank flag](#hblank-flag), [DMA count latch](#dma-count-latch)                                                                                                         |

### Deviations from the recorded hardware

Each of these is a core behavior that the suite's hardware table
contradicts, inherited unchanged from ZiGBA. Candidate fixes and what they
must preserve are in [`roadmap.md`](roadmap.md#mgba-suite-rom-findings).

#### Write-only and unused I/O reads

The I/O read suite (`src/io-read.c`) reads each register with `LDRH` from
Thumb ROM code whose next words are `DEADDEADh`, so open bus reads `DEADh`.
On hardware, write-only registers read as open bus when the other half of
their word is also unreadable: the BG scroll offsets, affine parameters and
reference points, window bounds, `MOSAIC` and `BLDY` with the unused
halfwords beside them, and the DMA addresses. The core returns zero for
scroll offsets,
reference points and DMA addresses, and the written value for affine
parameters, window bounds, `MOSAIC` and `BLDY`
([`memory.md`](gba/hardware/memory.md#open-bus-and-unused-io)). The table
also records that bit 13 of `BG0CNT` and `BG1CNT` and bit 11 of
`DMA0CNT_H`–`DMA2CNT_H` read as zero; the core reads them back.

#### Thumb open bus

In Thumb state the core's open-bus word carries the fetched halfword in its
low half and zero above it. The hardware tables show the halfword in both
halves when Thumb code runs from ROM. In words the core already treats as
unused, the low halves read `DEADh` and pass, while the upper halves (`5Ah`,
`5Eh`, `8Eh`, `AAh`, `AEh`, `E2h`–`FEh`, `FIFO_A_HI`, `FIFO_B_HI`) read zero
instead of `DEADh`; the upper halves of write-only words need this fix as
well as the previous one. "DMA Prefetch" reads upward from `10000000h` and
expects every value to hold a Thumb opcode from `4000h`–`403Fh` in each half
until an HBlank DMA's last word, `DEAD0000h`,
appears on the bus at `10002A94h`; the core stops at the first read,
`00004003h`. This is the [open-bus composition](gba/hardware/memory.md#open-questions)
question, now answered for ROM code. The DMA part of that test, a CPU read
returning the last DMA value, is not reached.

#### HBlank flag

"H-blank bit start" wakes on the HBlank IRQ, then times the `DISPSTAT`
HBlank flag's edges with timer 0. On hardware the flag is clear for about
1,004 cycles and set for about 228 in each 1,232-cycle line, so it rises
about 46 cycles after the IRQ. The core raises flag, IRQ and HBlank DMA
together at cycle 960, as the [video page](gba/hardware/video.md#open-questions)
describes. The interval between two HBlank wakes also reads 3 cycles longer
than the recorded 1,232; see [unexplained results](#unexplained-results).

#### DMA count latch

"DMA count latching" enables a repeating VBlank DMA3 with a count of 2, then
rewrites `DMA3CNT_H` with the enable bit still set and `DMA3CNT_L` with 4.
On hardware the first transfer copies 2 units and the repeat 4: the count is
latched when the enable bit rises, like the addresses, and reloaded from
`DMA3CNT_L` on repeat. The core reads `DMAxCNT_L` whenever a transfer
starts, so the first transfer copies 4.

#### Long-multiply carry

The multiply-long results and N and Z flags all match. The failing checks
differ only in the C flag after `SMULLS` and `UMULLS`: the hardware table
sets C for these operand pairs, while the core leaves C unchanged. On the
ARM7TDMI the carry comes out of the multiplier's Booth stages, so it depends
on both operands.

#### Block transfers on SRAM

`CpuSet` and `CpuFastSet` word copies from and to SRAM at a misaligned
address read the byte at that exact address on hardware, repeated in all
four lanes, and store accordingly; the memory table records this for every
misalignment. The core's own `LDR` and `STR` rows at the same addresses
pass. The copies fail under both boot modes: HLE aligns the addresses before
copying, and the replacement BIOS copies words with `LDMIA`/`STMIA`
(`bios_calls/cpu_set.s`), whose addresses the core aligns before they reach
the bus. The hardware result implies that the 8-bit SRAM bus sees a
misaligned block-transfer address, assuming the retail routines also use
block transfers.

#### HLE `CpuSet` halfword copies

A halfword `CpuSet` from an odd source address stores, on hardware, the low
half of `LDRH`'s rotated result: `00BEh` and `00DEh` from `DEADBEEFh`. The
HLE service aligns the source first and copies the aligned halfwords. The
replacement BIOS, which uses `LDRH` on the odd address, passes these checks.

#### HLE `IntrWait` latch

"BIOS load" calls `VBlankIntrWait` with `IME` clear, which the BIOS sets
itself, then reads the BIOS region. Hardware returns `E3A02004h`, the latch
after an SWI returns. HLE leaves `E55EC002h`, the latch after an IRQ returns,
because the wait's completing IRQ return is the last latch update.

#### Timer and IRQ latency

The timer IRQ suite enables timer 0 near overflow with its IRQ on, then
reads the counter after zero to nine `NOP`s. On hardware the counter keeps
stepping for several of those reads after it overflows before the IRQ handler
stops it. In the core the IRQ is taken before the first read under both boot
modes, and with reload `FFFDh` the first read is already one count ahead.
The timing suite's Calibration row, the raw timer count across the
suite's `START` and `END` macros with nothing between them, agrees: with
prefetch on and in both RAMs the core reads one more than the table, in
ARM and Thumb. The prefetch-off ROM columns read zero to two less; the
other timing rows subtract this calibration, so they are unaffected.

The timer count-up and SIO timing suites run through BIOS IRQ and `Halt`
paths and inherit this latency along with the BIOS's own costs. Their
Normal-mode SIO transfer lengths match the table exactly; every completion
reads 54 cycles early under HLE and 9 under the replacement BIOS.

### HLE and replacement-BIOS effects

- **HLE service timing.** The timing suite's BIOS Division, Sqrt, ArcTan and
  CpuSet rows time retail BIOS code. HLE charges its own fixed costs
  ([`bios.md`](gba/hardware/bios.md#approximations)), so these rows fail; the
  replacement BIOS's different code fails them too, except ArcTan.
- **HLE IRQ dispatch** does not charge the retail trampoline's instructions,
  so IRQ-timed suites read earlier under HLE than under the replacement BIOS.
- **The replacement BIOS** returns different `ArcTan2` and division-by-zero
  results, copies sources below `02000000h` that the retail `CpuSet` refuses
  (its source marks that check as a TODO), and leaves its boot screen's
  sprites in OAM, which DMA0 reads through its 27-bit source mask in "SRAM
  mirror load". The BIOS-region loads fail as the
  [external BIOS protection latch](#known-divergences) describes.

### Known residuals

The four "to ROM" DMA timing rows fail only with ROM code, prefetch on and
`WAITCNT` `4000h` or `4004h`, one cycle below the table: the residual the
[hardware-table fixture](#mgba-suite-hardware-tables) already pins.

### Unexplained results

- **Prefetch-off calibration.** With prefetch off, the Calibration row's ROM
  columns read zero to two cycles below the table, while every other column
  reads one above; no single timer-start delay fits both.
- **HBlank IRQ interval.** Two consecutive `Halt` wakes on the HBlank IRQ
  are 1,235 cycles apart instead of the recorded 1,232, under both boot
  modes.

## mGBA suite hardware tables

The pinned [mGBA suite](https://github.com/mgba-emu/suite) publishes recorded
hardware values in source tables. Evenfall reproduces its measurement shape
through public Rust seams, independently of the [ROM build](#mgba-suite-rom),
and copies no emulator code. Values are transcribed from the suite, not
generated by the core.

| Fixture                                                             | Suite source                       | Checks                                                                                                  |
| ------------------------------------------------------------------- | ---------------------------------- | ------------------------------------------------------------------------------------------------------- |
| [`mgba_suite_timing.rs`](../systems/gba/tests/mgba_suite_timing.rs) | `src/timing.c` and DMA timing rows | ARM/Thumb fetch, branch, load/store, blocks, multiplies, BX and DMA3 timer results across ROM/RAM waits |
| [`mgba_suite_source.rs`](../systems/gba/tests/mgba_suite_source.rs) | `src/dma.c`                        | immediate DMA source masks, SRAM aliasing, stepping and shared latch                                    |

The timing fixture subtracts the suite-style empty-body calibration. Its
DMA trigger oracle intentionally pins ZiGBA's one-cycle-low result for
IWRAM-to-ROM transfers executed from ROM with prefetch enabled and `WAITCNT`
`0x4000` or `0x4004`; the recorded hardware table reads one cycle higher.
This is an open timing residual, not a passing claim for those hardware
columns. The suite's “nop ; b pc” row reuses the branch test's code, so that
label has no distinct oracle. The suite ROM's own results, including these
rows, are in [mGBA suite ROM](#mgba-suite-rom).

## Known divergences

**External BIOS protection latch.** `jsmolka-bios` checks the protected BIOS
read latch against Nintendo's BIOS: test 1 expects `0xE129F000`, the opcode
at `0xE4` that Nintendo's boot code fetches as it branches to the cartridge.
With an external image the core models the protection
([memory](gba/hardware/memory.md#bios-read-protection)), so the ROM reads the
opcode the replacement BIOS fetched last. The pinned Cult-of-GBA image
leaves boot with `BX lr` at `0x94`, and test 1 reads `0x01000020`, the word
at `0x9C`, so the replacement-BIOS case fails at test 1 as the image
dictates; its later checks cannot run. Of the ROM's four values, the image's
IRQ trampoline would reproduce the last two (`0xE25EF004`, `0xE55EC002`);
its SWI return leaves `0xEAFFFFFE` where Nintendo's leaves `0xE3A02004`.
Only a user-supplied Nintendo BIOS can check the retail values under
external boot. HLE seeds the post-boot value and passes the case.

## Guest faults in user-supplied games

A guest that executes an UNPREDICTABLE encoding stops with a typed fault
([containment](gba/hardware/cpu.md#undefined-versus-unpredictable)). Such a
fault in a game is a core bug only if the path to it departs from hardware.

**Game B's battle information menu.** Game B, a ROM hack of the
[rewind study](research/rewind-compression.md#codec-choice), faults after 82
emulated minutes of the study's seeded button walker with
`unpredictable_arm_encoding class=single_transfer offending_registers=8000`:
in IRQ mode and ARM state, the CPU reaches `E7FAFBCDh`,
`LDRB pc, [r10, sp, asr #23]!`, in IWRAM. The game causes the fault:

1. The move page of the game's battle information menu, in its description
   mode, prints all four move slots of the battler. For an empty slot, move 0,
   it copies the string at `descriptions[move − 1]`, the word before the
   pointer table: `0000FFADh`.
2. The string copy stops at an `FFh` byte. It reads upward from `0000FFADh`,
   unmapped memory that returns open bus, and writes from a text buffer
   onward through the EWRAM mirrors, overwriting all of EWRAM more than
   twice before a byte reads `FFh`.
3. The overwritten data include the sprite system's OAM entry limit, which
   becomes 209. A later OAM-buffer build fills the unused entries up to that
   limit, past the 128-entry shadow OAM in IWRAM and into the ARM interrupt
   dispatcher the game copies there, which the IRQ vector at `03007FFCh`
   names.
4. The next interrupt enters the dispatcher and executes the fill words,
   then the Thumb code after them, as ARM until it reaches the byte load
   into r15.

ZiGBA at the pinned commit faults at the same cycle from the same converted
state and input, with a byte-identical state at earlier frames, and stepping
the interpreter without the execution engine faults at the same cycle too.

Open bus decides only when the copy ends. Between DMA transfers, a Thumb read
of unmapped memory from Game Pak code returns the halfword the CPU last
prefetched, here the loop's `D1F9h`: repeated in both halves per GBATEK,
mGBA and NanoBoyAdvance, zero-extended in ZiGBA. Neither holds `FFh`. The
copy ends at a read that follows a Direct Sound FIFO DMA, whose word the core
returns as open bus; mGBA and NanoBoyAdvance also return the last DMA value
to a CPU read that follows a transfer. The copy's length on hardware thus
depends on sample data and DMA timing, but step 1 corrupts memory under
every model. mGBA and NanoBoyAdvance execute `LDRB` into r15 as a byte load
and a branch; ARMv4 leaves it UNPREDICTABLE, and the guest is already
running data as code.

## Limits and open questions

- The mGBA suite ROM runs under HLE and the replacement BIOS only; its
  retail-BIOS results, and every result that depends on retail BIOS timing,
  need a user-supplied Nintendo BIOS.
- No Nintendo BIOS, commercial game, audio capture or console framebuffer
  capture is part of this gate. Agreement with a comparison emulator does
  not establish hardware correctness.
- The mid-scanline probes assert structure, not an exact first changed pixel.
  The precise dot of a register write remains unmeasured on hardware.
- The noise LFSR reset state is disputed; `probe-noise` checks the tap
  recurrence after locating it in the output rather than assuming a reset
  phase. Keypad level-versus-edge behavior and HBlank DMA during VBlank also
  remain hardware questions.
- Commercial and homebrew title compatibility beyond these suites is not
  audited. User-supplied ROM checks are regression evidence for that input,
  not per-title compatibility claims.

The parity-trace workflow and its limitations live in
[`tools/parity/README.md`](../tools/parity/README.md).
