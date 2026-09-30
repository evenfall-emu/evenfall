# Timers, DMA and interrupts

This page covers the four hardware timers, the four DMA channels, the
interrupt controller and the HALT and STOP power states. All of them run on
the master clock owned by `Bus` ([ZiGBA architecture](../../../refs/zigba/docs/architecture.md)).
The per-device state lives in [`timer.rs`](../../../systems/gba/src/timer.rs) (`Timer`)
and [`dma.rs`](../../../systems/gba/src/dma.rs) (`Channel`); scheduling, arbitration
and interrupt delivery live in [`bus/mod.rs`](../../../systems/gba/src/bus/mod.rs).

How the CPU enters an IRQ is described in [`cpu.md`](cpu.md). The BIOS wait
services (`Halt`, `Stop`, `IntrWait`, `VBlankIntrWait`, `CustomHalt`) and the
BIOS interrupt-flag mirror at `03007FF8h` are described in
[`bios.md`](bios.md). Sound DMA is described in [`audio.md`](audio.md).

## Timers

The GBA has four incrementing 16-bit timers
([GBATEK, GBA Timers][gbatek-timers]).

| Register   | Address          | Read            | Write        |
| ---------- | ---------------- | --------------- | ------------ |
| `TMxCNT_L` | `04000100h + 4x` | current counter | reload value |
| `TMxCNT_H` | `04000102h + 4x` | control         | control      |

| `TMxCNT_H` bits | Field                                         |
| --------------- | --------------------------------------------- |
| 0–1             | Prescaler: `/1`, `/64`, `/256`, `/1024`       |
| 2               | Count-up (cascade) timing; unused for timer 0 |
| 6               | IRQ on overflow                               |
| 7               | Enable                                        |

The other control bits are not writable (mask `00C7h`).

- **Enable.** Setting bit 7 on a disabled timer loads the counter from the
  reload register. At `/1`, a timer enabled with reload `FFFEh` overflows two
  cycles after the enabling write completes.
- **Overflow.** The counter reloads from the reload register as it is at the
  overflow, so writing the reload of a running timer affects only later
  periods. An overflow sets IF bit 3 + x when bit 6 is set, clocks the next
  timer if that one is in count-up mode, and for timers 0 and 1 may clock a
  Direct Sound FIFO ([`audio.md`](audio.md)).
- **Cascade.** Timers 1–3 in count-up mode advance once per overflow of the
  preceding timer and ignore their prescaler; chains may span all four
  timers. The `probe-timer` probe checks a chain of three timers.
