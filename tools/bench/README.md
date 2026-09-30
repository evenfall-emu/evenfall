# Evenfall GBA performance tools

These tools measure how fast Evenfall runs, compare it with other GBA emulators, and keep its hot paths from getting slower by accident.
[`docs/performance.md`](../../docs/performance.md) shows the current results and explains where the speed comes from.

- [`run.mjs`](run.mjs) compares Evenfall with mGBA and Delta's VBA-M core on purpose-built ROMs.
- [`wasm.mjs`](wasm.mjs) does the same for WebAssembly builds, in Node.
- [`inline-stability.mjs`](inline-stability.mjs) checks that no hot-path function's inlining sits on a knife's edge.
- [`build-times.sh`](build-times.sh) measures cold and one-line rebuild times.
- [`diag/`](diag/README.md) holds three loops that measure the CPU's cost per guest instruction.

## Comparing emulators

```sh
tools/setup-refs.sh mgba delta gbadeltacore vbam
node tools/bench/run.mjs --list
node tools/bench/run.mjs --boot hle --cpu 3
node tools/bench/run.mjs --bios path/to/bios.bin --cpu 3
node tools/bench/run.mjs --boot hle --cpu 3 --baseline vbam
node tools/bench/run.mjs --emulators mgba --boot hle --workloads idle --frames 60 --rounds 1 --warmups 0
vp test tools/bench
```

