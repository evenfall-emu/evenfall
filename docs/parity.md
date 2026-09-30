# GBA differential parity

The parity harness runs the pinned ZiGBA `Machine` and the Rust GBA `Machine`
from the same scenario. At each requested guest cycle it compares actual
stopped cycle, completed frames, ARM registers/next PC/CPSR, the completed
240×160 BGR555 frame, native interleaved stereo DAC samples since the previous
checkpoint, cartridge save bytes and the complete `ZIGBAST1` save-state bytes.
It also runs each backend under every declared host slice size and requires
byte-identical traces across schedules. Input changes are timestamped in guest
cycles; the runner drains audio after every slice.

Agreement proves these observations for the selected media, boot mode,
configuration, inputs, checkpoints and slice schedules. It does not prove
unobserved cycles, arbitrary games, physical GBA accuracy, replacement-BIOS
compatibility with Nintendo's BIOS, or resampled host audio. ZiGBA is the
**port oracle**, not an independent hardware specification. A schema fixture
checks durable layout separately; a matching schema digest does not establish
matching state values. The [methodology](methodology.md) distinguishes the
port contract from hardware evidence.

## Scenarios and execution

Build the source ROMs and install the pinned, gitignored references before
running the harness. No ROM, BIOS image or save is committed here.

```sh
tools/setup-refs.sh zigba gba-tests cult-gba-bios
make -C probe
make -C tools/bench/roms
cargo run -p evenfall-parity -- oracle build
cargo run -p evenfall-parity -- run tools/parity/scenarios/jsmolka-arm-external_bios.json
cargo run -p evenfall-parity -- run --all
```

`run` executes the oracle afresh, verifies ZiGBA's slice independence, starts
a fresh Rust machine for each slice size, verifies Rust's slice independence,
and compares every checkpoint. `run --all` continues past individual scenario
errors, then fails if any scenario differs. `oracle run SCENARIO [OUTPUT]` runs
only ZiGBA. The CI parity job performs `oracle build` followed by `run --all`
with Zig and the Arm GNU ROM-build toolchain supplied by `.agents/setup`.

Manifests in [`tools/parity/scenarios/`](../tools/parity/scenarios/) cover
single-machine jsmolka validator ROMs, project probe ROMs and continuous
benchmark workloads. The jsmolka ROMs have both HLE and `external_bios`
variants; bench ROMs have HLE and `skip_bios` variants. The latter use the
pinned Cult-of-GBA replacement image through a path under `refs/`; nothing is
vendored in the manifests. External boot checkpoints include the BIOS startup
before the cartridge reaches its endpoint. The replacement BIOS leaves boot through other
opcodes than Nintendo's, so the jsmolka BIOS ROM fails its first
protected-read check under it; parity still compares what **both
emulators** do. Some key, STOP, audio and bench
scenarios add an odd, small slice size alongside quarter- and full-frame
slices. Linked probes run two to four machines on one cable timeline in
validator cable order, using the standard two-player or multiplayer cable.
The two-slave MultiBoot case creates two distinct machines from the same
child ROM. Each peer has its own terminal PC and r12 checks.

Each JSON manifest names relative ROM/optional BIOS/save assets, boot and
save type, RTC/sensor settings, absolute-cycle input changes and JoyBus host
actions, an end cycle, checkpoints and positive slice lengths. The optional
terminal PC/r12 guards test endpoints; continuous bench ROMs have no terminal
self-loop. [`tools/parity/README.md`](../tools/parity/README.md) describes the
manifest keys and the available CLI commands.

In linked manifests the top-level `assets`, `config`, and terminal guards
describe machine 0. `link.connection` selects `cable_2p` or `cable_multi`;
`link.peers` lists one to three additional machines in cable order, each with
its own assets, config and optional terminal guards. Linked scenarios do not
accept timed inputs or JoyBus host actions; the linked probes need neither.
Each backend advances every machine to a checkpoint with its linked scheduler
and drains each machine's audio after every host slice.

## Traces and first differences

`run` writes `target/parity/traces/<scenario>/oracle.trace` and
`rust-slice-N.trace`, with the oracle's individual `slice-N.trace` files beside
them. A linked run instead writes one standard trace per machine in cable
order: `oracle-machine-I.trace`, `slice-N-machine-I.trace` and
`rust-slice-N-machine-I.trace`. Slice independence and parity are checked for
every machine. Compare saved traces independently with:

```sh
cargo run -p evenfall-parity -- compare target/parity/traces/SCENARIO/oracle.trace target/parity/traces/SCENARIO/rust-slice-0.trace
cargo run -p evenfall-parity -- compare target/parity/traces/probe-link/oracle-machine-1.trace target/parity/traces/probe-link/rust-slice-0-machine-1.trace
```

