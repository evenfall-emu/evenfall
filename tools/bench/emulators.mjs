// Native adapters for the benchmark harness contract (README.md). An adapter
// builds a harness before timing and declares the boot modes it supports.
import { spawnSync } from "node:child_process";
import { copyFileSync, existsSync, readFileSync } from "node:fs";
import { cpus, homedir } from "node:os";
import { join } from "node:path";
import { ROOT, run, sha256 } from "./common.mjs";

export const git = (args, cwd = ROOT) => {
  const result = spawnSync("git", args, { cwd, encoding: "utf8" });
  return result.status === 0 ? result.stdout.trim() : null;
};

const HARNESS = join(ROOT, "tools/bench/harness");
const JOBS = `-j${Math.min(cpus().length, 4)}`;

// A pinned reference checkout (refs/README.md); a mismatch is an error.
const checkRef = (name, pin) => {
  const dir = join(ROOT, "refs", name);
  const head = git(["rev-parse", "HEAD"], dir);
  if (head !== pin)
    throw new Error(
      `refs/${name} is ${head ?? "missing"}, expected ${pin}; run tools/setup-refs.sh ${name}`,
    );
  return {
    dir,
    source: pin,
    trackedChanges:
      git(["status", "--porcelain", "--untracked-files=no"], dir) || null,
  };
};

// VBA-M as Delta ships it: GBADeltaCore's libVBA-M source set and flags
// (harness/vbam/CMakeLists.txt), compiled by Clang like Xcode's toolchain.
// Delta's Release build sets no optimization level, so Xcode's -Os applies.
const vbam = (opt) => ({
  title:
    opt === "-Os"
      ? "Delta's VBA-M core (GBADeltaCore build), clang -Os"
      : `VBA-M in Delta's GBADeltaCore build, but clang ${opt}`,
  boots: ["hle", "skip-bios"],
  build(buildDir) {
    const ref = checkRef("vbam", "453fa0decf179360926fb417725a794aa496d6e3");
    const core = checkRef(
      "gbadeltacore",
      "869c34aeca9dd2b3fbd329092bb2743b0ae80c98",
    );
    const dir = join(ROOT, `target/bench/vbam${opt}`);
    run("cmake", [
      "-S",
      join(HARNESS, "vbam"),
      "-B",
      dir,
      `-DVBAM_SOURCE=${ref.dir}`,
      `-DGBADELTACORE_SOURCE=${core.dir}`,
      `-DVBAM_OPT=${opt}`,
      "-DCMAKE_C_COMPILER=clang",
      "-DCMAKE_CXX_COMPILER=clang++",
    ]);
    run("cmake", ["--build", dir, "--target", "vbam-harness", JOBS]);
    const binary = join(buildDir, `vbam${opt}-harness`);
    copyFileSync(join(dir, "vbam-harness"), binary);
    return {
      binary,
      source: ref.source,
      trackedChanges: ref.trackedChanges,
      gbadeltacore: core.source,
      build: `CMake, GBADeltaCore flags, clang ${opt} -ffast-math (${dir})`,
    };
  },
});

