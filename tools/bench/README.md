# Evenfall GBA performance tools

The runner ports ZiGBA's `tools/emu-bench` measurement method. It builds the
purpose-built ROMs in a run-local directory, builds native harnesses before
timing, selects one allowed idle/fast CPU, takes the host-wide `flock` lane,
and pins fresh harness processes there with `taskset`. Emulator order rotates
per round and workload. Its report records source/binary/ROM/BIOS hashes,
cycles, images, observations and median frame rates. Time ratios are the
geometric mean of **matched-round** loop-time ratios, then the geometric mean
over workloads; values below 1 are faster than the baseline. Synthetic ROMs
are a path comparison, not a game-wide performance or accuracy claim.

Requires Linux, Node, `flock`, `taskset`, Zig 0.16, Rust and
`arm-none-eabi-gcc`/`arm-none-eabi-objcopy`. Keep BIOS images and ROMs
user-supplied; no assets are downloaded or modified. The adapter compiles
ZiGBA's pinned `refs/zigba/tools/emu-bench/harness/zigba-harness.zig` at
`ReleaseFast` into `target/bench`, with both Zig caches there. Nothing is
written into `refs/zigba`. The Rust adapter builds `evenfall-harness` with the
`release-lto` profile (fat LTO, one codegen unit, abort on panic), the native
equivalent of the shipped WebAssembly build and of Zig's whole-program
ReleaseFast.

```sh
node --test tools/bench/*.test.mjs
node tools/bench/run.mjs --list
node tools/bench/run.mjs --emulators zigba --boot hle --workloads idle --frames 60 --rounds 1 --warmups 0
node tools/bench/run.mjs --emulators zigba,evenfall --boot hle
node tools/bench/run.mjs --emulators zigba,evenfall --bios path/to/bios.bin
```

Default boot is `skip-bios`, using a user-provided BIOS (`--bios`) or a local
`refs/cult-gba-bios/bios.bin` if available. `--boot hle` needs no BIOS.
Results are in `target/bench/runs/<run>/result.json`. `--harness NAME=PATH`
can substitute a prebuilt binary. `--work-dir DIR`, `--cpu N`, `--frames N`,
`--rounds N`, `--warmups N`, `--baseline NAME` and `--workloads LIST` control
the plan. Images agree only when all harnesses deterministically reach the
same visible frame; agreement does not prove hardware accuracy.

Each harness accepts `ROM FRAMES [--bios=PATH] [--dump-frame=PATH]`. The
`benchmark: ` JSON line has schema 1, frames, timed-loop seconds, decimal
cycle string, `boot`, `video:full`, `audio:exact`, and a lowercase 16-digit
FNV-1a 64 framebuffer hash. The hash consumes visible 240×160 RGB555 pixels,
low byte then high byte (masking bit 15); the P6 PPM expands each 5-bit
channel as `(c << 3) | (c >> 2)`. Harnesses execute one 280896-cycle guest
frame at a time with full video and emulated audio never drained. Loading,
initialization and hashing are outside the timed loop.

Run device microbenchmarks (APU audible/constant, APU and SRAM state
encode/decode, and a rendered frame through `Bus::elapse`) with:

```sh
cargo bench -p evenfall-gba --bench hot_paths
```

For a symbolized whole-machine profile, build and run the Rust harness with
the same ROM and boot mode you intend to compare. Keep setup out of the
profiled loop when interpreting samples:

```sh
cargo build --profile profiling -p evenfall-bench --bin evenfall-harness
perf record --call-graph dwarf -- target/profiling/evenfall-harness \
  target/bench/runs/<run>/build/roms/audio.gba 600
perf report
```

## WebAssembly

[`wasm.mjs`](wasm.mjs) times WebAssembly modules that export the
`@evenfall/gba` ABI, Evenfall's and ZiGBA's alike, on the same ROMs and frame
counts in Node. Each observation is a fresh Node process pinned with
`taskset`; module order rotates per round and workload, and ratios are the
geometric mean of matched-round loop-time ratios against the first module.
A process loads the ROM with HLE boot, disables audio output (emulation
continues), runs `--warm` untimed frames (default 60, counted in the total)
and times the rest one `zb_run_until` per frame. The final framebuffer hash
is the native harnesses' FNV-1a, so wasm and native images can be compared.

```sh
web/gba/scripts/build-wasm.sh
node tools/bench/wasm.mjs --module zigba=path/to/zigba.wasm \
  --module evenfall=web/gba/evenfall-gba.wasm --cpu 3 --rounds 5 --warmups 1
```

`--workloads`, `--frames`, `--rounds`, `--warmups`, `--warm` and `--cpu`
control the plan; results are in `target/bench/wasm-runs/<run>/result.json`.
ZiGBA's module comes from `zig build web-sdk` in a copy of the pinned
checkout (the step writes into its source tree). The driver does not handle
STOP or linked sessions.
