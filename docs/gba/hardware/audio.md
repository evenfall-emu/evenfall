# Audio

The GBA makes sound in two very different ways, and most games use both.

- **PSG channels.** Four simple synthesizer voices inherited from the Game Boy: two square waves, a short programmable waveform, and noise.
  Games play them by setting a pitch, a volume and an envelope, and the hardware does the rest.
  They give the GBA its chiptune sound effects and some of its music.
- **Direct Sound.** Two channels that play 8-bit samples, like a tiny sound card.
  The hardware doesn't mix or pitch anything here.
  A game's sound driver mixes all its instruments in software into a buffer, and the hardware plays that buffer back.

Direct Sound shows how the GBA's parts work together.
A timer sets the sample rate: each time it overflows, each channel plays its next sample.
Each channel has a small queue (a FIFO) of samples.
When the queue runs low, it asks DMA to refill it from the game's buffer, and the CPU never has to touch it.

Both kinds of sound are added together, offset, clipped and sent to the speaker as a pulse-width-modulated signal.

## A few terms

- **Sample.** One number that says where the speaker should be at one moment.
- **FIFO.** A first-in, first-out queue.
  Each Direct Sound channel has one.
- **DAC.** The digital-to-analog converter, the last step before the speaker.
  Here it is where the mixed value is turned into output.
- **Envelope.** An automatic change of volume over time, such as a fade-out.
- **Sweep.** An automatic change of pitch over time, on the first square channel.
- **Length.** A countdown that turns a PSG channel off after a set time.
- **Frame sequencer.** A 512 Hz clock that drives lengths, envelopes and sweep.

## Where it lives

