# Compatibility

This page says what Evenfall is known to get right, and how that is known.
Evenfall doesn't keep a list of games that work.
Game lists are hard to trust: "it boots and looks fine" says little about timing, sound or saves, and nobody can re-run someone else's play session.
Instead, compatibility is measured with test ROMs and recorded hardware results that run headless, check exact values, and give the same answer every time.

Four kinds of evidence are used:

- **[jsmolka/gba-tests](#jsmolkagba-tests)**, a public suite of test ROMs for GBA emulators.
- **[Project probes](#project-probes)**, small bare-metal ROMs written for Evenfall, each checking one behavior against a value worked out from GBATEK, the probe's source or arithmetic.
- **[mGBA suite hardware tables](#mgba-suite-hardware-tables)**, timing numbers measured on a real console by the mGBA test suite, checked as ordinary tests.
- **[The mGBA suite ROM](#mgba-suite-rom)**, the same suite's own test ROM, built from source and run on the core.

Everything here passes, except what the [mGBA suite ROM](#mgba-suite-rom) section and [Known differences](#known-differences) list.
[`testing.md`](testing.md) explains how to run it all.
The external test ROMs are pinned checkouts described in [`refs/README.md`](../refs/README.md), and the probes are built from [`probe/`](../probe/README.md).
No commercial ROM or Nintendo BIOS is involved.

## How a test ROM is judged

The [validator](../systems/gba/tests/validator/main.rs) runs every available case and checks the registers it reports, and any expected frame, audio, save or device output.
A case fails on a guest fault or any wrong output.

- **Register cases** must end in a stable idle loop and report success in a register.
- **Table cases** also leave a table of values in EWRAM, which must match the expected table entry for entry.
- **Result-screen cases** compare every pixel of the finished screen with the suite's font or an expected pattern.
- **Still-image cases** capture the first completed frame after boot.
- **Audio cases** check the complete stereo output stream, including levels and run lengths.
- **Split cases** check a register write in the middle of a line.
  The exact dot of the write moves with CPU timing, so they require a genuine split on the target line and exact pixels everywhere else.

## jsmolka/gba-tests

Every jsmolka ROM is checked against the SHA-256 recorded in [`jsmolka_cases.rs`](../systems/gba/tests/validator/jsmolka_cases.rs) before it runs.
The "real BIOS" column uses the open Cult-of-GBA replacement BIOS, not Nintendo's.

| Case                    | What it checks                                                                    | Result        | HLE  | Real BIOS    |
| ----------------------- | --------------------------------------------------------------------------------- | ------------- | ---- | ------------ |
| `jsmolka-arm`           | ARM instructions, flags, modes, memory and branches                               | register      | pass | pass         |
| `jsmolka-thumb`         | Thumb arithmetic, shifts, memory and control flow                                 | register      | pass | pass         |
| `jsmolka-memory`        | EWRAM, IWRAM, palette, VRAM, OAM, ROM mirrors and byte writes to video memory     | register      | pass | pass         |
| `jsmolka-bios`          | protected BIOS reads, `Sqrt` and the interrupt handler                            | register      | pass | fails test 1 |
| `jsmolka-save-none`     | reads with no save chip                                                           | register      | pass | pass         |
| `jsmolka-save-sram`     | SRAM mirroring and the 8-bit save bus under wider accesses                        | register      | pass | pass         |
| `jsmolka-save-flash64`  | Flash commands, erasing and writing                                               | register      | pass | pass         |
| `jsmolka-save-flash128` | Flash commands with bank switching                                                | register      | pass | pass         |
| `jsmolka-unsafe`        | SRAM mirroring and reads past the end of the ROM                                  | result screen | pass | pass         |
| `jsmolka-nes`           | running code from VRAM, self-modifying code, and a decrementing `STM` seen by DMA | result screen | pass | pass         |
| `jsmolka-ppu-hello`     | text in the suite's font                                                          | still image   | pass | pass         |
| `jsmolka-ppu-shades`    | tiled blue shades                                                                 | still image   | pass | pass         |
| `jsmolka-ppu-stripes`   | alternating two-color tiled columns                                               | still image   | pass | pass         |

The suite's own README calls the `unsafe` results on flash-cartridge hardware inconclusive, so passing it shows agreement with the ROM's expectations, not with hardware.

## Project probes

The probes boot with Evenfall's own BIOS, whatever the validator's `--bios` option says.
Each stops at the idle address `0x08000304` with its result in `r12`, and linked probes check every console's `r12`.
The validator supplies the keys, link-cable partners, a JoyBus master, a fixed clock time and sensor values each case needs.
Expected values are in [`probe_cases.rs`](../systems/gba/tests/validator/probe_cases.rs), and what each probe is for is in [`probe/README.md`](../probe/README.md).
Every probe passes.

| Probe                     | What it checks                                                                   | Result                    |
| ------------------------- | -------------------------------------------------------------------------------- | ------------------------- |
| `probe-timer`             | timer 0 interrupts, and chained timers 1 and 2 bracketed by an independent event | register                  |
| `probe-timer-start`       | the counter holding its reload through the cycle after the enabling write        | table                     |
| `probe-timer-irq-latency` | how many cycles after a timer overflow the CPU takes its interrupt               | table                     |
| `probe-display-irq`       | VBlank, HBlank and VCOUNT-match interrupts                                       | register                  |
| `probe-keypad`            | chosen key presses raise keypad interrupts, on the press only                    | register                  |
| `probe-halt-stop`         | a timer waking HALT, then a key waking STOP                                      | register                  |
| `probe-dma`               | an immediate DMA3 copy from ROM to EWRAM                                         | register                  |
| `probe-hdma`              | repeating HBlank DMA2 feeding a background's scroll                              | frame                     |
| `probe-vdma`              | a one-shot VBlank DMA3 uploading a tile, with no VBlank interrupt                | frame                     |
| `probe-eeprom`            | EEPROM detection, reading back, and the saved block                              | register, save type/bytes |
| `probe-rtc`               | the clock's date and time from a given starting time                             | register                  |
| `probe-rtc-write`         | writing the clock and reading it back                                            | register                  |
| `probe-gyro`              | the gyro value read over GPIO                                                    | register                  |
| `probe-solar`             | a given light level tripping at the predicted pulse                              | register                  |
| `probe-tilt`              | a two-axis tilt reading with no save chip                                        | register                  |
| `probe-rumble`            | the motor state reaching the host                                                | register, device output   |
| `probe-link`              | a two-player Normal exchange                                                     | linked registers          |
| `probe-multi4`            | a Multiplayer exchange among four consoles                                       | linked registers          |
| `probe-stop-sio`          | a finished transfer waking a console in STOP with interrupts off                 | linked registers          |
| `probe-uart`              | UART sending pace and polled receiving                                           | linked registers          |
| `probe-joybus`            | answering a JoyBus WRITE                                                         | register                  |
| `probe-joybustx`          | the bytes and status of a JoyBus READ answer                                     | register, master response |
| `probe-mb2slave`          | HLE MultiBoot to two checking Multiplayer clients                                | linked registers          |
| `probe-mb2mhz`            | HLE MultiBoot over Normal 32-bit at 2 MHz                                        | linked registers          |
| `probe-psg`               | a square wave's duty and sample runs                                             | samples                   |
| `probe-fifo`              | a sample stream through FIFO A with DMA1 on timer 0                              | samples                   |
| `probe-noise`             | the noise register's recurrence in both widths                                   | samples                   |
| `probe-wave`              | a ramp on the wave channel                                                       | samples                   |
| `probe-wavechain`         | playing both wave banks in a row                                                 | samples                   |
| `probe-wavereadback`      | reading wave RAM back after playing, in both bank modes                          | register                  |
| `probe-sweep`             | an upward sweep shortening the period                                            | samples                   |
| `probe-sweepdown`         | a downward sweep lengthening the period                                          | samples                   |
| `probe-envelope`          | a falling envelope's levels                                                      | samples                   |
| `probe-obj`               | overlapping sprites and sprite priority                                          | frame                     |
| `probe-objrot`            | a rotated sprite                                                                 | frame                     |
| `probe-objdouble`         | a double-size rotated sprite                                                     | frame                     |
| `probe-objflip`           | horizontal and vertical flips                                                    | frame                     |
| `probe-objmosaic`         | sprite mosaic alignment, clamping and right edge                                 | frame                     |
| `probe-objsemi`           | a semi-transparent sprite blending                                               | frame                     |
| `probe-objwin`            | a sprite window revealing a background                                           | frame                     |
| `probe-affinerot`         | a rotated background and its transparent edge                                    | frame                     |
| `probe-affinewrap`        | a wrapping rotated background                                                    | frame                     |
| `probe-mode1`             | text over a rotated layer with a transparent edge                                | frame                     |
| `probe-blend`             | alpha blending                                                                   | frame                     |
| `probe-brighten`          | brightening                                                                      | frame                     |
| `probe-darken`            | darkening                                                                        | frame                     |
| `probe-window`            | window 0's edges                                                                 | frame                     |
| `probe-mosaic`            | background mosaic with transparent color 0                                       | frame                     |
| `probe-text8`             | a 256-color text map ignoring palette-bank bits                                  | frame                     |
| `probe-scrollwrap`        | a scrolled background wrapping around its map                                    | frame                     |
| `probe-bitmap3`           | direct-color mode 3                                                              | frame                     |
| `probe-bitmap4`           | paletted mode 4 and page selection                                               | frame                     |
| `probe-bitmap5`           | mode 5's bounds and backdrop                                                     | frame                     |
| `probe-greenswap`         | green swap between neighboring pixels                                            | frame                     |
| `probe-forceblank`        | forced blank drawing white                                                       | frame                     |
| `probe-midcombo`          | scroll and palette writes mid-line under a window                                | split                     |
| `probe-midwin`            | window 0's edges changed mid-line                                                | split                     |
| `probe-midbldy`           | brightness changed mid-line                                                      | split                     |
| `probe-midaffine`         | a rotation matrix changed mid-line                                               | split                     |
| `probe-midmosaic`         | mosaic size changed mid-line                                                     | split                     |

## mGBA suite hardware tables

The [mGBA test suite](https://github.com/mgba-emu/suite) publishes numbers measured on real hardware in its source.
Evenfall reproduces the suite's measurements through its own Rust API, without building the suite's ROMs or copying emulator code.
The expected numbers are copied from the suite, never generated by the core.

| Test                                                                | Suite source                       | What it checks                                                                                         |
| ------------------------------------------------------------------- | ---------------------------------- | ------------------------------------------------------------------------------------------------------ |
| [`mgba_suite_timing.rs`](../systems/gba/tests/mgba_suite_timing.rs) | `src/timing.c` and DMA timing rows | instruction fetch, branch, load and store, block, multiply, `BX` and DMA3 timing at every wait setting |
| [`mgba_suite_source.rs`](../systems/gba/tests/mgba_suite_source.rs) | `src/dma.c`                        | DMA source masks, SRAM aliasing, stepping and the shared latch                                         |

The timing test subtracts an empty calibration run, as the suite does.
Every value matches.
The suite's `nop ; b pc` row reuses the `b pc` code and so has no value of its own.

## mGBA suite ROM

The mGBA suite's ROM runs a menu of test suites and compares what the console does with tables of results recorded on hardware.
[`tools/mgba-suite/build.sh`](../tools/mgba-suite/build.sh) builds it from the pinned commit in a pinned devkitARM container image, and the ROM stays a gitignored build product.
One scenario per suite and boot mode boots the ROM, selects the suite, runs it and pages through its result list, saving a state on every page.
`evenfall-parity mgba-suite report` reads each page from the text tilemap in those states, the list a player sees on screen.
Each suite runs under HLE boot and under the pinned Cult-of-GBA replacement BIOS, and gives the same results under three host slicing schedules.
Commands are in [`tools/mgba-suite/README.md`](../tools/mgba-suite/README.md).

The ROM draws only the first 29 characters of a test name.
"Video tests", the last menu entry, is interactive and records no results.
The four multiplayer rows of "SIO timing tests" check nothing without a link partner.

| Suite                  | Fails under HLE                  | Fails under the Cult-of-GBA BIOS                                         |
| ---------------------- | -------------------------------- | ------------------------------------------------------------------------ |
| Memory tests           | none                             | SRAM mirror load, BIOS load, BIOS out-of-bounds load, Out-of-bounds load |
| I/O read tests         | none                             | none                                                                     |
| Timing tests           | BIOS Sqrt 2, BIOS Sqrt 3, CpuSet | BIOS Division, Division 2, Sqrt, Sqrt 2, Sqrt 3, CpuSet                  |
| Timer count-up tests   | none                             | the twelve prescaled rows                                                |
| Timer IRQ tests        | none                             | none                                                                     |
| Shifter tests          | none                             | none                                                                     |
| Carry tests            | none                             | none                                                                     |
| Multiply long tests    | none                             | none                                                                     |
| BIOS math tests        | none                             | `ArcTan2` of zero and a nonzero value, `Div` by zero                     |
| DMA tests              | none                             | none                                                                     |
| SIO register R/W tests | none                             | none                                                                     |
| SIO timing tests       | none                             | Normal8/256k, Normal8/2M, Normal32/256k, Normal32/2M                     |
| Misc. edge case tests  | DMA Prefetch, H-blank bit start  | DMA Prefetch, H-blank bit start                                          |

### BIOS routine costs under HLE

The HLE BIOS charges the retail call's accesses and fitted costs where the suite's tables fix them ([`bios.md`](gba/hardware/bios.md#approximations)).
Three timing rows still time routine bodies that HLE doesn't model cycle by cycle.
"BIOS Sqrt 2" (`Sqrt` of `FFh`) reads 115 cycles more on hardware, "BIOS Sqrt 3" (`Sqrt` of `12345678h`) 1031 more, and "CpuSet", which calls `CpuFastSet` for 256 words, 272 more.
Each difference is the same in all twenty columns, ARM and Thumb from every memory and wait setting, so it lies inside the routine, not in the call.

### DMA Prefetch

"DMA Prefetch" reads upward from `10000000h` with `LDMIA` from Thumb ROM code while a repeating HBlank DMA3 copies `DEAD0000h` within IWRAM.
Every read returns the opcode the CPU prefetched, `4003h` in both halves, until one read follows an HBlank DMA and returns the DMA's word.
The core returns both values as the table does.
On hardware the DMA's word appears at the read of `10002A94h`.
The core reaches it at `10002A90h`, one word early, under the replacement BIOS, and at `1000264Ch` under HLE.
Which read it is depends on the loop's cycles per read, the cycles each DMA takes from the CPU, and where in the line the loop starts, which the boot path before the test moves.

### H-blank bit start

"H-blank bit start" wakes from `Halt` on the HBlank interrupt, then times the `DISPSTAT` HBlank flag's edges with timer 0.
The flag rises at cycle 1006 of each line ([`video.md`](gba/hardware/video.md#display-timing)), and under HLE the fourth to sixth flip intervals match the table.
The other checks fail for reasons outside the flag:

- The first flip interval is timed from the timer read after the second `Halt` wake, so it includes the BIOS path from the wake to that read.
  It reads 41 cycles more than the table in both boot modes, and the second and third flips inherit its polling phase.
  Under the replacement BIOS the fourth and sixth flips fail too.
- The interval between the two `Halt` wakes reads 1,235 cycles instead of 1,232.
  In this build of the suite, the compiler schedules three Thumb `MOVS` between the second `Halt` and its timer read that don't follow the first.
  They are three IWRAM cycles that any emulator, and hardware running this build, would count, so the recorded table came from a different build.

### Under the replacement BIOS

The Cult-of-GBA BIOS is different code from Nintendo's, so every row that times BIOS code measures that code:

- **Timing, timer count-up and SIO timing.** Its `SWI` routines, `IntrWait` and `Halt` take their own cycles.
  The prescaled count-up rows read one to sixteen counts more than the table, and the Normal-mode SIO transfers complete 5 cycles early.
- **BIOS math.** It returns different `ArcTan2` results for a zero first operand and different division-by-zero results.
- **Memory.** It leaves its boot screen's sprites in OAM, which DMA0 reads in "SRAM mirror load" through its 27-bit source mask.
  The BIOS loads read the opcodes its own code leaves in the protection latch ([Known differences](#known-differences)).

## Known differences

- **BIOS protection with a replacement BIOS.** `jsmolka-bios` checks the protected BIOS read latch against Nintendo's BIOS.
  Test 1 expects `0xE129F000`, the opcode at `0xE4` that Nintendo's boot code fetches as it branches to the cartridge.
  With a BIOS image, Evenfall models the protection ([`memory.md`](gba/hardware/memory.md#bios-read-protection)), so the ROM reads the opcode the replacement BIOS fetched last.
  The pinned Cult-of-GBA image leaves boot with `BX lr` at `0x94`, and test 1 reads `0x01000020`, the word at `0x9C`.
  So this case fails at test 1 with that BIOS, as the image dictates, while every other jsmolka case passes.
  Of the ROM's four values, the image's interrupt trampoline would reproduce the last two, `0xE25EF004` and `0xE55EC002`.
  Its `SWI` return leaves `0xEAFFFFFE` where Nintendo's leaves `0xE3A02004`.
  Only a user-supplied Nintendo BIOS can check the retail values with a real image.
  Evenfall's own BIOS sets the expected values and passes.

## Known limits

- The mGBA suite ROM runs under HLE and the replacement BIOS only.
  Its results with Nintendo's BIOS, and every result that depends on that BIOS's timing, need a user-supplied image.
- No Nintendo BIOS, commercial game, audio recording or screen capture from a real console is part of these checks.
  Agreeing with another emulator doesn't prove hardware accuracy.
- The mid-line probes check the shape of a split, not its exact first pixel.
  Where exactly a mid-line write lands has never been measured on hardware.
- The noise register's starting value is disputed, so `probe-noise` finds the recurrence in the output instead of assuming a start.
  Whether the keypad interrupt is edge- or level-triggered, and whether HBlank DMA runs during VBlank, are also unmeasured.
- Commercial and homebrew games beyond these suites haven't been audited.
  Checks run on your own ROMs are regression evidence for that ROM, not a compatibility claim for the title.