The runner needs Linux, Node, `flock`, `taskset`, Rust (rustup's `~/.cargo/bin/cargo`), CMake, a C compiler, Clang, zlib, and `arm-none-eabi-gcc` with `arm-none-eabi-objcopy`.
It downloads and changes no ROM or BIOS, writes nothing into `refs/`, and keeps its build caches in `target/bench`.

For a fair comparison, the runner:

1. builds the ROMs and every emulator before timing anything,
2. picks one idle, fast CPU core and takes a host-wide `flock` lock, so two benchmarks never overlap,
3. runs each emulator in a fresh process pinned to that core with `taskset`,
4. rotates the order of emulators in each round and workload,
5. reports each ratio as the geometric mean of loop-time ratios from matched rounds, then the geometric mean across workloads.

Below 1 means faster than the baseline, which is the first emulator listed (`--baseline` changes it).
The report in `target/bench/runs/<run>/result.json` records the hashes of every source, binary, ROM and BIOS, the cycles, images and median frame rates.
All emulators must reach the same final picture, which catches one that went astray, though it doesn't prove hardware accuracy.
The ROMs each stress one part of the machine, so the results compare code paths, not whole games.

The default boot is skip-BIOS, with the BIOS from `--bios` or `refs/cult-gba-bios/bios.bin`.
`--boot hle` needs no BIOS.
`--harness NAME=PATH` uses a prebuilt binary, and `--work-dir`, `--cpu`, `--frames`, `--rounds`, `--warmups` and `--workloads` shape the run.

### The emulators

The defaults, in order:

- **`mgba`** is mGBA 0.10.5 (`refs/mgba`), built as its headless CMake `Release` library in `target/bench/mgba-build`, with [`harness/mgba-harness.c`](harness/mgba-harness.c) built at `-O2`.
  It uses the software renderer, and keeps its default idle-loop detection.
- **`vbam`** is the VBA-M core the Delta iOS emulator ships.
  `refs/delta` pins `refs/gbadeltacore`, which pins `refs/vbam`.
  [`harness/vbam/CMakeLists.txt`](harness/vbam/CMakeLists.txt) compiles GBADeltaCore's `libVBA-M` sources (the portable `C_CORE` interpreter, link code and bundled SFML included) with its definitions and `-ffast-math`, using Clang at `-Os`, which is what Xcode's release build uses when a target sets no optimization level.
  [`harness/vbam-harness.cpp`](harness/vbam-harness.cpp) sets the core up the way GBADeltaCore does.
- **`evenfall`** is `evenfall-harness` ([`src/main.rs`](src/main.rs)) built with the `release-lto` profile (fat LTO, one codegen unit, abort on panic), the native twin of the shipped WebAssembly build.
  It builds with rustup's cargo and no `RUSTC_WRAPPER`, because output-cache wrappers that compile LTO profiles incrementally produce slower code.

Two more run only when named in `--emulators`:

- **`vbam-o3`** is the `vbam` build at `-O3`, to show how much of VBA-M's speed depends on Delta's optimization level.
- **`zigba`** builds Evenfall's predecessor from `refs/zigba`, for internal comparisons.
  Its last measurement put Evenfall at 0.41x its loop time under both boots.

The mGBA and VBA-M harnesses link those emulators (MPL-2.0 and GPL-2.0) into local binaries that are never distributed, and no code from either is copied into Evenfall.

Two VBA-M details:

- VBA-M's `CPULoadRomData`, which GBADeltaCore calls, allocates a 240x160 pixel buffer, but the 32-bit renderer writes 241-pixel rows, running past the end of the buffer every frame.
  The harness reallocates it at the 241x162 size VBA-M's own `CPULoadRom` uses, so the overrun can't corrupt the emulator.
  The emulated work is unchanged.
- VBA-M has no global cycle counter, so its harness works out guest time from the VBlank count and the display position.
  It produces 32,768 Hz sound, as Delta does, into a driver that discards it.

### The harness protocol

Every harness takes `ROM FRAMES [--bios=PATH] [--dump-frame=PATH]`, runs one 280,896-cycle frame at a time with full drawing and emulated audio (never drained), and prints one `benchmark: ` JSON line.
Loading, setup and hashing happen outside the timed loop.

The JSON line has schema 1: frames, timed-loop seconds, the cycle as a decimal string, `boot`, `video:full`, `audio:exact`, and a lowercase 16-digit FNV-1a 64 hash of the frame.
The hash covers the visible 240x160 pixels as 15-bit color, low byte then high byte, with bit 15 masked.
`--dump-frame` writes a P6 PPM, expanding each 5-bit channel as `(c << 3) | (c >> 2)`.

## Microbenchmarks and profiling

The divan benches time APU runs (audible and silent), APU and SRAM state encoding and decoding, and a drawn frame through `Bus::elapse`:

```sh
cargo bench -p evenfall-gba --bench hot_paths
```

For a whole-machine profile with symbols, build the Rust harness with the `profiling` profile, and run it on the same ROM and boot mode you compare.
Keep setup out of the part of the profile you read.

```sh
cargo build --profile profiling -p evenfall-bench --bin evenfall-harness
perf record --call-graph dwarf -- target/profiling/evenfall-harness \
  target/bench/runs/<run>/build/roms/audio.gba 600
perf report
```

## Inline stability

With fat LTO, LLVM inlines a call when its cost is under a threshold.
A call just under or over it can flip when an unrelated edit changes its caller's size, and that moves benchmarks by several percent.
[`inline-stability.mjs`](inline-stability.mjs) finds such calls.
It builds the native `release-lto` harness and the `simd128` web module with the inline threshold at 200, 250 (the default) and 300, and compares the sizes of the out-of-line functions in the CPU engine, device step, DMA, APU mixer and compositor.

A function whose size changes holds a call near the threshold.
Pin that call's callee with `#[inline(always)]` or `#[inline(never)]`, whichever measures faster.
The tool exits 1 when anything changes.
It bypasses output-cache wrappers, as `build-times.sh` does, and needs `nm` and `c++filt`.

```sh
node tools/bench/inline-stability.mjs
node tools/bench/inline-stability.mjs --targets native --filter 'thumb::run_loop'
```

## WebAssembly

[`wasm.mjs`](wasm.mjs) times WebAssembly modules that export the `@evenfall/gba` interface on the same ROMs and frame counts, in Node.
Each measurement is a fresh Node process pinned with `taskset`, module order rotates per round and workload, and ratios are the geometric mean of matched-round loop-time ratios against the first module.

A process loads the ROM with HLE boot, turns off audio output (emulation continues), runs `--warm` untimed frames (default 60, counted in the total), and times the rest one `ef_run_until` per frame.
The final frame hash is the native harnesses' FNV-1a, so WebAssembly and native pictures can be compared.

```sh
web/gba/scripts/build-wasm.sh
node tools/bench/wasm.mjs --module simd=web/gba/evenfall-gba.wasm \
  --module scalar=web/gba/evenfall-gba-scalar.wasm --cpu 3 --rounds 5 --warmups 1
```

`--workloads`, `--frames`, `--rounds`, `--warmups`, `--warm` and `--cpu` shape the run, and results go to `target/bench/wasm-runs/<run>/result.json`.
The driver doesn't handle STOP or linked sessions.