The compact `EFPAR01\0` binary format is little-endian: an eight-byte magic,
a u32 checkpoint count, then for each checkpoint three u64 values (requested
cycle, actual cycle, completed frames), sixteen u32 registers, u32 next PC,
u32 CPSR, u32 native sample rate and four blobs in order: framebuffer, audio,
battery save and full save state. Every blob has a u32 length, 32-byte BLAKE3
digest of its raw content and exactly that many bytes. The frame is row-major
u16 BGR555; audio consists of signed i16 left/right pairs. State bytes retain
ZiGBA's entire versioned envelope and payload digest. Truncation, trailing
data and invalid blob digests are rejected; ROM and BIOS bytes are not embedded.

The comparator prints the first difference **per kind** across checkpoints:
pixel coordinates and BGR555 values, stereo sample index, save-byte offset,
state-byte offset, cycle, frames or CPU state. `compare` needs only two trace
files and thus reports a raw state-byte offset. `run` also has the Rust
machine: it reloads the corresponding checkpoint if necessary, obtains
payload-relative field ranges from `evenfall-state` through
`Machine::state_field_ranges`, and labels the **narrowest containing field**
after adding the 133-byte `ZIGBAST1` header. Header fields and the trailing
payload digest have separate names. A field such as `cpu.core.registers` or
`bus.vram` identifies the owning encoded region, not necessarily an individual
register or array element. Read the reported offset together with the first
CPU, pixel and cycle differences; a later state difference can be a consequence
of an earlier guest divergence.

## Oracle source and fixture

`oracle build` copies ZiGBA's `src/` to `target/parity/zigba`, copies the
schema exporter into that scratch tree and applies
[`schema-root.patch`](../tools/parity/patches/schema-root.patch) there to
export it from `root.zig`. It then applies
[`bios-protection.patch`](../tools/parity/patches/bios-protection.patch), the
one deliberate guest-visible difference from ZiGBA
([`roadmap.md`](roadmap.md)): with a BIOS image, reads of the BIOS while the
CPU's latest opcode fetch was elsewhere return the last fetched BIOS opcode
([memory](gba/hardware/memory.md#bios-read-protection)). The patch adds the
transient `bios_code_active` gate to ZiGBA's bus, sets it on every fetch
through `Bus.read` and clears it where a refill fetches outside `Bus.read`.
It changes no HLE behavior and no durable layout. The pinned `refs/zigba`
checkout stays read-only.
Zig 0.16 builds the native oracle and exports ZiGBA's version-six schema
description; the build compares it byte for byte with the committed
[`schema-v6.bin`](../tools/parity/fixtures/schema-v6.bin) fixture. The
external-BIOS and skip-BIOS scenarios therefore compare Evenfall with the
patched oracle; HLE scenarios are unaffected by the patch.

One further patch changes guest behavior, and only where ZiGBA would stop:
[`dma-stall.patch`](../tools/parity/patches/dma-stall.patch) replaces ZiGBA's
terminal DMA livelock fault with the stall the Rust core implements
([roadmap](roadmap.md#dma-that-never-releases-the-bus)). Its new bus fields
are transient, so the schema check still holds. No scenario ZiGBA completes
changes; `probe-dma_stall` exercises the patched path. Its manifest sets
`"audioOutput": false` in `config`: the host call that reaches the stall's
limit runs about one second of guest time, more than the host sample queue
holds, so drained audio would depend on slicing. The key defaults to true
and only suppresses host samples; emulated sound is unchanged.

The remaining patches apply the fixes that the mGBA suite ROM's hardware
tables justify ([`roadmap.md`](roadmap.md#mgba-suite-rom-findings)), in
this order:

| Patch                                                         | Guest-visible change                                                       | Durable layout |
| ------------------------------------------------------------- | -------------------------------------------------------------------------- | -------------- |
| [`io-read.patch`](../tools/parity/patches/io-read.patch)       | write-only I/O registers read as open bus or zero; two read-only bit masks | unchanged      |
| [`thumb-open-bus.patch`](../tools/parity/patches/thumb-open-bus.patch) | Thumb opcode fetches compose the open-bus word per region                  | unchanged      |
| [`multiply-carry.patch`](../tools/parity/patches/multiply-carry.patch) | flag-setting multiplies set C from the multiplier model                    | unchanged      |
| [`cpu-set-halfword.patch`](../tools/parity/patches/cpu-set-halfword.patch) | HLE halfword `CpuSet` loads and stores like `LDRH` and `STRH`              | unchanged      |

Each changes the Rust core and the oracle alike, so every scenario still
compares byte for byte; none adds a durable field, so the schema check still
holds.
