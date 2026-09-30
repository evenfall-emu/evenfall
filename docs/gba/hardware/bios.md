# BIOS: external image and HLE

The GBA's 16 KiB BIOS ROM boots the console, holds the exception vectors and
the IRQ trampoline, and provides services through `SWI`: waits, arithmetic,
block copies, affine setup, decompression, resets, MultiBoot and Nintendo's
sound driver. Software can observe its startup state, the protected-read
latch, register clobbers, memory effects and timing, so Evenfall, like ZiGBA, offers two
explicit modes and never blurs them:

1. **External BIOS.** A user-supplied 16 KiB image runs through the ordinary
   CPU and bus, exactly like cartridge code.
2. **HLE.** A port of ZiGBA's original replacement, the default, starts from the
   post-boot state and implements each service in Rust with its documented
   inputs, outputs, memory effects, errors, timing and interrupt behavior. It
   contains no Nintendo code and is labeled as a replacement, because it
   cannot reproduce Nintendo's boot sequence exactly.

[`hle/mod.rs`](../../../systems/gba/src/hle/mod.rs) dispatches services;
[`hle/bulk.rs`](../../../systems/gba/src/hle/bulk.rs) owns multi-step work,
[`hle/sound.rs`](../../../systems/gba/src/hle/sound.rs) the sound driver, and
[`cpu/mod.rs`](../../../systems/gba/src/cpu/mod.rs) owns the continuation.

GBATEK catalogs the retail services ([GBATEK, BIOS Functions][gbatek-bios]),
but it derives from reverse engineering, not from an official BIOS ABI.
mGBA 0.10.5 demonstrates the same split with an external-BIOS path and a
built-in replacement ([`bios.c`][mgba-bios], [`hle-bios.s`][mgba-hle-bios]).

## Boot modes

| Mode          | `Machine.Config.boot` | CLI             | Start                                             | SWI and IRQ      |
| ------------- | --------------------- | --------------- | ------------------------------------------------- | ---------------- |
| HLE           | `.hle` (default)      | no `--bios`     | `08000000h`, System mode                          | ZiGBA's services |
| External BIOS | `.external_bios`      | `--bios <path>` | reset vector, Supervisor mode, IRQ and FIQ masked | the image's code |
| Skip BIOS     | `.skip_bios`          | library only    | `08000000h`, System mode                          | the image's code |

The [`validator`](../../../systems/gba/tests/validator/main.rs) `--bios <path>`
option reruns the jsmolka suites under external boot (see ZiGBA's
[`testing.md`](../../../refs/zigba/docs/testing.md)). A save state records the boot
mode and the BIOS image's hash and loads only into a machine with the same
pair ([`design/save-states.md`](../../design/save-states.md)).

What a guest can tell apart:

