# Probe ROMs

Small bare-metal GBA ROMs that each check one piece of hardware behavior end to end, from real GBA code.
They are built with `arm-none-eabi-gcc` for ARMv4T, without libgba, and cover what the public test suites in [`refs/`](../refs/README.md) don't: timers and interrupts, cartridge devices, link cables, sound and rendering.
Each probe's expected result is in [`probe_cases.rs`](../systems/gba/tests/validator/probe_cases.rs), and [`compatibility.md`](../docs/compatibility.md#project-probes) lists them all.
Built ROMs (`probe/build/*.gba`) are kept out of Git.

## Building and running

Install an ARM bare-metal GCC first: `gcc-arm-none-eabi` on Debian and Ubuntu, `arm-none-eabi-gcc` on Arch, or Arm's GNU toolchain release on any OS.
The Makefile also needs `python3` to patch the header complement.

```sh
make -C probe                      # -> probe/build/*.gba
cargo test -p evenfall-gba --test validator -- probe
cargo test -p evenfall-gba --test validator -- probe-timer
```

The validator runs every probe, linked ones included, and checks it against its expected result ([`testing.md`](../docs/testing.md#test-roms)).

Every `*.c` file except `report.c` is one ROM, linked with `header.s`, `report.c` and `rom.ld`.
Three probes are variants: a two-line source that defines one switch and includes the probe it varies (`affine_wrap.c`, `brighten_dark.c`, `sweep_down.c`).

## Reporting protocol

A probe reports one 32-bit value in `r12` and then parks the CPU in a self-loop at a fixed address:

- [`report.c`](report.c) defines `report_and_idle(value)`, which copies its argument into `r12` and falls into `idle_loop`, a branch to itself.
- [`rom.ld`](rom.ld) places `report_and_idle` at `0x08000300`, so `idle_loop` sits at `0x08000304`.
  All other code links after it, so a probe of any size links with any compiler version.
  The Makefile checks the symbol with `nm` after every link and fails the build if `idle_loop` has moved.
- The validator runs the machine until the next instruction address is `0x08000300`, steps once, and requires the CPU to be at `0x08000304`.
  Only then does it read `r12` and whatever else the case observes: the framebuffer, the audio stream, save memory or a host-visible device flag.
  Linked cases require every machine to sit in its idle loop for two consecutive chunks of cable time.

The protocol is the one the jsmolka `gba-tests` ROMs use (a result register plus a terminal self-loop at a known address), for three reasons:

- **It needs nothing from the device under test.** A register and a program counter are visible through `Machine::read_register` and `Machine::next_instruction_address` whether or not the PPU, a save device or any MMIO path works.
  A probe that printed its result to the screen would depend on the renderer it might be testing.
- **It proves the probe finished.** Stepping from the entry into the self-loop shows the ROM ran to completion and settled, rather than passing through the address or faulting nearby.
  A fixed address makes this a cheap check that works for every probe without symbol files.
- **It freezes guest-visible state.** The idle loop touches no memory or registers, so RAM, VRAM and I/O stay as the probe left them while peripherals keep running.

Headers carry a zeroed Nintendo logo, the fixed byte `0x96` and a correct complement at `0xBD`, which the Makefile computes over `0xA0..0xBC`.
They are test fixtures and make no claim to boot on hardware.
The game code (`ZB01`) matches none of the retail game codes that [`cartridge/mod.rs`](../systems/gba/src/cartridge/mod.rs) maps to cartridge hardware, so no GPIO, RTC, solar, gyro, rumble or tilt device attaches unless the case enables one.

## Writing a probe and its oracle

The expected value must come from GBATEK, the probe source or arithmetic, never from the emulator's output.
A few rules keep the oracle honest:

- **Bracket every count with an independent event.** A probe that spins until a counter reaches N and reports N passes when an IRQ fires twice, on the wrong edge or on too few lines.
  Stop on a second timer, a frame boundary or a host key press instead, and report the count.
- **Report what you measured and let the validator decide.** When a ROM compares values itself (RTC, EEPROM), back it with a validator-side check on state the ROM cannot fake, such as the persisted save bytes.
- **Assert structure when timing drifts.** A mid-scanline write lands after a VCOUNT poll whose phase moves with CPU timing, so those oracles require a genuine split on one line over a sane span instead of exact pixels.
- **Check the probe against GBATEK before changing the emulator.** A failing probe is often the probe's fault.
  For example, a ROM-to-EWRAM copy through DMA0 copies zeros because DMA0's 27-bit source mask cannot reach the Game Pak.
  That is documented hardware behavior, so `dma_copy.c` uses DMA3.

Toolchain details that trip up new probes:

- `rom.ld` places `.bss` in EWRAM and has no `.data` copy.
  Uninitialized globals work, but initialized mutable globals don't.
- GBA SWI numbers live in instruction bits 23-16 in ARM state, so GNU as `swi 2` dispatches service 0.
  Spell it `swi 0x020000`.
- PSG register writes are ignored while SOUNDCNT_X bit 7 (master enable) is clear.
  Sound probes enable it first.
- Probes store their interrupt handler's address at `0x03007FFC`, where the BIOS IRQ trampoline reads it (`IRQ_HANDLER_ADDR` in [`gba.h`](gba.h)).

## CPU, interrupts and DMA

- `timer_irq.c` (`probe-timer`): timer0 at the /64 prescaler from reload `0xFFF0` with IRQ, timer1 cascading from it with IRQ (reload `0xFFFB`, one overflow per five), and timer2 cascading from timer1 without IRQ.
  One ISR counts IRQs per source.
  Main waits for the second timer1 IRQ and reports `timer0 IRQs << 16 | timer1 IRQs << 8 | TM2CNT_L`, which the cascade rule fixes at `0x000A0202`.
  A cascade timer that counts twice per overflow reports `0x00050204`.
- `timer_start.c` (`probe-timer-start`): the mGBA suite's timer IRQ sequence (`src/timer-irq.c`), written into IWRAM for reloads `FFFFh` down to `FFF7h` and zero to nine `NOP`s, with the IRQ off.
  Each run enables timer 0, rewrites its reload to 0, and reads the counter after the `NOP`s.
  The table of reads must match the suite's hardware rows, extended by one per `NOP` past the point where the suite's IRQ stops the timer.
  The first read after the enabling write is the reload value, so the counter holds it through the cycle after the write.
- `timer_irq_latency.c` (`probe-timer-irq-latency`): the same start sequence with the IRQ on, followed by one-cycle `add r5, r5, #1` instructions.
  The IRQ handler records how many additions ran before it took the IRQ, for reloads `FFFFh` to `FFFAh`.
  The suite's table fixes them at 4 to 9: the interrupt reaches the CPU four cycles after the overflow.
- `display_irq.c` (`probe-display-irq`): VBlank, HBlank and VCount-match (line 100) IRQs counted by one ISR over exactly two frames, bracketed between entries into line 80, far from every LCD interrupt.
  GBATEK flags HBlank on all 228 lines, so the report is `456 << 16 | 2 << 8 | 2`.
  HBlank IRQs on visible lines only would report 320 in the top field.
- `keypad_irq.c` (`probe-keypad`): A selected in KEYCNT with IRQ enabled.
  The validator presses and releases A three times, then presses START, which KEYCNT does not select.
  The ROM reports the IRQ count, expected 3: the condition must turn true to fire.
  IRQs on release as well would report 6.
- `halt_stop.c` (`probe-halt-stop`): HALT (SWI 2) woken by a timer0 IRQ, then STOP (SWI 3) woken by a validator A press once the STOP power state is visible.
  One ISR counts both wakes, expected 2.
- `dma_copy.c` (`probe-dma`): immediate 32-bit DMA3 copy of 64 ROM words (`0xCAFE0000..0xCAFE003F`) to EWRAM, checksummed by the ROM.
  The expected sum `0xBF8007E0` was computed independently of both the ROM and the emulator.
- `hdma_scroll.c` (`probe-hdma`): Mode 0 two-stripe BG0 whose BG0HOFS comes from a 160-entry EWRAM table through repeated HBlank DMA2, rewound inside VBlank so that a line-0 rewind cannot split row 0.
  Row 0 shows the CPU-written `table[0]`, and row y >= 1 shows `table[y - 1]`, because the DMA for a line runs in the HBlank before the next one.
  Like mGBA, Evenfall runs HBlank DMA on visible lines only.
- `vdma_upload.c` (`probe-vdma`): one-shot VBlank DMA3 of a solid tile into VRAM, shown full-screen.
  A missed trigger exposes the backdrop.
- `dma_stall.c`: Direct Sound DMA that never releases the bus, until a repeating VBlank DMA0 stops the timers that request it after 64 frames.
  The CPU must stay stalled throughout: the ROM reports 1 when its poll count stays small and the VBlank and DMA0 interrupt flags, raised only during the stall, are both set.
  It has no validator case.
  The [`probe-dma_stall`](../tools/parity/scenarios/probe-dma_stall.json) parity scenario checks the final PC and `r12`.

## Cartridge devices

- `eeprom_rw.c` (`probe-eeprom`): 8-byte write to 512-byte EEPROM block 5 through DMA3 serial commands, busy poll, read-back and compare (0 = match, 1 = busy timeout, 2 = mismatch).
  Nothing configures a save type, so the validator also requires `Machine::save_type()` to resolve to `Eeprom512` from the DMA lengths.
  Persisted bytes 40..47 must hold the payload, which pins the addressing independently of the ROM's own read-back.
- `rtc_datetime.c` (`probe-rtc`): S3511 date and time read by GPIO bit-banging against the injected epoch 981205445 (2001-02-03 13:04:05, a Saturday).
  The ROM compares the control register and all seven BCD bytes and reports the first mismatch.
- `rtc_write.c` (`probe-rtc-write`): writes 2002-12-31 23:59:58 (a Tuesday) and reads the latch back inside the same second.
  An ignored write reads back the injected 2001 epoch.
- `gyro_sample.c` (`probe-gyro`): WarioWare gyro ADC read through the GPIO clock sequence against the injected sample `0x6C0`, echoed verbatim.
- `solar_count.c` (`probe-solar`): Boktai reset plus clock pulses against injected level 252.
  The threshold is 255 - level = 3 with a strict-greater comparator, so the flag trips on the fourth pulse.
- `tilt_sample.c` (`probe-tilt`): Yoshi 55h/AAh latch plus byte reads of both axes against the injected `0x392`/`0x3A0`, packed as `0x03A08392` (the X high byte carries the ready flag in bit 7).
  The save device is disabled, so the SRAM-window reads can only come from the tilt adapter.
- `rumble_flag.c` (`probe-rumble`): GPIO3 driven high with rumble enabled.
  The host-visible motor flag (`Machine::rumble_active()`) must be set.
  Physical haptics are a frontend concern.

## Serial links

Linked cases run every machine on one deterministic cable timeline with `run_linked_until`, and each ROM reports what it received.

- `link_parent.c` + `link_child.c` (`probe-link`): Normal-8 full-duplex exchange.
  The parent sends `0xA5` on the internal 256 Kbit/s clock and the child sends `0x5A` on the external clock.
  Both ROMs first clear RCNT bit 15: RCNT powers up at `0x8000`, which selects no SIO mode, and the machines never pair until it is cleared.
- `multi4_parent.c` + `multi4_child1/2/3.c` (`probe-multi4`): 4-player multiplayer exchange (1111/2222/3333/4444).
  Each machine reports a different peer's SIOMULTI slot, proving full vector distribution.
- `multi_parent.c` + `stop_sio_child.c` (`probe-stop-sio`): a 2-player multiplayer exchange (parent `0x1234`, child `0xABCD`).
  The child arms the SIO completion IRQ and executes STOP with IME off, and the parent's transfer wakes it.
  A stopped bus advances only through SIO, UART and JoyBus deadlines (`Bus::idle_stop_until`), matching hardware STOP and its wake mask.
- `uart_tx.c` + `uart_rx.c` (`probe-uart`): UART byte stream (`0xA5`, `0x5A`, `0x3C`) with TX-busy pacing and RX-empty polling.
  The receiver packs `0x3C5AA5`.
  Entering UART mode clears FIFO state, so the sender must write SIOCNT before SIODATA8.
- `joybus_slave.c` (`probe-joybus`): JoyBus slave receive, with the validator playing master: a WRITE carrying `0x44332211` after setup, which the ROM reports from JOY_RECV.
- `joybus_tx.c` (`probe-joybustx`): JoyBus slave transmit.
  The ROM publishes `0x44332211`.
  The validator's READ must return it LSB first plus JOYSTAT, and the ROM must see its transmit flag clear.
- `multiboot_2slave_parent.c` + two `multiboot_2slave_child.c` (`probe-mb2slave`): the parent fills a 0x100-byte payload and calls BIOS SWI 25h in multiplayer mode for two clients, and the HLE continuation drives the real SIO registers.
  Each child mimics the slave BIOS (`0x73rr`, `0xC0+2j` echoes, `0x75`, the computed CRC) and checks every data half against its own replica of the multiboot encoder.
  Expected: parent result 0 and 0 mismatches on each child.
  The child encodes three constraints of the slave protocol:
  - the master snapshots SIOMLT_SEND when it launches a transfer and leaves only about 600 cycles between transfers, so every response is a precomputed positional value latched before the next launch, and the CRC answer is latched as soon as `0x66` arrives,
  - the next launch clears the receive registers, so the child reads SIOMULTI0 before doing any other work,
  - the master encodes once per 32-bit word and sends the halves in two transfers, so the replica encodes on even halves only.
- `multiboot_2mhz_parent.c` + `multiboot_normal_child.c` (`probe-mb2mhz`): the same payload over Normal-32 at 2 MHz (`r1 = 2`), one 32-bit transfer per word with echoes in the high half.
  A 2 MHz transfer (256 cycles) is shorter than encoder verification, so the slave snapshots words and verifies after the exchange.
  It enters Normal-32 before preloading SIODATA32, because data writes are gated by the submode.
  The 256 KHz branch differs only in the SIOCNT speed bit.

## Sound

Sound cases drain the whole DAC stream from boot and require one stereo sample per 512 cycles (32768 Hz at the default SOUNDBIAS resolution) with equal left and right.
Silence mixes to exactly 0, and a full-volume PSG level v mixes to v * 16.

- `psg_square.c` (`probe-psg`): square channel 2 at frequency `0x700`, 50% duty, volume 15.
  Every duty step lasts 16 * (2048 - `0x700`) = 4096 cycles (8 samples), and duty steps {0,5,6,7} are four cyclically contiguous high steps, so the steady state alternates runs of exactly 32 samples at 0 and 240.
- `envelope_down.c` (`probe-envelope`): channel 2 with a decreasing envelope from 15.
  Run levels must never rise, must start at 240, and must decay through at least three levels to 208 or below.
- `sweep_up.c` (`probe-sweep`) and `sweep_down.c` (`probe-sweepdown`): channel 1 with sweep period 2 and shift 1, adding from `0x080` or subtracting from `0x700`.
  Each update changes the frequency by f >> 1, so an increasing sweep multiplies it by 1.5.
  Updates land every 262144 cycles (the 128 Hz sweep clock at period 2).
  Complete runs must change monotonically in the sweep's direction, by at least 1.25x overall.
- `noise_poly.c` (`probe-noise`): noise at r=1, s=4, first in 7-bit then in 15-bit mode.
  The validator reimplements GBATEK's recurrence from its taps (`0x60` narrow, `0x6000` wide) and slides a 64-step match over both sections, which proves the taps without depending on the LFSR reset value.
- `wave_ramp.c` (`probe-wave`): wave bank 1 holding nibbles 0..15 at frequency `0x700`, four samples per nibble.
  Runs must be exactly 4 samples long and ascend 0, 16, ..., 240 cyclically.
- `wave_chain.c` (`probe-wavechain`): two-bank mode chaining bank 0 (zeros) into bank 1 (ramp, then a constant-7 marker).
  Setup writes land in bank 1, the bank the CPU sees while the channel is off, so bank 0 stays zero.
  Every cycle must show the zero run, sixteen ascending 4-sample runs, and a 64-sample marker at 112.
- `wave_readback.c` (`probe-wavereadback`): wave-RAM shift-register read-back.
  Bank 1 plays for exactly 3 steps, then the 64-nibble two-bank span for 67 steps.
  The ROM hashes the read-back with FNV-1a, and the validator recomputes the hash from GBATEK's rotation model.
- `fifo_stream.c` (`probe-fifo`): a 1 KiB EWRAM pattern of 16-byte +64/-64 blocks streamed through FIFO A by DMA1 on timer0, one byte per sample.
  It plays as alternating runs of exactly 16 at +256/-256 after a zero underrun head.
  DMA1 carries the stream because only DMA1 and DMA2 have the sound FIFO start timing.

## Rendering

These cases recompute all 38400 pixels of the final frame.

- `obj_sprite.c` (`probe-obj`): two overlapping 8x8 4bpp sprites.
  OBJ0 wins the overlap on priority.
  Unused OAM is parked offscreen because zeroed entries are live sprites at (0,0).
- `obj_flip.c` (`probe-objflip`): one quadrant tile drawn plain, H-flipped, V-flipped and both.
  The backdrop reads BG palette entry 0.
- `obj_rot.c` (`probe-objrot`): a 90-degree affine OBJ about the integer centre (4,4).
  Screen (8+i,8+j) samples texel (j,8-i), so column 8 is transparent.
  It agrees with pinned mGBA exactly.
- `obj_doublesize.c` (`probe-objdouble`): an identity-matrix double-size sprite, whose 8x8 image is centred in its 16x16 area.
  Attribute 1 bit 8 is X bit 8, not an affine flag.
- `obj_mosaic.c` (`probe-objmosaic`): 2x2 OBJ mosaic on an aligned and an odd-placed sprite.
  This pins group-base clamping and the right-edge group extension described in [`video.md`](../docs/gba/hardware/video.md#mosaic).
- `obj_semitransparent.c` (`probe-objsemi`): a semi-transparent OBJ over green BG1 with no BLDCNT first targets.
  Such objects force themselves to be first target, so they blend to `0x01EF`.
- `obj_window.c` (`probe-objwin`): an OBJ-window sprite enabling BG0 inside its opaque texels only, matching mGBA's OBJWIN flag.
- `affine_rot.c` (`probe-affinerot`): Mode 2, a 128x128 8bpp map rotated 90 degrees from reference (127,0), transparent outside the square.
- `affine_wrap.c` (`probe-affinewrap`): the same rotation with the overflow bit set, wrapping negative coordinates modulo 128.
- `mode1_mixed.c` (`probe-mode1`): red text columns over a green identity-mapped affine layer with overflow transparency.
- `blend_alpha.c` (`probe-blend`), `brighten.c` (`probe-brighten`), `brighten_dark.c` (`probe-darken`): full-screen red BG0 under the three colour effects at weight 8.
  They must render the GBATEK equations' `0x01EF` (red over green), `0x3DFF` and `0x0010`.
- `window_rect.c` (`probe-window`): red BG0 through WIN0 x=[80,160), y=[40,120), pinning the inclusive and exclusive edges.
- `mosaic_bg.c` (`probe-mosaic`): 4x4 BG mosaic over an all-distinct 8bpp tile.
  Coordinates quantize to group bases, and index 0 stays transparent.
- `text_8bpp.c` (`probe-text8`): 8bpp text BG whose map entries set palette bank 3 on the right half. 256-colour mode ignores the bank bits.
- `scroll_wrap.c` (`probe-scrollwrap`): a 32x32 map scrolled by (200,100), wrapping both ways.
- `bitmap_mode3.c`, `bitmap_mode4.c`, `bitmap_mode5.c` (`probe-bitmap3`/`4`/`5`): direct colour at 240 stride, paletted page 1 (a stuck page bit shows page 0), and 160x128 direct colour with the backdrop outside.
  Bitmap modes need the BG2 enable bit in DISPCNT like any other layer.
- `green_swap.c` (`probe-greenswap`): red and green column pairs exchange green, rendering yellow and black.
- `forced_blank.c` (`probe-forceblank`): forced blank renders white.

## Mid-scanline writes

Each probe polls VCOUNT for line 40, then spins before two register writes, so its split pixels drift with CPU timing.
The oracle requires every other row to be exact and line 40 to carry a genuine split over a sane span.
Each case covers one of the per-scanline segment logs in `Bus` (scroll, backdrop, window, blend, affine, mosaic).
Registers without a segment log resolve once, at end of line.

- `midline_combo.c` (`probe-midcombo`): stripes through a static WIN0 x=[0,120) over a blue backdrop.
  Line 40 writes BG0HOFS 0/4/0 and palette entry 0 blue/red/blue together.
  Left of 120 the pixels must follow one of the two stripe phases, and right of it they must be backdrop red or blue.
  This covers the scroll and backdrop logs coexisting with a window.
- `midline_win.c` (`probe-midwin`): WIN0's right edge narrows from 240 to 80 and back, biting black out of red BG0.
- `midline_bldy.c` (`probe-midbldy`): BLDY 0/16/0 under brightness increase, turning a span white.
- `midline_affine.c` (`probe-midaffine`): BG2PA switches to 2x zoom and back over an identity map.
  Bitmap-mode matrices are covered separately by mid-scanline Mode 3 tests in [`tests/bus/video.rs`](../systems/gba/tests/bus/video.rs).
- `midline_mosaic.c` (`probe-midmosaic`): 4x4 mosaic, re-armed each frame, switches to 1x1 mid-line.
  Line 10 must be all 4-wide blocks, line 100 all 1-wide runs, and line 40 blocks then sharp runs.

## Open questions

- Keypad IRQs: Evenfall fires on the edge where the KEYCNT condition turns true.
  Hardware level-versus-edge behavior has not been measured.
- HBlank DMA during VBlank lines: Evenfall and mGBA fire it on visible lines only.
  Whether silicon also fires it during VBlank is unverified.
  `probe-hdma` rewinds its table inside VBlank, so it does not depend on the answer.
- Noise LFSR reset value: GBATEK gives `0x40`/`0x4000`, mGBA uses all ones.
  `probe-noise` is deliberately agnostic.
- Solar sensor threshold: 255 - level is the model Evenfall shares with NanoBoyAdvance, not a hardware measurement.
