# Native GBA parity oracle

Build source ROMs first (`make -C probe && make -C tools/bench/roms`); the
jsmolka ROMs live in the pinned, gitignored `refs/gba-tests` checkout. The BIOS
variants use the pinned Cult-of-GBA replacement image under `refs/`; no media
or initial save is vendored. See [GBA differential parity](../../docs/parity.md)
for the evidence boundary and trace interpretation.

```sh
cargo run -p evenfall-parity -- oracle build
cargo run -p evenfall-parity -- oracle run tools/parity/scenarios/probe-timer_irq.json
cargo run -p evenfall-parity -- run tools/parity/scenarios/probe-timer_irq.json
cargo run -p evenfall-parity -- run --all
cargo run -p evenfall-parity -- compare target/parity/traces/probe-timer_irq/oracle.trace OTHER.trace
```

`oracle build` copies ZiGBA `src/` into `target/parity/zigba`, adds only the
schema-export hook there, builds a Zig 0.16 ReleaseFast native executable in
`target/parity/bin`, and checks the exported schema against
`fixtures/schema-v6.bin` (its BLAKE3 digest is `fixtures/schema-v6.blake3`).
`refs/zigba` is never modified. `oracle run` writes
all per-schedule traces to `target/parity/traces/<name>/slice-N.trace`, checks
that they agree on _every byte_, and writes `oracle.trace` (or an optional
destination). It fails on a stopped/faulted machine or failed terminal PC/r12
assertion. `compare` prints the first discrepancy per kind and exits nonzero.

`run SCENARIO` first re-runs the Zig oracle, verifying its slice independence,
then starts a fresh Rust `Machine` for **each** slice size, writes the Rust
traces as `rust-slice-N.trace`, checks Rust slice independence and compares
every checkpoint with the Zig oracle. A state-byte mismatch includes the
narrowest field path from the Rust save-state codec's field tracer (or the
header/digest name); for an earlier checkpoint, it reloads that Rust snapshot
before tracing. `run --all` processes every JSON scenario and reports failures
per scenario instead of stopping at the first failing ROM. Both commands exit
nonzero on any mismatch or execution error. Keep the trace files when
investigating a difference with `compare`.

The JSON scenario uses the web developer-tool naming for `assets`, `config`,
`inputs`, and `checkpoints`, with absolute decimal-string cycles and paths
relative to the scenario file. Each input has `{ "at": "...", "change":
{ "keys": 8 } }` (Start pressed); changes may instead specify `solar_level`,
`gyro_sample`, or `tilt: [x,y]`. `endCycle` must equal the final checkpoint;
`sliceCycles` holds positive cycle lengths (a quarter frame is 70224). Native
DAC audio is drained after _every_ slice. Configuration selects `hle`,
`skip_bios`, or `external_bios`, save type (`auto`, `none`, `sram`,
`flash64k`, `flash128k`, `eeprom512`, `eeprom8k`, or
`flash64k_atmel`), optional save asset, `rtcEpoch`, sensor levels and rumble.
Optional `terminalPc` and `terminalR12` enforce known self-loop endpoints.
For the JoyBus probes, `hostActions` contains a cycle and a `command` with
`joybus_write` (u32) or `joybus_read: true`; the master command is scheduled
1000 cycles after the first instruction boundary at or after that cycle.

The binary trace format `EFPAR01\0` is little-endian: 32-bit checkpoint
count, then per checkpoint three u64 values (requested cycle, actual stopped
cycle, completed frames), sixteen u32 ARM registers, u32 next PC, u32 CPSR,
u32 native sample rate, then four blobs in order: frame, audio, battery save,
full save state. A blob is u32 byte length, 32-byte BLAKE3 hash of the raw
bytes, and that many raw bytes. The frame is 240×160 BGR555 (u16 LE); audio
is i16 LE interleaved left/right since the previous checkpoint. State bytes
contain the complete `ZIGBAST1` envelope, not just its payload. The format
rejects corrupt, truncated and trailing data; ROM and BIOS are not embedded.

The manifests cover the single-machine validator ROMs and continuous bench
workloads; bench ROMs have no terminal self-loop. The link-cable probes
require simultaneous machines and are intentionally excluded. JoyBus probes
use explicit host actions, not timed cartridge inputs. Other input-dependent
probes use timed key changes, including START and STOP wake. The generator
accepts a successful HLE validator log to choose durations with room beyond
the terminal loop; it does not derive expected bytes from the emulator.

The Rust backend maps the manifest to `evenfall_gba::machine::Config`, passes
timed inputs through `evenfall_replay::run_with_host_actions`, and schedules
JoyBus commands on the GBA bus 1000 cycles after the crossed boundary. The
shared trace codec and comparison code do not depend on either backend.