| Observable                          | External BIOS                                                                        | HLE                                                 |
| ----------------------------------- | ------------------------------------------------------------------------------------ | --------------------------------------------------- |
| Boot                                | the image's boot code (the retail one shows the Nintendo logo for about two seconds) | none: the cartridge starts at cycle 0               |
| BIOS reads                          | image bytes ([`memory.md`](memory.md#bios-read-protection))                          | the protection latch                                |
| SWI entry                           | the image's handler: Supervisor-mode entry, stack writes, its own instruction timing | no mode change or stack traffic; approximate timing |
| IRQ dispatch                        | BIOS trampoline pushes `r0`–`r3`, `r12`, `lr` on the IRQ stack                       | saved registers kept in the CPU; no stack writes    |
| Internal BIOS routines as callbacks | execute                                                                              | typed fault ([below](#fault-containment))           |
| `MidiKey2Freq` BIOS-read loophole   | reads BIOS bytes                                                                     | impossible: no BIOS bytes exist                     |
| Cartridge-less MultiBoot client     | BIOS download loop                                                                   | not implemented                                     |

## Post-boot state

HLE mode starts from this state, modeled on where the retail boot leaves the
machine ([GBATEK, BIOS RAM Usage][gbatek-ram];
[GBATEK, SWI 01h][gbatek-reset]):

| State                        | Value                                                           |
| ---------------------------- | --------------------------------------------------------------- |
| PC, CPSR                     | `08000000h`, `0000001Fh`: ARM, System mode, IRQ and FIQ enabled |
| `r0`–`r12`, `lr`             | 0                                                               |
| `sp_sys`, `sp_irq`, `sp_svc` | `03007F00h`, `03007FA0h`, `03007FE0h`                           |
| `DISPCNT`                    | `0080h` (forced blank)                                          |
| BG2/BG3 affine `PA`, `PD`    | `0100h` (identity)                                              |
| `SOUNDBIAS`                  | `0200h`                                                         |
| `RCNT`                       | `8000h`                                                         |
| `WAITCNT`, `IE`, `IF`, `IME` | 0                                                               |
| `POSTFLG`                    | 0 (see [open questions](#open-questions))                       |
| BIOS protection latch        | `E129F000h`                                                     |
| RAM and video memory         | zero                                                            |

`DISPCNT = 0080h` follows from the boot calling `RegisterRamReset`, which
always forces blank (GBATEK SWI 01h); pinned mGBA installs the same default on
every reset path ([`io.c`][mgba-io]). The test "HLE boot exposes the post-BIOS
System stack pointer and forced blank"
([ZiGBA's `machine_test.zig`](../../../refs/zigba/test/machine_test.zig)) pins it.

## How HLE runs a service

An ARM `SWI` takes its service number from instruction bits 23–16; a Thumb
`SWI` from bits 7–0. In HLE mode `Cpu::execute_hle_software_interrupt` in
[`hle/dispatch.rs`](../../../systems/gba/src/hle/dispatch.rs) runs the service
inside the `SWI` instruction itself: there is no Supervisor-mode
entry, the next instruction is the one after the `SWI`, the protection latch
becomes `E3A02004h`, and a pending IRQ is taken at that boundary as after any
other instruction. Every memory access a service makes goes through the timed
bus, so DMA, FIFOs, timers and video observe it in order. Services whose
length the guest controls run across several steps
([below](#services-that-span-several-steps)). An unassigned service number
stops with an `unsupported_hle_swi` fault.

| SWI          | Service                                                                                          | HLE notes                                            |
| ------------ | ------------------------------------------------------------------------------------------------ | ---------------------------------------------------- |
| `00h`        | `SoftReset`                                                                                      | [reset services](#reset-services)                    |
| `01h`        | `RegisterRamReset`                                                                               | [reset services](#reset-services)                    |
| `02h`, `03h` | `Halt`, `Stop`                                                                                   | enter the power state after the service charge       |
| `04h`, `05h` | `IntrWait`, `VBlankIntrWait`                                                                     | [interrupt waits](#intrwait-and-the-bios-irq-mirror) |
| `06h`, `07h` | `Div`, `DivArm`                                                                                  | [arithmetic](#arithmetic)                            |
| `08h`        | `Sqrt`                                                                                           | integer square root of `r0`                          |
| `09h`, `0Ah` | `ArcTan`, `ArcTan2`                                                                              | [arithmetic](#arithmetic)                            |
| `0Bh`, `0Ch` | `CpuSet`, `CpuFastSet`                                                                           | [copies](#copies), multi-step                        |
| `0Dh`        | `GetBiosChecksum`                                                                                | `r0 = BAAE187Fh`, `r1 = 1`, `r3 = 4000h`             |
| `0Eh`, `0Fh` | `BgAffineSet`, `ObjAffineSet`                                                                    | [affine setup](#affine-setup), multi-step            |
| `10h`        | `BitUnPack`                                                                                      | [unpacking](#bitunpack), multi-step                  |
| `11h`, `12h` | `LZ77UnCompWram`, `LZ77UnCompVram`                                                               | [decompression](#decompression), multi-step          |
| `13h`        | `HuffUnComp`                                                                                     | [decompression](#decompression), multi-step          |
| `14h`, `15h` | `RLUnCompWram`, `RLUnCompVram`                                                                   | [decompression](#decompression), multi-step          |
| `16h`–`18h`  | `Diff8bitUnFilterWram`, `Diff8bitUnFilterVram`, `Diff16bitUnFilter`                              | [decompression](#decompression), multi-step          |
| `19h`        | `SoundBias`                                                                                      | [sound utilities](#sound-utilities)                  |
| `1Ah`–`1Eh`  | `SoundDriverInit`, `SoundDriverMode`, `SoundDriverMain`, `SoundDriverVSync`, `SoundChannelClear` | [sound driver](#the-mp2k-sound-driver)               |
| `1Fh`        | `MidiKey2Freq`                                                                                   | [sound utilities](#sound-utilities)                  |
| `20h`–`24h`  | music player open, start, stop, continue, fade out                                               | [sound driver](#the-mp2k-sound-driver)               |
| `25h`        | `MultiBoot`                                                                                      | [MultiBoot](#multiboot), multi-step                  |
| `26h`        | `HardReset`                                                                                      | [reset services](#reset-services)                    |
| `27h`        | `CustomHalt`                                                                                     | [sound utilities](#sound-utilities)                  |
| `28h`, `29h` | `SoundDriverVSyncOff`, `SoundDriverVSyncOn`                                                      | [sound driver](#the-mp2k-sound-driver)               |
| `2Ah`        | `SoundGetJumpList`                                                                               | [sound driver](#the-mp2k-sound-driver)               |

### Arithmetic

`Div` returns the quotient in `r0`, the remainder in `r1` and the absolute
quotient in `r3`; `DivArm` swaps the operands. Two edge cases return the
retail results rather than trapping: `80000000h / −1` gives `80000000h`
remainder 0, and `0 / 0` gives 1 remainder 0. Any other zero divisor gives
±1 by the numerator's sign, with the numerator as remainder. `ArcTan` and
`ArcTan2` evaluate the BIOS's fixed-point polynomial and reproduce its
register clobbers in `r1` and `r3`; `ArcTan2` covers both axes and every
quadrant. The mGBA suite's BIOS math table supplies the oracles, including
polynomial discontinuities and the register clobbers ("mGBA BIOS math table
ArcTan cases preserve fixed-point BIOS intermediates").

### Copies

`CpuSet` copies or fills halfwords or words (`r2` bits 0–20 count, bit 24
fill, bit 26 word size); only the word form writes its address registers
back. Each halfword unit is a load and a store as the retail loop's
`LDRH` and `STRH` make them ([GBATEK, BIOS Memory Copy][gbatek-copy]): from
an odd source the loaded halfword is rotated right by 8, so the copy stores
its high byte, and on the 8-bit save bus both use the exact address; the
mGBA suite's memory table records `00DE00BEh` copied from `DEADBEEFh` this
way. Word units, and `CpuFastSet`'s, are the retail loops' `LDMIA` and
`STMIA` beats: aligned, except on the 8-bit save bus, which sees the exact
address ([memory](memory.md#region-behavior)). ZiGBA aligns every address
first; these are deliberate differences
([`roadmap.md`](../../roadmap.md#mgba-suite-rom-findings)). `CpuFastSet` copies or fills words in whole eight-word bursts, rounding
the count up. Both return after one cycle without copying when the source
starts below `02000000h`; GBATEK documents that the retail routines silently
refuse sources reaching into the BIOS area
([GBATEK, BIOS Memory Copy][gbatek-copy]).

### Affine setup

`BgAffineSet` and `ObjAffineSet` follow GBATEK's signed 8.8 layouts: 20-byte
BG input records produce 16-byte `PA`–`PD` plus reference-point records;
8-byte OBJ inputs produce four parameters at a caller-chosen destination
stride, such as OAM's. The angle's low byte is discarded. The matrix and the
BG origin translation use the pinned mGBA 0.10.5 HLE formulas, in single
precision; see [approximations](#approximations). A result outside its
stored width, such as a scale of −128 rotated by 180 degrees (+128.0 in 8.8)
or a BG start point past 32 bits, keeps its low bits as integer arithmetic of
that width would, instead of trapping. That is containment, not a claim
about the retail routines.

### BitUnPack

`BitUnPack` reads its parameter block (source length, source width 1/2/4/8,
destination width 1/2/4/8/16/32, bias word), extracts source units low bits
first and packs them into 32-bit destination words. The bias is added to
nonzero units, or to every unit when bias bit 31 is set. Unsupported widths
return without writing.

### Decompression

All decompressors read a 32-bit header whose bits 8–31 give the output
length. The `Vram` variants write whole halfwords, because a byte write to
VRAM lands in both halves of its halfword
([`memory.md`](memory.md#region-behavior)); the `Wram` variants write
bytes. The mGBA 0.10.5 replacement BIOS supplied the control formats,
overlapping-copy behavior, VRAM halfword buffering, padding and timing shape;
no Nintendo BIOS data was used.

- **LZ77** expands literal and back-reference tokens under a flag byte per
  eight tokens; overlapping references copy byte by byte, and VRAM output
  reads its back-references as halfwords.
- **Huffman.** Child nodes are addressed from the current node's aligned
  address, left and right terminal flags select symbols, the bitstream is
  read MSB first, and 4-bit or 8-bit symbols are packed low first into 32-bit
  writes ([GBATEK, decompression][gbatek-decomp]). A zero symbol width means
  eight, as in mGBA; other widths return without writing. Output is written
  in whole words, so a declared length shorter than four bytes still writes
  one padded word. **Alignment rule:** the bitstream starts after a tree of
  `(size + 1) × 2` bytes, so an even tree size leaves it halfword-aligned.
  Its words are then read as an ARM `LDR` would read them, as mGBA's HLE
  does: the aligned word rotated right by 16 bits.
- **Run-length** expands literal and repeated blocks. **VRAM parity rule:**
  bytes pair into halfwords by destination address. An odd `r1` therefore
  starts with a halfword whose low byte is zero, ends one byte into its
  padding, and pads only what remains; an odd end with no padding left
  stops there instead of wrapping the count.
- **Differential filters** accumulate 8-bit or 16-bit deltas; the 8-bit VRAM
  form packs pairs of bytes into halfwords.

The Cult-of-GBA replacement BIOS
([`running_length.s`](https://github.com/Cult-of-GBA/BIOS/blob/a30e9a96df083628b650724b7d4d7112b4070b98/bios_calls/decompression/running_length.s))
disagrees with the VRAM parity rule: it writes every block whole, past a
shorter declared length, pairs bytes by count rather than address, and pads
nothing, and its comments attribute that to the original BIOS. Which of the
two the retail routine matches is unverified.

Invalid or cyclic Huffman trees are not sanitized, and a tree that never
reaches a terminal keeps the guest inside the service indefinitely, as the
retail routine would. The host still regains control after every step.
Compressed data from untrusted sources should not be treated as a hardened
input format.

ZiGBA tests in [`machine_test.zig`](../../../refs/zigba/test/machine_test.zig) cover each
format, including "HLE HuffUnComp reads a halfword-aligned bitstream as
rotated words" and "HLE run-length VRAM output from an odd destination ends
its padding".

### Reset services

The reset services follow GBATEK's reset table
([GBATEK, BIOS Reset Functions][gbatek-reset]) and mGBA 0.10.5's service
boundaries.

- **`SoftReset`** reads the flag byte at `03007FFAh` before clearing
  `03007E00h`–`03007FFFh`, zeros `r0`–`r12` and the IRQ and Supervisor `lr`
  and SPSR, installs the three stacks, enters ARM System mode, and refills at
  `08000000h` (flag zero) or `02000000h` (nonzero).
- **`RegisterRamReset`** clears what `r0` selects. It always forces blank
  (`DISPCNT = 0080h`) and always clears the low half of `SIODATA32`, a retail
  bug GBATEK documents.

  | `r0` bit | Resets                                                                         |
  | -------- | ------------------------------------------------------------------------------ |
  | 0        | EWRAM                                                                          |
  | 1        | IWRAM except its last `200h` bytes                                             |
  | 2        | palette RAM                                                                    |
  | 3        | VRAM                                                                           |
  | 4        | OAM                                                                            |
  | 5        | serial registers, cancelling transfers, UART queues and pending JoyBus traffic |
  | 6        | sound registers and wave RAM                                                   |
  | 7        | all other video, DMA, timer, interrupt and wait-state registers                |

- **`HardReset`** reboots without returning, as GBATEK describes. HLE clears
  all console RAM, video memory, frame buffers, CPU banks, DMA, timer,
  interrupt and prefetch state, serial transactions and transient audio
  queues, then restores the post-boot state (forced blank, identity affine
  parameters, `SOUNDCNT_H`, `SOUNDBIAS`, `RCNT`, and `POSTFLG = 1`) and
  refills at `08000000h` in ARM System mode. Keys, cartridge save contents
  and cartridge devices such as the RTC stay powered. The logo sequence and
  its roughly two-second duration are omitted; external-BIOS mode is the
  exact path for them.

The timer prescaler keeps running through both software resets
([`timers-dma-interrupts.md`](timers-dma-interrupts.md)).

### Sound utilities

These services sit beside the sound driver but are not part of it.

- **`SoundBias`** ramps the `SOUNDBIAS` level to `000h` (`r0 = 0`) or `200h`
  one unit at a time, eight cycles apart (GBATEK's fixed delay count),
  preserving the resolution bits. Each step is a real register write, so the
  APU hears the ramp.
- **`MidiKey2Freq`** reads the WaveData frequency word at `r0 + 4` through the
  timed bus and returns
  `freq / 2^((180 − key − fine / 256) / 12)` with byte-sized key (`r1`) and
  fine pitch (`r2`), using pinned mGBA 0.10.5's single-precision formula. A
  result past 32 bits keeps its low bits.
- **`CustomHalt`** enters Stop when `r2` bit 7 is set and Halt otherwise.

### `IntrWait` and the BIOS IRQ mirror

`IntrWait` (`r0` discard flag, `r1` interrupt mask) and `VBlankIntrWait`
(which sets `r0 = r1 = 1`) wait in Halt until a selected interrupt has been
serviced. They do not watch `IF` directly. The game's IRQ handler must also
OR each acknowledged flag into the BIOS IRQ mirror, a halfword at
`03007FF8h`, and the wait completes when a selected bit appears there
([GBATEK, IntrWait][gbatek-halt]). The HLE model is a state that persists
across IRQ handler calls, not a shortcut:

1. The service sets `IME = 1`, as the retail routine does.
2. With `r0` nonzero it first discards the selected bits in the mirror.
3. If a selected bit is already set, it clears it and returns without
   sleeping. Otherwise it records the mask and enters Halt.
4. Each time the HLE IRQ dispatcher returns to depth zero, it rechecks the
   mirror: a selected bit is cleared and the wait ends; otherwise the CPU
   halts again.

Halt itself wakes on `IE & IF` regardless of `IME` and CPSR.I, while IRQ
entry still obeys both gates; those rules are in
[`timers-dma-interrupts.md`](timers-dma-interrupts.md). Tests: "HLE IntrWait
completes only after the IRQ handler updates the BIOS mirror", "nested HLE
IRQ return does not complete the outer IntrWait", and the `IntrWait`
consume and discard cases.

### The HLE IRQ dispatcher

On hardware, IRQ entry jumps to the BIOS trampoline at `18h`, which saves
`r0`–`r3`, `r12` and `lr` on the IRQ stack, calls the user handler whose
address is stored at `03007FFCh` with `lr = 00000138h`, and returns with
`SUBS pc, lr, #4` ([GBATEK, GBA Interrupt Control][gbatek-irq]). In HLE mode,
`Cpu::enter_irq` performs the architectural IRQ entry, then:

- pushes a frame (return address, `SPSR_irq`, `r0`–`r3`, `r12`) onto a
  bounded eight-entry stack inside the CPU;
- reads the handler pointer from `03007FFCh` with a timed word read;
- sets `lr` to the `00000138h` return stub and branches to the handler, in
  Thumb state when bit 0 of the pointer is set;
- sets the protection latch to `E25EF004h`.

When execution reaches `00000138h` with a frame on the stack, the dispatcher
performs the old stream's sequential fetch, pops the frame, restores
`r0`–`r3` and `r12`, restores that frame's own `SPSR_irq`, returns to the
interrupted code, and sets the latch to `E55EC002h`.

Handlers may re-enable IRQs and nest. Each nesting level has its own frame,
so an inner IRQ cannot overwrite the outer return state, and the saved
`SPSR_irq` is restored per frame because a nested entry overwrites the banked
register. **Nested-return rule:** only the return that brings the stack back
to depth zero checks a pending `IntrWait`. A nested return resumes the outer
handler first, so that handler can still publish its bit in the BIOS mirror
before the wait is evaluated. A ninth nested entry stops with an
`hle_irq_nesting_limit` fault before it mutates any exception state. Tests:
"nested HLE IRQs preserve the outer exception return state" and "HLE IRQ
nesting limit faults before exception state changes".

## Services that span several steps

Some services run for as long as the guest's arguments say: a 32-bit affine
record count, a 21-bit copy count, a 24-bit decompressed length, or a Huffman
tree that never reaches a terminal. On hardware that time passes inside BIOS
code while video, sound, timers and DMA keep running. An HLE service that ran
to completion inside one instruction would block the host for minutes, or
forever. Two kinds of service therefore run as continuations held in one CPU
slot (`Cpu.hle_service`):

- **MultiBoot** advances one protocol phase per step while the link scheduler
  performs each exchange ([below](#multiboot)).
- **Bulk services** (`CpuSet`, `CpuFastSet`, `BgAffineSet`, `ObjAffineSet`,
  `BitUnPack` and the LZ77, Huffman, run-length and differential-filter
  decompressors) keep every value their loop carries between iterations
  (addresses, counts, bit buffers, the pending VRAM halfword, the current
  Huffman node) in an `HleBulkWork` continuation.

A step spends at most `hle_bulk_step_cycles`, 2^20 guest cycles (62.5 ms,
about 3.7 frames), in a bulk service, then suspends at the next iteration
boundary; Huffman suspends only between bitstream words, each of which walks
at most 32 nodes. The next step resumes it. The budget is comfortably above
the longest calls observed in commercial software, so ordinary calls finish
in the step that executes the `SWI`.

Suspension is invisible to the guest:

- **Guest-cycle suspension points.** Where a step stops depends only on the
  cycles the service has spent, never on a host deadline. Every host slicing
  makes the same bus accesses in the same order at the same cycles, and DMA
  and devices keep running inside those accesses.
- **No instruction boundary.** No IRQ is taken and nothing else runs between
  steps. The step that finishes the service sets the protection latch and
  then performs the `SWI`'s ordinary IRQ check, at the cycle the
  uninterrupted service would have ended.
- **Power states wait.** A Halt or Stop requested by the service's own
  `HALTCNT` writes takes effect once it returns, as it would after an
  uninterrupted loop.
- **Coherent CPU view.** The pipeline already holds the return instruction,
  so the next-instruction address names the return point; the registers keep
  the `SWI`'s inputs until it returns; and every step reports the `SWI` as
  the executed instruction. `Machine::wait_until` reports that the CPU is not
  waiting.
- **Save states.** Format version 6 serializes the continuation, and loading
  rejects loop states the services cannot reach, such as a Huffman output
  that would skip its 32-bit boundary or an unaligned copy pointer.

Consequences for hosts: `Machine::step` advances such a service by one step,
`Machine::run_slice` returns at the first step past its deadline while the guest stays
inside the service (overshooting by at most about one budget), and the CLI's
`--steps` and `--trace` count steps, so a long call appears as repeated
records of its `SWI`.

ZiGBA tests in [`machine_test.zig`](../../../refs/zigba/test/machine_test.zig) cover each
property, among them "HLE HuffUnComp without a terminal returns to runUntil
while video advances", "an IRQ raised inside a multi-step HLE service is
taken when the service returns" and "multi-step HLE services reach the same
state under any host slicing".

## MultiBoot

`MultiBoot` (`25h`) sends a program to up to three clients over the serial
port ([GBATEK, BIOS Multi Boot][gbatek-multiboot]). `r0` points to a
parameter block (handshake byte at `14h`, client bytes at `19h`–`1Bh`,
palette byte at `1Ch`, client bits at `1Eh`, payload start and end at `20h`
and `24h`); `r1` selects the transfer mode: 0 Normal at 256 KHz, 1
Multiplayer at 115 KHz, 2 Normal at 2 MHz. The service validates the mode,
at least one client bit within `0Eh` and no others, and a payload of `100h`
to `3FF40h` bytes in 16-byte units. A malformed call returns `r0 = 1` at the
following instruction.

The protocol state, including the length command, the separate Normal and
Multiplayer cipher streams and the CRC, is pure code in
[`multiboot.rs`](../../../systems/gba/src/multiboot.rs), checked against published
algorithm vectors. The CPU continuation drives the real SIO registers
through the timed bus: after an initial delay it sends the length command
and collects each client's random byte, sends the encrypted payload (as two
halfwords per word in Multiplayer mode) and checks every echo, requests the
CRC phase until the clients answer `75h`, and exchanges the final CRC. Each
exchange polls the start bit every 64 cycles and fails after 2^24 cycles;
successive transfers are 604 cycles apart. The service returns `r0 = 0` on
success and 1 on any failure.

The probes `probe-mb2slave` (Multiplayer, two clients) and `probe-mb2mhz`
(Normal 32-bit at 2 MHz) run the HLE parent against client ROMs that mimic
slave-BIOS responses and verify every word with their own cipher
([`probe/README.md`](../../../probe/README.md)). Interoperability with a
retail-BIOS client needs an external BIOS image and has not been claimed.
HLE boot also lacks the BIOS's cartridge-less client download loop, so a
BIOS-free single-cartridge multiplayer session is not possible.

## The MP2K sound driver

The BIOS services `1Ah`–`1Eh`, `20h`–`24h` and `28h`–`2Ah` form a version of
Nintendo's MP2K sound driver. The driver keeps its state in a SoundArea whose
address the BIOS publishes at `03007FF0h`; the word at its start is the
identifier `Smsh` (`68736D53h`) when idle and one more while a routine holds
the lock. HLE implements the driver's observable behavior with timed bus
accesses throughout, so FIFO resets, DMA address latching, Timer 0, video
advancement and APU state see the same sequence the retail routines produce.

- **`SoundDriverInit`** disables the old sound DMA, enables and routes
  sound, sets the `SOUNDBIAS` resolution while preserving its other high
  bits, points the FIFO A and B DMA channels at the SoundArea mix buffers,
  publishes the SoundArea pointer, zeroes the `FB0h`-byte area through
  `CpuSet`, installs the retail default callbacks and words, runs the
  frequency setup for mode 4, and finally writes the identifier.
- **`SoundDriverMode`** applies the reverb mask, channel count, master volume
  and DAC resolution (to the real `SOUNDBIAS` high byte). Its
  playback-frequency path uses the retail 15-entry mode table (BIOS offset
  `31E8h`), derives the SoundArea cadence fields and the Timer 0 reload with
  integer arithmetic, restarts DMA, and enables Timer 0 when `VCOUNT`
  reaches line 159.
- **`SoundDriverVSync`** follows the retail countdown: values above one only
  decrement; zero and one reload from `SoundArea + 0Bh` and restart DMA1 and
  DMA2 by writing 0 and then `B600h` to their controls.
  **`SoundDriverVSyncOn`** writes `B600h` directly, and
  **`SoundDriverVSyncOff`** accepts the idle and one-deep busy identifiers,
  disables DMA1 and DMA2, clears the countdown and zeroes the mix buffer
  `SoundArea + 350h`–`FAFh`.
- **`SoundChannelClear`** validates the identifier, clears the status byte of
  all twelve 64-byte software channels, and reproduces the routine's
  `r0`–`r2` clobbers. When the SoundArea extension pointer is nonzero, it
  calls the game's callback four times with the retail `r0`, `r4`–`r7`
  arguments, in ARM or Thumb, then clears the extension flag and restores
  the caller's CPSR, `lr`, `r4`–`r7` and the identifier.
- **`SoundDriverMain`** takes the lock, derives the optional `VCOUNT`
  budget, calls the music-player callback at `SoundArea + 20h` with the
  argument at `+ 24h`, then the CGB callback at `+ 28h`, and mixes. Each
  callback address is captured by one timed word read, and the captured value
  stays the call target even if HBlank DMA rewrites the word before dispatch.
  Completion preserves `r4`–`r11`, `lr` and the caller's CPSR. The mixer
  selects the current DMA slice, clears the buffer or applies the four-tap
  signed-byte reverb, advances attack, decay, sustain, release and
  pseudo-echo envelopes, derives left and right volumes, and mixes
  fixed-rate or linearly interpolated signed PCM with end and loop handling.
- **Music players** (`20h`–`24h`) are identified from the raw retail routines
  at `13C4h`, `1434h`, `14C0h`, `14FCh` and `1514h` as open, start, stop,
  continue and fade out, the names mGBA's HLE table uses. Open clears the
  64-byte player, clamps a positive track count to 16, clears the 80-byte
  track records, chains the player into the SoundArea and installs the
  retail player-main callback `2149h`. Start installs song, voice group,
  priority, tempo and command pointers; stop tears down each active track's
  channel chain; continue clears the pause bit; fade out sets the interval,
  counter and `100h` volume.
- **The scheduler.** HLE recognizes the retail player-main target `2149h`
  inside `SoundDriverMain` and runs the MP2K scheduler itself: it follows the
  linked chain of up to 16 players in the retail inner-to-outer order,
  observes each player's lock, pause and fade state, tempo accumulator,
  active-track bitmap and clock, and can suspend around a game's own wrapped
  callback and resume afterwards. Track work decodes running status, the
  clock table, all standard command handlers, notes, waits, gate expiry and
  triangular LFO modulation. Note setup resolves direct, rhythm and key-split
  voices, selects software or CGB channels by priority, links the channel to
  its track, installs envelope, echo, key, volume and frequency state, and
  leaves rendering to the mixer. Volume and pitch changes propagate with the
  MP2K integer formulas.
- **`SoundGetJumpList`** (`2Ah`) writes the 36 pointers the driver expects.
  HLE has no retail code to point at, so they are odd Thumb pointers into a
  private dispatch range at `3E00h`–`3E8Fh`, implementing the 30
  command-table entries plus `SampleFreqSet`, `TrackStop`, `FadeOutBody`,
  `TrkVolPitSet`, the channel-chain clear and the 64-byte clear helper.
  Calls return with ordinary `BX LR` interworking, and `SoundDriverMain`
  accepts exactly these pointers as callbacks. The range exists only in HLE
  mode; an external BIOS provides its native table.

**Sources.** The retail BIOS routines, read from a public GBA BIOS
disassembly (commit `24efa52ac04fedcba4a8b952b76454d3b11a6fa0`), and
[VisualBoyAdvance-M][vbam]'s control flow anchor the SoundArea ABI. Pinned
mGBA 0.10.5 and [SAT-R/sa2][sa2] cross-check the MP2K channel layout and
mixer state transitions. The Cult-of-GBA replacement BIOS maps
`SoundDriverMain` to a no-op, so it is not an oracle here. Where
VisualBoyAdvance-M disagrees with the retail instructions, ZiGBA follows the
instructions: its reverb shift, its SoundArea-relative DAC write, and its
`8600h` for `SoundDriverVSyncOn` (the retail routine builds `B600h` from
`B6h << 8` and `5Bh << 9`).

**What traps.** HLE has no BIOS code to run, so internal BIOS addresses used
as callbacks, and replacements for canonical MP2K functions, stop with a
typed fault rather than a success-shaped no-op
([fault containment](#fault-containment)). The external BIOS remains the
exact fallback for such games.

**What is approximate.** Each sound service charges a fixed stall plus its
bus accesses rather than the retail instruction timing, and the scheduler and
mixer run inside one step, resumable only around guest callbacks.

Tests include "HLE SoundDriverMain runs the internal MP2K scheduler and
starts a note", "HLE SoundDriverMain samples CGB callback once across HBlank
DMA" and "HLE MusicPlayerJumpTableCopy publishes 36 callable Thumb
targets".

## Fault containment

A guest can ask the HLE BIOS for something it cannot do faithfully: an
unassigned service, a BIOS-internal callback, malformed driver data, or a
pointer that would break a bus invariant. None of these aborts the host or
returns fake success. Each stops at a typed, sticky `Machine.Fault` stamped
with the cycle and instruction
([ZiGBA architecture](../../../refs/zigba/docs/architecture.md)). The
latch is transient: it is not part of portable state, a faulted machine
refuses to write a save state, and loading a valid pre-fault state clears
it. The CLI prints one stable diagnostic line and still persists the
cartridge save.

| Fault reason                            | Raised when                                                                                                          |
| --------------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| `unsupported_hle_swi`                   | the service number is not assigned                                                                                   |
| `hle_irq_nesting_limit`                 | a ninth nested IRQ would enter; raised before any exception state changes                                            |
| `invalid_hle_memory_access`             | a guest-supplied word pointer would reach the I/O page misaligned (service, read or write, address, width)           |
| `sound_channel_clear_internal_callback` | a `SoundChannelClear` extension callback points into the BIOS                                                        |
| `sound_driver_main_internal_callback`   | a `SoundDriverMain` callback points into the BIOS and is not an HLE jump-list pointer or a recognized retail default |
| `mp2k_internal_callback`                | a chained player callback points into the BIOS and is not the player main `2149h`                                    |
| `mp2k_replacement_callback`             | the game's MP2K command table holds a function other than the HLE jump-list entry                                    |
| `mp2k_player_chain_limit`               | more than 16 players are chained                                                                                     |
| `mp2k_missing_running_status`           | a track data byte arrives with no prior status byte                                                                  |
| `track_stop_cgb_callback`               | stopping a CGB channel would call a non-default channel-stop callback                                                |
| `sample_freq_set_mode_zero`             | `SampleFreqSet` is called with mode 0                                                                                |

**Pointer preflight.** The bus requires I/O word accesses to be aligned. The
CPU guarantees this for guest loads and stores, but HLE services use raw
guest pointers, so they check them before any time passes or any register or
memory changes: the affine, unpacking, decompression, sound-driver,
music-player, MultiBoot and jump-list services all preflight their direct
word pointers. `BgAffineSet` checks all its records in constant time rather
than walking a 32-bit count: records advance by 20 source and 16 destination
bytes, both multiples of four, so every word in a stream shares the first
word's alignment, and a misaligned stream first enters the I/O page within
one stride of its start, wrapping past the top of the address space if need
be. The first faulting record and access match a record-by-record check,
including wraparound and both page edges. `HuffUnComp` and `BitUnPack`
decide from their data how many words they write, so they check each
destination word as they write it: the fault follows any earlier output.
Tests: "HLE services preflight direct guest word pointers at the Machine
seam", "HLE BgAffineSet preflight faults on distant and wrapped records
before side effects", "HLE HuffUnComp and BitUnPack fault before an
unaligned MMIO output word" and "BgAffineSet analytic preflight matches the
record-by-record loop".

This is containment of invalid calls, not a claim about how the retail BIOS
treats malformed arguments.

## Approximations

HLE aims at exact observable effects, not at retail instruction timing.
Software that needs the retail BIOS's exact timing or rounding should use
external-BIOS mode.

- **Service timing.** Most services charge a fixed stall, usually 45 cycles,
  on top of their own timed bus accesses. The decompressors and `BitUnPack`
  add per-iteration charges shaped after the mGBA replacement BIOS.
  `Div` charges `11 + 13 × n` cycles, where n is the difference between the
  divisor's and the dividend's leading-zero counts (at least 1); `Sqrt`
  charges 53; `ArcTan` charges 37 plus the multiplier costs of its
  polynomial; `IntrWait` charges 12.
- **IRQ dispatch** charges the handler-pointer read and the pipeline refills,
  but not the retail trampoline's instructions or its stack traffic.
- **Rounding.** `BgAffineSet` and `ObjAffineSet` use single-precision sine
  and cosine, the same approximation class as mGBA's HLE, rather than the
  retail lookup table; `MidiKey2Freq` likewise uses mGBA's single-precision
  formula.
- **Resets** take a fixed charge; the logo sequence is omitted.
- **MultiBoot** delays (2^20 cycles before the first exchange, 64-cycle
  polls, 604 cycles between transfers, a 2^24-cycle timeout) are HLE
  choices, not retail measurements.

## Legal boundary

ZiGBA ships no Nintendo BIOS, commercial ROMs, keys or extracted assets, in
the repository or in release packages, and its tests use homebrew ROMs and
repository-owned probes. Users supply any content they are entitled to use;
BIOS images, ROMs and saves stay gitignored. The HLE BIOS is original code
with recorded provenance for every behavior. It was cross-checked against a
public disassembly but contains no BIOS bytes, which is also why it cannot
reproduce behavior that depends on reading them.

ZiGBA's external-boot validation uses the Cult-of-GBA open replacement BIOS
(pinned in [`refs/README.md`](../../../refs/zigba/refs/README.md)), never Nintendo's.
Agreement proves that the external boot path and bus behave; disagreement
with expectations derived from Nintendo's BIOS is replacement behavior, not
hardware truth. Copying mGBA code, rather than studying its behavior, would
carry its [MPL-2.0 file-level obligations][mgba-license]; ZiGBA copies none.

Emulator users should also know the law's limits. In the United States, the
Copyright Office explains that 17 U.S.C. §117's archival-copy rule is limited
to an owner of a copy and carries conditions; it is not a blanket "ROMs are
legal" rule ([Copyright Office FAQ][copyright-faq]). Anti-circumvention and
private-copy rules vary by jurisdiction.

## Open questions

- **`POSTFLG` after HLE boot.** HLE cold boot leaves `POSTFLG = 0`, while
  GBATEK states that the retail BIOS sets it to 1 after the initial reset;
  HLE `HardReset` does set 1.
- **No retail-BIOS run.** A Nintendo image has never been run by the
  maintainers; compatibility under a retail BIOS is not claimed.
- **Run-length VRAM parity.** Address-parity pairing (ZiGBA) or
  count-parity pairing without padding (Cult-of-GBA): which one the retail
  routine uses is unverified.
- **IRQ handler state.** GBATEK says the user handler is called in ARM state;
  HLE enters an odd handler pointer in Thumb state and does not write the
  trampoline's stack frame.
- **Interrupts inside services.** GBATEK notes that the BIOS may allow
  interrupts inside some services; HLE takes none until a service returns.
- **MP2K work inside one step.** Code inspection suggests that a cyclic
  channel chain, or a track that jumps without ever waiting, would hold the
  host call; this has not been reproduced.
- **Unsupported driver surface.** Custom replacement MP2K functions,
  non-default internal callbacks and BIOS-internal callback targets trap
  instead of running.
- **Sources of the sound services.** `SoundBias`, `MidiKey2Freq` and the
  music-player services come from GBATEK, replacement BIOSes and the retail
  disassembly, not from an official ABI.
- **MultiBoot client side.** HLE cannot act as a cartridge-less MultiBoot
  client, and interoperability with a retail-BIOS client is untested.

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
