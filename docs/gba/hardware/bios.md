# BIOS: the real one and Evenfall's replacement

Every GBA has a 16 KiB BIOS ROM built in.
It shows the Nintendo logo at power-on, catches interrupts before handing them to the game, and offers a small library of routines that games call with the `SWI` instruction: waiting for the next frame, dividing numbers, copying and decompressing data, setting up rotation matrices, and Nintendo's sound driver.
Almost every game calls it.

The BIOS is Nintendo's copyrighted code, so Evenfall can't ship it.
It offers two ways to run games instead, and never mixes them:

1. **Real BIOS.** The user supplies a 16 KiB BIOS image, and it runs on the emulated CPU like any other code.
   This is exact, but needs a file most users don't have.
2. **HLE (high-level emulation), the default.** Evenfall starts the game where the real boot would leave it, and implements each BIOS routine directly in Rust with the same inputs, outputs, memory effects, errors, timing and interrupt behavior.
   It contains no Nintendo code.
   It is a replacement, and it says so, because some things (the boot animation, reading the BIOS's own bytes) can't be reproduced without the real code.

A game can tell the two apart in small ways, listed [below](#boot-modes).
Well-behaved games run the same on both.

## Where it lives

[`hle/mod.rs`](../../../systems/gba/src/hle/mod.rs) dispatches BIOS calls.
[`hle/bulk.rs`](../../../systems/gba/src/hle/bulk.rs) holds the long-running copy and decompression routines, [`hle/sound.rs`](../../../systems/gba/src/hle/sound.rs) the sound driver, and [`cpu/mod.rs`](../../../systems/gba/src/cpu/mod.rs) keeps the state of a routine that is still running.

GBATEK lists the BIOS routines ([GBATEK, BIOS Functions][gbatek-bios]), but it comes from reverse engineering, not from an official specification.
mGBA 0.10.5 also offers both a real-BIOS path and a built-in replacement ([`bios.c`][mgba-bios], [`hle-bios.s`][mgba-hle-bios]).

## A few terms

- **SWI.** The "software interrupt" instruction a game uses to call a BIOS routine.
  Its operand is the routine number.
- **HLE.** High-level emulation: reproducing what a piece of code does without running the code itself.
- **Trampoline.** The small BIOS routine that runs on every interrupt, saves registers, and calls the game's interrupt handler.
- **Continuation.** The saved state of a BIOS routine that hasn't finished, so it can resume on the next emulation step.
- **MP2K.** Nintendo's standard sound engine for GBA games, partly built into the BIOS.

## Boot modes

| Mode      | `Config::boot`            | CLI             | Starts at                                         | `SWI` and interrupts |
| --------- | ------------------------- | --------------- | ------------------------------------------------- | -------------------- |
| HLE       | `BootMode::Hle` (default) | no `--bios`     | `08000000h`, System mode                          | Evenfall's routines  |
| Real BIOS | `BootMode::ExternalBios`  | `--bios <path>` | the reset vector, Supervisor mode, interrupts off | the image's code     |
| Skip BIOS | `BootMode::SkipBios`      | library only    | `08000000h`, System mode                          | the image's code     |

Skip-BIOS boot uses a real image but jumps past its boot animation.
A save state records the boot mode and the BIOS image's hash, and only loads into a machine with the same pair ([`design/save-states.md`](../../design/save-states.md)).

What a game can see differently:

| What                                            | Real BIOS                                                                     | HLE                                                 |
| ----------------------------------------------- | ----------------------------------------------------------------------------- | --------------------------------------------------- |
| Boot                                            | the image's boot code (Nintendo's shows the logo for about two seconds)       | none: the cartridge starts at cycle 0               |
| Reading the BIOS region                         | the image's bytes ([`memory.md`](memory.md#bios-read-protection))             | the protection value                                |
| `SWI` entry                                     | the image's handler, in Supervisor mode, with stack writes and its own timing | no mode change or stack traffic, approximate timing |
| Interrupt dispatch                              | the trampoline pushes `r0`–`r3`, `r12`, `lr` on the interrupt stack           | registers saved inside the CPU, no stack writes     |
| Using internal BIOS routines as callbacks       | runs them                                                                     | typed fault ([below](#fault-containment))           |
| `MidiKey2Freq` reading BIOS bytes               | reads them                                                                    | impossible, since there are no BIOS bytes           |
| Booting as a MultiBoot client with no cartridge | the BIOS download loop                                                        | not implemented                                     |

## State after HLE boot

HLE starts from where the real boot leaves the machine ([GBATEK, BIOS RAM Usage][gbatek-ram], [GBATEK, SWI 01h][gbatek-reset]):

| State                        | Value                                                          |
| ---------------------------- | -------------------------------------------------------------- |
| PC, CPSR                     | `08000000h`, `0000001Fh`: ARM, System mode, interrupts enabled |
| `r0`–`r12`, `lr`             | 0                                                              |
| `sp_sys`, `sp_irq`, `sp_svc` | `03007F00h`, `03007FA0h`, `03007FE0h`                          |
| `DISPCNT`                    | `0080h` (forced blank)                                         |
| BG2 and BG3 `PA`, `PD`       | `0100h` (no rotation or scaling)                               |
| `SOUNDBIAS`                  | `0200h`                                                        |
| `RCNT`                       | `8000h`                                                        |
| `WAITCNT`, `IE`, `IF`, `IME` | 0                                                              |
| `POSTFLG`                    | 0 ([open questions](#open-questions))                          |
| BIOS protection value        | `E129F000h`                                                    |
| RAM and video memory         | zero                                                           |

`DISPCNT = 0080h` comes from the boot calling `RegisterRamReset`, which always turns on forced blank (GBATEK, SWI 01h).
mGBA sets the same default on every reset ([`io.c`][mgba-io]).

## How HLE runs a BIOS call

An ARM `SWI` takes its routine number from instruction bits 23–16, and a Thumb `SWI` from bits 7–0.
In HLE mode, `Cpu::execute_hle_software_interrupt` ([`hle/dispatch.rs`](../../../systems/gba/src/hle/dispatch.rs)) runs the routine inside the `SWI` instruction itself.
There is no switch to Supervisor mode, the next instruction is the one after the `SWI`, and a pending interrupt is taken at that point like after any instruction.
The BIOS protection value becomes `E3A02004h`.

Every memory access a routine makes goes through the timed bus, so DMA, sound, timers and video see it in order.
Routines whose running time the game controls run over several steps ([below](#routines-that-run-over-several-steps)).
An unassigned routine number stops the machine with an `unsupported_hle_swi` fault.

| SWI          | Routine                                                                                          | Details                                                      |
| ------------ | ------------------------------------------------------------------------------------------------ | ------------------------------------------------------------ |
| `00h`        | `SoftReset`                                                                                      | [resets](#resets)                                            |
| `01h`        | `RegisterRamReset`                                                                               | [resets](#resets)                                            |
| `02h`, `03h` | `Halt`, `Stop`                                                                                   | enter the power state after the routine's cost               |
| `04h`, `05h` | `IntrWait`, `VBlankIntrWait`                                                                     | [interrupt waits](#intrwait-and-the-bios-interrupt-flags)    |
| `06h`, `07h` | `Div`, `DivArm`                                                                                  | [arithmetic](#arithmetic)                                    |
| `08h`        | `Sqrt`                                                                                           | integer square root of `r0`                                  |
| `09h`, `0Ah` | `ArcTan`, `ArcTan2`                                                                              | [arithmetic](#arithmetic)                                    |
| `0Bh`, `0Ch` | `CpuSet`, `CpuFastSet`                                                                           | [copies](#copies), several steps                             |
| `0Dh`        | `GetBiosChecksum`                                                                                | `r0 = BAAE187Fh`, `r1 = 1`, `r3 = 4000h`                     |
| `0Eh`, `0Fh` | `BgAffineSet`, `ObjAffineSet`                                                                    | [rotation setup](#rotation-and-scaling-setup), several steps |
| `10h`        | `BitUnPack`                                                                                      | [unpacking](#bitunpack), several steps                       |
| `11h`, `12h` | `LZ77UnCompWram`, `LZ77UnCompVram`                                                               | [decompression](#decompression), several steps               |
| `13h`        | `HuffUnComp`                                                                                     | [decompression](#decompression), several steps               |
| `14h`, `15h` | `RLUnCompWram`, `RLUnCompVram`                                                                   | [decompression](#decompression), several steps               |
| `16h`–`18h`  | `Diff8bitUnFilterWram`, `Diff8bitUnFilterVram`, `Diff16bitUnFilter`                              | [decompression](#decompression), several steps               |
| `19h`        | `SoundBias`                                                                                      | [sound helpers](#sound-helpers)                              |
| `1Ah`–`1Eh`  | `SoundDriverInit`, `SoundDriverMode`, `SoundDriverMain`, `SoundDriverVSync`, `SoundChannelClear` | [sound driver](#the-mp2k-sound-driver)                       |
| `1Fh`        | `MidiKey2Freq`                                                                                   | [sound helpers](#sound-helpers)                              |
| `20h`–`24h`  | music player open, start, stop, continue, fade out                                               | [sound driver](#the-mp2k-sound-driver)                       |
| `25h`        | `MultiBoot`                                                                                      | [MultiBoot](#multiboot), several steps                       |
| `26h`        | `HardReset`                                                                                      | [resets](#resets)                                            |
| `27h`        | `CustomHalt`                                                                                     | [sound helpers](#sound-helpers)                              |
| `28h`, `29h` | `SoundDriverVSyncOff`, `SoundDriverVSyncOn`                                                      | [sound driver](#the-mp2k-sound-driver)                       |
| `2Ah`        | `SoundGetJumpList`                                                                               | [sound driver](#the-mp2k-sound-driver)                       |

### Arithmetic

The ARM7TDMI has no divide instruction, so games call the BIOS to divide.
`Div` returns the quotient in `r0`, the remainder in `r1` and the absolute quotient in `r3`.
`DivArm` takes its operands the other way round.

Division by zero doesn't trap.
It returns what the real BIOS returns: `80000000h / −1` gives `80000000h` with remainder 0, `0 / 0` gives 1 with remainder 0, and any other division by zero gives ±1 (the sign of the numerator) with the numerator as remainder.

`ArcTan` and `ArcTan2` evaluate the BIOS's fixed-point polynomial, including the values it leaves behind in `r1` and `r3`.
`ArcTan2` covers every quadrant.
The mGBA suite's BIOS math table supplies the expected values, including the polynomial's discontinuities.

### Copies

`CpuSet` copies or fills halfwords or words.
`r2` holds the count (bits 0–20), the fill flag (bit 24) and the word flag (bit 26).
Only the word form writes its address registers back.
`CpuFastSet` copies or fills words in whole bursts of eight, rounding the count up.

Both return after one cycle without copying when the source starts below `02000000h`.
GBATEK documents that the real routines refuse any source reaching into the BIOS ([GBATEK, BIOS Memory Copy][gbatek-copy]).

### Rotation and scaling setup

`BgAffineSet` and `ObjAffineSet` turn a scale, an angle and a center into the matrices that rotate backgrounds and sprites ([`video.md`](video.md#affine-backgrounds)).
They follow GBATEK's signed 8.8 layouts:

- `BgAffineSet` reads 20-byte records and writes 16-byte records with `PA`–`PD` and the reference point.
- `ObjAffineSet` reads 8-byte records and writes four parameters at a caller-chosen stride, such as OAM's.

The low byte of the angle is ignored.
The math uses mGBA 0.10.5's HLE formulas in single precision ([approximations](#approximations)).
A result too large for its field, such as a scale of −128 rotated by 180 degrees, keeps its low bits as integer arithmetic would, instead of trapping.
That is Evenfall's choice, not a claim about the real routines.

### BitUnPack

`BitUnPack` expands packed pixel data, for example 1-bit font data into 4-bit tiles.
Its parameter block gives the source length, the source width (1, 2, 4 or 8 bits), the destination width (1 to 32 bits) and a bias.
It reads source units low bits first and packs them into 32-bit words.
The bias is added to non-zero units, or to every unit when bias bit 31 is set.
Unsupported widths return without writing.

### Decompression

Games store graphics compressed in the cartridge and decompress them with the BIOS.
Every decompressor reads a 32-bit header whose bits 8–31 give the output length.
The `Vram` variants write whole halfwords, because a byte write to VRAM lands in both halves ([`memory.md`](memory.md#region-behavior)).
The `Wram` variants write bytes.
The formats, overlap behavior, VRAM halfword buffering, padding and timing follow mGBA 0.10.5's replacement BIOS.

- **LZ77** expands literal bytes and back-references, with a flag byte per eight tokens.
  Overlapping references copy byte by byte, and VRAM output reads its back-references as halfwords.
- **Huffman** walks a tree stored before the bitstream ([GBATEK, decompression][gbatek-decomp]).
  Children are found from the current node's aligned address, flags mark leaf nodes, the bitstream is read most significant bit first, and 4-bit or 8-bit symbols are packed into 32-bit writes, low first.
  A symbol width of zero means eight, as in mGBA, and other widths return without writing.
  Output is written in whole words, so an output shorter than four bytes still writes one padded word.
  The bitstream starts after a tree of `(size + 1) * 2` bytes, so an even tree size leaves it only halfword-aligned.
  Its words are then read as an ARM `LDR` would read them, rotated by 16 bits, as mGBA's HLE does.
- **Run-length** expands literal and repeated blocks.
  In VRAM output, bytes pair into halfwords by destination address.
  An odd destination therefore starts with a halfword whose low byte is zero, and the end is padded only as far as needed.
- **Differential filters** undo 8-bit or 16-bit delta encoding.
  The 8-bit VRAM form packs byte pairs into halfwords.

The Cult-of-GBA replacement BIOS ([`running_length.s`](https://github.com/Cult-of-GBA/BIOS/blob/a30e9a96df083628b650724b7d4d7112b4070b98/bios_calls/decompression/running_length.s)) handles run-length VRAM output differently.
It writes every block whole, pairs bytes by count rather than address, and pads nothing, and its comments say the original BIOS does the same.
Which one matches the real routine is unverified.

Broken Huffman trees aren't repaired.
A tree that never reaches a leaf keeps the game inside the routine forever, as it would on a GBA, but the host still gets control back after every step.
Compressed data from untrusted sources shouldn't be treated as a safe input format.

### Resets

The reset routines follow GBATEK ([GBATEK, BIOS Reset Functions][gbatek-reset]) and mGBA 0.10.5.

**`SoftReset`** restarts the game without clearing most memory.
It reads the flag byte at `03007FFAh`, clears `03007E00h`–`03007FFFh`, zeros `r0`–`r12` and the interrupt and Supervisor `lr` and SPSR, sets up the three stacks, enters ARM System mode, and jumps to `08000000h` (flag zero) or `02000000h` (flag non-zero).

**`RegisterRamReset`** clears what the bits of `r0` select.
It always turns on forced blank (`DISPCNT = 0080h`), and always clears the low half of `SIODATA32`, a bug in the real BIOS that GBATEK documents.

| `r0` bit | Clears                                                                         |
| -------- | ------------------------------------------------------------------------------ |
| 0        | EWRAM                                                                          |
| 1        | IWRAM except its last `200h` bytes                                             |
| 2        | palette RAM                                                                    |
| 3        | VRAM                                                                           |
| 4        | OAM                                                                            |
| 5        | serial registers, cancelling transfers, UART queues and pending JoyBus traffic |
| 6        | sound registers and wave RAM                                                   |
| 7        | all other video, DMA, timer, interrupt and wait-state registers                |

**`HardReset`** reboots the console and never returns.
HLE clears all RAM, video memory, CPU registers, DMA, timer, interrupt and prefetch state, serial transfers and audio queues, then restores the boot state (with `POSTFLG = 1`) and jumps to `08000000h` in ARM System mode.
Keys, the cartridge's save and devices like its clock stay powered.
The logo sequence and its two seconds are skipped.
The real-BIOS mode is the exact path for them.

The timer prescaler keeps running through both software resets ([`timers-dma-interrupts.md`](timers-dma-interrupts.md)).

### Sound helpers

These routines sit next to the sound driver but aren't part of it.

- **`SoundBias`** ramps the `SOUNDBIAS` level to `000h` (`r0 = 0`) or `200h`, one unit at a time, eight cycles apart (GBATEK's delay), keeping the resolution bits.
  Each step is a real register write, so the sound chip hears the level change gradually instead of jumping.
- **`MidiKey2Freq`** converts a MIDI key into a sample rate.
  It reads the sample's base rate at `r0 + 4` and returns `freq / 2^((180 − key − fine / 256) / 12)`, with the key in `r1` and the fine pitch in `r2`, using mGBA 0.10.5's single-precision formula.
- **`CustomHalt`** enters Stop when `r2` bit 7 is set, and Halt otherwise.

### `IntrWait` and the BIOS interrupt flags

`IntrWait` and `VBlankIntrWait` are how most games wait for the next frame.
They sleep in Halt until a chosen interrupt has been handled, not just raised.

They don't look at `IF` directly.
The game's interrupt handler must also set each handled flag in a halfword at `03007FF8h`, the BIOS interrupt flags, and the wait ends when a chosen bit appears there ([GBATEK, IntrWait][gbatek-halt]).
`IntrWait` takes a discard flag in `r0` and the interrupt mask in `r1`.
`VBlankIntrWait` is the same with `r0 = r1 = 1`.

HLE keeps the wait as state that lasts across interrupt handler calls:

1. The routine sets `IME = 1`, as the real one does.
2. If `r0` is non-zero, it first clears the chosen bits at `03007FF8h`.
3. If a chosen bit is already set there, it clears it and returns without sleeping.
   Otherwise it remembers the mask and enters Halt.
4. Each time the interrupt dispatcher returns from the outermost handler, it checks `03007FF8h` again.
   A chosen bit ends the wait and is cleared.
   Otherwise the CPU halts again.

Halt wakes on `IE & IF` whatever `IME` and the CPU's interrupt mask say, while entering the handler still obeys both ([`timers-dma-interrupts.md`](timers-dma-interrupts.md)).

### The HLE interrupt dispatcher

On a GBA, an interrupt jumps to the BIOS trampoline at `18h`.
It saves `r0`–`r3`, `r12` and `lr` on the interrupt stack, calls the game's handler (whose address is at `03007FFCh`) with `lr = 00000138h`, and returns with `SUBS pc, lr, #4` ([GBATEK, GBA Interrupt Control][gbatek-irq]).

In HLE mode, `Cpu::enter_irq` does the CPU's normal interrupt entry, then:

- pushes a frame (return address, `SPSR_irq`, `r0`–`r3`, `r12`) onto an eight-entry stack inside the CPU,
- reads the handler address from `03007FFCh` with a timed read,
- sets `lr` to `00000138h` and jumps to the handler, in Thumb state when bit 0 of the address is set,
- sets the BIOS protection value to `E25EF004h`.

When execution reaches `00000138h` with a frame on the stack, the dispatcher pops the frame, restores `r0`–`r3`, `r12` and that frame's `SPSR_irq`, returns to the interrupted code, and sets the protection value to `E55EC002h`.

Handlers may re-enable interrupts and nest.
Each level has its own frame, so an inner interrupt can't overwrite the outer one's return state.
Only the return to the outermost level checks a pending `IntrWait`, so an outer handler can still set its bit at `03007FF8h` before the wait is checked.
A ninth nested interrupt stops the machine with an `hle_irq_nesting_limit` fault before it changes anything.

## Routines that run over several steps

Some routines run for as long as the game's arguments say: a 32-bit record count, a 21-bit copy count, a 24-bit output length, or a Huffman tree that never ends.
On a GBA that time passes inside BIOS code while video, sound, timers and DMA keep running.
An HLE routine that ran to completion inside one instruction could block the host for minutes, or forever.

So these routines run as continuations, stored in one CPU slot (`Cpu.hle_service`):

- **MultiBoot** advances one protocol phase per step while the link cable carries each exchange ([below](#multiboot)).
- **Bulk routines** (`CpuSet`, `CpuFastSet`, `BgAffineSet`, `ObjAffineSet`, `BitUnPack` and every decompressor) keep all the values their loop carries (addresses, counts, bit buffers, a half-built VRAM halfword, the current Huffman node) in an `HleBulkWork` value.

A step spends at most `hle_bulk_step_cycles`, 2^20 guest cycles (62.5 ms, about 3.7 frames), in a bulk routine, then pauses at the next loop iteration.
Huffman pauses only between bitstream words, each of which walks at most 32 tree nodes.
The next step resumes it.
The budget is well above the longest calls seen in commercial games, so ordinary calls finish in the step that runs the `SWI`.

The game can't tell that a routine paused:

- **Pauses depend only on guest time.** Where a step stops depends only on the cycles the routine has spent, never on a host deadline.
  Every way of slicing time makes the same bus accesses at the same cycles.
- **No instruction boundary.** No interrupt is taken and nothing else runs between steps.
  The step that finishes the routine does the `SWI`'s usual interrupt check, at the cycle an uninterrupted routine would have ended.
- **Power states wait.** A Halt or Stop that the routine's own writes request takes effect once it returns.
- **The CPU looks consistent.** The next instruction is the one after the `SWI`, the registers hold the `SWI`'s inputs until it returns, and every step reports the `SWI` as the instruction it ran.
- **Save states.** A save state includes the continuation, and loading rejects loop states the routines can't reach, such as an unaligned copy pointer.

What hosts see: `Machine::step` advances such a routine by one step, `Machine::run_slice` returns at the first step past its deadline (overshooting by at most one budget), and the CLI's `--steps` and `--trace` count steps, so a long call shows up as repeated records of its `SWI`.

## MultiBoot

`MultiBoot` (`25h`) sends a program over the link cable to up to three other GBAs that have no cartridge, so several players can share one game ([GBATEK, BIOS Multi Boot][gbatek-multiboot]).

`r0` points to a parameter block: handshake byte at `14h`, client bytes at `19h`–`1Bh`, palette byte at `1Ch`, client bits at `1Eh`, and the program's start and end at `20h` and `24h`.
`r1` selects the transfer mode: 0 for Normal at 256 KHz, 1 for Multiplayer at 115 KHz, 2 for Normal at 2 MHz.
The routine checks the mode, that at least one client bit within `0Eh` is set and no others, and that the program is `100h` to `3FF40h` bytes in 16-byte units.
A malformed call returns `r0 = 1`.

The protocol itself (the length command, the Normal and Multiplayer encryption streams and the CRC) is plain code in [`multiboot.rs`](../../../systems/gba/src/multiboot.rs), checked against published test vectors.
The continuation drives the real serial registers through the timed bus.
It waits, sends the length command and collects each client's random byte, sends the encrypted program and checks every echo, waits for the clients to answer `75h`, and exchanges the final CRC.
Each exchange polls the start bit every 64 cycles and gives up after 2^24 cycles, and transfers are 604 cycles apart.
The routine returns `r0 = 0` on success and 1 on any failure.

HLE can only be the sender.
It lacks the BIOS's download loop for a GBA with no cartridge, so HLE-only machines can't play a single-cartridge multiplayer game.
Working with a client running the real BIOS needs a BIOS image and hasn't been tested.

## The MP2K sound driver

Most commercial GBA games use Nintendo's MP2K sound engine, sometimes called the Sappy driver.
Part of it is built into the BIOS: routines `1Ah`–`1Eh`, `20h`–`24h` and `28h`–`2Ah`.
The game calls them to set up sound, start and stop songs, and, once per frame, to advance the music and mix the next buffer of samples.

The driver keeps its state in a block of RAM called the SoundArea, whose address the BIOS publishes at `03007FF0h`.
The word at its start reads `Smsh` (`68736D53h`) when idle, and one more while a routine holds the lock.
HLE reproduces the driver's visible behavior with timed bus accesses throughout, so FIFO resets, DMA, timer 0, video and the sound chip see the same sequence as with the real routines.

### Setup and control

- **`SoundDriverInit`** stops the old sound DMA, turns sound on and routes it, sets the output resolution, points the two sound DMA channels at the SoundArea's mix buffers, publishes the SoundArea address, clears the `FB0h`-byte area with `CpuSet`, installs the default callbacks, sets up mode 4's playback rate, and finally writes the `Smsh` identifier.
- **`SoundDriverMode`** applies the reverb, channel count, master volume and output resolution.
  To change the playback rate it looks up the real BIOS's 15-entry rate table, computes the SoundArea timing fields and timer 0's reload, restarts DMA, and starts timer 0 when the display reaches line 159.
- **`SoundDriverVSync`**, called once per frame, counts down.
  Values above one only decrement.
  At zero or one it reloads from `SoundArea + 0Bh` and restarts DMA1 and DMA2 by writing 0 and then `B600h` to their controls.
- **`SoundDriverVSyncOn`** writes `B600h` directly.
  **`SoundDriverVSyncOff`** stops DMA1 and DMA2, clears the countdown, and clears the mix buffer at `SoundArea + 350h`–`FAFh`.
- **`SoundChannelClear`** clears all twelve 64-byte software channels and reproduces the values the real routine leaves in `r0`–`r2`.
  If the SoundArea has an extension, it calls the game's callback four times with the real routine's arguments, then restores the caller's state.

### Mixing

**`SoundDriverMain`** does the per-frame work.
It takes the lock, works out an optional time budget from `VCOUNT`, calls the music-player callback at `SoundArea + 20h` and then the Game Boy-channel callback at `+ 28h`, and mixes.
Each callback address is read once, with a timed read, and that value is what gets called, even if HBlank DMA rewrites the word in the meantime.

The mixer picks the current slice of the DMA buffer, clears it or applies the four-tap reverb, advances each channel's attack, decay, sustain, release and echo envelopes, works out left and right volumes, and mixes 8-bit samples at a fixed rate or with linear interpolation, handling sample ends and loops.

### Music players

Routines `20h`–`24h` open, start, stop, continue and fade out a music player, the names mGBA's HLE uses.
They were identified from the real BIOS routines at `13C4h`, `1434h`, `14C0h`, `14FCh` and `1514h`.

- **Open** clears the 64-byte player, limits the track count to 16, clears the 80-byte track records, links the player into the SoundArea, and installs the real player-main callback address `2149h`.
- **Start** installs the song, instruments, priority, tempo and command pointers.
- **Stop** releases each active track's channels.
- **Continue** clears the pause bit.
- **Fade out** sets the fade interval and counter, with volume `100h`.

When `SoundDriverMain` finds the callback `2149h`, HLE runs the MP2K sequencer itself.
It walks the chain of up to 16 players in the real order, and respects each player's lock, pause, fade, tempo, active tracks and clock.
It can also step aside while a game's own wrapper callback runs, and resume afterwards.
For each track it decodes the song data (running status, the clock table, every standard command, notes, waits, note ends and vibrato), picks instruments (direct, rhythm and key-split), assigns software or Game Boy channels by priority, and leaves the rendering to the mixer.

**`SoundGetJumpList`** (`2Ah`) writes the 36 function pointers the driver expects.
HLE has no real code to point at, so it hands out Thumb addresses in a private range at `3E00h`–`3E8Fh`, one for each of the 30 command-table entries and six helpers.
Calling them works like calling the real functions, and `SoundDriverMain` accepts exactly these as callbacks.
The range only exists in HLE mode.
A real BIOS provides its own table.

### Sources and limits

The SoundArea layout comes from the real BIOS routines, read in a public GBA BIOS disassembly (commit `24efa52ac04fedcba4a8b952b76454d3b11a6fa0`), and from [VisualBoyAdvance-M][vbam]'s control flow.
mGBA 0.10.5 and the [SAT-R/sa2][sa2] decompilation cross-check the channel layout and the mixer.
The Cult-of-GBA replacement BIOS turns `SoundDriverMain` into a no-op, so it isn't a reference here.
Where VisualBoyAdvance-M disagrees with the real instructions, Evenfall follows the instructions: in the reverb shift, in where it writes the output resolution, and in the value `SoundDriverVSyncOn` writes (`B600h`, not VBA-M's `8600h`).

Some games point callbacks at internal BIOS code, or replace parts of the driver with their own functions.
HLE has no BIOS code to run for them, so they stop the machine with a typed fault instead of silently doing nothing ([fault containment](#fault-containment)).
The real-BIOS mode runs such games exactly.

Each sound routine charges a fixed cost plus its bus accesses, not the real instruction timing.
The sequencer and mixer run inside one step, and only pause around the game's callbacks.

## Fault containment

A game can ask the HLE BIOS for something it can't do faithfully: an unassigned routine, an internal BIOS address as a callback, malformed sound data, or a pointer that would break a bus rule.
None of these crashes the host or pretends to succeed.
Each stops the machine with a typed, sticky `Fault` that records the cycle and the instruction ([architecture](../../architecture.md#time-and-faults)).
A stopped machine refuses to write a save state, and loading an earlier valid state clears the fault.
The CLI prints one stable line and still saves the cartridge's save data.

| Fault reason                            | Raised when                                                                                              |
| --------------------------------------- | -------------------------------------------------------------------------------------------------------- |
| `unsupported_hle_swi`                   | the routine number isn't assigned                                                                        |
| `hle_irq_nesting_limit`                 | a ninth nested interrupt would start, before anything changes                                            |
| `invalid_hle_memory_access`             | a pointer from the game would make a misaligned word access to the I/O registers                         |
| `sound_channel_clear_internal_callback` | a `SoundChannelClear` extension callback points into the BIOS                                            |
| `sound_driver_main_internal_callback`   | a `SoundDriverMain` callback points into the BIOS, and isn't an HLE jump-list address or a known default |
| `mp2k_internal_callback`                | a chained player's callback points into the BIOS and isn't the player main `2149h`                       |
| `mp2k_replacement_callback`             | the game's command table holds a function other than the HLE jump-list entry                             |
| `mp2k_player_chain_limit`               | more than 16 players are chained                                                                         |
| `mp2k_missing_running_status`           | a track data byte arrives before any status byte                                                         |
| `track_stop_cgb_callback`               | stopping a Game Boy channel would call a non-default callback                                            |
| `sample_freq_set_mode_zero`             | `SampleFreqSet` is called with mode 0                                                                    |

**Checking pointers first.** The bus requires word accesses to I/O registers to be aligned.
The CPU guarantees that for the game's own loads and stores, but HLE routines use raw pointers from the game.
So they check them before any time passes or anything changes.
The rotation, unpacking, decompression, sound-driver, music-player, MultiBoot and jump-list routines all do.

`BgAffineSet` checks all its records at once instead of walking a 32-bit count.
Records advance by 20 source and 16 destination bytes, both multiples of four, so every word in a stream has the same alignment as the first, and a misaligned stream first reaches the I/O registers within one record of its start.
The first faulting record matches a record-by-record check, including wrapping past the top of memory.
`HuffUnComp` and `BitUnPack` only know from their data how many words they will write, so they check each word as they write it, and the fault comes after any earlier output.

This protects the host.
It says nothing about how the real BIOS treats bad arguments.

## Approximations

HLE aims for exact visible effects, not the real BIOS's instruction timing.
Software that needs exact BIOS timing or rounding should use a real BIOS image.

- **Timing.** Most routines charge a fixed 45 cycles on top of their own timed bus accesses.
  The decompressors and `BitUnPack` add per-iteration costs modeled on mGBA's replacement BIOS.
  `Div` charges `11 + 13 * n` cycles, where n is the difference between the leading-zero counts of divisor and dividend (at least 1).
  `Sqrt` charges 53, `ArcTan` 37 plus its multiplications, and `IntrWait` 12.
- **Interrupt dispatch** charges the handler address read and the pipeline refills, but not the trampoline's instructions or its stack writes.
- **Rounding.** `BgAffineSet`, `ObjAffineSet` and `MidiKey2Freq` use single-precision math, as mGBA's HLE does, instead of the real BIOS's lookup table.
- **Resets** take a fixed cost and skip the logo.
- **MultiBoot delays** (2^20 cycles before the first exchange, 64-cycle polls, 604 cycles between transfers, a 2^24-cycle timeout) are Evenfall's choices, not measurements.

## Legal boundary

Evenfall ships no Nintendo BIOS, commercial ROM, key or extracted asset, in the repository or in releases.
Its tests use homebrew ROMs and its own probe ROMs.
Users supply any content they are entitled to use, and BIOS images, ROMs and saves stay out of Git.

The HLE BIOS is original code, with a recorded source for every behavior.
It was cross-checked against a public disassembly but contains no BIOS bytes, which is also why it can't reproduce behavior that depends on reading them.
Copying mGBA's code, rather than studying its behavior, would carry its [MPL-2.0 obligations][mgba-license], and Evenfall copies none.

Real-BIOS testing uses the open Cult-of-GBA replacement BIOS, pinned in [`refs/README.md`](../../../refs/README.md), never Nintendo's.
Passing with it shows that the real-BIOS boot path and the bus work.
A difference from what Nintendo's BIOS would do is a property of the replacement, not of the hardware.

Users should also know the law's limits.
In the United States, the Copyright Office explains that the archival-copy rule of 17 U.S.C. §117 applies only to the owner of a copy, with conditions, and isn't a blanket "ROMs are legal" rule ([Copyright Office FAQ][copyright-faq]).
Rules on copying and copy protection vary by country.

## How it is checked

- [`tests/machine_a.rs`](../../../systems/gba/tests/machine_a.rs) covers the boot state, `IntrWait` completing only after the handler sets the BIOS flags, nested interrupts keeping the outer return state, and `RegisterRamReset`'s serial reset.
- [`tests/machine_b.rs`](../../../systems/gba/tests/machine_b.rs) covers each decompression format (including the halfword-aligned Huffman bitstream and odd run-length VRAM destinations), routines that run over several steps (a Huffman tree with no leaf, interrupts raised inside a routine, the same result however time is sliced), the nesting-limit fault, pointer checks before side effects, the MP2K sequencer starting a note, the callback read once across HBlank DMA, and the 36-entry jump list.
- [`hle/mod.rs`](../../../systems/gba/src/hle/mod.rs) checks `ArcTan` against the mGBA suite's BIOS math table, and the all-at-once `BgAffineSet` pointer check against a record-by-record loop.
- The probes `probe-mb2slave` (Multiplayer, two clients) and `probe-mb2mhz` (Normal 32-bit at 2 MHz) run the HLE MultiBoot sender against client ROMs that behave like the client BIOS, and check every word with their own decryption ([`probe/README.md`](../../../probe/README.md)).
- The [validator](../../../systems/gba/tests/validator/main.rs)'s `--bios` option reruns the jsmolka test ROMs with a real BIOS image ([`testing.md`](../../testing.md#rom-validators)).

## Open questions

- **`POSTFLG` after HLE boot.** HLE boot leaves it at 0, while GBATEK says the real BIOS sets it to 1 after the first reset.
  HLE `HardReset` does set 1.
- **No run with Nintendo's BIOS.** Nobody on the project has run a Nintendo BIOS image, so compatibility with one isn't claimed.
- **Run-length VRAM output.** Pairing by address (Evenfall) or by count without padding (Cult-of-GBA): which one the real routine does is unverified.
- **Interrupt handler state.** GBATEK says the handler is called in ARM state.
  HLE calls an odd handler address in Thumb state, and doesn't write the trampoline's stack frame.
- **Interrupts inside routines.** GBATEK notes that the BIOS may allow interrupts inside some routines.
  HLE takes none until a routine returns.
- **MP2K work inside one step.** Reading the code suggests that a looping channel chain, or a track that jumps without ever waiting, would never return control to the host.
  This hasn't been reproduced.
- **Unsupported driver uses.** Custom replacement MP2K functions, non-default callbacks and BIOS-internal callbacks stop the machine instead of running.
- **Where the sound routines come from.** `SoundBias`, `MidiKey2Freq` and the music players are based on GBATEK, replacement BIOSes and the disassembly, not on an official specification.
- **MultiBoot client side.** HLE can't boot as a client with no cartridge, and working with a real-BIOS client is untested.

[gbatek-bios]: https://rust-console.github.io/gbatek-gbaonly/#biosfunctions
[gbatek-ram]: https://rust-console.github.io/gbatek-gbaonly/#biosramusage
[gbatek-reset]: https://rust-console.github.io/gbatek-gbaonly/#biosresetfunctions
[gbatek-decomp]: https://rust-console.github.io/gbatek-gbaonly/#biosdecompressionfunctions
[gbatek-halt]: https://rust-console.github.io/gbatek-gbaonly/#bioshaltfunctions
[gbatek-irq]: https://rust-console.github.io/gbatek-gbaonly/#gbainterruptcontrol
[gbatek-multiboot]: https://rust-console.github.io/gbatek-gbaonly/#biosmultibootsinglegamepak
[gbatek-copy]: https://rust-console.github.io/gbatek-gbaonly/#biosmemorycopy
[vbam]: https://github.com/visualboyadvance-m/visualboyadvance-m/tree/7d902083ca8657f317b7d8bc863d30ea852a79b0
[sa2]: https://github.com/SAT-R/sa2/tree/393357f7c9d46b5c10b6c41a57b80fe3b58f3c8d
[mgba-bios]: https://github.com/mgba-emu/mgba/blob/0.10.5/src/gba/bios.c
[mgba-hle-bios]: https://github.com/mgba-emu/mgba/blob/0.10.5/src/gba/hle-bios.s
[mgba-io]: https://github.com/mgba-emu/mgba/blob/0.10.5/src/gba/io.c#L334-L348
[mgba-license]: https://github.com/mgba-emu/mgba/blob/0.10.5/LICENSE
[copyright-faq]: https://www.copyright.gov/help/faq/faq-digital.html
