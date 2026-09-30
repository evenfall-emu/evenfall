# Timers, DMA and interrupts

Three small pieces of hardware keep a GBA game on schedule.

- **Timers** count time.
  Games use them to measure intervals, and above all to set the sample rate of digital sound: each time a timer overflows, the sound chip plays its next sample.
- **DMA** copies memory without the CPU.
  Games use it to move graphics into video memory, to change a display register on every scanline, and to keep the sound chip fed with samples.
- **Interrupts** tell the CPU that something happened (a frame finished, a timer overflowed, a copy completed), so it can react instead of polling.

A fourth piece, the **HALT and STOP** power states, lets the CPU sleep until an interrupt wakes it.
Most games spend the end of every frame halted, waiting for the next one.

All of these run on the master clock, which the bus owns ([architecture](../../architecture.md#time-and-faults)).
Timer state is in [`timer.rs`](../../../systems/gba/src/timer.rs) (`Timer`) and DMA channel state in [`dma.rs`](../../../systems/gba/src/dma.rs) (`Channel`).
Scheduling, arbitration between channels and interrupt delivery are in [`bus/mod.rs`](../../../systems/gba/src/bus/mod.rs).

Related pages: how the CPU enters an interrupt is in [`cpu.md`](cpu.md).
The BIOS wait services (`Halt`, `Stop`, `IntrWait`, `VBlankIntrWait`, `CustomHalt`) and the BIOS interrupt flags at `03007FF8h` are in [`bios.md`](bios.md).
Sound DMA is in [`audio.md`](audio.md).

## Timers

The GBA has four 16-bit timers that count up ([GBATEK, GBA Timers][gbatek-timers]).

| Register   | Address          | Read            | Write        |
| ---------- | ---------------- | --------------- | ------------ |
| `TMxCNT_L` | `04000100h + 4x` | current counter | reload value |
| `TMxCNT_H` | `04000102h + 4x` | control         | control      |

| `TMxCNT_H` bits | Field                                            |
| --------------- | ------------------------------------------------ |
| 0–1             | prescaler: count every 1, 64, 256 or 1024 cycles |
| 2               | count-up (cascade), unused for timer 0           |
| 6               | interrupt on overflow                            |
| 7               | enable                                           |

The other control bits can't be written (mask `00C7h`).

A timer runs like this:

- **Enable.** Turning a timer on loads the counter from the reload value.
  At a prescaler of 1, a timer enabled with reload `FFFEh` overflows two cycles after the enabling write completes.
- **Overflow.** When the counter passes `FFFFh`, it reloads from the reload value as it is at that moment, so changing the reload of a running timer only affects later periods.
  The overflow requests an interrupt if bit 6 is set, clocks the next timer if that one is in count-up mode, and for timers 0 and 1 can advance a sound FIFO ([`audio.md`](audio.md)).
- **Cascade.** Timers 1–3 in count-up mode ignore their prescaler and count one per overflow of the timer before them.
  Chaining all four makes a 64-bit counter.
- **Timer 0 always counts the clock.** GBATEK marks its count-up bit as unused, and mGBA only lets timers 1–3 cascade.
  Evenfall stores the bit but never lets it change timer 0's clock, including in a restored save state.

Evenfall doesn't tick timers every cycle.
`Timer::cycles_until_overflow` computes the cycle of the next overflow, and the bus treats it as a deadline.
Counters are brought up to date only when a program reads a timer register or when that deadline arrives, so interrupts, cascades and sound samples land on their exact cycle however the host slices time.

### The prescaler is one free-running counter

A timer with a prescaler of 64 counts once every 64 cycles.
The question is where those 64-cycle steps start.
One possibility is that each timer restarts its own divider when it is enabled.
The other is that all timers read one counter that never stops, and tick whenever it crosses a multiple of 64.

GBATEK doesn't say.
The hardware results recorded by the [mGBA test suite][suite-timers] (`src/timers.c`) answer it: the GBA uses one free-running counter.

The suite's test synchronizes on a timer 0 overflow, waits a short delay (3, 8 or 18 cycles, so the later enables land 5 and 15 cycles after the first), enables the timer under test, and counts loop iterations until it overflows.
With reload `FFF0h` and one interrupt:

| Prescaler | Recorded count, delay 1/2/4 | If the divider restarts at enable | If one counter runs freely |
| --------- | --------------------------- | --------------------------------- | -------------------------- |
| 64        | `7B`, `7B`, `81`            | `82` each                         | `7B`, `7B`, `81`           |
| 256       | `1EB`, `1EB`, `1E9`         | `202` each                        | `1EB`, `1EB`, `1E9`        |
| 1024      | `7EB`, `7EB`, `7E9`         | `802` each                        | `7EB`, `7EB`, `7E9`        |

A divider that restarts at enable takes the same time whatever the delay, so it can't produce these rows.
With a free-running counter, the first tick comes where the counter next crosses a multiple of the prescaler.
At 64, the enable 15 cycles later just misses a tick and overflows one period later.
At 256 and 1024, a later enable just leaves less time before the same tick.

The fit is tight.
After calibrating the test's loop and interrupt overheads on the unscaled rows alone, one counter phase (182 cycles after the synchronizing overflow, modulo 1024) reproduces all 216 prescaled checks.
The next-best phase fits 200, and a restarting divider fits 9.
A counter that resets when a timer is disabled fits less well, but can't be ruled out without running the test on a console with the official BIOS.

Evenfall implements the counter as `Bus::prescaler_counter`: the timer clock position minus `Bus.prescaler_origin`, modulo 1024.

- Every control write to an enabled timer takes the timer's phase from the counter (`Timer::write_control`).
  That covers enabling, changing the prescaler of a running timer, and leaving count-up mode.
- Nothing resets the counter: not enabling, disabling or changing a prescaler, and not the HLE `HardReset` and `SoftReset` services.
  Only STOP moves the origin ([below](#halt-and-stop)).
- Each timer caches its phase (`Timer.prescaler_phase`) for the overflow schedule.
  Loading a save state rejects a running timer whose phase is off the counter's grid.

mGBA also aligns prescaled ticks to its global cycle count ([`timer.c`][mgba-timer]), and NanoBoyAdvance takes the prescaler offset from its global timestamp.
Neither cites a hardware capture.

## DMA

Four DMA channels copy halfwords or words from one address range to another ([GBATEK, GBA DMA Transfers][gbatek-dma]).
The CPU stops while a channel has the bus.

| Channel | Source address | Destination address | A count of 0 means | Special trigger             |
| ------- | -------------- | ------------------- | ------------------ | --------------------------- |
| DMA0    | 27-bit         | 27-bit              | `4000h`            | none                        |
| DMA1    | 28-bit         | 27-bit              | `4000h`            | sound FIFO                  |
| DMA2    | 28-bit         | 27-bit              | `4000h`            | sound FIFO                  |
| DMA3    | 28-bit         | 28-bit              | `10000h`           | video capture (not modeled) |

Each channel has a source (`DMAxSAD`), a destination (`DMAxDAD`), a count (`DMAxCNT_L`, write-only) and a control register (`DMAxCNT_H`), starting at `040000B0h + 12x`.

| `DMAxCNT_H` bits | Field                                                              |
| ---------------- | ------------------------------------------------------------------ |
| 5–6              | destination: increment, decrement, fixed, or increment then reload |
| 7–8              | source: increment, decrement, fixed, or prohibited                 |
| 9                | repeat                                                             |
| 10               | 32-bit units                                                       |
| 11               | cartridge DRQ (DMA3), not modeled                                  |
| 12–13            | when to start: immediately, at VBlank, at HBlank, or special       |
| 14               | interrupt when done                                                |
| 15               | enable                                                             |

Bits 0–4 can't be written (mask `FFE0h`).
DMA3 transfers to the EEPROM port also help detect the save chip ([`cartridge.md`](cartridge.md)).

### Starting a transfer

When a game sets the enable bit, the channel copies the source and destination into internal pointers ([Addresses](#addresses)).
The copy doesn't start at once, even when "immediate".
Evenfall keeps a start deadline per channel (`dma_start_cycles`).
Until then the CPU keeps running and the enable bit reads set.
At the deadline, the channel competes for the bus and its first beat follows directly.
Disabling the channel cancels the start.

| When to start                           | What triggers it in Evenfall                                                             |
| --------------------------------------- | ---------------------------------------------------------------------------------------- |
| Immediately                             | the enabling write, two cycles later                                                     |
| VBlank                                  | the start of line 160, three cycles later                                                |
| HBlank                                  | cycle 960 of lines 0–159, three cycles later                                             |
| Special, DMA1 or DMA2                   | a sound FIFO asking for data at a timer overflow, with no delay ([`audio.md`](audio.md)) |
| Special, other channels or destinations | nothing: the channel stays enabled and never starts                                      |

The two-cycle delay for immediate transfers is pinned down from both sides by the mGBA suite's DMA rows (`src/tests/dma-timing.s`).
Code in IWRAM that reads a timer one cycle after the enabling write sees no transfer yet.
Thumb code in ROM that reads it two cycles after sees the transfer counted.
NanoBoyAdvance also uses two cycles.
mGBA uses three, but it only runs DMA between instructions, so its constant doesn't carry over to Evenfall's per-access model.
The three-cycle delay for VBlank and HBlank follows mGBA, and no suite row times it.

The delay matters to real games.
Golden Sun: The Lost Age copies code with immediate DMA and jumps into the copy two Thumb instructions later.

HBlank DMA doesn't run on the invisible lines 160–227.
GBATEK doesn't say, but mGBA and NanoBoyAdvance agree.

### Priority

DMA0 has the highest priority and DMA3 the lowest.
A beat (one read and one write) can't be split, but between beats a higher-priority channel that has become ready runs first.
The interrupted channel then resumes where it stopped (`Bus::service_higher_priority_dma`).
A channel's last beat finishes the whole transfer, including its completion cycles and interrupt, before another channel starts.
This matches GBATEK and mGBA, which also transfers one unit at a time and arbitrates again after each.

### Transfer timing

GBATEK gives a transfer's cost as `2N + 2(n−1)S + xI`, with x = 2, or 4 when both ends are in cartridge memory.
In Evenfall each beat is a read and a write that the bus prices like any other access, by region, width and wait states:

- There is no gap between beats.
- Only the first cartridge access of a transfer, source or destination, is non-sequential.
  Every other access is sequential.
  This is NanoBoyAdvance's model, and it fits the suite's rows exactly.
- Every transfer ends with two internal cycles, and nothing else.
  The suite's cartridge-to-cartridge rows fit two, not GBATEK's four.
  Evenfall charges them while the channel is still active, then clears the enable bit and raises the interrupt, so nothing sees the channel finish early.

A transfer also slows the CPU access it interrupts.
When a DMA takes the bus in the middle of a sequential cartridge access by the CPU, the CPU's access becomes non-sequential and costs N − S more once the DMA returns (`Bus::reprice_interrupted_game_pak_access`).
The suite shows this charge for ARM and Thumb code at every wait-state setting with prefetch off.
It doesn't show it for EWRAM code, where N and S cost the same, or for instructions the prefetch buffer already holds.

### Addresses

The address widths in the table above are applied as each register half is written (`Channel::write_source_high`, `Channel::write_destination_high`).
When the channel is enabled, it copies both addresses into internal pointers and aligns only those copies to the unit size, leaving the registers as programmed.
mGBA does the same, and the suite's unaligned-DMA results require it.

A source below `02000000h` (the BIOS and the unmapped space after it) becomes the internal pointer zero, which reads the shared data latch ([below](#shared-data-latch-and-source-zero)).
Because DMA0's source is only 27 bits wide, two surprising things happen, and the suite's recorded results confirm both:

- A cartridge ROM address loses its top bit and falls below `02000000h`, so DMA0 can't read the cartridge.
- An SRAM address `0E000000h` becomes `06000000h`, which is VRAM.

A cartridge source always steps forward, whatever the source control says.
Decrement and fixed don't work on the cartridge bus.
This follows mGBA.

### Repeat and reload

Repeat (bit 9) applies to VBlank, HBlank and special transfers.
An immediate transfer always clears its enable bit when done.
A repeating channel keeps its source pointer and reads its count again for each run.

With destination control 3 (increment then reload), the destination pointer is copied from `DAD` again at each repeat.
This is how a game rewrites one display register on every scanline.
Evenfall aligns that copy the same way as the first one (`Channel::reload_latched_destination`), so every repeat writes where the first transfer wrote.
For example, a 32-bit HBlank repeat aimed at `BG0VOFS` (`04000012h`) writes the whole word at `04000010h`, both scroll registers, on every visible line.
mGBA and NanoBoyAdvance produce the same writes.
With any other destination control, the destination continues from where the previous run stopped.

### Shared data latch and source zero

The DMA controller has one internal 32-bit data latch shared by all channels.
A word read replaces it, and a halfword read copies the value into both halves.

A channel whose source pointer is zero doesn't read the BIOS.
Every beat spends its source cycle, writes the latch's value, and leaves the source at zero.
This matches mGBA and the suite's DMA0 source-zero cases, whose results depend on earlier transfers.

### Bursts that never release the bus

Some DMA setups ask for more transfers than the bus can deliver.
Two sound channels whose timers overflow every cycle drain their buffers as fast as the refills arrive.
A repeating channel whose trigger fires again during its own transfer is ready as soon as it finishes.
The CPU then never gets the bus back.
On a GBA the CPU stalls for good while video and audio keep running.

Evenfall runs DMA inside the CPU access it follows, so such a burst would never return control to the host.
A burst therefore ends after 2^24 cycles (`Bus.max_dma_burst_cycles`), about one second of guest time and far longer than any real transfer.
Before the next transfer starts, the bus records a `DmaLivelock` with the channel and start cycle, drops pending requests and runs no more DMA.
`Machine` then reports a terminal `dma_livelock` fault at the next instruction boundary, on the same cycle however the host slices time.
Everything is exact up to that point.
Unlike a GBA, the stalled instruction then completes and the machine stops.

## Interrupts

| Register | Address     | Meaning                                   |
| -------- | ----------- | ----------------------------------------- |
| `IE`     | `04000200h` | which interrupts are enabled, bits 0–13   |
| `IF`     | `04000202h` | which are pending; writing 1 clears a bit |
| `IME`    | `04000208h` | bit 0: master enable                      |

| Bit  | Source       | Requested when                                                                         |
| ---- | ------------ | -------------------------------------------------------------------------------------- |
| 0    | VBlank       | line 160 begins and `DISPSTAT` bit 3 is set                                            |
| 1    | HBlank       | cycle 960 of any line, with `DISPSTAT` bit 4 set                                       |
| 2    | VCOUNT match | the line equal to `DISPSTAT`'s target begins, with bit 5 set                           |
| 3–6  | Timers 0–3   | the timer overflows, with `TMxCNT_H` bit 6 set                                         |
| 7    | Serial       | a serial transfer completes or changes status ([`serial.md`](serial.md))               |
| 8–11 | DMA 0–3      | a transfer completes, with `DMAxCNT_H` bit 14 set                                      |
| 12   | Keypad       | the `KEYCNT` condition becomes true, with bit 14 set                                   |
| 13   | Game Pak     | an access to the cartridge clock's interrupt register ([`cartridge.md`](cartridge.md)) |

A source sets its `IF` bit whenever its own conditions hold, whatever `IE`, `IME` and the CPU's interrupt mask say.
Those only decide whether the CPU reacts.
Writing `IF` clears only the bits written as 1, byte by byte.
After any change to the three registers, `Bus::update_irq_line` recomputes `IME && (IE & IF) != 0`.
The CPU takes the interrupt at the next instruction boundary where that line is set and CPSR.I is clear ([`cpu.md`](cpu.md), [GBATEK, GBA Interrupt Control][gbatek-irq]).

`KEYCNT` (`04000132h`) selects keys in bits 0–9, chooses between "all of them" (bit 15 set) and "any of them", and enables the interrupt in bit 14.
Evenfall requests the keypad interrupt when the condition turns from false to true, not while it stays true.

## HALT and STOP

Writing `HALTCNT` (`04000301h`) enters Halt when bit 7 is clear and Stop when it is set ([GBATEK, GBA System Control][gbatek-system]).
A halfword write to `04000300h`, which covers `POSTFLG` and `HALTCNT` together, does the same and selects by bit 15.
The BIOS wait services write the same register ([`bios.md`](bios.md)).
A multi-step HLE BIOS service holds back a Halt or Stop from its own writes until it returns.

**Halt** pauses only the CPU, until `IE & IF` is non-zero.
`IME` and the CPU's interrupt mask don't matter for waking up: GBATEK says Halt "passes through even if either one has disabled interrupts" ([BIOS Halt Functions][gbatek-halt]).
Everything else keeps running: the display, timers, DMA, sound, serial and keypad.
On waking, the CPU enters the interrupt handler first if `IME` and its mask allow it, and otherwise continues after the instruction that halted.
A halted machine jumps from event to event (`Bus::idle_halt_until`), so it wakes on the exact cycle of the waking event.

**Stop** is deep sleep.
GBATEK says most of the hardware, sound and video included, is paused.
Evenfall wakes only on an enabled keypad (bit 12), Game Pak (bit 13) or serial (bit 7) request (`Bus::stop_wake_pending`).
It freezes everything driven by the system clock: timers and the prescaler counter, the display, sound, DMA and cartridge prefetch.
The serial port is the exception.
Link transfers are clocked from outside the console, so a stopped machine still reaches their deadlines, and a finished transfer can wake it ([`serial.md`](serial.md)).
The master cycle count keeps advancing across those spans, so `Bus::idle_stop_until` moves `prescaler_origin` by the same amount to keep the prescaler counter frozen.

**What the host sees.** GBATEK notes that a GBA locks up if nothing ever wakes it from Halt.
Evenfall never blocks the host.
`Machine::step` returns a "waiting" result without advancing time while the CPU sleeps.
`Machine::run_until` and `Machine::wait_until` advance a sleeping machine only up to the caller's deadline, and stop at the first wake.
A stopped machine with no pending serial event reports `power_stopped` without advancing its clock ([`machine.rs`](../../../systems/gba/src/machine.rs)).

## How it is checked

- [`tests/bus.rs`](../../../systems/gba/tests/bus.rs) covers timers (exact overflow cycles however time is sliced, the timer 0 count-up bit, the shared prescaler grid, prescaler changes on running timers), DMA start delays, priority between channels, cartridge source stepping and the shared latch.
- [`tests/machine_a.rs`](../../../systems/gba/tests/machine_a.rs) and [`tests/machine_b.rs`](../../../systems/gba/tests/machine_b.rs) cover the Golden Sun copy-then-jump, Halt and Stop wake rules, and the DMA livelock fault landing on the same cycle under any slicing.
- [`tests/mgba_suite_timing.rs`](../../../systems/gba/tests/mgba_suite_timing.rs) runs all sixteen of the mGBA suite's DMA rows in ARM and Thumb, from ROM at all eight wait-state settings and from EWRAM and IWRAM.
  Every value matches except one, listed below.
- [`tests/mgba_suite_source.rs`](../../../systems/gba/tests/mgba_suite_source.rs) reproduces the suite's DMA source and latch table.
- Probe ROMs check whole-machine behavior: `probe-timer` (a three-timer chain), `probe-hdma` (a per-line scroll table), `probe-dma`, `probe-keypad` (the keypad interrupt edge), `probe-halt-stop` and `probe-stop-sio`.

## Open questions

- **One shared prescaler counter.** The suite tests only timer 0.
  Per-timer counters that never reset and start together would behave the same.
- **Prescaler phase at power-on.** Evenfall starts the counter at cycle zero in every boot mode.
  The suite can't see the absolute phase, and alyosha's second-hand `gba-tests` notes say it depends on exact power-on timing.
- **Changing a running timer's prescaler.** Evenfall adds no extra tick when the selected tap switches.
- **The prescaler during STOP.** Evenfall freezes it with the timers.
  No hardware test covers it.
- **Counter reset on disable.** Less likely than a free-running counter, but not excluded without an official-BIOS run of the suite.
- **Timer enable delay.** NanoBoyAdvance applies a control write one cycle late and the reload one cycle after that.
  Evenfall keeps its own timing until a focused hardware test decides.
- **HBlank DMA on invisible lines.** Three implementations agree it doesn't run.
  A hardware capture would make it a measured fact.
- **Repeat reload of a misaligned destination.** No hardware capture isolates it.
- **IWRAM-to-ROM DMA with prefetch on.** With ROM code and `WAITCNT` `4000h` or `4004h`, the suite's rows read one cycle more than Evenfall, in ARM and Thumb.
- **Why an interrupted cartridge access costs more.** The N − S charge fits every prefetch-off column, but the mechanism behind it isn't established.
- **VBlank and HBlank start delay.** The three cycles follow mGBA, and no suite row times them.
- **Features not modeled.** DMA3 video capture, the cartridge DRQ bit and DMA0's special trigger do nothing.
- **After a DMA livelock.** A GBA stalls the CPU forever while video and audio continue.
  Evenfall stops with a fault instead.
- **Keypad interrupt trigger.** Evenfall triggers on the change to true.
  Whether the hardware keeps triggering while the condition holds hasn't been measured.

[gbatek-timers]: https://rust-console.github.io/gbatek-gbaonly/#gbatimers
[gbatek-dma]: https://rust-console.github.io/gbatek-gbaonly/#gbadmatransfers
[gbatek-irq]: https://rust-console.github.io/gbatek-gbaonly/#gbainterruptcontrol
[gbatek-system]: https://rust-console.github.io/gbatek-gbaonly/#gbasystemcontrol
[gbatek-halt]: https://rust-console.github.io/gbatek-gbaonly/#bioshaltfunctions
[suite-timers]: https://github.com/mgba-emu/suite/blob/e6942030d25ffe3ba76c72b73a86da073ec857cc/src/timers.c
[mgba-timer]: https://github.com/mgba-emu/mgba/blob/0.10.5/src/gba/timer.c
