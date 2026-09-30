# Evenfall GBA performance tools

[`run.mjs`](run.mjs) compares Evenfall with open GBA emulators on
purpose-built ROMs. It builds the ROMs in a run-local directory, builds native
harnesses before timing, selects one allowed idle/fast CPU, takes the
host-wide `flock` lane, and pins fresh harness processes there with
`taskset`. Emulator order rotates per round and workload. Its report records
source/binary/ROM/BIOS hashes, cycles, images, observations and median frame
rates. Time ratios are the geometric mean of **matched-round** loop-time
ratios, then the geometric mean over workloads; values below 1 are faster than
the baseline. Synthetic ROMs are a path comparison, not a game-wide
performance or accuracy claim.

Requires Linux, Node, `flock`, `taskset`, Rust (rustup's
`~/.cargo/bin/cargo`), CMake, a C compiler, Clang, zlib and
`arm-none-eabi-gcc`/`arm-none-eabi-objcopy`. Keep BIOS images and ROMs
user-supplied; no assets are downloaded or modified. Nothing is written into
`refs/`; build caches live in `target/bench`. The default emulators, in
baseline-first order, are:

- `mgba`: mGBA 0.10.5 (`refs/mgba`) as its headless CMake `Release` library
  in `target/bench/mgba-build`, with
  [`harness/mgba-harness.c`](harness/mgba-harness.c) built `cc -O2`. Software
  renderer; idle-loop detection stays at mGBA's default, `detect`.
- `vbam`: the VBA-M core that Delta ships. `refs/delta` pins
  `refs/gbadeltacore`, which pins `refs/vbam`.
  [`harness/vbam/CMakeLists.txt`](harness/vbam/CMakeLists.txt) compiles
  GBADeltaCore's `libVBA-M` source set (the portable `C_CORE` interpreter,
  link code and bundled SFML included) with its definitions and
  `-ffast-math`, by Clang at `-Os`: that Xcode target's Release build sets no
  optimization level, so Xcode's default applies.
  [`harness/vbam-harness.cpp`](harness/vbam-harness.cpp) sets the core up as
  GBADeltaCore does.
- `evenfall`: `evenfall-harness` ([`src/main.rs`](src/main.rs)) with the
  `release-lto` profile (fat LTO, one codegen unit, abort on panic), the
  native equivalent of the shipped WebAssembly build. It builds with rustup's
  cargo without `RUSTC_WRAPPER`, since output-cache wrappers that compile LTO
  profiles incrementally produce slower code.

Two more adapters run only when listed in `--emulators`. `vbam-o3` is the
`vbam` build at `-O3`, which shows how much of VBA-M's speed depends on
Delta's optimization level. `zigba` builds Evenfall's predecessor ZiGBA from
`refs/zigba` at `ReleaseFast`, for internal comparisons; its last measurement
put Evenfall at 0.41× ZiGBA's loop time under both boots.

The mGBA and VBA-M harnesses link their emulators (MPL-2.0 and GPL-2.0) into
local binaries that are never distributed; no source from either is copied
into Evenfall. VBA-M's `CPULoadRomData`, which GBADeltaCore calls, allocates a
240×160 pixel buffer while the 32-bit renderer writes 241-pixel rows from the
second row on, past the end of the buffer every frame. The harness reallocates
it at the 241×162 size VBA-M's `CPULoadRom` uses, so the overrun cannot
corrupt emulator state; the emulated work is unchanged. VBA-M has no global
cycle counter, so its harness derives guest time from the VBlank count and the
LCD position. It produces 32,768 Hz sound, as Delta does, into a driver that
discards the samples.

```sh
node --test tools/bench/*.test.mjs
node tools/bench/run.mjs --list
node tools/bench/run.mjs --emulators mgba --boot hle --workloads idle --frames 60 --rounds 1 --warmups 0
node tools/bench/run.mjs --boot hle --cpu 3
node tools/bench/run.mjs --bios path/to/bios.bin --cpu 3
node tools/bench/run.mjs --boot hle --cpu 3 --baseline vbam
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

## Inline stability

[`inline-stability.mjs`](inline-stability.mjs) builds the native
`release-lto` harness and the `simd128` web module with LLVM's inline
threshold at 200, 250 (the default) and 300, then compares the sizes of the
out-of-line functions in the CPU engine, device step, DMA, APU mixer and
compositor. A function whose size changes holds a call whose inlining sits
near the threshold, which an unrelated edit can flip; pin that call's callee
with `#[inline(always)]` or `#[inline(never)]`, keeping whichever measures
faster. The tool exits 1 when anything changes. It bypasses output-cache
wrappers, as `build-times.sh` does, and needs `nm` and `c++filt`.

```sh
node tools/bench/inline-stability.mjs
node tools/bench/inline-stability.mjs --targets native --filter 'thumb::run_loop'
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
