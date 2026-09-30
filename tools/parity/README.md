# Parity harness

Evenfall's GBA core replaced ZiGBA, an earlier emulator written in Zig.
This internal tool checks that the two still agree byte for byte: it runs the same scenario on both, sliced into different chunk sizes, and compares frames, audio, battery saves and complete save states at every checkpoint.
[`internal-docs/parity.md`](../../internal-docs/parity.md) explains what this does and doesn't prove, and how to read a trace.

## Running it

Build the ROMs first (`make -C probe && make -C tools/bench/roms`).
The jsmolka ROMs come from the `refs/gba-tests` checkout, and the real-BIOS scenarios use the Cult-of-GBA replacement BIOS in `refs/` ([`refs/README.md`](../../refs/README.md)).
No ROM, BIOS or save is stored here.

```sh
cargo run -p evenfall-parity -- oracle build
cargo run -p evenfall-parity -- oracle run tools/parity/scenarios/probe-timer_irq.json
cargo run -p evenfall-parity -- run tools/parity/scenarios/probe-timer_irq.json
cargo run -p evenfall-parity -- run --all
cargo run -p evenfall-parity -- compare target/parity/traces/probe-timer_irq/oracle.trace OTHER.trace
```

- **`oracle build`** copies ZiGBA's `src/` into `target/parity/zigba`, adds the one hook that exports its state layout, and builds a native Zig 0.16 `ReleaseFast` executable in `target/parity/bin`.
  It checks the exported layout against `fixtures/schema-v6.bin`, whose BLAKE3 digest is in `fixtures/schema-v6.blake3`.
  `refs/zigba` itself is never changed.
- **`oracle run`** runs a scenario on ZiGBA once per slice size, writes each trace to `target/parity/traces/<name>/slice-N.trace`, checks that every byte agrees, and writes `oracle.trace` (or a destination you give).
  It fails on a stopped or faulted machine, or a failed check of the final program counter or `r12`.
- **`run SCENARIO`** first runs the ZiGBA oracle again, checking that its slices agree.
  It then starts a fresh Evenfall `Machine` for each slice size, writes `rust-slice-N.trace`, checks that Evenfall's slices agree, and compares every checkpoint with ZiGBA.
  A state mismatch names the narrowest field that differs, from the save-state codec's field tracer, or the header or digest.
  For an earlier checkpoint it reloads that state before tracing.
- **`run --all`** runs every scenario and reports failures per scenario instead of stopping at the first.
- **`compare`** prints the first difference of each kind and exits non-zero.

`run` and `run --all` exit non-zero on any mismatch or error.
Keep the trace files when you investigate a difference with `compare`.

## Scenarios

Scenarios are JSON, with the same `assets`, `config`, `inputs` and `checkpoints` names as the [developer tools](../../docs/devtools.md), absolute cycles as decimal strings, and paths relative to the scenario file.

- Each input is `{ "at": "...", "change": { "keys": 8 } }` (Start pressed).
  A change can instead set `solar_level`, `gyro_sample` or `tilt: [x, y]`.
- `endCycle` must equal the last checkpoint.
- `sliceCycles` lists positive chunk sizes, such as 70224, a quarter frame.
  Audio is drained after every slice.
- `config` selects the boot (`hle`, `skip_bios` or `external_bios`), the save type (`auto`, `none`, `sram`, `flash64k`, `flash128k`, `eeprom512`, `eeprom8k` or `flash64k_atmel`), an optional save asset, `rtcEpoch`, sensor levels and rumble.
- Optional `terminalPc` and `terminalR12` check the final idle loop.
- For the JoyBus probes, `hostActions` lists a cycle and a `command`, either `joybus_write` (a `u32`) or `joybus_read: true`.
  The command is scheduled 1000 cycles after the first instruction boundary at or after that cycle.
- For linked probes, the top-level machine is cable position 0, and `link` holds `connection` (`cable_2p` or `cable_multi`) and `peers`, one to three other machines, each with its `assets`, `config` and optional final checks.
  Linked scenarios have no timed inputs or host actions.
  Audio is drained for each machine after every slice, and traces are named `oracle-machine-I.trace`, `slice-N-machine-I.trace` and `rust-slice-N-machine-I.trace`.

The scenarios cover the single and linked probes, the jsmolka ROMs, and the benchmark workloads, which run continuously and have no final idle loop.
The generator takes a successful HLE validator log to choose run lengths with room past the final idle loop, and `--linked` regenerates the linked scenarios from the validator's cable order and expected results.
Neither derives expected trace bytes from either emulator.

On the Evenfall side, the scenario maps to `evenfall_gba::machine::Config`, timed inputs go through `evenfall_replay::run_with_host_actions`, and JoyBus commands are scheduled on the bus 1000 cycles after the boundary.
The trace format and comparison code depend on neither emulator.

## Trace format

A trace starts with the magic `EFPAR01\0` and is little-endian.
After a 32-bit checkpoint count, each checkpoint holds:

1. three `u64` values: the requested cycle, the cycle it stopped at, and completed frames,
2. the sixteen `u32` ARM registers, then the next program counter, the CPSR and the native sample rate, each a `u32`,
3. four blobs, in order: the frame, the audio, the battery save and the full save state.

A blob is a `u32` byte length, the 32-byte BLAKE3 hash of the bytes, then the bytes.
The frame is 240x160 15-bit BGR pixels (`u16` little-endian).
The audio is interleaved `i16` little-endian stereo since the previous checkpoint.
The state is the complete `ZIGBAST1` file, not just its payload.
Corrupt, truncated and trailing data are rejected, and the ROM and BIOS are never embedded.