export const EMULATORS = {
  mgba: {
    title: "mGBA 0.10.5, software renderer",
    boots: ["hle", "skip-bios"],
    build(buildDir) {
      const ref = checkRef("mgba", "26b7884bc25a5933960f3cdcd98bac1ae14d42e2");
      // The headless library (with the debugger build that exposes
      // mTimingGlobalTime) persists between runs; make keeps it current.
      const library = join(ROOT, "target/bench/mgba-build");
      if (!existsSync(join(library, "CMakeCache.txt")))
        run("cmake", [
          "-S",
          ref.dir,
          "-B",
          library,
          "-DCMAKE_BUILD_TYPE=Release",
          "-DCMAKE_POLICY_VERSION_MINIMUM=3.5",
          "-DBUILD_QT=OFF",
          "-DBUILD_SDL=OFF",
          "-DBUILD_TEST=OFF",
          "-DBUILD_SUITE=OFF",
          "-DBUILD_CINEMA=OFF",
          "-DBUILD_PYTHON=OFF",
          "-DBUILD_EXAMPLE=OFF",
          "-DUSE_FFMPEG=OFF",
          "-DUSE_LIBZIP=OFF",
          "-DUSE_SQLITE3=OFF",
          "-DBUILD_GL=OFF",
          "-DBUILD_GLES2=OFF",
          "-DBUILD_GLES3=OFF",
          "-DSKIP_GIT=ON",
        ]);
      run("cmake", ["--build", library, "--target", "mgba", JOBS]);
      const binary = join(buildDir, "mgba-harness");
      run(process.env.CC ?? "cc", [
        "-O2",
        join(HARNESS, "mgba-harness.c"),
        "-o",
        binary,
        "-I",
        join(ref.dir, "include"),
        "-I",
        join(library, "include"),
        "-L",
        library,
        "-lmgba",
        `-Wl,-rpath,${library}`,
      ]);
      return {
        binary,
        source: ref.source,
        trackedChanges: ref.trackedChanges,
        libmgbaSha256: sha256(readFileSync(join(library, "libmgba.so"))),
        build: `libmgba CMake Release (${library}), harness cc -O2`,
      };
    },
  },
  vbam: vbam("-Os"),
  "vbam-o3": vbam("-O3"),
  evenfall: {
    title: "Evenfall Rust, release-lto",
    boots: ["hle", "skip-bios"],
    build() {
      // rustup's cargo without rustc wrappers: output caches that compile
      // LTO profiles incrementally produce slower code.
      const env = { ...process.env };
      delete env.RUSTC_WRAPPER;
      delete env.CARGO_BUILD_RUSTC_WRAPPER;
      run(
        join(homedir(), ".cargo/bin/cargo"),
        [
          "build",
          "--locked",
          "--profile",
          "release-lto",
          "-p",
          "evenfall-bench",
          "--bin",
          "evenfall-harness",
        ],
        { cwd: ROOT, env },
      );
      return {
        binary: join(ROOT, "target/release-lto/evenfall-harness"),
        source: git(["rev-parse", "HEAD"]),
        build: "cargo release-lto",
      };
    },
  },
  // Evenfall's predecessor, kept for internal comparisons; not a default.
  zigba: {
    title: "pinned ZiGBA, ReleaseFast",
    boots: ["hle", "skip-bios"],
    build() {
      const ref = checkRef("zigba", "add59e5957b75f29d084813336ee20e300fc0b6e");
      const binary = join(ROOT, "target/bench/zigba-harness");
      run(
        "zig",
        [
          "build-exe",
          "-O",
          "ReleaseFast",
          "--cache-dir",
          join(ROOT, "target/bench/zig-cache"),
          "--global-cache-dir",
          join(ROOT, "target/bench/zig-global-cache"),
          "--dep",
          "zigba",
          `-Mmain=${join(ref.dir, "tools/emu-bench/harness/zigba-harness.zig")}`,
          `-Mzigba=${join(ref.dir, "src/root.zig")}`,
          `-femit-bin=${binary}`,
        ],
        { cwd: ROOT },
      );
      return {
        binary,
        source: ref.source,
        trackedChanges: ref.trackedChanges,
        build: "zig ReleaseFast",
      };
    },
  },
};

// Adapters a run uses without --emulators; the first is the default baseline.
export const DEFAULT_EMULATORS = ["mgba", "vbam", "evenfall"];

export const ROMS_DIR = join(ROOT, "tools/bench/roms");
export const WORKLOADS = [
  {
    name: "idle",
    rom: "idle",
    frames: 3600,
    stresses: "halt, scheduler and an empty mode 0 frame",
  },
  {
    name: "arm-mixer",
    rom: "arm_mixer",
    frames: 600,
    stresses: "ARM code in IWRAM (sound mixer loop)",
  },
  {
    name: "thumb-logic",
    rom: "thumb_logic",
    frames: 600,
    stresses: "Thumb code in ROM with wait states (game logic)",
  },
  {
    name: "tiles",
    rom: "tiles",
    frames: 600,
    stresses: "mode 0 compositor: 4 BGs, 128 sprites, blending, window",
  },
  {
    name: "affine",
    rom: "affine",
    frames: 600,
    stresses: "affine BGs and sprites, HBlank DMA and HBlank IRQs",
  },
  {
    name: "bitmap-dma",
    rom: "bitmap_dma",
    frames: 600,
    stresses: "mode 4 page flipping, many small DMAs",
  },
  {
    name: "audio",
    rom: "audio",
    frames: 600,
    stresses: "Direct Sound FIFOs by timer DMA, all PSG channels",
  },
];

export const buildRoms = (outDir) => {
  run("make", [
    "-s",
    "-C",
    ROMS_DIR,
    `BUILD=${outDir}`,
    `-j${Math.min(cpus().length, 4)}`,
  ]);
  return (workload) => join(outDir, `${workload.rom}.gba`);
};
