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

## mGBA suite hardware tables

The pinned [mGBA suite](https://github.com/mgba-emu/suite) publishes recorded
hardware values in source tables. Evenfall reproduces its measurement shape
through public Rust seams; it does not build the suite ROMs or copy emulator
code. Values are transcribed from the suite, not generated by the core.

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
label has no distinct oracle. No result here claims that the suite's own ROM
build passes.

## Known divergences

**External BIOS protection latch.** `jsmolka-bios` test 1 expects Nintendo's
boot code to leave `0xE129F000` in the protected BIOS read latch. With an
external BIOS image, ZiGBA and the Rust port read that image's bytes instead;
the Cult-of-GBA replacement also leaves boot with different code. The
replacement-BIOS case fails at test 1 while every other jsmolka case passes.
Only a user-supplied Nintendo BIOS, together with an implementation of the
external-boot latch, could establish the retail expectation. HLE seeds the
post-boot value and passes the case.

## Limits and open questions

- Public mGBA-suite hardware tables are checked as host fixtures; its ROMs
  need devkitARM/libgba and are not built here.
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
