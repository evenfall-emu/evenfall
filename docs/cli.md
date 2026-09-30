# Command-line runner

`evenfall-gba` runs a GBA ROM without a window, for a fixed number of instructions, and prints what happened.
It is meant for testing and debugging, not for playing: scripts use it to check that a ROM reaches a known frame, to trace the instructions around a bug, or to produce a save state for a bug report.
Its `gdb` mode lets a debugger such as gdb, Ghidra or IDA stop, step and inspect the guest instead ([GDB remote debugging](#gdb-remote-debugging)).

```sh
cargo run --release -p evenfall-gba --bin evenfall-gba -- run --steps 100 path/to/homebrew.gba
```

Everything after `--` goes to the runner.
Evenfall ships no ROM, BIOS or save file, so bring your own.
The runner lives in [`systems/gba/src/bin/evenfall-gba.rs`](../systems/gba/src/bin/evenfall-gba.rs).

## Running a ROM

```text
evenfall-gba run --steps <count> [--bios <path>] [--save-type <type>]
          [--save-file <path>] [--state-in <path>] [--state-out <path>]
          [--trace <path>] [--trace-from <steps>] [--input <movie>]
          [--cheat <[format:]code>]... [--cheats <file>] [--quiet]
          [--frame-hash] [--no-rtc | --rtc-epoch <seconds>]
          [--no-solar | --solar-level <0..255>]
          [--no-gyro | --gyro-sample <0..4095>] [--rumble | --no-rumble]
          [--no-tilt | --tilt <x> <y>] <rom.gba|rom.zip>
```

`run --steps <count>` must come first.
The other flags and the ROM path can follow in any order.

`--steps` counts instructions, not cycles or frames.
A long BIOS routine may show its `SWI` on several steps ([`bios.md`](gba/hardware/bios.md#routines-that-run-over-several-steps)).
While the CPU sleeps, the runner advances time until something wakes it, up to a frame at a time, including a scheduled button press.
If nothing can wake it, the run ends early.

| Option                 | Effect                                                                    |
| ---------------------- | ------------------------------------------------------------------------- |
| `--bios <path>`        | boot a real 16 KiB BIOS image instead of Evenfall's replacement           |
| `--save-type <type>`   | choose the save chip instead of detecting it ([Saves](#saves-and-states)) |
| `--save-file <path>`   | use this battery save instead of the ROM's path with a `.sav` extension   |
| `--state-in <path>`    | load a save state after boot, before running                              |
| `--state-out <path>`   | write a save state after the run, unless the machine faulted              |
| `--trace <path>`       | write a binary record of each instruction ([Trace](#trace))               |
| `--trace-from <steps>` | start tracing after this many instructions                                |
| `--input <movie>`      | press buttons on given frames ([Input movies](#input-movies))             |
| `--cheat <code>`       | turn on a cheat code, repeatable ([Cheats](#cheats))                      |
| `--cheats <file>`      | turn on the cheats listed in a file ([Cheats](#cheats))                   |
| `--quiet`              | don't print a line per instruction or per wait                            |
| `--frame-hash`         | print a hash of the last completed frame                                  |

The ROM may be a `.gba` file of at most 32 MiB, or a `.zip` (any case) holding exactly one `.gba` file.
The ZIP is unpacked in memory, and must be unencrypted, stored or deflated, without Zip64 or split disks.
The archive and the ROM inside are each limited to 32 MiB, and the ROM's CRC-32 is checked.
Other files in the archive, including macOS metadata, are ignored.
The ZIP is unpacked on every run, so an extracted ROM starts slightly faster.

## Output

Each instruction prints its start and end cycle, its address and its opcode:

```text
<start>..<end> pc=<address> arm=<word>
<start>..<end> pc=<address> thumb=<halfword>
```

Other lines:

- `wait cycle=<cycle> power=<halted|stopped|dma_stall>` while the CPU sleeps or a DMA burst holds the bus, unless `--quiet`.
- `emulation fault cycle=... pc=... reason=...` when the machine stops on a fault ([`cpu.md`](gba/hardware/cpu.md#undefined-versus-unpredictable)).
  The run then ends.
- `stop cycle=<cycle> next=<address> cpsr=<CPSR> r0=<R0>` at the end of every run that got going, including one that faulted.
- `frame count=<frames> wyhash=<16 hex digits>` with `--frame-hash`.
  The hash is a seed-zero Wyhash (the variant in Zig's standard library) over the last completed 240x160 frame, as 15-bit BGR pixels in little-endian order.

### Exit status

Scripts need to look at the output as well as the exit status:

- Invalid arguments print the usage text and exit with status 0.
- A ROM, BIOS, save or state that fails to load prints a message on stdout and exits with status 0, without a `stop` line.
- An invalid input movie, a file that can't be written and other errors after setup print to stderr and exit non-zero.
- A guest fault is reported, but doesn't make the exit status non-zero by itself.

So check the exit status, check that a `stop` line was printed, and look for a fault line if faults matter to you.

### Trace

`--trace` writes a 16-byte little-endian record per traced instruction: the address (`u32`), the opcode (`u32`, Thumb opcodes in the low half), the CPSR after the instruction (`u32`), the instruction set (0 for ARM, 1 for Thumb), and three zero bytes.

## Input movies

`--input` presses buttons during given frames, as comma-separated `key:first-last` ranges (both ends included):

```sh
cargo run --release -p evenfall-gba --bin evenfall-gba -- run --steps 5000000 --quiet \
  --frame-hash --input start:250-255,a:400-405 game.gba
```

The keys are `a`, `b`, `select`, `start`, `right`, `left`, `up`, `down`, `r` and `l`.
Frame n starts at cycle `n * 280,896`, and each press and release lands on that exact cycle, even while the CPU sleeps.
Overlapping ranges hold a key until the last one ends.
A movie can have up to 32 ranges.
After `--state-in`, ranges that have already started set the initial keys.

The same ROM, flags and movie always give the same output, which makes the runner useful for regression tests.

## Cheats

`--cheat` and `--cheats` turn on cheat codes from boot.
File cheats come first, then each `--cheat` in order.
They are the only way to turn cheats on: save states never carry cheats, so `--state-in` and `--state-out` neither restore nor record them.
Cheats run at the first instruction boundary at or after each VBlank start, so a run with cheats is as repeatable as one without.

A `--cheat` value is `[FORMAT:]CODE`.
FORMAT is `auto` (the default), `raw`, `gameshark`, `gameshark-decrypted`, `action-replay`, `action-replay-decrypted` or `codebreaker`.
Code lines are separated by `+`, `,`, `;` or spaces:

```sh
cargo run --release -p evenfall-gba --bin evenfall-gba -- run --steps 5000000 --quiet \
  --cheat 'action-replay:D8BAE4D9 4864DCE5+A86CDBA5 19BA49B3' \
  --cheat 'C051CCF6 975E8DA1' --cheat raw:02025E90:0F423F00 game.gba
```

A `--cheats` file holds one cheat per paragraph, with blank lines between them.
A paragraph may start with a `!FORMAT` line, and `#` starts a comment:

```text
# Pokémon Emerald
!action-replay
D8BAE4D9 4864DCE5
A86CDBA5 19BA49B3

82025BC4 E0FF   # CodeBreaker, detected
```

An invalid or unsupported code prints its cheat and line on stderr, and the runner exits nonzero before running.
[`design/cheats.md`](design/cheats.md) describes the formats, detection and limits.

## Saves and states

The battery save defaults to the ROM's path with a `.sav` extension.
The runner writes it when the run ends, even after a guest fault, through a temporary file that it syncs and then renames, so an interrupted run never leaves half a save.
It writes nothing when the game never used its save, or with `--save-type none`.

| `--save-type`              | Save chip                                                       |      Size |
| -------------------------- | --------------------------------------------------------------- | --------: |
| `auto` (default)           | detected from the title, the save file, ROM markers or the game |         — |
| `none`                     | no battery save                                                 |         — |
| `sram`                     | SRAM                                                            |    32 KiB |
| `flash64`, `flash64-atmel` | 64 KiB Flash                                                    |    64 KiB |
| `flash128`                 | 128 KiB Flash                                                   |   128 KiB |
| `eeprom512`                | EEPROM                                                          | 512 bytes |
| `eeprom8k`                 | EEPROM                                                          |     8 KiB |

An existing save must fit the detected type.
A longer file is accepted and its extra bytes, such as another emulator's metadata, are ignored, but a shorter one is refused.
An explicit type settles conflicting ROM markers.
[`cartridge.md`](gba/hardware/cartridge.md#save-type-detection) explains how detection works.

A save state holds the whole machine, the save chip included, and only loads with the same ROM, BIOS, boot mode and format version.
A failed load leaves the machine as it was.
A faulted machine skips `--state-out` and prints `state save skipped: machine faulted`.
The format is described in [`design/save-states.md`](design/save-states.md).

## Cartridge hardware

The real-time clock, light sensor, gyro, rumble and tilt sensor are switched on from the game code, and the clock also when the ROM contains the `RTC_V` library string ([`cartridge.md`](gba/hardware/cartridge.md#detecting-extra-hardware)).
These flags override the detection.
The on and off forms of each device can't be combined.

| Flags                                  | Effect                                                   |
| -------------------------------------- | -------------------------------------------------------- |
| `--no-rtc`, `--rtc-epoch <seconds>`    | remove the clock, or set its starting time (Unix, UTC)   |
| `--no-solar`, `--solar-level <0..255>` | remove the light sensor, or set its level                |
| `--no-gyro`, `--gyro-sample <0..4095>` | remove the gyro, or set its raw 12-bit value             |
| `--rumble`, `--no-rumble`              | force the rumble motor on or off                         |
| `--no-tilt`, `--tilt <x> <y>`          | remove the tilt sensor, or set its two raw 12-bit values |

`--gyro-sample` and `--tilt` take decimal or `0x` hexadecimal.

A detected clock starts from the host's current time and then advances with emulated time, so two runs a minute apart differ.
Set `--rtc-epoch` for reproducible runs.
The battery save doesn't include the clock, and a save state does.

```sh
cargo run --release -p evenfall-gba --bin evenfall-gba -- run --steps 100 --no-rtc game.gba
cargo run --release -p evenfall-gba --bin evenfall-gba -- run --steps 100 \
  --save-type eeprom8k --save-file game.sav game.gba
```

## GDB remote debugging

`evenfall-gba gdb` boots a ROM as `run` does and then waits for one debugger to connect over TCP.
It speaks the GDB remote serial protocol, as mGBA does, so `gdb-multiarch`, an ARM-capable `gdb`, Ghidra's GDB connector and IDA's remote GDB debugger can all drive it.

```text
evenfall-gba gdb [--listen <address:port>] [run options other than --steps]
          <rom.gba|rom.zip>
```

```sh
cargo run --release -p evenfall-gba --bin evenfall-gba -- gdb game.gba
gdb-multiarch -ex 'target remote 127.0.0.1:2345' game.elf
```

`--listen` defaults to `127.0.0.1:2345`, mGBA's port.
Port 0 picks a free port.
The runner prints `gdb listening=<address:port>` once it listens and `gdb connected=<peer>` when the debugger arrives.
The session starts stopped at the first instruction, after `--state-in` if given.
`--input` and `--trace` work as in `run`.

The session ends when the debugger detaches, kills the target or disconnects.
The runner then writes the battery save and `--state-out`, and prints the `--frame-hash` and `stop` lines, as at the end of `run`.
It never prints instruction or wait lines, so `--quiet` changes nothing.

### How it stays exact

The debugger never changes what the guest computes.
The runner executes whole instructions through the same step and sleep operations as `run`, and checks for breakpoints and for Ctrl-C only between instructions.
Reads inspect memory without the side effects a CPU read would have.
A session that only stops, steps and reads therefore ends in the same state, with the same trace and `stop` line, as `run --steps` with the number of instructions it executed.
Register and memory writes are the only way a debugger changes the guest, and they take no emulated time.

### What the debugger sees

The target is one thread of an ARMv4T CPU.

- **Registers.**
  r0 to r12, `sp`, `lr` and `cpsr` are those of the current CPU mode.
  `pc` is the address of the next instruction.
  The CPSR's T bit tells the debugger whether to disassemble and step ARM or Thumb code.
  `cpsr` is register 25 in gdb's ARM numbering and follows `pc` in the full register packet.
- **Memory map.**
  The runner describes the BIOS and cartridge ROM as read-only, and work RAM, I/O, palette, VRAM, OAM and cartridge save memory as writable.
  Mirrors are left out of the map, so gdb refuses addresses in them.
- **Breakpoints.**
  Software and hardware breakpoints behave alike.
  Both are addresses checked before each instruction, and neither patches guest memory.
  gdb picks hardware breakpoints in the read-only regions by itself.
- **Stepping and continuing.**
  `stepi` runs one instruction.
  `continue` runs until a breakpoint's address is the next instruction.
  A sleeping CPU keeps advancing emulated time, as in `run`, until something wakes it.
- **Ctrl-C.**
  The runner looks for the interrupt every 16,384 instructions and after every frame of sleep, then stops with SIGINT.
- **Faults.**
  An emulation fault stops the session with SIGABRT and prints its fault line.
  The machine can't run on, and `--state-out` is skipped.
- **Register writes.**
  Any register can be written, and a CPSR write must name a valid mode.
  Writing `pc` or the T bit refills the instruction pipeline, as at power-on.
- **Memory writes.**
  Only EWRAM and IWRAM can be written.
  Decoded code and prefetched instructions over the written bytes are refreshed, so patched code runs next.

## Known limits

- Invalid arguments and load failures exit with status 0.
  Scripts have to check the output.
- The runner has no window, sound or real-time pacing.
  The [SDK](sdk.md) is the way to play.
- The GDB stub serves no watchpoints.
  gdb's `watch` still works by single-stepping, slowly, and `rwatch` and `awatch` are unavailable.
- The GDB stub can't write to BIOS, cartridge ROM, I/O, palette, VRAM, OAM or save memory.
- The GDB stub refuses register writes while a BIOS routine that spans several steps is running.
  `stepi` inside such a routine stops between its steps, with `pc` after its `SWI`.
