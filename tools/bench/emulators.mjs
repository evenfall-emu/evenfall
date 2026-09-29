// Native adapters for the shared emu-bench harness contract.
import { spawnSync } from "node:child_process";
import { cpus } from "node:os";
import { join } from "node:path";
import { ROOT, run } from "./common.mjs";

export const git = (args, cwd = ROOT) => {
  const result = spawnSync("git", args, { cwd, encoding: "utf8" });
  return result.status === 0 ? result.stdout.trim() : null;
};

export const EMULATORS = {
  zigba: {
    title: "pinned ZiGBA, ReleaseFast",
    boots: ["hle", "skip-bios"],
    build() {
      const root = join(ROOT, "refs/zigba");
      const source = git(["rev-parse", "HEAD"], root);
      const pin = "add59e5957b75f29d084813336ee20e300fc0b6e";
      if (source !== pin) throw new Error(`refs/zigba is ${source ?? "missing"}, expected ${pin}`);
      const binary = join(ROOT, "target/bench/zigba-harness");
      run("zig", [
        "build-exe", "-O", "ReleaseFast",
        "--cache-dir", join(ROOT, "target/bench/zig-cache"),
        "--global-cache-dir", join(ROOT, "target/bench/zig-global-cache"),
        "--dep", "zigba",
        `-Mmain=${join(root, "tools/emu-bench/harness/zigba-harness.zig")}`,
        `-Mzigba=${join(root, "src/root.zig")}`,
        `-femit-bin=${binary}`,
      ], { cwd: ROOT });
      return { binary, source, trackedChanges: git(["status", "--porcelain", "--untracked-files=no"], root) || null, build: "zig ReleaseFast" };
    },
  },
  evenfall: {
    title: "Evenfall Rust, release",
    boots: ["hle", "skip-bios"],
    build() {
      run("cargo", ["build", "--release", "-p", "evenfall-bench", "--bin", "evenfall-harness"], { cwd: ROOT });
      return { binary: join(ROOT, "target/release/evenfall-harness"), source: git(["rev-parse", "HEAD"]), build: "cargo release" };
    },
  },
};

export const ROMS_DIR = join(ROOT, "tools/bench/roms");
export const WORKLOADS = [
  { name: "idle", rom: "idle", frames: 3600, stresses: "halt, scheduler and an empty mode 0 frame" },
  { name: "arm-mixer", rom: "arm_mixer", frames: 600, stresses: "ARM code in IWRAM (sound mixer loop)" },
  { name: "thumb-logic", rom: "thumb_logic", frames: 600, stresses: "Thumb code in ROM with wait states (game logic)" },
  { name: "tiles", rom: "tiles", frames: 600, stresses: "mode 0 compositor: 4 BGs, 128 sprites, blending, window" },
  { name: "affine", rom: "affine", frames: 600, stresses: "affine BGs and sprites, HBlank DMA and HBlank IRQs" },
  { name: "bitmap-dma", rom: "bitmap_dma", frames: 600, stresses: "mode 4 page flipping, many small DMAs" },
  { name: "audio", rom: "audio", frames: 600, stresses: "Direct Sound FIFOs by timer DMA, all PSG channels" },
];

export const buildRoms = (outDir) => {
  run("make", ["-s", "-C", ROMS_DIR, `BUILD=${outDir}`, `-j${Math.min(cpus().length, 4)}`]);
  return (workload) => join(outDir, `${workload.rom}.gba`);
};
