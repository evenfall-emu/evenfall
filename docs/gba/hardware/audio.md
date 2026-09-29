# Audio

The GBA sound hardware combines four Game Boy-style PSG channels (square with
frequency sweep, square, programmable wave and noise) with two signed 8-bit
Direct Sound channels fed through FIFOs. Their outputs are mixed, biased,
clipped and emitted as a pulse-width-modulated signal.

No public Nintendo register specification for the sound hardware exists. This
page rests on the unofficial [GBATEK sound chapter][gbatek-sound], a
[DS Lite hardware experiment and test ROM][mgba-1847] reported against mGBA,
pinned [mGBA 0.10.5](https://github.com/mgba-emu/mgba/tree/0.10.5) and
NanoBoyAdvance as comparisons, and ZiGBA's analytic sound probes
([`probe/README.md`](../../../probe/README.md)). The model lives in
[`apu.rs`](../../../systems/gba/src/apu.rs) (`Apu`); timer and DMA
coupling lives in [`bus/mod.rs`](../../../systems/gba/src/bus/mod.rs).

Everything runs in the master-clock domain: register writes, oscillator
steps, frame-sequencer steps, timer overflows, FIFO actions and DAC samples
all happen at exact master-cycle timestamps, never at host sample-rate
boundaries.

## PSG channels

| Register      | Address                 | Fields                                                                       | Readable bits |
| ------------- | ----------------------- | ---------------------------------------------------------------------------- | ------------- |
| `SOUND1CNT_L` | `04000060h`             | sweep shift 0–2, direction 3, time 4–6                                       | `007Fh`       |
| `SOUND1CNT_H` | `04000062h`             | length 0–5, duty 6–7, envelope step 8–10, direction 11, initial volume 12–15 | `FFC0h`       |
| `SOUND1CNT_X` | `04000064h`             | frequency 0–10, length enable 14, trigger 15                                 | `4000h`       |
| `SOUND2CNT_L` | `04000068h`             | as `SOUND1CNT_H`                                                             | `FFC0h`       |
| `SOUND2CNT_H` | `0400006Ch`             | as `SOUND1CNT_X`                                                             | `4000h`       |
| `SOUND3CNT_L` | `04000070h`             | two-bank mode 5, bank select 6, DAC enable 7                                 | `00E0h`       |
| `SOUND3CNT_H` | `04000072h`             | length 0–7, volume 13–14, force 75% 15                                       | `E000h`       |
| `SOUND3CNT_X` | `04000074h`             | rate 0–10, length enable 14, trigger 15                                      | `4000h`       |
| `SOUND4CNT_L` | `04000078h`             | length 0–5, envelope step 8–10, direction 11, initial volume 12–15           | `FF00h`       |
| `SOUND4CNT_H` | `0400007Ch`             | divisor r 0–2, 7-bit width 3, shift s 4–7, length enable 14, trigger 15      | `40FFh`       |
| `WAVE_RAM`    | `04000090h`–`0400009Fh` | 32 four-bit samples of the bank not selected for playback                    | all           |

Lengths, frequencies and trigger bits are write-only
([GBATEK channel sections][gbatek-ch1]). Byte writes reach the individual
register halves, so writing a frequency's low byte neither retriggers the
channel nor erases its high bits (ZiGBA test "APU byte writes preserve
write-only frequency lanes" in
[`bus_test.zig`](../../../refs/zigba/test/bus_test.zig)).
`SOUNDCNT_X` bits 0–3 report which channels are on; a channel turns off when
its length expires, its DAC is disabled or a sweep overflows, not when its
envelope reaches zero.

PSG channel outputs are levels 0–15. ZiGBA does not assume full DMG/CGB
identity: pinned mGBA selects a distinct GBA style for these channels, and its
trigger paths ([square][mgba-gb-square], [wave and noise][mgba-gb-wave-noise])
are the main implementation comparison for edge cases.

### Square channels

Channel 1 has four duty patterns, a 6-bit length, a volume envelope and a
frequency sweep; channel 2 is the same without the sweep. A duty step lasts
`16 × (2048 − frequency)` cycles and a period has eight steps, giving
GBATEK's tone frequency `131072 / (2048 − frequency)` Hz. The duties are
12.5%, 25%, 50% and 75%. The DAC is on while the envelope's initial volume
or direction bit is non-zero; turning it off silences the channel. A trigger
reloads a zero length to 64, restarts the envelope at the initial volume and
restarts the current duty step. `probe-psg` checks a 50% square wave's exact
run lengths and levels.

### Wave channel

Wave RAM holds two banks of 32 four-bit samples, played high nibble first.
Playback uses the bank selected by `SOUND3CNT_L` bit 6 while CPU accesses to
`04000090h`–`0400009Fh` reach the other bank; in two-bank mode playback chains
all 64 samples (`probe-wavechain`). A sample step lasts `8 × (2048 − rate)`
cycles. Volume selects mute, 100%, 50% or 25% (a right shift of the sample),
and bit 15 forces 75%, the GBA-specific addition (`probe-wave`).

GBATEK describes each bank as a 128-bit shift register
([Sound Channel 3][gbatek-ch3]), so data read back after playback is rotated.
ZiGBA rotates the playing span, one bank or both banks as one 64-sample span
in two-bank mode, left by one sample per step, and always outputs its top
nibble. Output samples are the same as a position-pointer model by
construction. Pinned mGBA rotates the same way in `gb/audio.c`.
`probe-wavereadback` plays for 3 steps, then 67 steps across both banks, and
compares the read-back hash with one computed from GBATEK's model.

### Noise channel

The noise channel steps a linear-feedback shift register X at
`524288 / r / 2^(s+1)` Hz, with r = 0 counting as 0.5
([Sound Channel 4][gbatek-ch4]); in master cycles one step takes
`64 × r × 2^s`, or `32 × 2^s` when r = 0. Each step computes

```text
X = X >> 1; if the bit shifted out was 1, output high and X ^= 6000h (15-bit)
                                            or X ^= 60h (7-bit); else output low
```

`probe-noise` pins these taps: its validator reimplements GBATEK's recurrence
and slides a 64-step match over 7-bit and 15-bit runs. The sliding match
absorbs phase, so it proves the taps under any reset value.

The reset value on trigger is disputed. GBATEK states X = 40h (7-bit) or
4000h (15-bit) for the GBA, and NanoBoyAdvance independently uses
`lfsr_init[] = { 0x4000, 0x0040 }`. Pinned mGBA resets to all ones (7Fh or
7FFFh) in its shared Game Boy core. No available source measures the first
samples on silicon, so ZiGBA follows the higher-ranked GBATEK claim.

### Frame sequencer, length, envelope and sweep

An eight-step frame sequencer runs at 512 Hz, one step every 32,768 cycles
(pinned mGBA shows the step actions in [`gb/audio.c`][mgba-gb-sequencer]):

| Steps      | Action                     | Rate   |
| ---------- | -------------------------- | ------ |
| 0, 2, 4, 6 | length counters            | 256 Hz |
| 2, 6       | channel 1 sweep            | 128 Hz |
| 7          | square and noise envelopes | 64 Hz  |

Enabling or disabling master sound restarts the sequencer so that the next
step is step 0.

- **Length.** Square and noise lengths count `64 − n`; the wave length
  counts `256 − n`. With length enabled, reaching zero turns the channel off.
  The length unit keeps clocking while the channel's output or DAC is off
  (test "PSG length unit keeps clocking after channel output stops").
- **Extra length clock.** Enabling length when the next sequencer step will
  not clock it applies one immediate extra clock, as on the Game Boy. A
  simultaneous trigger that finds the length at zero first reloads 64 or 256
  and then applies the same clock, leaving 63 or 255. This follows pinned
  mGBA's GBA-style state machine (test "PSG length enable and trigger apply
  the frame-phase extra clock").
- **Envelope.** A step time of zero disables the envelope. Otherwise each
  period moves the volume one level up or down, stopping at 15 or 0
  (`probe-envelope`).
- **Sweep.** A trigger copies the frequency into a shadow register and, when
  the shift is non-zero, checks the next value for overflow at once. Each
  sweep clock that completes a period computes `shadow ± (shadow >> shift)`;
  a result above 2047 turns channel 1 off, otherwise it becomes the new
  frequency and is checked for overflow again (`probe-sweep`,
  `probe-sweepdown`; test "PSG channel one sweep disables after its
  post-update overflow probe").

### Master enable

While `SOUNDCNT_X` bit 7 is clear, writes to `04000060h`–`04000081h` are
lost and those registers read zero; software must re-initialize them after
re-enabling ([GBATEK][gbatek-control]; mGBA gates the same range in
[`io.c`][mgba-io]). Clearing the bit resets every PSG channel and
`SOUNDCNT_L`, silences PSG and Direct Sound output, and clears the PSG-active
flags. Wave RAM, the FIFO ports, `SOUNDCNT_H` and `SOUNDBIAS` stay
accessible. Following pinned mGBA ([`audio.c`][mgba-audio-disable]),
`SOUNDCNT_H` loses its low byte (mixing ratios) but keeps its routing and
timer selections, and the FIFO contents are not cleared; the FIFOs do not
advance while sound is off. This differs from a machine reset, which
initializes queues, latches, routing and bias. The test "PSG registers
ignore writes while master sound is disabled" pins the gate, and the sound
probes enable master sound before programming channels.

## Direct Sound

| Register     | Address     | Fields                                                                                                                                                                               |
| ------------ | ----------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `SOUNDCNT_L` | `04000080h` | PSG master volume right 0–2 and left 4–6; PSG routing right 8–11 and left 12–15                                                                                                      |
| `SOUNDCNT_H` | `04000082h` | PSG ratio 0–1 (25/50/100%; 3 is prohibited); FIFO A volume 2 and FIFO B volume 3 (50/100%); FIFO A right 8, left 9, timer 10, reset 11; FIFO B right 12, left 13, timer 14, reset 15 |
| `SOUNDCNT_X` | `04000084h` | PSG-active flags 0–3 (read-only); master enable 7                                                                                                                                    |
| `SOUNDBIAS`  | `04000088h` | bias level 1–9 (default `200h`); resolution 14–15                                                                                                                                    |
| `FIFO_A`     | `040000A0h` | four signed 8-bit samples, least significant byte first (write-only)                                                                                                                 |
| `FIFO_B`     | `040000A4h` | as `FIFO_A`                                                                                                                                                                          |

A PSG master volume n means `(n + 1)/8`, so zero is 1/8, not mute. ZiGBA
treats the prohibited PSG ratio 3 as 100%. The reset bits 11 and 15 are
write-one strobes, not stored flags: `SOUNDCNT_H` keeps only mask `770Fh`,
as pinned mGBA does ([`io.c`][mgba-io]), so they read back zero. GBATEK marks
`SOUNDBIAS` bit 0 as unused and ZiGBA, like mGBA, does not store it (mask
`C3FEh`; test "Direct Sound control and bias expose only hardware-backed
bits").

### FIFO model

GBATEK gives each FIFO a capacity of eight 32-bit words
([DMA Sound][gbatek-dma-sound]). The hardware experiment refines that into
**seven queued words plus a separate four-byte playback word**, and ZiGBA
models exactly that (`DirectSoundFifo`: an eight-slot ring whose read and
write indices distinguish at most seven queued words, plus the playback
word and its remaining byte count):

- **Clocking.** Each Direct Sound channel is clocked by overflows of the
  timer its `SOUNDCNT_H` bit selects (timer 0 or 1), and only while master
  sound is enabled and the channel is routed to at least one side, as in
  pinned mGBA ([`timer.c`][mgba-timer-fifo]).
- **Refill request, then playback.** At each clocking overflow the channel
  first requests sound DMA if fewer than four words are queued. Then, if its
  playback word is empty and a queued word exists, it moves that word into
  the playback word. It outputs the playback word's lowest remaining byte,
  which later overflows follow in order. Testing before pulling prevents two
  requests without an overflow between them. The threshold follows the
  hardware experiment and pinned mGBA ([`audio.c`][mgba-audio-fifo]); it
  differs by one word from GBATEK's "only 4 × 32 bits" description.
- **CPU writes.** A 32-bit write queues its four bytes. An 8- or 16-bit write
  replaces only the addressed byte lanes of the word already in the current
  write slot, keeps that slot's other lanes, and then advances by a whole
  word; it does not append one or two samples (test "FIFO halfword write
  commits a whole word slot with untouched lanes").
- **Overflow.** Writing an eighth word when seven are queued wraps the ring,
  which then appears empty, as the experiment reported.
- **Underrun.** With the playback word and the queue both empty, ZiGBA
  outputs zero. The experiment only inferred the hardware's output here.
- **Reset strobes.** Bits 11 and 15 of `SOUNDCNT_H` clear the queued words
  but keep the playback word and the sample currently output, as pinned mGBA
  does ([reset handling][mgba-audio-reset]; test "FIFO reset clears queued
  words but preserves the active playback word").

`probe-fifo` streams a pattern through FIFO A with DMA1 on timer 0 and checks
exact run lengths and levels, including the zero underrun before the first
refill.

### Sound DMA

For normal playback, DMA1 or DMA2 is set to special start timing with repeat
and its destination at `FIFO_A` or `FIFO_B`
([GBATEK][gbatek-sound-dma]; Nintendo's [US7445551B2][patent-dma-sound]).
Each FIFO request then transfers exactly four 32-bit words: the word count is
ignored, the unit is forced to 32 bits and the destination stays fixed. mGBA
implements the same in its [request scheduler][mgba-audio-request] and
[DMA engine][mgba-dma-fifo]. Everything else is ordinary DMA: arbitration,
bus timing, source stepping, the completion IRQ and the source pointer
carried across requests
([`timers-dma-interrupts.md`](timers-dma-interrupts.md)).

In ZiGBA:

- When such a channel is enabled with its programmed destination at a FIFO,
  `Bus::write_mmio_half` rewrites its control to fixed destination and 32-bit
  units, so the control reads back with those bits.
- A refill request makes pending the enabled special-timing channel among
  DMA1 and DMA2 whose destination is that FIFO, unless it is the channel
  currently transferring. The request takes effect at the timer overflow
  itself, without the start delay of other DMA triggers, as in pinned mGBA.
- A special-timing DMA1 or DMA2 whose destination is not a FIFO never starts.

The hardware experiment found that width is forced but an arbitrary
destination is not: the threshold request drove a four-word DMA to it. mGBA
rejects such a destination and GBATEK calls it invalid, so ZiGBA does not
model it (see [Open questions](#open-questions)). DMA3 cannot feed the
FIFOs.

## Mixing and the DAC

For each side the mixer adds the routed channels:

- **Direct Sound:** the current sample (−128 to 127) times 2 at 50% volume
  or 4 at 100%, so each FIFO spans about ±200h.
- **PSG:** each routed channel's level (0–15) times 8, times the side's master
  volume (1–8), shifted right by 4, 3 or 2 for the 25%, 50% or 100% ratio.
  Each channel's contribution is truncated on its own. A full-volume level v
  at 100% and master volume 7 contributes `16v`.

It then adds the `SOUNDBIAS` level and clips to the unsigned 10-bit range
`0`–`3FFh`
([GBATEK][gbatek-control]; pinned mGBA performs routing, volume scaling,
bias, clipping and cadence selection in [`audio.c`][mgba-audio-mix]).
Tests "mixed audio is biased and clipped to the ten-bit DAC range" and
"SOUNDBIAS resolution truncates the clipped DAC code to PWM precision" pin
the arithmetic.

`SOUNDBIAS` bits 14–15 select the PWM resolution and sample rate:

| Resolution  | Output depth | Sample rate | Cycles per sample |
| ----------- | ------------ | ----------- | ----------------- |
| 0 (default) | 9 bits       | 32,768 Hz   | 512               |
| 1           | 8 bits       | 65,536 Hz   | 256               |
| 2           | 7 bits       | 131,072 Hz  | 128               |
| 3           | 6 bits       | 262,144 Hz  | 64                |

GBATEK describes resolution 0 as the 10-bit value divided by two, and
resolutions 1–3 analogously discard two, three or four low bits. The
hardware then outputs each code as a 1-bit PWM pulse train at the master
clock ([GBATEK][gbatek-control]).

ZiGBA's DAC seam is the selected-depth code, not the PWM bit stream. It
keeps samples signed and in 10-bit units: it clears the discarded low bits
from both the clipped output and the bias, and emits their difference. This
is equivalent to the narrower PWM code: it preserves the analog scale while
exposing the quantization steps. Pinned mGBA's `_applyBias` and
NanoBoyAdvance's mixer instead keep the clipped 10-bit value and use the
resolution only to change the sample cadence, which is a reasonable host-audio
shortcut but not the raw selected-depth DAC output.

## Host-facing sample seam

At every sample boundary of the selected rate the APU emits one
`StereoSample`, two signed 16-bit values in the units above, into a queue of
2,048 samples. While master sound is off it emits zeros at the same cadence.
The emitted stream is deterministic and independent of the host: the probe
validators check it sample by sample, and golden hashes can be taken from it.

- `Machine::drain_audio_samples` moves queued samples to the caller. A full
  queue drops its oldest samples, so hosts drain after each bounded advance.
- `Machine::audio_sample_rate` reports the current DAC rate, which changes when
  software writes `SOUNDBIAS` bits 14–15.
- `Machine::set_audio_output(false)` stops producing host samples while all
  emulated sound, timer and DMA state stays exact.
- `Machine::set_high_speed_audio(true)` is an optional fast-forward policy: it
  silences output and freezes the square and noise oscillator phases,
  including the noise LFSR. Register writes, length, sweep, envelopes, wave
  RAM rotation, FIFO consumption, timers and sound DMA stay exact, but saved
  square and noise phases then differ from normal execution, and leaving the
  mode resumes them without catching up.

Both preferences belong to the host and are not saved in states.

Resampling, buffering, latency control and device callbacks sit downstream
of this seam. [`crates/audio/src/lib.rs`](../../../crates/audio/src/lib.rs)
(`Resampler` in Rust) converts the DAC rate to a host rate with
linear interpolation and an exact rational phase, so long runs do not drift.
It allocates nothing and reports how many input samples it consumed and how
many output samples it wrote, so the caller keeps any unconsumed input; the
first input sample only seeds interpolation. `Session::drain_audio`
([`session.rs`](../../../systems/gba/src/session.rs)) recreates it when the DAC rate
changes. Band-limited reconstruction is not implemented.
[`crates/audio/src/lib.rs`](../../../crates/audio/src/lib.rs) tests cover
split buffers, backpressure, upsampling, downsampling, reset and the absence
of drift.

ZiGBA advances the square and noise oscillators lazily. Their elapsed cycles
accumulate and are applied before anything can observe them (a DAC sample
while one of them is audible, a frame-sequencer step, a register write that
affects them, or a state write), so the result is identical to advancing
them every cycle. The wave channel is never deferred, because its RAM
rotation is visible to the CPU between samples.

## Save states

A save state (format version 6,
[`design/save-states.md`](../../design/save-states.md)) holds every register
value, the master enable, the bias, both FIFO rings with their indices, the
playback words and their remaining bytes, the current Direct Sound samples,
all channel state (oscillator phases, the noise LFSR, length, envelope and
sweep counters), both wave RAM banks and their bank settings, the DAC sample
phase, and the frame-sequencer phase and step. Square and noise channels are
written fully advanced. The timers and DMA channels that feed the FIFOs are
saved with the rest of the bus state.

The host sample queue and the output preferences are not saved: a restored
machine starts with an empty queue and continues from the restored
deterministic state. Loading rejects impossible phases: a sample phase beyond
the sample interval, a frame phase beyond 32,768 cycles, an enabled sweep
with an out-of-range counter, or a playback word with more than four bytes
left.

## Open questions

- **Noise reset value.** GBATEK and NanoBoyAdvance give 40h/4000h, mGBA all
  ones; no silicon capture of the first noise samples is available.
- **Wave RAM rotation.** The rotation direction rests on GBATEK and two
  implementations; no measured read-back trace exists.
- **Extra length clock.** The frame-phase rule follows mGBA's GBA-style state
  machine without a measured edge trace.
- **FIFO reset strobe.** Whether every hardware revision keeps the playing
  word and current DAC sample across a reset strobe needs a focused test.
- **Unrouted Direct Sound channels.** ZiGBA, like mGBA, does not consume FIFO
  data for a channel routed to neither side; hardware is untested.
- **FIFO underrun output.** ZiGBA outputs zero; the experiment only inferred
  the hardware's value.
- **Master disable and re-enable.** FIFO survival across a master-enable
  cycle and the first sample after re-enabling are not established by the
  available hardware report.
- **Sound DMA to a non-FIFO destination.** Hardware apparently performs the
  four-word transfer to any destination; ZiGBA never starts it.
- **Sound DMA control read-back.** ZiGBA rewrites a FIFO channel's control
  bits on enable; whether hardware changes the readable bits is untested.
- **PSG output level.** ZiGBA mixes PSG levels as unsigned values, so a
  channel spans 0 to about +F0h at full volume; GBATEK describes each PSG as
  spanning ±80h. The difference is a DC offset that changes clipping
  headroom.
- **Prohibited PSG ratio.** ZiGBA treats `SOUNDCNT_H` ratio 3 as 100%.

[gbatek-sound]: https://rust-console.github.io/gbatek-gbaonly/#gbasoundcontroller
[gbatek-ch1]: https://rust-console.github.io/gbatek-gbaonly/#gbasoundchannel1tonesweep
[gbatek-ch3]: https://rust-console.github.io/gbatek-gbaonly/#gbasoundchannel3waveoutput
[gbatek-ch4]: https://rust-console.github.io/gbatek-gbaonly/#gbasoundchannel4noise
[gbatek-dma-sound]: https://rust-console.github.io/gbatek-gbaonly/#gbasoundchannelaandbdmasound
[gbatek-sound-dma]: https://rust-console.github.io/gbatek-gbaonly/#gbadmatransfers
[gbatek-control]: https://rust-console.github.io/gbatek-gbaonly/#gbasoundcontrolregisters
[mgba-1847]: https://github.com/mgba-emu/mgba/issues/1847
[patent-dma-sound]: https://patents.google.com/patent/US7445551B2/en
[mgba-gb-square]: https://github.com/mgba-emu/mgba/blob/0.10.5/src/gb/audio.c#L154-L259
[mgba-gb-wave-noise]: https://github.com/mgba-emu/mgba/blob/0.10.5/src/gb/audio.c#L260-L402
[mgba-gb-sequencer]: https://github.com/mgba-emu/mgba/blob/0.10.5/src/gb/audio.c#L633-L720
[mgba-io]: https://github.com/mgba-emu/mgba/blob/0.10.5/src/gba/io.c#L419-L430
[mgba-audio-disable]: https://github.com/mgba-emu/mgba/blob/0.10.5/src/gba/audio.c#L243-L278
[mgba-audio-reset]: https://github.com/mgba-emu/mgba/blob/0.10.5/src/gba/audio.c#L223-L241
[mgba-audio-fifo]: https://github.com/mgba-emu/mgba/blob/0.10.5/src/gba/audio.c#L306-L374
[mgba-audio-request]: https://github.com/mgba-emu/mgba/blob/0.10.5/src/gba/audio.c#L120-L155
[mgba-audio-mix]: https://github.com/mgba-emu/mgba/blob/0.10.5/src/gba/audio.c#L376-L437
[mgba-timer-fifo]: https://github.com/mgba-emu/mgba/blob/0.10.5/src/gba/timer.c#L13-L33
[mgba-dma-fifo]: https://github.com/mgba-emu/mgba/blob/0.10.5/src/gba/dma.c#L112-L136
