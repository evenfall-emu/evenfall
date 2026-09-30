# Serial I/O

The link port on the side of the GBA connects it to other consoles.
Games used it for two-to-four-player battles and trading, to send a game to other GBAs that have no cartridge, and to talk to a GameCube.
One port does all of this through a handful of modes:

- **Normal** mode sends 8 or 32 bits at a time between two consoles, one providing the clock.
- **Multiplayer** mode exchanges one 16-bit word per console among up to four consoles on the multi-player cable.
- **UART** mode behaves like a classic serial port, with queues and flow control.
- **General-purpose** mode turns the four pins into plain inputs and outputs.
- **JoyBus** mode makes the GBA answer commands from a GameCube.

Evenfall models each mode as a device behind the same registers.
Linked consoles are separate emulated machines that only exchange data through one function, `synchronize_sio`, which puts all their transfers on one timeline.
The result is deterministic: the same inputs on every console give the same exchange, however the host runs them.

## Where it lives

The registers, transfer events and cable coupling are in [`bus/mod.rs`](../../../systems/gba/src/bus/mod.rs) (`synchronize_sio`, and the SIO, UART and JoyBus types).
The scheduler that runs several machines together is `run_linked_until` in [`machine.rs`](../../../systems/gba/src/machine.rs).
The BIOS MultiBoot routine uses the same registers and is described in [`bios.md`](bios.md#multiboot).

The hardware reference is GBATEK's [communication ports](https://rust-console.github.io/gbatek-gbaonly/#gbacommunicationports) chapter.
mGBA 0.10.5 and the mGBA hardware suite ([`sio-read.c`](https://github.com/mgba-emu/suite/blob/e6942030d25ffe3ba76c72b73a86da073ec857cc/src/sio-read.c), [`sio-timing.c`](https://github.com/mgba-emu/suite/blob/e6942030d25ffe3ba76c72b73a86da073ec857cc/src/sio-timing.c)) are the comparisons and test sources.
[`refs/README.md`](../../../refs/README.md) records the exact versions.

## A few terms

- **Parent and child.** In Multiplayer mode, one console (the parent, first in cable order) starts every transfer, and the others (children) follow.
- **Internal and external clock.** In Normal mode, one console drives the clock that paces the bits (internal clock), and the other follows it (external clock).
- **CTS.** "Clear to send": in UART mode, a signal from the receiver saying it has room for another byte.
- **Baud.** Bits per second.

## Choosing a mode

`RCNT` bits 15–14 and `SIOCNT` bits 13–12 select the mode together ([GBATEK summary](https://rust-console.github.io/gbatek-gbaonly/#siocontrolregisterssummary)):

| `RCNT.15` | `RCNT.14` | `SIOCNT.13` | `SIOCNT.12` | Mode                   |
| --------- | --------- | ----------- | ----------- | ---------------------- |
| 0         | x         | 0           | 0           | Normal, 8-bit          |
| 0         | x         | 0           | 1           | Normal, 32-bit         |
| 0         | x         | 1           | 0           | Multiplayer, 16-bit    |
| 0         | x         | 1           | 1           | UART                   |
| 1         | 0         | x           | x           | general purpose (GPIO) |
| 1         | 1         | x           | x           | JoyBus                 |

`RCNT` starts at `8000h`, which is general-purpose mode, so a game must clear `RCNT.15` before any Normal, Multiplayer or UART transfer can start.

Changing mode has immediate side effects:

- Setting `RCNT.15` cancels a pending Normal or Multiplayer transfer.
- Entering or leaving UART mode clears all UART state: both queues, the byte being sent, any byte arriving, the error flag and the CTS grant.
- Entering or leaving JoyBus mode cancels a pending JoyBus command and any response not yet taken.
- Every `SIOCNT` write cancels a pending Normal or Multiplayer completion.
  If the written value has START set (and `RCNT.15` is clear), a new transfer starts at that cycle.

## Registers by mode

The same addresses mean different things in each mode:

| Address             | Normal-8   | Normal-32        | Multiplayer              | UART                           |
| ------------------- | ---------- | ---------------- | ------------------------ | ------------------------------ |
| `04000120h`         | reads 0    | `SIODATA32_L`    | `SIOMULTI0` (parent)     | reads 0                        |
| `04000122h`         | reads 0    | `SIODATA32_H`    | `SIOMULTI1`              | reads 0                        |
| `04000124h`, `126h` | reads 0    | reads 0          | `SIOMULTI2`, `SIOMULTI3` | reads 0                        |
| `04000128h`         | `SIOCNT`   | `SIOCNT`         | `SIOCNT`                 | `SIOCNT` with live status      |
| `0400012Ah`         | `SIODATA8` | `SIODATA8` latch | `SIOMLT_SEND`            | send on write, receive on read |
| `04000134h`         | `RCNT`     | `RCNT`           | `RCNT`                   | `RCNT`                         |

`SIODATA32` can only be read and written while `SIOCNT` selects 32-bit Normal mode.
That is why GBATEK tells software to select the mode before writing the data.
`SIOMULTI0`–`3` are read-only to the CPU.
The JoyBus registers are described under [JoyBus](#joybus).
The unused halfwords at `136h`, `142h` and `15Ah` read zero and ignore writes.

`SIOCNT` mixes settings with live status:

| Mode        | Writable bits                                                                                                                      | Read-only bits                                        |
| ----------- | ---------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------- |
| Normal      | `7F8Bh`: clock source (0), 2 MHz (1), SO when idle (3), START (7), 8–14                                                            | SI state (2) reads 1, 4–6 and 15 read 0               |
| Multiplayer | `7F83h`: baud (1:0), START (7), 8–14                                                                                               | SI terminal (2), SD terminal (3), ID (5:4), error (6) |
| UART        | `7F8Fh`: baud (1:0), CTS (2), parity kind (3), data length (7), FIFO (8), parity (9), send enable (10), receive enable (11), 12–14 | send full (4), receive empty (5), error (6)           |

`RCNT` bits 4–8 and 14 can be written, and bits 9–13 read zero in every mode.
Bits 0–3 show the levels of the four lines, which GBATEK calls undocumented.
Evenfall doesn't model line levels on a cable.
It returns what the suite saw with nothing plugged in: `5h` in Normal mode, `Fh` in Multiplayer and UART, `Ch` in JoyBus.
In general-purpose mode, bits 0–3 read back what was written.

### With nothing plugged in

The suite's `sio-read.c` selects six modes on a console with nothing plugged in, writes all ones to fifteen addresses and reads them back.
For `SIOCNT` and `RCNT` the write keeps the mode bits (`CFFFh` or `3FFFh` plus the mode), and every other address gets `FFFFh`.
Evenfall reproduces all 90 results:

| Address                | Normal-8 | Normal-32 | Multiplayer | UART   | GPIO   | JoyBus |
| ---------------------- | -------- | --------- | ----------- | ------ | ------ | ------ |
| `120h`                 | `0000`   | `FFFF`    | `0000`      | `0000` | `0000` | `0000` |
| `122h`                 | `0000`   | `FFFF`    | `0000`      | `0000` | `0000` | `0000` |
| `124h`, `126h`         | `0000`   | `0000`    | `0000`      | `0000` | `0000` | `0000` |
| `128h`                 | `4F8F`   | `5F8F`    | `6F8F`      | `7FAF` | `4F8F` | `4F8F` |
| `12Ah`                 | `FFFF`   | `FFFF`    | `FFFF`      | `0000` | `FFFF` | `FFFF` |
| `134h`                 | `01F5`   | `01F5`    | `01FF`      | `01FF` | `81FF` | `C1FC` |
| `136h`, `142h`, `15Ah` | `0000`   | `0000`    | `0000`      | `0000` | `0000` | `0000` |
| `140h`                 | `0040`   | `0040`    | `0040`      | `0040` | `0040` | `0040` |
| `150h`–`156h`          | `0000`   | `0000`    | `0000`      | `0000` | `0000` | `0000` |

The GPIO and JoyBus rows leave `SIOCNT` in its Normal-8 setting.
`JOYSTAT` (`158h`) isn't in the table because the suite's rows labelled `JOYSTAT` actually read `15Ah` ([JoyBus](#joybus)).

## Normal mode

Normal mode shifts 8 or 32 bits, most significant first, between the console that drives the clock (`SIOCNT.0 = 1`) and the one that follows it.
With the internal clock, one bit takes 64 master cycles at 256 KHz or 8 cycles at 2 MHz ([GBATEK Normal mode](https://rust-console.github.io/gbatek-gbaonly/#sionormalmode)):

| Length | 256 KHz      | 2 MHz      |
| ------ | ------------ | ---------- |
| 8-bit  | 512 cycles   | 64 cycles  |
| 32-bit | 2,048 cycles | 256 cycles |

The finish time is fixed when START is written.
At completion, Evenfall clears START and stores the received value (all ones with nothing plugged in).
In 8-bit mode it goes in the low byte of `SIODATA8`, keeping the high byte as written, and in 32-bit mode in both `SIODATA32` halves.
If `SIOCNT.14` is set, the serial interrupt (`IF` bit 7) is requested.
Completion is an ordinary device event, so an enabled serial interrupt wakes a halted CPU on exactly that cycle.

These times agree with the suite's `sio-timing.c` Normal rows: each result is the transfer time plus 121 cycles.
Those 121 cycles are the test's own overhead, inferred from the four cases.
The suite's Multiplayer rows expect a timeout with nothing plugged in, and Evenfall likewise leaves a Multiplayer START with no partner pending.

A transfer on the external clock waits until a console driving the clock is linked.
A START in general-purpose or JoyBus mode never runs.

## Link cables

How many consoles are linked doesn't say which cable connects them, so the caller names it with `SioConnection`:

| `SioConnection` | Consoles            | Normal mode      | Multiplayer                   | UART                          |
| --------------- | ------------------- | ---------------- | ----------------------------- | ----------------------------- |
| `Cable2p`       | exactly 2           | two-way exchange | not wired (`UnsupportedMode`) | byte delivery                 |
| `CableMulti`    | 2–4, in cable order | one-way chain    | supported                     | not wired (`UnsupportedMode`) |

On the two-player cable, a Normal transfer is a two-way exchange.
Exactly one console drives the clock, each receives what the other sent, and both finish at the time set by the clock driver's START.
Two internal clocks are a `ClockConflict`, and two external clocks just wait.

GBATEK describes Normal mode over the multi-player cable as a one-way chain.
Data moves from the first console to the last, each console receives the data register of the one before it, and the parent receives zero because its input line is grounded ([GBATEK, one-way multiplayer](https://rust-console.github.io/gbatek-gbaonly/#sionormalmode)).
Evenfall requires the first console to drive the clock and every other to follow it.
Each child receives what its predecessor's data register holds when the transfer is linked, and the parent receives zero.
mGBA 0.10.5 has the same chain, but gives the parent all ones in a [receive path marked TODO](https://github.com/mgba-emu/mgba/blob/0.10.5/src/gba/sio/lockstep.c#L249-L276).
Evenfall follows GBATEK.

GBATEK marks direct GBA-to-GBA UART wiring and its voltages as uncertain.
UART over `Cable2p` is a way for emulated consoles to talk, not a claim about a safe physical cable.

## Multiplayer

Multiplayer mode exchanges one 16-bit word per console among up to four consoles, and only works on `CableMulti` ([GBATEK Multi-Player mode](https://rust-console.github.io/gbatek-gbaonly/#siomultiplayermode)).
It is how most four-player GBA games talk.

- Cable order sets the IDs.
  The first console is the parent (ID 0), and the rest are children 1–3.
- While every console is in Multiplayer mode, `SIOCNT.3` reads 1 on all of them, and `SIOCNT.2` reads 0 on the parent and 1 on children.
  Before they are linked, both read 1.
- Only the parent starts a transfer.
  A child's START does nothing while the parent is idle.
- Once the parent's START is linked, every console's `SIOMULTI0`–`3` read `FFFFh` and START reads 1 (busy) until the transfer completes.
- At completion every console receives the same four words: slot n holds console n's `SIOMLT_SEND`, and slots with no console stay `FFFFh`.
  `SIOCNT.5:4` gets the console's ID, START clears, and the serial interrupt is requested if `SIOCNT.14` is set.
- The error bit (`SIOCNT.6`) is never set.

The transfer takes a time set by the parent's baud and the number of consoles.
Evenfall uses mGBA 0.10.5's [`GBASIOCyclesPerTransfer`](https://github.com/mgba-emu/mgba/blob/0.10.5/src/gba/sio.c#L14-L19) table:

| `SIOCNT.1:0` | Baud    | 2 consoles | 3 consoles | 4 consoles |
| ------------ | ------- | ---------- | ---------- | ---------- |
| 0            | 9,600   | 73,003     | 107,680    | 142,356    |
| 1            | 38,400  | 18,251     | 26,920     | 35,589     |
| 2            | 57,600  | 12,167     | 17,947     | 23,726     |
| 3            | 115,200 | 6,075      | 8,973      | 11,863     |

No available hardware test measures these times.

## UART

UART mode works much like a PC's RS-232 serial port ([GBATEK UART mode](https://rust-console.github.io/gbatek-gbaonly/#siouartmode)).
mGBA 0.10.5 recognizes the mode but doesn't implement any of its sending, queues, status, errors or interrupts, so it is no comparison for anything below except reading the registers back.

### Registers

`SIOCNT` in UART mode:

| Bit   | Meaning                                                           | Access                                 |
| ----- | ----------------------------------------------------------------- | -------------------------------------- |
| 1:0   | baud: 9,600, 38,400, 57,600 or 115,200 bits per second            | read and write                         |
| 2     | CTS: 0 sends regardless, 1 sends only while the receiver is ready | read and write                         |
| 3     | parity: 0 even, 1 odd                                             | read and write                         |
| 4     | send queue full                                                   | read-only                              |
| 5     | receive queue empty                                               | read-only                              |
| 6     | error                                                             | read-only, cleared by reading `SIOCNT` |
| 7     | data length: 0 = 7 bits, 1 = 8 bits                               | read and write                         |
| 8     | FIFO (four-byte queues) enable                                    | read and write                         |
| 9     | parity enable                                                     | read and write                         |
| 10    | send enable                                                       | read and write                         |
| 11    | receive enable                                                    | read and write                         |
| 13:12 | mode, both 1                                                      | read and write                         |
| 14    | interrupt enable                                                  | read and write                         |
| 15    | unused, reads 0                                                   | read-only                              |

Idle status is `0020h` (receive queue empty), so writing every setting bit reads back `7FAFh`, as in the suite.
GBATEK's description of CTS contradicts its own bit table, and Evenfall follows the table.

`SIODATA8` (`12Ah`) has separate send and receive paths.
A read takes the oldest received byte (0 when there is none), and a write queues a byte to send.
Only the low byte carries data.
The high byte reads 0, and a byte write to `12Bh` is ignored.

### Sending

A written byte is queued even while sending is disabled.
The send queue holds one waiting byte with the FIFO off and four with it on, besides the byte being sent, and a write beyond that is lost.
The oldest byte starts sending when `SIOCNT.10` is set and either CTS is off or the receiver has granted it.
When it finishes, the next one starts.

On a GBA, CTS is a wire driven by the receiver.
Evenfall models the grant directly: at each `synchronize_sio`, a sender gets CTS while its partner has receiving enabled, has room in its queue, and has no byte already arriving.
Each byte sent uses up the grant.
A sender waiting for CTS with no partner therefore never starts, while a sender ignoring CTS sends on time and its byte is lost.

`SIOCNT.4` (send full) reads 1 while any byte is being sent or waiting with the FIFO off, and while four bytes wait with it on.

### Receiving

When a byte is sent, the receiver is due to get it at the sender's completion cycle, if receiving is enabled then.
On arrival, if receiving is still enabled, the byte goes into the receive queue (one byte, or four with the FIFO).
If the queue is full, the byte is lost and the error flag is set.
`SIOCNT.5` (receive empty) reads 1 while the queue is empty.

Turning the FIFO off in UART mode empties both queues, as GBATEK documents.
Since entering UART mode also clears all UART state, a game must write `SIOCNT` before queuing data.

### Errors and interrupts

GBATEK lists three causes of error: a bad stop bit, a parity mismatch and a full receive queue.
Evenfall links consoles byte by byte, not bit by bit, so only a full queue can happen.
Reading `SIOCNT` returns the error bit and then clears it, so the read that clears it sees the 1.

With `SIOCNT.14` set, Evenfall requests the serial interrupt whenever status bit 4, 5 or 6 changes from 0 to 1.
That is a strict reading of GBATEK's "become set": turning the interrupt on while a bit is already 1 requests nothing.
Since "receive empty" is 1 at idle, reading the last received byte raises an interrupt, and so does a write that fills the send queue.

### Timing

The only known timing values are the four nominal baud rates.
Evenfall derives how long a byte takes from them, in [`bus/mod.rs`](../../../systems/gba/src/bus/mod.rs):

```text
cycles = ceil(frame_bits * 16777216 / baud)
frame_bits = 1 start bit + 7 or 8 data bits + optional parity bit + 1 stop bit
```

| Frame         | 9,600  | 38,400 | 57,600 | 115,200 |
| ------------- | ------ | ------ | ------ | ------- |
| 8N1 (10 bits) | 17,477 | 4,370  | 2,913  | 1,457   |
| 7N1 (9 bits)  |        |        |        | 1,311   |
| 8E1 (11 bits) |        |        |        | 1,602   |

No available source gives the GBA's actual baud divider, its rounding or its bit timing, and the suite has no UART timing test.
The formula sits in one place, so a hardware measurement can replace it without changing anything else.

## JoyBus

In JoyBus mode (`RCNT.15:14 = 11b`) the GBA answers commands from an outside master, such as a GameCube.
Games like The Legend of Zelda: Four Swords Adventures and Final Fantasy Crystal Chronicles use it to turn a GBA into a controller with its own screen.
The GBA can't send commands itself, so Evenfall models JoyBus as a way for the host to act as the master, not as another cable ([GBATEK JOY BUS mode](https://rust-console.github.io/gbatek-gbaonly/#siojoybusmode)).
`SIOCNT` plays no part.

### Sending commands

```rust
pub fn schedule_joy_bus(
    &mut self,
    command: JoyBusCommand,
    completion_cycle: Cycle,
) -> Result<(), JoyBusError>
pub fn take_joy_bus_response(&mut self) -> Option<JoyBusResponse>
```

`JoyBusCommand` is `Reset`, `Status`, `ReadFromGba` or `WriteToGba(u32)`, and a `JoyBusResponse` holds up to five bytes.
One command can be in flight at a time.
Scheduling fails with `NotInJoyBusMode`, `CompletionBeforeCurrentCycle`, or `TransactionPending` while a command or an untaken response is outstanding.
The command takes effect all at once at its completion cycle, as an ordinary device event.

The caller chooses that cycle because no available source gives JoyBus's bit rate, framing or synchronization, and the core has no default.
Leaving JoyBus mode and the serial part of `RegisterRamReset` ([Reset](#reset)) cancel the pending command and response.
The commands are typed, so a malformed command can never reach the core.

### Commands

Names are from the master's side.
Payloads travel least significant byte first, so the type bytes `00h 04h` read as `0400h`.

| Command      | Master sends     | GBA replies                   | `JOYCNT` flag | `JOYSTAT`    | Serial interrupt if `JOYCNT.6` |
| ------------ | ---------------- | ----------------------------- | ------------- | ------------ | ------------------------------ |
| `FFh` RESET  | command          | `00h 04h`, status             | bit 0         | unchanged    | yes                            |
| `00h` STATUS | command          | `00h 04h`, status             | none          | unchanged    | no                             |
| `14h` READ   | command          | `JOY_TRANS` (4 bytes), status | bit 2         | clears bit 3 | yes                            |
| `15h` WRITE  | command, 4 bytes | status                        | bit 1         | sets bit 1   | yes                            |

The status byte is the low byte of `JOYSTAT`.
The order of events within a command follows mGBA.
RESET sets its flag before building the reply.
WRITE stores `JOY_RECV` and sets its flags before reading the status byte.
READ takes the payload and status before setting `JOYCNT.2` and clearing `JOYSTAT.3`.
RESET only notifies the GBA, and doesn't reset the machine or clear any register.

### Registers

- **`JOYCNT` (`140h`).** Bits 0–2 are the RESET, received and sent flags, cleared by writing 1.
  Bit 6 enables the serial interrupt.
  Other bits read 0.
- **`JOY_RECV` (`150h`, `152h`).** Filled by WRITE.
  Reading either half clears `JOYSTAT.1`.
  CPU writes are ignored.
- **`JOY_TRANS` (`154h`, `156h`).** Read by READ.
  Writing either half sets `JOYSTAT.3`.
  CPU reads return 0.
- **`JOYSTAT` (`158h`).** Bit 1 is "data received", bit 3 "data to send", and bits 4–5 are free for the game to write.
  The CPU can't change bits 1 and 3.

A completed RESET, READ or WRITE with `JOYCNT.6` set requests the serial interrupt, which wakes a halted CPU at the exact cycle the caller chose.

### Where the sources disagree

GBATEK is sure the `JOYCNT.6` interrupt fires for RESET, but not whether READ and WRITE fire it, nor which of them sets bits 1 and 2.
mGBA fires it for all three, sets bit 1 for WRITE and bit 2 for READ, and Evenfall follows mGBA.

For `JOYSTAT`, GBATEK says a CPU write to `JOY_TRANS` sets bit 1 and a CPU read of `JOY_RECV` clears bit 3.
mGBA pairs them the other way round, treating bit 1 consistently as "received data not read yet" and bit 3 as "data waiting to be sent".
Evenfall uses mGBA's consistent pairing.
The suite can't settle it, because every row it labels `JOYSTAT` actually reads the reserved address `0400015Ah`.

The suite's JoyBus row writes each payload halfword and reads 0 back, which Evenfall matches.
mGBA 0.10.5 returns the written payload instead, so it is only a reference for the commands here.

## Linking consoles

The core has no network code.
Linked consoles are separate `Machine` values whose cycle counters start from the same origin.
Two functions couple them: `synchronize_sio` works on their buses, and `run_linked_until` schedules the machines around it.

### `synchronize_sio`

```rust
pub fn synchronize_sio(
    peers: &mut [&mut Bus],
    connection: SioConnection,
) -> Result<bool, SioLinkError>
```

`synchronize_sio` takes two to four buses in cable order and links whatever transfer is ready: a Normal exchange once every console has START set, a Multiplayer transfer once the parent has started, or UART bytes waiting to be sent.
It returns `false` while nothing can progress, including when the consoles are in different modes or any has `RCNT.15` set.

| Error                  | Cause                                                                          |
| ---------------------- | ------------------------------------------------------------------------------ |
| `UnsupportedPeerCount` | fewer than 2 or more than 4 buses, or `Cable2p` with other than 2              |
| `UnsupportedMode`      | Multiplayer on `Cable2p`, or UART on `CableMulti`                              |
| `ClockConflict`        | two internal clocks on `Cable2p`, or a child driving the clock on `CableMulti` |
| `MissedDeadline`       | a console's clock is already past the transfer's completion cycle              |

The completion cycle always comes from the clock driver's START (or the UART sender's start), so linking late within the transfer doesn't move it.
A console that has already run past that cycle can't receive data in its past.
Evenfall reports `MissedDeadline` instead of quietly reordering events.

### `run_linked_until`

```rust
pub fn run_linked_until(
    machines: &mut [&mut Machine],
    deadline: Cycle,
    connection: SioConnection,
) -> Result<LinkedRunResult, LinkedRunError>
```

`run_linked_until` advances two to four machines to `deadline` on one timeline.
Each round, it:

1. stops with `LinkedStopReason::Faulted` if any machine has faulted, naming it,
2. calls `synchronize_sio`, treating `UnsupportedMode` as "nothing to link",
3. stops with `LinkedStopReason::Deadline` once every machine has reached `deadline`,
4. runs one instruction on the machine furthest behind that can run: one that is running, halted or stopped with a wake pending, or inside a BIOS routine that is still going,
5. otherwise advances the devices of the halted machine furthest behind toward `deadline`, stopping early if something wakes it,
6. otherwise advances stopped machines through their pending serial transfers only,
7. stops with `LinkedStopReason::PowerStopped` if nothing could progress.

Instructions can't be split, so machines can end slightly past `deadline`.
`LinkedRunResult` reports the earliest and latest machine cycles, so the difference is explicit.
Passing the same machine twice is `DuplicateMachine`, and any other `SioLinkError` is passed on.

### Serial transfers during STOP

STOP freezes timers, video, sound, DMA and the cartridge prefetch, and only a serial, keypad or cartridge interrupt wakes it ([`timers-dma-interrupts.md`](timers-dma-interrupts.md)).
The serial port is clocked from outside the console, so `Bus::idle_stop_until` still advances a stopped machine through its pending Normal, Multiplayer, UART and JoyBus transfers, and nothing else.
A finished transfer that raises an enabled serial interrupt wakes the CPU.
This is how a MultiBoot client sleeps while the parent sends it a game.
A timed host input inside the frozen span stops the advance, so the caller can apply it.
A stopped machine with nothing pending is reported without advancing its clock.
`Machine::run_until` uses the same path for a single machine.

## Reset

| Event                         | Effect on the serial port                                                                                                                         |
| ----------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| Power-on                      | `RCNT = 8000h`, every other serial register and queue empty                                                                                       |
| `RegisterRamReset`, any flags | clears `04000120h` (`SIODATA32_L`, `SIOMULTI0`), a BIOS bug [GBATEK documents](https://rust-console.github.io/gbatek-gbaonly/#biosresetfunctions) |
| `RegisterRamReset` bit 5      | resets the serial port, listed below                                                                                                              |
| HLE `HardReset`               | `RegisterRamReset(FFh)`, plus all four data halfwords `120h`–`126h` cleared                                                                       |
| JoyBus RESET command          | only `JOYCNT.0` and the optional interrupt                                                                                                        |

`RegisterRamReset` bit 5 sets `SIOCNT = 0`, `RCNT = 8000h` and `SIODATA8 = 0`, cancels a pending transfer, sets the Multiplayer words to `FFFFh` and the ID to 0, clears the UART queues, the byte being sent, any arrival, the error and CTS, zeroes `JOYCNT`, `JOY_RECV`, `JOY_TRANS` and `JOYSTAT`, and cancels a JoyBus command and response.
It leaves `122h`–`126h` alone.
mGBA's `RegisterRamReset` doesn't clear `JOYSTAT`, and Evenfall does.
The HLE reset routines are described in [`bios.md`](bios.md#resets).

## Not implemented

- **Linking over a network or between browser tabs.** The CLI and the web player run one machine.
  Linking is only available in Rust, through `run_linked_until` and `synchronize_sio`.
- **Line levels.** In Normal mode `SIOCNT.2` always reads 1 and `SIOCNT.3` isn't passed to the partner, so the ready handshake GBATEK recommends isn't modeled.
  `RCNT` bits 0–3 return fixed values even on a cable.
  General-purpose mode has no outside pin source and never raises its interrupt, including the one GBATEK says can wake STOP.
- **Multiplayer cable faults.** No error bit, bad connection or unplugging.
- **UART line errors.** No framing or parity errors, and no bit-level timing.
- **A JoyBus master.** No GameCube, wire timing or default duration.
- **Wireless and infrared adapters.**

## How it is checked

- [`tests/bus.rs`](../../../systems/gba/tests/bus.rs) holds the serial tests: the 90-value read matrix above, Normal timing, both cables, Multiplayer timing and empty slots, UART queues, overflow, CTS and the frame times above, JoyBus commands and scheduling, and waking from Halt.
- [`tests/machine_a.rs`](../../../systems/gba/tests/machine_a.rs) and [`tests/machine_b.rs`](../../../systems/gba/tests/machine_b.rs) cover the linked scheduler and the reset table above.
- The probe ROMs `probe-link`, `probe-multi4`, `probe-stop-sio`, `probe-uart`, `probe-joybus`, `probe-joybustx`, `probe-mb2slave` and `probe-mb2mhz` run complete exchanges between machines through `run_linked_until` ([`probe/README.md`](../../../probe/README.md)).
  `probe-stop-sio` checks a child that sleeps in STOP with interrupts off and is woken by the parent's Multiplayer transfer.

## Open questions

- Multiplayer transfer times come from mGBA's table, and no hardware measurement confirms them.
- The 121-cycle overhead that matches `sio-timing.c` is inferred from four cases.
- Normal mode on the multi-player cable gives the parent zero per GBATEK.
  No hardware capture confirms it against mGBA's `FFh`.
- GBATEK says many games test `RCNT.0` in Multiplayer mode.
  Evenfall returns a fixed 1.
- General-purpose mode reads back written bits for input pins, while GBATEK describes internal pull-ups.
- The UART baud divider, rounding and bit timing are unknown.
- Whether UART status bits raise interrupts on a change or while set, and whether turning the interrupt on with a bit already set raises one.
- Whether a UART byte can wait behind the one being sent with the FIFO off, and what writes to a full queue and reads from an empty one return.
- Whether the read that clears the UART error bit sees it, and GBATEK's unusual "stop bit must be 0".
- Whether JoyBus READ and WRITE raise the serial interrupt, and which `JOYSTAT` bits CPU accesses touch.
- JoyBus bit rate, framing, when the payload, flags and interrupt become visible, and whether JoyBus runs during STOP.
  Evenfall lets a pending JoyBus command wake STOP.
- 32-bit access to the JoyBus payload registers, and reading `JOY_TRANS` back after a command.
- Whether `RegisterRamReset` clears `JOYSTAT` on hardware.