No public Nintendo specification of the sound hardware exists.
This page rests on the unofficial [GBATEK sound chapter][gbatek-sound], a [DS Lite hardware experiment and test ROM][mgba-1847] reported to the mGBA project, [mGBA 0.10.5](https://github.com/mgba-emu/mgba/tree/0.10.5) and NanoBoyAdvance as comparisons, and Evenfall's own sound probe ROMs ([`probe/README.md`](../../../probe/README.md)).

The sound chip is `Apu` in [`apu.rs`](../../../systems/gba/src/apu.rs).
The timer and DMA wiring is in [`bus/mod.rs`](../../../systems/gba/src/bus/mod.rs).

Everything happens at an exact master-clock cycle: register writes, oscillator steps, sequencer steps, timer overflows, FIFO actions and output samples.
Nothing depends on the host's audio sample rate.

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
| `WAVE_RAM`    | `04000090h`–`0400009Fh` | 32 four-bit samples of the bank not currently playing                        | all           |

Lengths, frequencies and trigger bits are write-only ([GBATEK channel sections][gbatek-ch1]).
Byte writes reach each half of a register separately, so writing the low byte of a frequency neither restarts the channel nor clears the high bits.
`SOUNDCNT_X` bits 0–3 show which channels are on.
A channel turns off when its length runs out, its DAC is turned off or its sweep overflows, not when its envelope reaches zero.

Each PSG channel outputs a level from 0 to 15.
Evenfall doesn't assume they behave exactly like the Game Boy's.
mGBA has a separate GBA style for them, and its trigger code ([square][mgba-gb-square], [wave and noise][mgba-gb-wave-noise]) is the main comparison for edge cases.

### Square channels

Channel 1 has four duty cycles, a 6-bit length, a volume envelope and a pitch sweep.
Channel 2 is the same without the sweep.
The duty cycles are 12.5%, 25%, 50% and 75% of the wave spent high.
A period has eight steps of `16 * (2048 − frequency)` cycles each, which gives GBATEK's pitch of `131072 / (2048 − frequency)` Hz.

The channel's DAC is on while the envelope's initial volume or direction bit is non-zero.
Turning the DAC off silences the channel.
Triggering the channel reloads a zero length to 64, restarts the envelope at its initial volume, and restarts the current duty step.

### Wave channel

The wave channel plays a waveform the game writes itself.
Wave RAM holds two banks of 32 four-bit samples, played high nibble first.
The channel plays the bank that `SOUND3CNT_L` bit 6 selects, while the CPU reads and writes the other one at `04000090h`–`0400009Fh`.
In two-bank mode it plays all 64 samples in a row.
Each sample lasts `8 * (2048 − rate)` cycles.
The volume is mute, 100%, 50% or 25% (a right shift of the sample), and bit 15 forces 75%, which the Game Boy didn't have.

GBATEK describes each bank as a 128-bit shift register ([Sound Channel 3][gbatek-ch3]), so data read back after playing comes back rotated.
Evenfall rotates the playing span (one bank, or both banks as one span in two-bank mode) left by one sample per step and always plays the top nibble.
The samples played are the same as with a simple position pointer, and mGBA rotates the same way.

### Noise channel

The noise channel steps a linear-feedback shift register X at `524288 / r / 2^(s+1)` Hz, with r = 0 counting as 0.5 ([Sound Channel 4][gbatek-ch4]).
In master cycles, one step takes `64 * r * 2^s`, or `32 * 2^s` when r = 0.
Each step does:

```text
X = X >> 1
if the bit shifted out was 1:
    output high, and X ^= 6000h (15-bit mode) or X ^= 60h (7-bit mode)
else:
    output low
```

The starting value of X on trigger is disputed.
GBATEK says 40h (7-bit) or 4000h (15-bit), and NanoBoyAdvance independently uses the same values.
mGBA starts from all ones (7Fh or 7FFFh).
No source measures the first samples on a GBA, so Evenfall follows GBATEK, the higher-ranked source.

### Frame sequencer, length, envelope and sweep

The frame sequencer steps eight times at 512 Hz, once every 32,768 cycles ([mGBA's `gb/audio.c`][mgba-gb-sequencer] shows the same schedule):

| Steps      | Action                     | Rate   |
| ---------- | -------------------------- | ------ |
| 0, 2, 4, 6 | length counters            | 256 Hz |
| 2, 6       | channel 1 sweep            | 128 Hz |
| 7          | square and noise envelopes | 64 Hz  |

Turning master sound on or off restarts the sequencer, so the next step is step 0.

- **Length.** Square and noise lengths count `64 − n`, and the wave length `256 − n`.
  With length enabled, reaching zero turns the channel off.
  The counter keeps running while the channel is silent or its DAC is off.
- **Extra length clock.** Enabling length just before a step that won't clock it gives one immediate extra clock, as on the Game Boy.
  A trigger at the same time that finds the length at zero first reloads 64 or 256 and then applies that clock, leaving 63 or 255.
  This follows mGBA.
- **Envelope.** A step time of zero turns the envelope off.
  Otherwise each period moves the volume one level up or down, stopping at 15 or 0.
- **Sweep.** A trigger copies the frequency into a shadow register and, if the shift isn't zero, checks the next value for overflow at once.
  Each sweep period computes `shadow ± (shadow >> shift)`.
  A result above 2047 turns channel 1 off.
  Otherwise it becomes the new frequency and is checked for overflow again.

### Master enable

While `SOUNDCNT_X` bit 7 is clear, writes to `04000060h`–`04000081h` are lost and those registers read zero ([GBATEK][gbatek-control], mGBA's [`io.c`][mgba-io]).
Games must set the channels up again after turning sound back on.

Turning sound off resets every PSG channel and `SOUNDCNT_L`, silences all output and clears the channel-on flags.
Wave RAM, the FIFO ports, `SOUNDCNT_H` and `SOUNDBIAS` stay accessible.
`SOUNDCNT_H` loses its mixing ratios but keeps its routing and timer choices, and the FIFOs keep their contents without advancing, as in mGBA ([`audio.c`][mgba-audio-disable]).
This is lighter than a console reset, which clears queues, routing and bias too.

## Direct Sound

| Register     | Address     | Fields                                                                                   |
| ------------ | ----------- | ---------------------------------------------------------------------------------------- |
| `SOUNDCNT_L` | `04000080h` | PSG master volume right 0–2 and left 4–6, PSG routing right 8–11 and left 12–15          |
| `SOUNDCNT_H` | `04000082h` | PSG ratio 0–1, FIFO volumes 2–3, FIFO A settings 8–11, FIFO B settings 12–15 (see below) |
| `SOUNDCNT_X` | `04000084h` | channel-on flags 0–3 (read-only), master enable 7                                        |
| `SOUNDBIAS`  | `04000088h` | bias level 1–9 (default `200h`), resolution 14–15                                        |
| `FIFO_A`     | `040000A0h` | four signed 8-bit samples, lowest byte first (write-only)                                |
| `FIFO_B`     | `040000A4h` | as `FIFO_A`                                                                              |

`SOUNDCNT_H` in detail:

- bits 0–1: PSG volume ratio, 25%, 50% or 100% (3 is prohibited, and Evenfall treats it as 100%),
- bit 2 and bit 3: FIFO A and FIFO B volume, 50% or 100%,
- bits 8, 9, 10, 11: FIFO A to the right, to the left, its timer (0 or 1), and a reset strobe,
- bits 12, 13, 14, 15: the same for FIFO B.

A PSG master volume n means `(n + 1) / 8`, so zero is 1/8, not silence.
The reset bits are one-shot strobes, not stored settings, so `SOUNDCNT_H` keeps only mask `770Fh` and they read back zero, as in mGBA.
GBATEK marks `SOUNDBIAS` bit 0 as unused, and Evenfall doesn't store it (mask `C3FEh`), like mGBA.

### How a FIFO works

GBATEK gives each FIFO room for eight 32-bit words ([DMA Sound][gbatek-dma-sound]).
The hardware experiment refines this into **seven queued words plus a separate four-byte word being played**.
Evenfall models exactly that (`DirectSoundFifo`: an eight-slot ring that holds at most seven words, plus the playing word and how many of its bytes are left).

- **Clocking.** Each channel advances on overflows of the timer its `SOUNDCNT_H` bit selects (timer 0 or 1), but only while master sound is on and the channel goes to at least one speaker, as in mGBA ([`timer.c`][mgba-timer-fifo]).
- **Refill, then play.** On each overflow, the channel first asks for DMA if fewer than four words are queued.
  Then, if its playing word is used up and a word is queued, it takes that word.
  It outputs the playing word's lowest remaining byte, and later overflows play the other bytes in order.
  Checking before taking a word means the channel never asks twice between two overflows.
  The threshold follows the hardware experiment and mGBA ([`audio.c`][mgba-audio-fifo]), one word off GBATEK's description.
- **CPU writes.** A 32-bit write queues four bytes.
  An 8- or 16-bit write replaces only the bytes it covers in the word being filled, keeps the others, and then moves on by a whole word.
  It doesn't queue just one or two samples.
- **Overflow.** Writing an eighth word when seven are queued wraps the ring, which then looks empty, as the experiment reported.
- **Underrun.** With nothing playing and nothing queued, Evenfall outputs zero.
  The experiment only inferred what the hardware outputs here.
- **Reset.** The reset strobes clear the queued words but keep the playing word and the current output, as in mGBA ([reset handling][mgba-audio-reset]).

### Sound DMA

For normal playback, a game sets DMA1 or DMA2 to the special start timing with repeat, aimed at `FIFO_A` or `FIFO_B` ([GBATEK][gbatek-sound-dma], Nintendo's patent [US7445551B2][patent-dma-sound]).
Each refill request then moves exactly four 32-bit words: the word count is ignored, the unit is forced to 32 bits, and the destination stays fixed.
mGBA does the same ([request scheduler][mgba-audio-request], [DMA engine][mgba-dma-fifo]).
Everything else is ordinary DMA: priority, bus timing, source stepping, the completion interrupt, and the source address carried from one request to the next ([`timers-dma-interrupts.md`](timers-dma-interrupts.md)).

In Evenfall:

- When such a channel is enabled aimed at a FIFO, `Bus::write_mmio_half` rewrites its control to a fixed destination and 32-bit units, and the control reads back that way.
- A refill request triggers the enabled special-timing channel, DMA1 or DMA2, aimed at that FIFO, unless it is the channel already transferring.
  It takes effect at the timer overflow itself, without the start delay other DMA triggers have, as in mGBA.
- A special-timing DMA1 or DMA2 aimed anywhere else never starts.

The hardware experiment found that the width is forced but the destination isn't: the refill request drove a four-word DMA to an arbitrary address.
mGBA rejects such a destination and GBATEK calls it invalid, so Evenfall doesn't model it.
DMA3 can't feed the FIFOs.

## Mixing and output

For each speaker, the mixer adds up the channels routed to it:

- **Direct Sound:** the current sample (−128 to 127) times 2 at 50% volume, or times 4 at 100%, so each FIFO spans about ±200h.
- **PSG:** each channel's level (0–15) times 8, times that side's master volume (1–8), shifted right by 4, 3 or 2 for the 25%, 50% or 100% ratio.
  Each channel is rounded down on its own.
  At full volume, level v contributes `16v`.

It then adds the `SOUNDBIAS` level and clips to the unsigned 10-bit range `0`–`3FFh` ([GBATEK][gbatek-control], mGBA's [`audio.c`][mgba-audio-mix]).

`SOUNDBIAS` bits 14–15 trade precision for sample rate:

| Resolution  | Output depth | Sample rate | Cycles per sample |
| ----------- | ------------ | ----------- | ----------------- |
| 0 (default) | 9 bits       | 32,768 Hz   | 512               |
| 1           | 8 bits       | 65,536 Hz   | 256               |
| 2           | 7 bits       | 131,072 Hz  | 128               |
| 3           | 6 bits       | 262,144 Hz  | 64                |

GBATEK describes resolution 0 as the 10-bit value divided by two, and the others as dropping two, three or four low bits.
The hardware then sends each value to the speaker as a 1-bit pulse train at the master clock ([GBATEK][gbatek-control]).

Evenfall stops one step before the pulse train and outputs the value at the selected depth.
It keeps samples signed and in 10-bit units: it clears the dropped low bits from both the clipped value and the bias, and outputs the difference.
That keeps the loudness scale while showing the real quantization steps.
mGBA and NanoBoyAdvance instead keep the full 10-bit value and only change the sample rate, which is a reasonable shortcut for host audio but not what the DAC outputs.

## What the host gets

At every sample of the selected rate, the sound chip emits one `StereoSample` (two signed 16-bit values in the units above) into a queue of 2,048 samples.
While master sound is off it emits zeros at the same rate.
This stream is deterministic and independent of the host, so tests can check it sample by sample and hash it.

- `Machine::drain_audio_samples` hands queued samples to the caller.
  A full queue drops its oldest samples, so hosts drain after each bounded run.
- `Machine::audio_sample_rate` reports the current rate, which changes when a game writes `SOUNDBIAS` bits 14–15.
- `Machine::set_audio_output(false)` stops producing samples while every emulated sound, timer and DMA state stays exact.
- `Machine::set_high_speed_audio(true)` is an optional fast-forward mode.
  It silences output and freezes the square and noise oscillators, including the noise register.
  Everything else stays exact, but the saved square and noise phases then differ from normal play, and leaving the mode resumes them without catching up.

Neither setting is saved in states.

Everything after this point belongs to the host: resampling, buffering, latency and the audio device.
[`crates/audio/src/lib.rs`](../../../crates/audio/src/lib.rs) provides a `Resampler` that converts the output rate to the host's with linear interpolation and an exact fractional position, so long sessions don't drift.
It allocates nothing, and reports how many samples it consumed and produced so the caller can keep the rest.
`Session::drain_audio` ([`session.rs`](../../../systems/gba/src/session.rs)) creates a new one when the output rate changes.
There is no band-limited reconstruction.

Evenfall advances the square and noise oscillators lazily.
Their elapsed time is applied only before something could observe them: an output sample while one is audible, a sequencer step, a register write that affects them, or a save state.
The result is the same as advancing them every cycle.
The wave channel is always advanced, because the CPU can see its RAM rotate between samples.

## Save states

A save state ([`design/save-states.md`](../../design/save-states.md)) holds all sound state:

- every register value, the master enable and the bias,
- both FIFO rings with their positions, the playing words and their remaining bytes, and the current Direct Sound samples,
- each channel's oscillator phase, the noise register, and the length, envelope and sweep counters,
- both wave RAM banks and their bank settings,
- the output sample phase and the sequencer's phase and step.

Square and noise channels are saved fully advanced.
The timers and DMA channels that feed the FIFOs are saved with the rest of the bus.

The host sample queue and the output settings aren't saved.
A restored machine starts with an empty queue.
Loading rejects impossible states, such as a sample phase past the sample interval, a sequencer phase past 32,768 cycles, an enabled sweep with an out-of-range counter, or a playing word with more than four bytes left.

## How it is checked

- [`tests/bus.rs`](../../../systems/gba/tests/bus.rs) covers byte writes to frequency registers, the sweep overflow check, writes ignored while sound is off, the stored bits of `SOUNDCNT_H` and `SOUNDBIAS`, 16-bit FIFO writes, the FIFO reset, mixing, clipping and output depth.
- [`apu/tests.rs`](../../../systems/gba/src/apu/tests.rs) covers the length counter and its extra clock, and compares the lazy oscillators with an eager reference.
- [`crates/audio/src/lib.rs`](../../../crates/audio/src/lib.rs) tests the resampler with split buffers, backpressure, upsampling, downsampling, reset and long runs.
- Probe ROMs check the output stream sample by sample:
  - `probe-psg` checks a 50% square wave's exact run lengths and levels.
  - `probe-wave`, `probe-wavechain` and `probe-wavereadback` check the wave channel, the two-bank chain and the rotated read-back.
  - `probe-noise` reimplements GBATEK's noise recurrence and matches 64 steps at any phase in both widths, which proves the taps whatever the starting value.
  - `probe-envelope`, `probe-sweep` and `probe-sweepdown` check the envelope and both sweep directions.
  - `probe-fifo` streams a pattern through FIFO A with DMA1 on timer 0, including the silent sample before the first refill.

## Open questions

- **Noise starting value.** GBATEK and NanoBoyAdvance say 40h or 4000h, mGBA all ones.
  No capture of the first noise samples on a GBA is available.
- **Wave RAM rotation.** The direction rests on GBATEK and two emulators, with no measured read-back.
- **Extra length clock.** Follows mGBA without a measured trace.
- **FIFO reset.** Whether every hardware revision keeps the playing word and current output across a reset needs a focused test.
- **Unrouted Direct Sound channels.** Evenfall, like mGBA, doesn't consume samples for a channel routed to neither speaker.
  The hardware is untested.
- **FIFO underrun.** Evenfall outputs zero.
  The experiment only inferred the hardware's value.
- **Turning sound off and on.** Whether FIFOs survive, and what the first sample after re-enabling is, isn't established.
- **Sound DMA to another destination.** The hardware apparently performs the four-word transfer to any address.
  Evenfall never starts it.
- **Sound DMA control read-back.** Evenfall rewrites a FIFO channel's control bits when it is enabled.
  Whether the hardware changes the readable bits is untested.
- **PSG output level.** Evenfall mixes PSG levels as positive values, so a channel spans 0 to about +F0h at full volume.
  GBATEK describes each channel as spanning ±80h.
  The difference is a constant offset that changes how much room is left before clipping.
- **Prohibited PSG ratio.** Evenfall treats ratio 3 as 100%.

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
