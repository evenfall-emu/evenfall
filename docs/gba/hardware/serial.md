# Serial I/O

The GBA link port has four signal lines (SC, SD, SI, SO) and one register
set whose meaning changes with the selected mode: Normal 8- or 32-bit,
Multiplayer, UART, general-purpose (GPIO) and JoyBus. Evenfall models each mode
as a device behind those shared registers, keeps write latches separate from
read visibility, and exchanges data between machines only through a
host-independent seam, `synchronize_sio`, which couples endpoints whose
transfers are ready and places them on one deterministic timeline.

The register model, transfer events and cable coupling live in
[`bus/mod.rs`](../../../systems/gba/src/bus/mod.rs) (`synchronize_sio` and the
SIO, UART and JoyBus types); the multi-machine scheduler is `run_linked_until`
in [`machine.rs`](../../../systems/gba/src/machine.rs). The BIOS MultiBoot service runs
over the same registers and is documented in [`bios.md`](bios.md).

The hardware reference is GBATEK's
[communication ports](https://rust-console.github.io/gbatek-gbaonly/#gbacommunicationports)
chapter. Pinned mGBA 0.10.5 and the pinned mGBA hardware suite
([`sio-read.c`](https://github.com/mgba-emu/suite/blob/e6942030d25ffe3ba76c72b73a86da073ec857cc/src/sio-read.c),
[`sio-timing.c`](https://github.com/mgba-emu/suite/blob/e6942030d25ffe3ba76c72b73a86da073ec857cc/src/sio-timing.c))
are the comparison and test sources.
[`refs/README.md`](../../../refs/README.md) records the exact commits.

## Mode selection

`RCNT.15:14` and `SIOCNT.13:12` jointly select the mode
([GBATEK summary](https://rust-console.github.io/gbatek-gbaonly/#siocontrolregisterssummary)):

| `RCNT.15` | `RCNT.14` | `SIOCNT.13` | `SIOCNT.12` | Mode                   |
| --------- | --------- | ----------- | ----------- | ---------------------- |
| 0         | x         | 0           | 0           | Normal, 8-bit          |
| 0         | x         | 0           | 1           | Normal, 32-bit         |
| 0         | x         | 1           | 0           | Multiplayer, 16-bit    |
| 0         | x         | 1           | 1           | UART                   |
| 1         | 0         | x           | x           | General purpose (GPIO) |
| 1         | 1         | x           | x           | JoyBus                 |

`RCNT` resets to `8000h`, which selects general-purpose mode, so software must
clear `RCNT.15` before any Normal, Multiplayer or UART transfer can start.
The `probe-link` ROMs do this explicitly.

Mode changes have side effects that Evenfall applies at the write:

- Setting `RCNT.15` cancels a pending Normal or Multiplayer transfer.
- Entering or leaving UART mode clears all UART state: both queues, the byte
  being shifted, any pending arrival, the error flag and the CTS grant.
- Entering or leaving JoyBus mode cancels a pending JoyBus command and any
  untaken response.
- Every `SIOCNT` write cancels a pending Normal or Multiplayer completion. If
  the written value has START set (and `RCNT.15` is clear), a new transfer
  starts at the current cycle.

## Registers by mode

The same addresses change meaning with the mode:

| Address             | Normal-8   | Normal-32        | Multiplayer              | UART                      |
| ------------------- | ---------- | ---------------- | ------------------------ | ------------------------- |
| `04000120h`         | reads 0    | `SIODATA32_L`    | `SIOMULTI0` (parent)     | reads 0                   |
| `04000122h`         | reads 0    | `SIODATA32_H`    | `SIOMULTI1`              | reads 0                   |
| `04000124h`, `126h` | reads 0    | reads 0          | `SIOMULTI2`, `SIOMULTI3` | reads 0                   |
| `04000128h`         | `SIOCNT`   | `SIOCNT`         | `SIOCNT`                 | `SIOCNT` with live status |
| `0400012Ah`         | `SIODATA8` | `SIODATA8` latch | `SIOMLT_SEND`            | TX write / RX read        |
| `04000134h`         | `RCNT`     | `RCNT`           | `RCNT`                   | `RCNT`                    |

`SIODATA32` is readable and writable only while `SIOCNT` selects 32-bit
Normal mode, which is why GBATEK tells software to select the mode before
writing the data; the `probe-mb2mhz` slave depends on this. `SIOMULTI0`–`3`
are read-only to the CPU. The JoyBus registers `JOYCNT` (`140h`), `JOY_RECV`
(`150h`), `JOY_TRANS` (`154h`) and `JOYSTAT` (`158h`) are described under
[JoyBus](#joybus). The unused halfwords at `136h`, `142h` and `15Ah` read
zero and ignore writes.

`SIOCNT` mixes writable control with live status. Evenfall's write masks and
fixed read bits are:

| Mode        | Writable bits                                                                                                               | Read-only bits                                        |
| ----------- | --------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------- |
| Normal      | `7F8Bh`: clock source (0), 2 MHz (1), SO when idle (3), START (7), 8–14                                                     | SI state (2) reads 1; 4–6 and 15 read 0               |
| Multiplayer | `7F83h`: baud (1:0), START (7), 8–14                                                                                        | SI terminal (2), SD terminal (3), ID (5:4), error (6) |
| UART        | `7F8Fh`: baud (1:0), CTS (2), parity kind (3), data length (7), FIFO (8), parity (9), TX enable (10), RX enable (11), 12–14 | TX full (4), RX empty (5), error (6)                  |

`RCNT` bits 4–8 and 14 are writable and bits 9–13 read zero in every mode.
Bits 0–3 report the SC, SD, SI and SO line levels, which GBATEK calls
undocumented. Evenfall does not model live line levels on a cable; it returns
the disconnected-port values the suite observed: `5h` in Normal mode, `Fh` in
Multiplayer and UART, `Ch` in JoyBus. In general-purpose mode bits 0–3 read
back the written data bits.

### Disconnected-port read matrix

The pinned suite's `sio-read.c` selects six modes on a console with nothing
plugged in, writes all ones to each of fifteen halfword addresses and reads
the value back. For `SIOCNT` and `RCNT` the write preserves the mode bits
(`CFFFh` or `3FFFh` plus the mode); every other address receives `FFFFh`.
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

The GPIO and JoyBus rows leave `SIOCNT` in its Normal-8 selection. The test
"SIO register masks and disconnected line states match the pinned mGBA
suite" in [`tests/bus.rs`](../../../systems/gba/tests/bus.rs) pins the matrix.
`JOYSTAT` (`158h`) is absent because the suite's rows labelled `JOYSTAT`
address `15Ah`; see [JoyBus](#joybus).

## Normal mode

Normal mode shifts 8 or 32 bits, MSB first, between a clock master (internal
clock, `SIOCNT.0 = 1`) and a slave (external clock). With the internal clock
one bit takes 64 master cycles at 256 KHz or 8 cycles at 2 MHz
([GBATEK Normal mode](https://rust-console.github.io/gbatek-gbaonly/#sionormalmode)):

| Length | 256 KHz      | 2 MHz      |
| ------ | ------------ | ---------- |
| 8-bit  | 512 cycles   | 64 cycles  |
| 32-bit | 2,048 cycles | 256 cycles |

The deadline is fixed when START is written. At completion Evenfall clears
START, stores the received value (all ones from a disconnected port) in the
low byte of `SIODATA8`, keeping the high byte as written, or in both
`SIODATA32` halves, and requests `IF` bit 7 if `SIOCNT.14` is set. The
completion is an ordinary device event, so a pending enabled serial IRQ wakes
HALT at exactly that cycle.

These durations agree with the suite's `sio-timing.c` Normal rows: each
reported timer value is the transfer duration plus a constant 121 cycles.
That 121-cycle harness overhead is inferred from the four cases, not a
published hardware rule. The suite's Multiplayer rows expect a timeout on a
disconnected port, and Evenfall likewise leaves a peerless Multiplayer START
pending.

An external-clock transfer stays pending until a clocked peer is coupled. A
START in general-purpose or JoyBus mode never runs.

## Link cables

The physical cable cannot be inferred from the number of machines, so the
caller names it with `SioConnection`:

| `SioConnection` | Endpoints           | Normal mode          | Multiplayer                   | UART                          |
| --------------- | ------------------- | -------------------- | ----------------------------- | ----------------------------- |
| `.cable_2p`     | exactly 2           | full-duplex exchange | not wired (`UnsupportedMode`) | byte delivery                 |
| `.cable_multi`  | 2–4, in cable order | one-way daisy chain  | supported                     | not wired (`UnsupportedMode`) |

On the standard two-player cable a Normal transfer is a full-duplex exchange:
exactly one endpoint drives the internal clock, each receives the other's
outgoing data, and both complete at the master's START deadline. Two
internal clocks are a `ClockConflict`; two external clocks simply wait.

GBATEK describes Normal mode over the multi-player cable as a one-way daisy
chain: data shifts from the first endpoint to the last, each endpoint
receives the preceding endpoint's data register, and the parent receives
zero because its SI line is grounded
([GBATEK, one-way multiplayer](https://rust-console.github.io/gbatek-gbaonly/#sionormalmode)).
Evenfall requires the first endpoint to use the internal clock and every other
endpoint the external clock, delivers each child the data register value its
predecessor holds when the transfer is coupled, and returns zero to the
parent. Pinned mGBA 0.10.5 has the same topology, but its
[TODO-marked receive path](https://github.com/mgba-emu/mgba/blob/0.10.5/src/gba/sio/lockstep.c#L249-L276)
gives the parent disconnected-high data (`FFh` or `FFFFFFFFh`). Evenfall follows
the hardware description.

GBATEK marks direct GBA-to-GBA UART wiring and voltage levels as uncertain, so
`.cable_2p` UART is an emulator connection model, not a claim about an
electrically safe physical cable.

## Multiplayer

Multiplayer exchanges one 16-bit word per endpoint among up to four consoles
and is valid only on `.cable_multi`
([GBATEK Multi-Player mode](https://rust-console.github.io/gbatek-gbaonly/#siomultiplayermode)).
Evenfall's protocol:

- Cable order assigns IDs: the first endpoint is the parent (ID 0), the rest
  are children 1–3.
- While every endpoint is in Multiplayer mode, `SIOCNT.3` (SD terminal) reads
  1 on all of them and `SIOCNT.2` (SI terminal) reads 0 on the parent and 1
  on children. Before coupling, both read 1 (disconnected).
- Only the parent starts a transfer. A child's START write has no effect
  while the parent is idle.
- When the parent's START is coupled, every endpoint's `SIOMULTI0`–`3` reads
  `FFFFh` and START (busy) reads 1 until completion.
- At completion every endpoint receives the same vector: slot _n_ holds
  endpoint _n_'s `SIOMLT_SEND`, and slots without an endpoint stay `FFFFh`.
  `SIOCNT.5:4` gets the endpoint's ID, START clears, and `IF` bit 7 is
  requested if `SIOCNT.14` is set.
- The error bit (`SIOCNT.6`) is never set.

The deadline is the parent's START cycle plus a duration chosen by the
parent's baud setting and the endpoint count. Evenfall uses pinned mGBA 0.10.5's
[`GBASIOCyclesPerTransfer`](https://github.com/mgba-emu/mgba/blob/0.10.5/src/gba/sio.c#L14-L19)
table:

| `SIOCNT.1:0` | Baud    | 2 consoles | 3 consoles | 4 consoles |
| ------------ | ------- | ---------- | ---------- | ---------- |
| 0            | 9,600   | 73,003     | 107,680    | 142,356    |
| 1            | 38,400  | 18,251     | 26,920     | 35,589     |
| 2            | 57,600  | 12,167     | 17,947     | 23,726     |
| 3            | 115,200 | 6,075      | 8,973      | 11,863     |

No available hardware test measures these durations.

## UART

UART mode behaves much like an RS-232 port
([GBATEK UART mode](https://rust-console.github.io/gbatek-gbaonly/#siouartmode)).
Pinned mGBA 0.10.5 recognizes the mode number but implements no UART
shifting, FIFO, status, error or IRQ behavior: it stores the raw `SIOCNT`
write and passes data writes through. It is therefore not a comparison for
anything below except the register readback.

### Registers

`SIOCNT` in UART mode:

| Bit   | Meaning                                                        | Access                                 |
| ----- | -------------------------------------------------------------- | -------------------------------------- |
| 1:0   | Baud: 9,600, 38,400, 57,600, 115,200 bit/s                     | R/W                                    |
| 2     | CTS: 0 sends blindly, 1 sends only while the receiver is ready | R/W                                    |
| 3     | Parity kind: 0 even, 1 odd                                     | R/W                                    |
| 4     | TX full                                                        | read-only                              |
| 5     | RX empty                                                       | read-only                              |
| 6     | Error                                                          | read-only, cleared by reading `SIOCNT` |
| 7     | Data length: 0 = 7 bits, 1 = 8 bits                            | R/W                                    |
| 8     | FIFO enable                                                    | R/W                                    |
| 9     | Parity enable                                                  | R/W                                    |
| 10    | TX enable                                                      | R/W                                    |
| 11    | RX enable                                                      | R/W                                    |
| 13:12 | Mode, both 1                                                   | R/W                                    |
| 14    | IRQ enable                                                     | R/W                                    |
| 15    | Unused, reads 0                                                | read-only                              |

The idle status is `0020h` (RX empty), so writing every control bit reads
back `7FAFh`, matching the suite. GBATEK's CTS prose says the flag is "set to
always/blindly", contradicting its own bit table; Evenfall follows the table.

`SIODATA8` (`12Ah`) has separate transmit and receive paths: a read pops the
oldest received byte (0 when empty), a write queues a byte for transmission.
Only the low byte carries data; the high byte reads 0 and a byte write to
`12Bh` is ignored. With transmit and receive disabled and no peer, the suite
writes `FFFFh` and reads `0000h`, which this model reproduces.

### Transmit path

A written byte is queued even while TX is disabled. Transmit storage holds
one waiting byte with the FIFO disabled and four with it enabled, in addition
to the byte being shifted; a write beyond that is discarded. The oldest byte
starts shifting when `SIOCNT.10` is set and either CTS is disabled or the
peer has granted CTS. When it finishes, the next queued byte starts.

GBATEK defines CTS as the SC input being low, which the crossed cable ties to
the peer's receiver state. Evenfall models the grant directly: at each
`synchronize_sio`, a sender's CTS is granted while the peer has RX enabled,
has room in its receive storage and has no byte already arriving. Each
completed byte consumes the grant. A CTS-gated sender with no peer therefore
never starts, while a blind sender shifts on schedule and its byte is lost.

`SIOCNT.4` (TX full) reads 1 while any byte is shifting or waiting if the
FIFO is disabled, and while four bytes wait if it is enabled.

### Receive path

When a transmission is coupled, the receiver is scheduled to receive the byte
at the sender's completion cycle, provided its RX is enabled at that point.
On arrival, if RX is still enabled the byte enters receive storage (one byte,
or four with the FIFO); if storage is full the byte is dropped and the error
flag is set. `SIOCNT.5` (RX empty) reads 1 while storage is empty.

Disabling the FIFO while in UART mode discards both FIFOs, as GBATEK
documents. Because entering UART mode also clears UART state, software must
write `SIOCNT` before queuing data; the `probe-uart` sender does so.

### Errors and interrupts

GBATEK lists three error causes: a bad stop bit, a parity mismatch and
receive overrun. The peer seam carries whole bytes, so Evenfall produces only
overrun. A `SIOCNT` read returns the error bit and then clears it, so the
clearing read observes the 1.

With `SIOCNT.14` set, Evenfall requests `IF` bit 7 whenever status bit 4, 5 or 6
changes from 0 to 1. This is a strict edge reading of GBATEK's "become set":
enabling the IRQ while a bit is already 1 requests nothing. Because RX empty
is 1 at idle, reading the last received byte raises an interrupt, as does a
write that fills transmit storage.

### Timing

The only sourced timing values are the four nominal baud rates. Evenfall derives
a frame duration from them; Rust keeps the calculation in
[`bus/mod.rs`](../../../systems/gba/src/bus/mod.rs):

```text
cycles = ceil(frame_bits * 16777216 / baud)
frame_bits = 1 start + 7 or 8 data + optional parity + 1 stop
```

| Frame         | 9,600  | 38,400 | 57,600 | 115,200 |
| ------------- | ------ | ------ | ------ | ------- |
| 8N1 (10 bits) | 17,477 | 4,370  | 2,913  | 1,457   |
| 7N1 (9 bits)  |        |        |        | 1,311   |
| 8E1 (11 bits) |        |        |        | 1,602   |

This is a nominal model, not a measured divider: no available source gives
the GBA's baud divider, fractional rounding, bit-edge phase or character
latency, and the suite has no UART timing case. The formula is confined to
the UART scheduling calculation so a hardware measurement can replace it without changing
the register, FIFO, peer-delivery or scheduler behavior. The test "UART
nominal frame deadlines follow baud, data-length, and parity selection" pins
the values above.

## JoyBus

In JoyBus mode (`RCNT.15:14 = 11b`) the GBA is always a slave: an external
master, such as a GameCube, sends commands and the GBA answers. It cannot
originate a command, so Evenfall models JoyBus as an external-master command
seam rather than another cable
([GBATEK JOY BUS mode](https://rust-console.github.io/gbatek-gbaonly/#siojoybusmode)).
`SIOCNT` plays no part in the protocol.

### Command seam

```zig
pub fn scheduleJoyBus(
    self: *Bus,
    command: JoyBusCommand,
    completion_cycle: Cycle,
) JoyBusError!void
pub fn takeJoyBusResponse(self: *Bus) ?JoyBusResponse
```

`JoyBusCommand` is `.reset`, `.status`, `.read_from_gba` or
`.write_to_gba: u32`; `JoyBusResponse` holds up to five bytes. One command is
in flight at a time: scheduling fails with `NotInJoyBusMode`,
`CompletionBeforeCurrentCycle`, or `TransactionPending` while a command or an
untaken response is outstanding. The command is applied atomically at its
completion cycle as an ordinary device event, immediately if that cycle is
the current one. The caller supplies the deadline because no available
source gives the physical bit rate, framing or synchronization; the core
supplies no default duration. Leaving JoyBus mode and the serial group of
`RegisterRamReset` (see [Reset](#reset)) cancel the pending command and
response.

The seam is typed, so unknown or malformed commands never reach the core.
mGBA answers an unknown complete command with no bytes; a transport that
receives one owns that choice.

### Commands

Command names are from the master's side. Payloads travel least significant
byte first; the `00h 04h` type bytes form `0400h` read little-endian.

| Command      | Master sends     | GBA replies                   | `JOYCNT` flag | `JOYSTAT`    | Serial IRQ if `JOYCNT.6` |
| ------------ | ---------------- | ----------------------------- | ------------- | ------------ | ------------------------ |
| `FFh` RESET  | command          | `00h 04h` status              | bit 0         | unchanged    | yes                      |
| `00h` STATUS | command          | `00h 04h` status              | none          | unchanged    | no                       |
| `14h` READ   | command          | `JOY_TRANS` (4 bytes), status | bit 2         | clears bit 3 | yes                      |
| `15h` WRITE  | command, 4 bytes | status                        | bit 1         | sets bit 1   | yes                      |

The status byte is the low byte of `JOYSTAT`. The ordering within a command
follows mGBA's synchronous handler: RESET sets its flag before building the
reply; WRITE stores `JOY_RECV` and sets its flags before sampling the status
byte; READ samples the payload and status before setting `JOYCNT.2` and
clearing `JOYSTAT.3`. RESET notifies the GBA; it does not reset the machine
or clear any register.

### Registers

| Register                     | Evenfall behavior                                                                                                                            |
| ---------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| `JOYCNT` (`140h`)            | Bits 0–2 are the RESET, receive-complete and send-complete flags, cleared by writing 1. Bit 6 enables the serial IRQ. All other bits read 0. |
| `JOY_RECV` (`150h`, `152h`)  | Filled by WRITE. Reading either half clears `JOYSTAT.1`. CPU writes are ignored.                                                             |
| `JOY_TRANS` (`154h`, `156h`) | Read by READ. Writing either half sets `JOYSTAT.3`. CPU reads return 0.                                                                      |
| `JOYSTAT` (`158h`)           | Bit 1 receive status, bit 3 send status, bits 4–5 general-purpose and writable. CPU writes cannot change bits 1 and 3.                       |

A completed RESET, READ or WRITE with `JOYCNT.6` set requests `IF` bit 7,
which wakes HALT at the exact caller-provided cycle when enabled in `IE`.
Setting a `JOYCNT` flag without an IRQ request does not.

### Where the sources disagree

GBATEK confidently ties the `JOYCNT.6` interrupt to RESET but questions
whether READ and WRITE interrupt, and is unsure which of commands `14h` and
`15h` sets bits 1 and 2. mGBA raises the interrupt for RESET, READ and WRITE
and sets bit 1 for WRITE and bit 2 for READ. Evenfall follows mGBA.

For `JOYSTAT`, GBATEK says a CPU write to `JOY_TRANS` sets bit 1 and a CPU
read of `JOY_RECV` clears bit 3. mGBA uses the opposite pairing, consistently
treating bit 1 as "received data unread" and bit 3 as "transmit data
pending". Evenfall adopts mGBA's coherent mapping. The suite cannot settle the
question: every row it labels `JOYSTAT` reads the reserved address
`0400015Ah` instead of `04000158h`, so its zero results say nothing about the
real register.

The suite's JoyBus row writes each payload halfword and reads 0 back, which
Evenfall matches. mGBA 0.10.5 instead stores and returns the written payload, so
it is a command-semantics reference here, not evidence for disconnected
payload access.

## Deterministic linking

The core has no host transport. Linked consoles are separate `Machine`
instances whose cycle counters share one origin, coupled by two functions:
`synchronize_sio` works on their buses, and `run_linked_until` schedules the
machines around it.

### `synchronize_sio`

```rust
pub fn synchronize_sio(
    peers: &mut [&mut Bus],
    connection: SioConnection,
) -> Result<bool, SioLinkError>
```

`synchronize_sio` takes two to four buses in cable order and couples whatever
transfer is ready: a Normal exchange once every endpoint has START set, a
Multiplayer transfer once the parent has started, or pending UART bytes. It
returns `false` while nothing can progress, including when the endpoints are
in different modes or any has `RCNT.15` set. Errors:

| Error                  | Cause                                                                            |
| ---------------------- | -------------------------------------------------------------------------------- |
| `UnsupportedPeerCount` | fewer than 2 or more than 4 buses, or `.cable_2p` with other than 2              |
| `UnsupportedMode`      | Multiplayer on `.cable_2p`, or UART on `.cable_multi`                            |
| `ClockConflict`        | two internal clocks on `.cable_2p`, or an internal-clock child on `.cable_multi` |
| `MissedDeadline`       | an endpoint's clock is already past the transfer's completion cycle              |

The completion cycle always comes from the clock master's START write (or the
UART sender's start), so coupling late within the transfer window does not
move it. An endpoint that has already executed beyond that cycle cannot
receive data in its past; Evenfall reports `MissedDeadline` rather than silently
reordering events.

### `run_linked_until`

```rust
pub fn run_linked_until(
    machines: &mut [&mut Machine],
    deadline: Cycle,
    connection: SioConnection,
) -> Result<LinkedRunResult, LinkedRunError>
```

`run_linked_until` advances two to four distinct machines to `deadline` on one
timeline. Each iteration:

1. reports a latched fault on any machine as `.faulted` with its index;
2. calls `synchronize_sio`, treating `UnsupportedMode` as "nothing to couple";
3. returns `.deadline` once every machine has reached `deadline`;
4. executes one instruction on the least-advanced runnable machine: one that
   is running, halted or stopped with a pending wake, or inside a resumable
   HLE BIOS service;
5. otherwise advances the least-advanced halted machine's devices toward
   `deadline`, stopping early if a wake source asserts;
6. otherwise advances stopped machines through their armed serial
   completions only;
7. returns `.power_stopped` if nothing could progress.

Because instructions are atomic, machines can finish slightly beyond
`deadline`; `LinkedRunResult` reports the earliest and latest machine cycles
so the skew is explicit. Passing the same machine twice is `DuplicateMachine`,
and any other `SioLinkError` propagates.

### STOP

STOP freezes timers, LCD, audio, DMA and Game Pak prefetch, and wakes only on
an enabled serial, keypad or Game Pak interrupt (see
[`timers-dma-interrupts.md`](timers-dma-interrupts.md)). The serial
peripherals run on clocks independent of those, so `Bus::idle_stop_until`
advances a stopped machine through its armed Normal, Multiplayer, UART and
JoyBus deadlines and nothing else. A completion that raises an enabled serial
IRQ wakes the CPU; this is the MultiBoot-slave case, where the master clocks
data in while the slave sleeps. Timed host inputs that fall inside the frozen
interval stop the advance so the caller can retain them. A stopped machine
with no armed serial event and no wake source is reported without advancing
its clock. The single-machine `Machine::run_until` uses the same STOP path.

The `probe-stop-sio` pair covers a child that enters STOP with IME off and is
woken by the parent's Multiplayer transfer.

## Reset

| Event                         | Serial effect                                                                                                                                                                                                                                                                                                  |
| ----------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Power-on                      | `RCNT = 8000h`; every other serial register and queue is zero or empty                                                                                                                                                                                                                                         |
| `RegisterRamReset`, any flags | `04000120h` (`SIODATA32_L`/`SIOMULTI0`) is cleared, reproducing the BIOS bug [GBATEK documents](https://rust-console.github.io/gbatek-gbaonly/#biosresetfunctions)                                                                                                                                             |
| `RegisterRamReset` bit 5      | `SIOCNT = 0`, `RCNT = 8000h`, `SIODATA8 = 0`; pending transfer cancelled; Multiplayer receive vector `FFFFh` and ID 0; UART queues, shifting byte, arrival, error and CTS cleared; `JOYCNT`, `JOY_RECV`, `JOY_TRANS` and `JOYSTAT` zeroed; JoyBus command and response cancelled. `122h`–`126h` are untouched. |
| HLE `HardReset`               | `RegisterRamReset(FFh)` plus all four `120h`–`126h` data halfwords cleared                                                                                                                                                                                                                                     |
| JoyBus RESET command          | only `JOYCNT.0` and the optional IRQ                                                                                                                                                                                                                                                                           |

mGBA's `RegisterRamReset` clears `JOYCNT` and both payload registers but does
not write `JOYSTAT`; Evenfall also clears `JOYSTAT`. The test "HLE
RegisterRamReset exposes the SIO reset group and unconditional data-low bug"
in [`tests/machine_a.rs`](../../../systems/gba/tests/machine_a.rs) pins this table.
The HLE reset services themselves are described in [`bios.md`](bios.md).

## Not implemented

- **Host link transports.** No network or browser adapter exists; the CLI
  and the web player run one machine. Linking is available only
  through `run_linked_until` and `synchronize_sio` in Rust.
- **Line levels.** In Normal mode `SIOCNT.2` always reads 1 and `SIOCNT.3`
  is not propagated to the peer, so the SI/SO ready handshake that GBATEK
  recommends is not modeled; `RCNT` bits 0–3 return fixed values even on a
  cable. General-purpose mode has no external pin source and never raises
  its SI interrupt, including the one GBATEK says can wake STOP.
- **Multiplayer cable faults.** No error bit, bad-connection SD level or hot
  unplug.
- **UART line errors.** No framing or parity errors, and no bit-level baud
  phase.
- **JoyBus transport.** No concrete master, wire timing or default duration.
- **Wireless and infrared adapters.**

## Tests and probes

Register, timing and cable behavior is pinned by the serial tests in
[`tests/bus.rs`](../../../systems/gba/tests/bus.rs) (register matrix, Normal
deadlines, both cable topologies, Multiplayer timing and absent slots, UART
FIFO, overrun, CTS and deadlines, JoyBus commands and scheduling, HALT wake)
and by the linked-scheduler and reset tests in
[`tests/machine_a.rs`](../../../systems/gba/tests/machine_a.rs) and [`tests/machine_b.rs`](../../../systems/gba/tests/machine_b.rs). The bare-metal probes
listed under "Serial links" in [`probe/README.md`](../../../probe/README.md)
(`probe-link`, `probe-multi4`, `probe-stop-sio`, `probe-uart`,
`probe-joybus`, `probe-joybustx`, `probe-mb2slave`, `probe-mb2mhz`) run
complete ROM exchanges through `run_linked_until`.

## Open questions

- Multiplayer durations come from mGBA's table; no hardware measurement
  confirms them.
- The 121-cycle overhead used to match `sio-timing.c` is an inference from
  four cases.
- Normal mode over the multi-player cable gives the parent zero per GBATEK;
  no hardware capture here confirms it against mGBA's `FFh`.
- GBATEK says many games test `RCNT.0` (SC) in Multiplayer mode; Evenfall
  returns a fixed 1.
- General-purpose mode reads back written `RCNT` data bits for input pins;
  GBATEK describes internal pull-ups.
- UART baud divider, rounding and phase are unknown; the nominal UART frame timing is
  nominal.
- UART IRQ: whether status bits interrupt on an edge or a level, and
  whether enabling the IRQ with a status bit already set interrupts.
- UART storage: whether one byte can wait behind the shifting byte with the
  FIFO disabled, and what writes while full and reads while empty return.
- UART: whether the read that clears the error bit observes it, and
  GBATEK's unusual "stop bit must be 0" polarity.
- JoyBus: whether READ and WRITE request the serial IRQ, and which
  `JOYSTAT` bits the CPU-side accesses touch.
- JoyBus: bit rate, framing, the phase at which payload, flags and `IF`
  become visible, and whether JoyBus runs in or wakes STOP; Evenfall lets an
  armed JoyBus completion wake STOP.
- JoyBus payload registers: 32-bit access, and `JOY_TRANS` readback after a
  command.
- Whether `RegisterRamReset` clears `JOYSTAT` on hardware.
