# Scenario runner

Evenfall's results must never depend on how a host slices execution into calls.
This tool checks that rule on real guest code: it runs each scenario on a fresh machine once per slice size, and requires every checkpoint's frame, audio, battery save, CPU registers and full save state to agree across the sizes.
It also runs the [mGBA suite ROM](../mgba-suite/README.md) and reads the suite's results from those traces.

## Running it

Build the ROMs first (`make -C probe && make -C tools/bench/roms`).
The jsmolka ROMs come from the `refs/gba-tests` checkout, and the real-BIOS scenarios use the Cult-of-GBA replacement BIOS in `refs/` ([`refs/README.md`](../../refs/README.md)).
No ROM, BIOS or save is stored here.

```sh
cargo run --release -p evenfall-scenarios -- run tools/scenarios/manifests/probe-timer_irq.json
cargo run --release -p evenfall-scenarios -- run --all
cargo run --release -p evenfall-scenarios -- compare A.trace B.trace
```

- **`run SCENARIO`** starts a fresh `Machine` for each slice size and writes its trace to `target/scenarios/traces/<name>/slice-N.trace`.
  It checks that every slice size's trace matches the first's.
  A state mismatch names the narrowest field that differs, from the save-state codec's field tracer, or the header or digest.
  For an earlier checkpoint it reloads that state before tracing.
  A stopped or faulted machine, or a failed check of the final program counter or `r12`, fails the run.
- **`run --all`** runs every manifest and reports failures per scenario instead of stopping at the first.
  It includes the mGBA suite scenarios, so it needs that ROM built as well.
- **`compare`** prints the first difference of each kind between two traces and exits non-zero.
  Use it on traces from two revisions to find where they diverge.

`run` and `run --all` exit non-zero on any mismatch or error.

## Scenarios

Scenarios are JSON, with the same `assets`, `config`, `inputs` and `checkpoints` names as the [developer tools](../../docs/devtools.md), absolute cycles as decimal strings, and paths relative to the scenario file.

- Each input is `{ "at": "...", "change": { "keys": 8 } }` (Start pressed).
  A change can instead set `solar_level`, `gyro_sample` or `tilt: [x, y]`.
- `endCycle` must equal the last checkpoint.
- `sliceCycles` lists positive chunk sizes, such as 70224, a quarter frame.
  Audio is drained after every slice.
- `config` selects the boot (`hle`, `skip_bios` or `external_bios`), the save type (`auto`, `none`, `sram`, `flash64k`, `flash128k`, `eeprom512`, `eeprom8k` or `flash64k_atmel`), an optional save asset, `rtcEpoch`, sensor levels, rumble and `audioOutput`.
  `audioOutput` defaults to true. False suppresses host samples, for a scenario whose single host call outlasts the sample queue.
- Optional `terminalPc` and `terminalR12` check the final idle loop.
- For the JoyBus probes, `hostActions` lists a cycle and a `command`, either `joybus_write` (a `u32`) or `joybus_read: true`.
  The command is scheduled on the bus 1000 cycles after the first instruction boundary at or after that cycle.
- For linked probes, the top-level machine is cable position 0, and `link` holds `connection` (`cable_2p` or `cable_multi`) and `peers`, one to three other machines, each with its `assets`, `config` and optional final checks.
  Linked scenarios have no timed inputs or host actions.
  Audio is drained for each machine after every slice, and traces are named `slice-N-machine-I.trace`.

The manifests cover the single and linked probes, the jsmolka ROMs, the benchmark workloads, which run continuously and have no final idle loop, and the mGBA suite.
Their run lengths leave room past each ROM's final idle loop.
No manifest holds expected trace bytes: each run is checked against itself under other slice sizes.

The scenario maps to `evenfall_gba::machine::Config`, and timed inputs go through `evenfall_replay::run_with_host_actions`.

## Trace format

A trace starts with the magic `EFPAR01\0` and is little-endian.
After a 32-bit checkpoint count, each checkpoint holds:

1. three `u64` values: the requested cycle, the cycle it stopped at, and completed frames,
2. the sixteen `u32` ARM registers, then the next program counter, the CPSR and the native sample rate, each a `u32`,
3. four blobs, in order: the frame, the audio, the battery save and the full save state.

A blob is a `u32` byte length, the 32-byte BLAKE3 hash of the bytes, then the bytes.
The frame is 240x160 15-bit BGR pixels (`u16` little-endian).
The audio is interleaved `i16` little-endian stereo since the previous checkpoint.
The state is the complete save-state file, not just its payload.
Corrupt, truncated and trailing data are rejected, and the ROM and BIOS are never embedded.
