# Command-line runner

`evenfall-gba` is Evenfall's headless GBA binary. It executes a user-supplied ROM for
a fixed number of instructions and prints cycles, instructions, faults and an
optional framebuffer hash. It uses the HLE BIOS unless given an external BIOS.
The runner lives in [`systems/gba/src/bin/evenfall-gba.rs`](../systems/gba/src/bin/evenfall-gba.rs).
Its flags and output are ZiGBA's `zigba` CLI's; only the program name
differs.

```sh
cargo run -p evenfall-gba --bin evenfall-gba -- run --steps 100 path/to/homebrew.gba
```

Everything after `--` belongs to the runner. No ROM, BIOS or save image is
distributed with Evenfall.

## Invocation

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

`run --steps <count>` comes first. Other flags and the ROM path may follow in
any order. `--steps` counts executed instructions, not cycles. A split HLE BIOS
service may report its SWI on several steps. If the CPU sleeps, the runner
waits for a wake event up to a frame at a time, including scheduled input;
without a wake event it ends early.

| Option                 | Effect                                                                    |
| ---------------------- | ------------------------------------------------------------------------- |
| `--bios <path>`        | Boot a user-supplied BIOS of exactly 16 KiB instead of HLE.               |
| `--save-type <type>`   | Override cartridge save detection (see [Saves](#saves)).                  |
| `--save-file <path>`   | Use this battery save path instead of the ROM path with `.sav` extension. |
| `--state-in <path>`    | Restore a portable state after boot, before the instruction run.          |
| `--state-out <path>`   | Write a portable state after the run unless the machine faulted.          |
| `--trace <path>`       | Write binary instruction records (see [Trace](#trace)).                   |
| `--trace-from <steps>` | Begin tracing at this executed-instruction count.                         |
| `--input <movie>`      | Schedule button changes by frame (see [Input movies](#input-movies)).     |
| `--cheat <code>`       | Activate a cheat code; repeatable (see [Cheats](#cheats)).                |
| `--cheats <file>`      | Activate the cheats listed in a file (see [Cheats](#cheats)).             |
| `--quiet`              | Suppress per-instruction and wait lines.                                  |
| `--frame-hash`         | Print the final completed framebuffer hash.                               |

A raw `.gba` image must be at most 32 MiB. Paths with a case-insensitive
`.zip` extension are unpacked in memory; the ZIP must contain exactly one
unencrypted `.gba` entry, stored or deflated, without Zip64 or split disks.
Both the archive and extracted ROM are limited to 32 MiB; the entry's CRC-32
is checked. Other entries and macOS sidecars are ignored. The ZIP is decoded
on every invocation; an extracted ROM avoids that startup work.

### Output and exit behavior

Ordinary instruction lines contain start and end cycles, address and opcode:

```text
<start>..<end> pc=<address> arm=<word>
<start>..<end> pc=<address> thumb=<halfword>
```

A sleeping CPU prints `wait cycle=<cycle> power=<halted|stopped>` unless quiet;
a terminal compatibility fault prints `emulation fault cycle=... pc=...
reason=...` and stops. A completed execution path prints
`stop cycle=<cycle> next=<address> cpsr=<CPSR> r0=<R0>`, even if it faulted.
`--frame-hash` adds `frame count=<frames> wyhash=<16 hex digits>`: seed-zero
Zig-compatible Wyhash over the last completed 240×160 BGR555 image in
little-endian pixel order.

Invalid command-line arguments print usage on stdout and exit with status 0.
ROM, BIOS, save setup and state-load failures also print a diagnostic on
stdout and exit with status 0, without a final `stop` line. Invalid input
movies, write/trace errors and other errors returned after setup print to
stderr and exit nonzero. A guest fault is reported but does not by itself
make the process exit nonzero. Scripts should check both exit status and the
final `stop` line, and parse a fault line if faults matter.

### Trace

`--trace` writes a 16-byte little-endian record for each traced instruction:
address (`u32`), opcode (`u32`, Thumb in the low half), CPSR after execution
(`u32`), instruction set (0 ARM, 1 Thumb), then three zero bytes.

## Input movies

`--input` takes comma-separated `key:first-last` ranges of inclusive frames:

```sh
cargo run -p evenfall-gba --bin evenfall-gba -- run --steps 5000000 --quiet \
  --frame-hash --input start:250-255,a:400-405 game.gba
```

Keys are `a`, `b`, `select`, `start`, `right`, `left`, `up`, `down`, `r` and
`l`. Frame `n` starts at master cycle `n × 280,896`. Overlapping ranges
hold a key until its last range ends. Up to 32 ranges produce at most 64
events. After `--state-in`, already-started ranges set the initial keys;
future changes are delivered at their exact cycles, including during sleep.

## Cheats

`--cheat` and `--cheats` are Evenfall additions to ZiGBA's flags. The cheats
are active from boot, file cheats first, then `--cheat` codes in order. With
`--state-in`, they replace the cheats the state carries; without cheat flags,
the state's cheats stay active. They run at the first instruction boundary at
or after each VBlank start.

A `--cheat` value is `[FORMAT:]CODE`. FORMAT is `auto` (the default), `raw`,
`gameshark`, `gameshark-decrypted`, `action-replay`,
`action-replay-decrypted` or `codebreaker`; code lines are separated by `+`,
`,`, `;` or spaces:

```sh
cargo run -p evenfall-gba --bin evenfall-gba -- run --steps 5000000 --quiet \
  --cheat 'action-replay:D8BAE4D9 4864DCE5+A86CDBA5 19BA49B3' \
  --cheat 'C051CCF6 975E8DA1' --cheat raw:02025E90:0F423F00 game.gba
```

A `--cheats` file holds one cheat per paragraph, separated by blank lines. A
paragraph may start with a `!FORMAT` line; `#` starts a comment:

```text
# Pokémon Emerald
!action-replay
D8BAE4D9 4864DCE5
A86CDBA5 19BA49B3

82025BC4 E0FF   # CodeBreaker, detected
```

An invalid or unsupported code prints its cheat and line on stderr and exits
nonzero before the run. `--state-out` then writes a state that carries the
cheats; formats, detection and limits are in
[`design/cheats.md`](design/cheats.md).

## Saves and states

Battery saves default to the ROM path with extension `.sav`. An existing save
must match the resolved type; known types can ignore trailing emulator metadata
but reject short files. The runner writes cartridge save bytes through a
synced temporary file and atomic replacement, including after a guest fault;
unresolved or `none` saves produce no file.

| `--save-type`              | Medium                                                         |      Size |
| -------------------------- | -------------------------------------------------------------- | --------: |
| `auto` (default)           | Detect from title, existing file, exact ROM markers or traffic |         — |
| `none`                     | No battery save                                                |         — |
| `sram`                     | SRAM                                                           |    32 KiB |
| `flash64`, `flash64-atmel` | 64 KiB Flash                                                   |    64 KiB |
| `flash128`                 | 128 KiB Flash                                                  |   128 KiB |
| `eeprom512`                | EEPROM                                                         | 512 bytes |
| `eeprom8k`                 | EEPROM                                                         |     8 KiB |

An explicit type resolves conflicting ROM markers. A state includes cartridge
save memory and loads only with the same ROM, BIOS, boot mode and format; a
failed load does not change the machine. Faulted machines skip `--state-out`
with `state save skipped: machine faulted`. See
[`design/save-states.md`](design/save-states.md) for the format.

## Cartridge peripherals

The clock, solar sensor, gyro, rumble and tilt are detected from cartridge
metadata. The clock may also be detected by the `RTC_V` library string. Flags
override this detection; the on/off variants of each device are mutually
exclusive:

| Flags                                  | Effect                                               |
| -------------------------------------- | ---------------------------------------------------- |
| `--no-rtc`, `--rtc-epoch <seconds>`    | Disable the clock or set its UTC Unix starting time. |
| `--no-solar`, `--solar-level <0..255>` | Disable or set the light sensor.                     |
| `--no-gyro`, `--gyro-sample <0..4095>` | Disable or set the raw 12-bit gyro sample.           |
| `--rumble`, `--no-rumble`              | Force the rumble motor on or off.                    |
| `--no-tilt`, `--tilt <x> <y>`          | Disable or set two raw 12-bit tilt samples.          |

`--gyro-sample` and `--tilt` accept decimal or `0x`-prefixed hexadecimal.
An automatically detected RTC starts from the host UTC time; thereafter it
advances with emulated time. Set `--rtc-epoch` for reproducible runs. Battery
saves do not include RTC state; portable states do.

```sh
cargo run -p evenfall-gba --bin evenfall-gba -- run --steps 100 --no-rtc game.gba
cargo run -p evenfall-gba --bin evenfall-gba -- run --steps 100 \
  --save-type eeprom8k --save-file game.sav game.gba
```