- **Timer 0 always counts the master clock.** GBATEK marks control bit 2 as
  unused for timer 0, and pinned mGBA restricts cascade to timers 1–3
  (`src/gba/timer.c`). ZiGBA keeps the written bit in the register but never
  lets it select the clock. Eager advancement, batched deadlines, HALT
  advancement and state validation all apply that rule, including to state
  restored without register writes (tests "timer zero unused count-up bit
  preserves the exact batched IRQ deadline" and "timer zero unused count-up
  bit cannot bypass state phase validation" in
  [ZiGBA's `bus_test.zig`](../../../refs/zigba/test/bus_test.zig)).

Evenfall, like ZiGBA, does not tick timers every cycle.
`Timer::cycles_until_overflow` gives the
next master-clock overflow, which the bus treats as a device deadline;
counters are materialized before any access to a timer register and at that
deadline, so IRQs, cascades and FIFO consumption land on their exact cycle
however the host slices time (test "no-event materialization preserves every
prescaler and cascade until the exact overflow").

### The prescaler is a free-running counter

The `/64`, `/256` and `/1024` prescalers are taps of one free-running
counter, not dividers restarted when a timer is enabled. GBATEK does not say
so. The evidence is the hardware-recorded timer table of the
[mGBA suite][suite-timers] (`src/timers.c`).

Its harness (`runTest`, lines 438–538, ARM code in IWRAM) synchronizes on a
`/1024` overflow of timer 0 with reload `FFFEh` and an IRQ; the IRQ handler
stops the timer. A delay loop of one, two or four iterations (3, 8 or 18
cycles, so the later enables land 5 and 15 cycles after the first) precedes
the enable of the tested configuration on timer 0, and a tight loop counts
until the handler stops the timer after the last IRQ. The `/1` rows never
depend on the delay; 64 of the 72 prescaled groups do. With reload `FFF0h`
and one IRQ:

| Divisor | Recorded count, delay 1/2/4 | Lines         | Prescaler restarted at enable | Free-running counter |
| ------- | --------------------------- | ------------- | ----------------------------- | -------------------- |
| `/64`   | `7B`, `7B`, `81`            | 242, 246, 250 | `82` each                     | `7B`, `7B`, `81`     |
| `/256`  | `1EB`, `1EB`, `1E9`         | 302, 306, 310 | `202` each                    | `1EB`, `1EB`, `1E9`  |
| `/1024` | `7EB`, `7EB`, `7E9`         | 362, 366, 370 | `802` each                    | `7EB`, `7EB`, `7E9`  |

A prescaler restarted at enable takes the same time to overflow whatever the
delay, so no choice of harness overheads can produce these rows. With a
free-running counter the first tick comes where the counter next crosses a
multiple of the divisor: at `/64` the enable 15 cycles later misses that tick
and overflows one period later; at `/256` and `/1024` the later enable simply
leaves less time before the same tick.

The fit is tight. Calibrating the loop and IRQ overheads on the `/1` rows
alone (exactly one setting fits all 96 `/1` checks), one counter phase
reproduces all 216 prescaled checks (12 rows × 3 delays × 3 IRQ counts × 2
loops): 182 cycles, modulo 1024, from the synchronizing overflow to the
first enable. The next-best phase fits 200 checks, and a restarted prescaler
fits 9. The fit depends on the harness's exact ARM timing; any other
delay-loop cost fits at most 215. The counter keeps running while timer 0 is
stopped, and neither enable nor divisor writes reset it. A counter reset by
the disable would fit only if about 182 cycles separated disable and enable;
the `/1` counter columns put the disable inside the IRQ handler, roughly 90
cycles before the enable. Without an official-BIOS run of the harness that
variant is disfavored rather than excluded.

Evenfall implements the counter as `Bus::prescaler_counter`: the timer clock
position minus `Bus.prescaler_origin`, modulo 1024. A prescaled timer ticks
where the counter crosses a multiple of its divisor.

- Every control write to an enabled timer takes the timer's phase from the
  counter (`Timer::write_control`): an enable, a divisor change on a running
  timer, and a return from count-up to the master clock. Growing or
  shrinking, a divisor change puts the next tick at the new divisor's next
  multiple.
- Enable, disable and divisor writes never reset the counter, and neither do
  the HLE `HardReset` and `SoftReset` services. Only STOP moves the origin
  (see [HALT and STOP](#halt-and-stop)).
- The per-timer phase (`Timer.prescaler_phase`) is a cache that the overflow
  schedule reads. For a running master-clock timer it always equals the
  counter modulo the divisor; loading a save state (format version 6, which
  stores the origin) rejects a timer off that grid.

Tests "prescaled timers tick on one free-running prescaler grid", "a /64
enable after a /1024 overflow follows the free-running prescaler" (enables
182, 187 and 197 cycles after a `/1024` overflow reach a `/64`, `FFF0h`
overflow 970, 965 and 1,019 cycles later), "a running timer's prescaler
change ticks on the shared grid when batched and eager", "a count-up stage
returns to the master clock on the shared prescaler grid" and "STOP freezes
the prescaler counter with the timers" pin the model.

Pinned mGBA aligns every prescaled tick to its global cycle count
(`GBATimerUpdateRegister` and `GBATimerWriteTMCNT_HI` in
[`timer.c`][mgba-timer]), and pinned NanoBoyAdvance takes the prescaler
offset from its global timestamp (`OnControlWritten` in
`src/nba/src/hw/timer/timer.cc`). Both are comparisons, not specifications;
neither cites a hardware capture. The assumptions ZiGBA makes where the
suite is silent are listed under [Open questions](#open-questions).

## DMA

Four channels copy halfwords or words without the CPU
([GBATEK, GBA DMA Transfers][gbatek-dma]). The CPU is stalled while a channel
holds the bus.

| Channel | Source address | Destination address | Count (0 means) | Special timing              |
| ------- | -------------- | ------------------- | --------------- | --------------------------- |
| DMA0    | 27-bit         | 27-bit              | `4000h`         | none                        |
| DMA1    | 28-bit         | 27-bit              | `4000h`         | sound FIFO                  |
| DMA2    | 28-bit         | 27-bit              | `4000h`         | sound FIFO                  |
| DMA3    | 28-bit         | 28-bit              | `10000h`        | video capture (not modeled) |

Each channel has `DMAxSAD`, `DMAxDAD` and `DMAxCNT_L` (write-only; reads
return zero) and `DMAxCNT_H`, at `040000B0h + 12x` onwards.

| `DMAxCNT_H` bits | Field                                                              |
| ---------------- | ------------------------------------------------------------------ |
| 5–6              | Destination control: increment, decrement, fixed, increment/reload |
| 7–8              | Source control: increment, decrement, fixed, prohibited            |
| 9                | Repeat                                                             |
| 10               | 32-bit units                                                       |
| 11               | Game Pak DRQ (DMA3); not modeled                                   |
| 12–13            | Start timing: immediate, VBlank, HBlank, special                   |
| 14               | IRQ at end of transfer                                             |
| 15               | Enable                                                             |

Bits 0–4 are not writable (mask `FFE0h`). DMA3 transfers to the EEPROM port
also inform save-type detection ([`cartridge.md`](cartridge.md)).

### Starting a transfer

When the enable bit rises, the channel copies `SAD` and `DAD` into internal
pointers ([Addresses](#addresses)). "Immediate" does not mean synchronous with
that write. Evenfall keeps a per-channel start deadline (`dma_start_cycles`).
Until then the CPU keeps running and the enable bit reads set; at the
deadline, ready channels enter the arbiter and the first beat follows
directly. Disabling a channel cancels its scheduled start.

An immediate transfer starts two cycles after the end of the enabling write.
GBATEK says to wait two cycles after setting the enable bit before touching
the DMA registers again. The mGBA suite's DMA rows (`src/tests/dma-timing.s`)
bound the delay from both sides, because END reads timer 0 with `ldrh` before
stopping it and a transfer that starts after that read is not counted:

- IWRAM code reads the timer one cycle after the trigger store, and the
  suite's IWRAM columns read 2 for every row, so the transfer has not
  started by then.
- Thumb ROM code at `WAITCNT` `0014h` reads the timer two cycles after the
  trigger store, and the suite counts the transfer there, so it has started.

NanoBoyAdvance also starts immediate DMA two cycles after the write. Pinned
mGBA uses three, but mGBA's clock during a store still reads the start of the
storing instruction and it runs DMA only between instructions, so its
constant does not carry over to a per-access model. VBlank and HBlank
requests start three cycles after their trigger, as in pinned mGBA.

The test "DMA3 immediate starts two cycles after its enabling write, copies
halfwords, clears enable, and requests IRQ" measures the whole sequence, and
"code branched to after an immediate DMA copy runs the copied opcodes"
([ZiGBA's `machine_test.zig`](../../../refs/zigba/test/machine_test.zig)) covers Golden Sun:
The Lost Age's boot copy, where Thumb ROM code branches into the destination
two instructions after the trigger.

| Start timing                                         | Trigger in ZiGBA                                                                                                |
| ---------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| Immediate                                            | the enabling write                                                                                              |
| VBlank                                               | start of line 160                                                                                               |
| HBlank                                               | cycle 960 of lines 0–159 only                                                                                   |
| Special, DMA1/DMA2                                   | Direct Sound FIFO request ([`audio.md`](audio.md)); raised at the timer overflow, without the three-cycle delay |
| Special, DMA0/DMA3, or DMA1/DMA2 not aimed at a FIFO | none; the channel stays enabled and never starts                                                                |

GBATEK does not say whether HBlank DMA runs on VBlank lines. Two independent
implementations agree that it does not: pinned mGBA gates `GBADMARunHblank`
on `vcount < GBA_VIDEO_VERTICAL_PIXELS` in `_startHblank`, and
NanoBoyAdvance requests HBlank DMA only in `BeginHBlankVDraw`. The
`probe-hdma` probe drives a per-line scroll table this way, and the test
"VBlank DMA starts after the access that crosses into line 160" pins the
VBlank start.

### Priority and arbitration

GBATEK orders DMA0 above DMA1, DMA2 and DMA3 and says lower-priority channels
pause until higher-priority ones complete. Pinned mGBA transfers one unit at a
time, updates the channel's count and addresses, then arbitrates again.
ZiGBA uses the equivalent rule: a read/write beat is indivisible, but after
each non-final beat a newly ready higher-priority channel runs before the
suspended channel's next beat, which resumes from its preserved addresses and
index (`Bus::service_higher_priority_dma`). A final beat completes its channel,
including the completion cycles below, and raises its IRQ before another
ready channel begins. The test "HBlank DMA0 preempts an active lower-priority
DMA3 between beats" pins the order.

### Transfer timing

GBATEK gives a transfer's cost as `2N + 2(n−1)S + xI`, with x = 2 normally or
4 when source and destination are both in Game Pak memory. In ZiGBA each
beat is a read and a write that the bus prices by region, width and wait
states like any other access:

- There is no gap between beats.
- Only the first Game Pak access of the whole transfer, source or
  destination, is non-sequential; every other access is sequential. This is
  NanoBoyAdvance's model; a non-sequential first destination write instead
  overshoots the suite's ROM-destination rows by exactly N − S.
- Completion costs two internal cycles for every transfer, and there is no
  other per-transfer overhead. The suite's Game Pak-to-Game Pak rows fit two,
  not GBATEK's four. ZiGBA charges them after the final beat while the
  channel is still active, then applies the destination reload, enable
  clearing and the completion IRQ, so neither the CPU nor another channel
  sees the channel finish early.

The CPU also pays for a transfer that interrupts it. A DMA that takes the bus
after a CPU sequential Game Pak access began, up to and including the cycle
that access completes, turns the access non-sequential: it costs N − S more
once the DMA returns (`Bus::reprice_interrupted_game_pak_access`). The suite's
rows show this charge at every prefetch-off wait-state column, for ARM and
Thumb code, and never for EWRAM code, where N and S are equal, or for opcodes
served from the prefetch buffer.

With ROM code and prefetch on, the transfer's first Game Pak access stops
the prefetch buffer. It waits one cycle whenever the buffer's halfword read
has begun, where a CPU access waits only for a read in its last cycle
([memory](memory.md#game-pak-prefetch-buffer)). The suite's IWRAM-to-ROM
rows measure this at `WAITCNT` `4000h` and `4004h`, where the IWRAM read
lets a three-cycle prefetch read begin before the ROM write; Evenfall
diverges from ZiGBA here.

[`tests/mgba_suite_timing.rs`](../../../systems/gba/tests/mgba_suite_timing.rs)
runs all sixteen of the suite's DMA rows (16- and 32-bit units, IWRAM or ROM
source and destination, one and sixteen units) in ARM and Thumb, from ROM at
all eight wait-state columns and from EWRAM and IWRAM, and reads the result
through timer 0 as the suite does. Every value matches.

### Addresses

GBATEK gives DMA0 a 27-bit source, DMA1–3 28-bit sources, DMA0–2 27-bit
destinations and DMA3 a 28-bit destination. ZiGBA applies these masks as the
register halves are written (`Channel::write_source_high`,
`Channel::write_destination_high`). When the enable bit rises it copies both
registers into internal pointers and aligns only those copies down to the
unit size, leaving the programmed registers intact, as pinned mGBA does and
as the suite's unaligned-DMA expectations require. A source below
`02000000h` becomes the internal pointer zero, regardless of the order in
which the register halves were written, as in pinned mGBA; see
[Shared data latch](#shared-data-latch-and-source-zero).

Two consequences of the 27-bit DMA0 source are pinned by
[`tests/mgba_suite_source.rs`](../../../systems/gba/tests/mgba_suite_source.rs)
against the suite's recorded results: a Game Pak ROM address masks to below
`02000000h` and therefore takes the latch path, and an SRAM address
`0E000000h` masks to `06000000h`, VRAM. (The `probe-dma` probe uses DMA3 for
its ROM copy for this reason.)

A Game Pak source always steps forward, whatever the legal source-control
value; decrement and fixed are not honored on the cartridge bus. This
follows pinned mGBA; destination control stays ordinary (test "Game Pak DMA
sources increment despite decrement or fixed control").

### Repeat and reload

Repeat (bit 9) applies to VBlank, HBlank and special timing; an immediate
channel always clears its enable bit at completion. A repeating channel keeps
its source pointer and reads its count from `DMAxCNT_L` again for each run.

With destination control 3 (increment/reload), GBATEK copies `DAD` into the
internal destination pointer again at each repeat; it does not mention the
low address bits. The suite's unaligned DMA store rows (`memory.c`, "DMA0–3
16 (unaligned)" and "32 (unaligned 1–3)") exercise only the copy at enable,
where the unit lands on the rounded-down halfword or word. ZiGBA treats the
reload as the same copy and drops the same bits
(`Channel::reload_latched_destination`), so every repeat writes the unit the
first transfer wrote: a 32-bit HBlank repeat programmed with `BG0VOFS`
(`04000012h`) writes the `BG0HOFS`/`BG0VOFS` word at `04000010h` on every
visible line. Pinned mGBA reloads the raw `DAD` and lets its store handlers
round the address, and NanoBoyAdvance masks the reloaded pointer; both
produce the same writes. With any other destination control, the internal
destination continues from where the previous run stopped.

### Shared data latch and source zero

The DMA controller has one internal 32-bit data latch shared by all channels.
A word source read replaces it; a halfword source read copies the value into
both halves. A channel whose internal source pointer is zero performs no BIOS
read: every beat spends its source cycle, presents the latch, and leaves the
source at zero. This matches pinned mGBA's `dmaTransferRegister` behavior and
the suite's DMA0 source-zero cases, whose results otherwise depend on earlier
transfers. Tests "DMA source zero reuses the shared transfer latch without
advancing" and "DMA normalizes every source below EWRAM to the transfer-latch
source" pin it.

### Bursts that never release the bus

DMA requests can return faster than transfers complete. Two Direct Sound
channels whose timers overflow every cycle drain 32 bytes in the time their
two four-word refills take, and a repeating channel whose trigger recurs
within its own transfer is ready again as it completes. The arbiter then
never releases the bus: on hardware the CPU stalls for good while video and
audio keep running.

The core services DMA inside the CPU access it follows, as ZiGBA does, so
such a burst would never return control to the host. For up to 2^24 cycles
(`Bus::MAX_DMA_BURST_CYCLES`, about one second of guest time and many times
the longest possible single transfer) it runs there exactly as ZiGBA's does.
At that limit it yields at its next transfer start or beat boundary: it
records the transfers it interrupted, keeps its pending requests and the
bus, the interrupted instruction completes without DMA, and from that
instruction boundary the machine continues the burst up to each host
deadline. The CPU stays stalled while video, audio, timers, interrupt flags
and DMA itself run; if the burst ever ends (a DMA that stops the timers or
disables its own channel), the CPU takes any interrupt it would take at that
boundary and runs on. The limit falls on the same cycle, and the stall gives
the same results, under any host slicing (tests
`a_dma_livelock_stalls_the_cpu_at_the_same_cycle_under_any_host_slicing`,
`a_dma_stall_that_guest_dma_ends_resumes_the_cpu_identically_under_any_host_slicing`
and `a_dma_livelock_during_halt_keeps_the_cpu_halted_under_any_host_slicing`
in [`machine_b.rs`](../../../systems/gba/tests/machine_b.rs), and the
`probe-dma_stall` parity scenario).

ZiGBA instead ends the burst at the limit with a terminal `dma_livelock`
fault. Against hardware, the interrupted instruction's remaining accesses
happen before the stall instead of after it; for hosts, the call that
reaches the limit returns about one second late. Save states are refused
during a stall. The decision and its evidence
are in the [roadmap](../../roadmap.md#dma-that-never-releases-the-bus).

## Interrupts

| Register | Address     | Meaning                                |
| -------- | ----------- | -------------------------------------- |
| `IE`     | `04000200h` | enable mask, bits 0–13                 |
| `IF`     | `04000202h` | request flags; writing 1 clears a flag |
| `IME`    | `04000208h` | bit 0: master enable                   |

| Bit  | Source       | Requested when                                                                       |
| ---- | ------------ | ------------------------------------------------------------------------------------ |
| 0    | VBlank       | line 160 begins and `DISPSTAT` bit 3 is set                                          |
| 1    | HBlank       | cycle 960 of any line with `DISPSTAT` bit 4 set                                      |
| 2    | VCOUNT match | the line equal to LYC begins and `DISPSTAT` bit 5 is set                             |
| 3–6  | Timers 0–3   | overflow with `TMxCNT_H` bit 6 set                                                   |
| 7    | Serial       | serial completion or status event ([`serial.md`](serial.md))                         |
| 8–11 | DMA 0–3      | transfer completes with `DMAxCNT_H` bit 14 set                                       |
| 12   | Keypad       | the `KEYCNT` condition becomes true with bit 14 set                                  |
| 13   | Game Pak     | an access to the cartridge RTC's interrupt register ([`cartridge.md`](cartridge.md)) |

A source sets its `IF` bit whenever its own enable conditions hold,
independently of `IE`, `IME` and the CPU's CPSR.I bit; those only decide
whether the CPU reacts. `IF` writes clear only the bits written as one, per
byte lane. After every change to the three registers, `Bus::update_irq_line`
recomputes `IME && (IE & IF) != 0`, and the CPU takes the IRQ at the next
instruction boundary where that line is set and CPSR.I is clear
([`cpu.md`](cpu.md); [GBATEK, GBA Interrupt Control][gbatek-irq]).

`KEYCNT` (`04000132h`) selects keys in bits 0–9, AND (bit 15 set) or OR
condition, and the IRQ enable in bit 14. ZiGBA requests the keypad IRQ when
the condition turns from false to true; `probe-keypad` pins that edge model,
which rejects IRQs on release. Whether hardware is edge- or level-triggered
is unmeasured.

## HALT and STOP

Writing `HALTCNT` (`04000301h`) with a byte enters Halt when bit 7 is clear
and Stop when it is set ([GBATEK, GBA System Control][gbatek-system]). ZiGBA
also enters them on a halfword write to `04000300h` (`POSTFLG` and `HALTCNT`
together), selecting by bit 15. The BIOS wait services write the same
register ([`bios.md`](bios.md)). A multi-step HLE BIOS service defers Halt
and Stop requests made by its own writes until it returns.

**Halt.** The CPU stops until `IE & IF` is non-zero. `IME` and CPSR.I do not
matter for waking: GBATEK says Halt "passes through even if either one has
disabled interrupts" ([BIOS Halt Functions][gbatek-halt]). Everything else
keeps running: the LCD, timers, DMA (including VBlank, HBlank and sound DMA),
audio, serial and keypad. When the CPU wakes it enters the IRQ handler before
the next instruction if `IME` and CPSR.I allow; otherwise it continues with
the instruction after the one that halted. A halted bus advances from event
to event (`Bus::idle_halt_until`), so the wake happens at the exact cycle of the
waking event however the host slices time. Tests "Halt wake enters an enabled
IRQ before executing the interrupted instruction", "direct step resumes a
pending Halt wake independently of IME" and "HALTCNT byte writes select Halt
or Stop and Halt resumes at the following instruction" pin these rules.

**Stop.** GBATEK says most of the hardware, including sound and video, is
paused. ZiGBA wakes only on an enabled keypad (bit 12), Game Pak (bit 13) or
serial (bit 7) request (`Bus::stop_wake_pending`), and freezes every device
clocked by the system clock: timers and the prescaler counter, the LCD,
audio, DMA and Game Pak prefetch. The serial peripherals are the exception.
SIO, UART and JoyBus transfers are clocked from outside, so a stopped machine
still advances through their armed deadlines, and a completion can wake it
([`serial.md`](serial.md)). The master cycle count advances across those
spans, so `Bus::idle_stop_until` moves `prescaler_origin` by each one to keep
the prescaler counter frozen with the timers. `probe-halt-stop` wakes Halt
with a timer IRQ and Stop with a key press; `probe-stop-sio` wakes Stop with
a link transfer; ZiGBA's test "HLE Stop freezes system clocks and runUntil
reports the stopped state" checks the frozen timer and LCD.

**Host view.** GBATEK notes that the GBA locks up if Halt's condition never
becomes true. Evenfall never blocks the host: `Machine::step` returns a waiting
outcome without advancing time while the CPU is halted or stopped, and
`Machine::run_until` and `Machine::wait_until` advance a waiting machine only up
to the caller's deadline, stopping at the first wake. A stopped machine with
no armed serial event reports `power_stopped` without advancing its clock
(see [`machine.rs`](../../../systems/gba/src/machine.rs)).

## Open questions

- **One shared prescaler counter.** ZiGBA uses one counter for all four
  timers. The suite tests only timer 0, and per-timer counters that are never
  reset and start together cannot be told apart from one.
- **Prescaler phase at power-on and after the BIOS boot.** ZiGBA starts the
  counter at cycle zero in every boot mode. The suite cannot see the absolute
  phase, and HLE boot and `IntrWait` skip BIOS code, so HLE runs do not
  reproduce the suite's absolute counts. alyosha's `gba-tests` timer notes
  (second-hand) describe the post-power-on divider state as dependent on
  exact power-on timing.
- **Divisor changes.** ZiGBA adds no extra tick when a running timer's
  divisor changes and the selected tap switches.
- **Prescaler during STOP.** ZiGBA freezes the counter with the timers; no
  hardware test covers it.
- **Counter reset by disable.** Disfavored by the suite fit, not excluded
  without an official-BIOS run of the harness.
- **Timer enable edge.** Pinned NanoBoyAdvance applies a control write one
  cycle late and the reload one cycle after that (its `CHANGELOG.md` credits
  alyosha for the enable-delay overflow case). ZiGBA keeps its own latency
  until a focused hardware test decides.
- **HBlank DMA on VBlank lines.** Three implementations agree it does not
  run; a hardware capture across lines 160–227 would make it a measured
  fact.
- **Repeat reload of a misaligned `DAD`.** No hardware capture isolates the
  reload path.
- **DMA prefetch collision.** Whether a DMA read of ROM, not only a DMA
  write, waits for a prefetch read with more than one cycle left is
  unmeasured ([memory](memory.md#open-questions)).
- **Interrupted Game Pak access.** The N − S charge for a CPU Game Pak access
  that a DMA interrupts fits every prefetch-off suite column, but the bus
  mechanism behind it, including why a transfer starting on the access's
  final cycle still counts, is not established.
- **VBlank and HBlank start delay.** The three-cycle delay follows pinned
  mGBA; no suite row times it.
- **Unmodeled DMA features.** DMA3 video capture, the Game Pak DRQ bit and
  the special timing of DMA0 do nothing.
- **Release after a long stall.** No capture times how the CPU resumes when
  DMA releases the bus after a stall; the core takes a pending interrupt at
  the instruction boundary where the stall ends.
- **Keypad IRQ trigger.** ZiGBA is edge-triggered; level behavior has not
  been measured.

[gbatek-timers]: https://rust-console.github.io/gbatek-gbaonly/#gbatimers
[gbatek-dma]: https://rust-console.github.io/gbatek-gbaonly/#gbadmatransfers
[gbatek-irq]: https://rust-console.github.io/gbatek-gbaonly/#gbainterruptcontrol
[gbatek-system]: https://rust-console.github.io/gbatek-gbaonly/#gbasystemcontrol
[gbatek-halt]: https://rust-console.github.io/gbatek-gbaonly/#bioshaltfunctions
[suite-timers]: https://github.com/mgba-emu/suite/blob/e6942030d25ffe3ba76c72b73a86da073ec857cc/src/timers.c
[mgba-timer]: https://github.com/mgba-emu/mgba/blob/0.10.5/src/gba/timer.c
