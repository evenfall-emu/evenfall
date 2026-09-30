#!/usr/bin/env node
// WebAssembly counterpart of run.mjs: times modules that export the
// @evenfall/gba ABI on the benchmark ROMs in Node.
// Each observation is a fresh Node process pinned to one CPU; module order
// rotates per round and workload. Ratios are geometric means of matched-round
// loop-time ratios against the first module. The final framebuffer hash uses
// the native harness's FNV-1a so wasm and native images can be compared.
import { spawnSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync, writeSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import { ROOT } from "./common.mjs";
import { WORKLOADS, buildRoms } from "./emulators.mjs";

const FRAME_CYCLES = 280896n;

if (process.argv[2] === "--child") {
  const [module, rom, frames, warm] = process.argv.slice(3);
  writeSync(1, JSON.stringify(observe(module, rom, +frames, +warm)));
  process.exit(0);
}

/** Run `frames` frames with HLE boot; time the frames after `warm`. */
function observe(modulePath, romPath, frames, warm) {
  const module = new WebAssembly.Module(readFileSync(modulePath));
  const w = new WebAssembly.Instance(module, {}).exports;
  const check = (result) => {
    if (result < 0) {
      const text = new Uint8Array(
        w.memory.buffer,
        w.ef_error_ptr(),
        w.ef_error_len(),
      );
      throw new Error(new TextDecoder().decode(text));
    }
    return result;
  };
  const rom = readFileSync(romPath);
  check(w.ef_prepare(rom.length));
  new Uint8Array(w.memory.buffer, w.ef_input_ptr(), rom.length).set(rom);
  check(w.ef_load(rom.length, 0, 0, 0, 1, 0, 0n, 48000));
  check(w.ef_audio_output(0));
  const start = w.ef_cycle();
  const run = (frame) => {
    const deadline = start + BigInt(frame + 1) * FRAME_CYCLES;
    check(w.ef_run_until(deadline, 0, deadline));
  };
  for (let frame = 0; frame < warm; frame++) run(frame);
  const began = performance.now();
  for (let frame = warm; frame < frames; frame++) run(frame);
  const seconds = (performance.now() - began) / 1000;
  const pixels = new Uint16Array(w.memory.buffer, w.ef_frame_ptr(), 240 * 160);
  let hash = 0xcbf29ce484222325n;
  for (const pixel of pixels) {
    for (const byte of [pixel & 0xff, (pixel >> 8) & 0x7f]) {
      hash = ((hash ^ BigInt(byte)) * 0x100000001b3n) & 0xffffffffffffffffn;
    }
  }
  return {
    seconds,
    frames: frames - warm,
    hash: hash.toString(16).padStart(16, "0"),
  };
}

const { values } = parseArgs({
  options: {
    module: { type: "string", multiple: true },
    workloads: { type: "string" },
    rounds: { type: "string", default: "5" },
    warmups: { type: "string", default: "1" },
    frames: { type: "string" },
    warm: { type: "string", default: "60" },
    cpu: { type: "string", default: "3" },
  },
});
const modules = (values.module ?? []).map((spec) => {
  const at = spec.indexOf("=");
  if (at < 1) throw new Error(`--module NAME=PATH, got ${spec}`);
  return { name: spec.slice(0, at), path: spec.slice(at + 1) };
});
if (!modules.length) {
  console.error(
    "usage: node tools/bench/wasm.mjs --module NAME=PATH... [--workloads LIST]\n" +
      "  [--rounds N] [--warmups N] [--frames N] [--warm N] [--cpu N]",
  );
  process.exit(2);
}
const selected = values.workloads?.split(",");
const workloads = WORKLOADS.filter(
  (w) => !selected || selected.includes(w.name),
);
const outDir = join(
  ROOT,
  "target/bench/wasm-runs",
  new Date().toISOString().replace(/[:.]/g, "-"),
);
mkdirSync(outDir, { recursive: true });
const romOf = buildRoms(join(outDir, "roms"));
const rounds = +values.rounds;
const warmups = +values.warmups;
const warm = +values.warm;
const self = fileURLToPath(import.meta.url);
const samples = {};

for (let round = 0; round < warmups + rounds; round++) {
  workloads.forEach((workload, index) => {
    const frames = values.frames ? +values.frames : workload.frames;
    const shift = (round + index) % modules.length;
    for (const module of [
      ...modules.slice(shift),
      ...modules.slice(0, shift),
    ]) {
      const child = spawnSync(
        "taskset",
        [
          "-c",
          values.cpu,
          process.execPath,
          self,
          "--child",
          module.path,
          romOf(workload),
          `${frames}`,
          `${warm}`,
        ],
        { encoding: "utf8", timeout: 600_000 },
      );
      if (child.status !== 0) {
        throw new Error(
          `${module.name} ${workload.name}: ${child.error ?? child.stderr}`,
        );
      }
      if (round < warmups) continue;
      ((samples[workload.name] ??= {})[module.name] ??= []).push(
        JSON.parse(child.stdout),
      );
    }
  });
  console.error(
    `wasm-bench: ${round < warmups ? "warmup" : "round"} ${round + 1} done`,
  );
}

const geomean = (values) =>
  Math.exp(values.reduce((sum, v) => sum + Math.log(v), 0) / values.length);
const median = (values) =>
  values.toSorted((a, b) => a - b)[Math.floor(values.length / 2)];
const ratios = Object.fromEntries(modules.map((m) => [m.name, []]));
const header = ["workload", ...modules.map((m) => m.name), "image"];
const rows = [header];
for (const workload of workloads) {
  const byModule = samples[workload.name];
  const base = byModule[modules[0].name];
  const cells = modules.map(({ name }) => {
    const runs = byModule[name];
    const ratio = geomean(
      runs.map((run, round) => run.seconds / base[round].seconds),
    );
    ratios[name].push(ratio);
    const fps = median(runs.map((run) => run.frames / run.seconds));
    return `${fps.toFixed(0)} fps ${ratio.toFixed(2)}`;
  });
  const hashes = new Set(modules.map(({ name }) => byModule[name][0].hash));
  rows.push([workload.name, ...cells, hashes.size === 1 ? "agree" : "DIFFER"]);
}
rows.push([
  "geomean",
  ...modules.map(({ name }) => geomean(ratios[name]).toFixed(2)),
  "",
]);
const widths = header.map((_, column) =>
  Math.max(...rows.map((row) => row[column].length)),
);
for (const row of rows)
  console.log(
    row.map((cell, column) => cell.padStart(widths[column])).join("  "),
  );
writeFileSync(
  join(outDir, "result.json"),
  JSON.stringify(
    { node: process.version, cpu: values.cpu, warm, modules, samples },
    null,
    2,
  ),
);
console.log(`report: ${join(outDir, "result.json")}`);
