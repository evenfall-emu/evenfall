#!/usr/bin/env node
// Light cross-emulator benchmark on purpose-built ROMs. See README.md.

import { spawnSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import { cpus, loadavg, release } from "node:os";
import { dirname, join, resolve } from "node:path";
import { parseArgs } from "node:util";
import { detectTaskset, pickCpu, ROOT, sha256 } from "./common.mjs";
import { buildRoms, EMULATORS, git, WORKLOADS } from "./emulators.mjs";
import {
  formatTable,
  parseObservation,
  roundOrder,
  summarize,
} from "./measure.mjs";

const USAGE = `usage: node tools/bench/run.mjs [options]

  --emulators LIST     comma-separated adapters (default: ${Object.keys(EMULATORS).join(",")})
  --harness NAME=PATH  use a prebuilt harness binary for NAME instead of
                       building it; a NAME outside the registry adds an
                       emulator (repeatable)
  --workloads LIST     comma-separated workloads (default: all)
  --boot MODE          skip-bios (default) or hle
  --bios PATH          BIOS image for skip-bios (default: refs/cult-gba-bios/bios.bin)
  --baseline NAME      emulator the time ratios divide by (default: first listed)
  --rounds N           measured rounds (default 5)
  --warmups N          unmeasured rounds before them (default 1)
  --frames N           override every workload's frame count
  --cpu auto|N         CPU both pinned runs use (default auto)
  --work-dir DIR       parent of the run directory (default target/bench/runs)
  --list               print workloads and emulators, then exit
`;

const fail = (message) => {
  console.error(`emu-bench: ${message}`);
  process.exit(2);
};

const positive = (value, label, allowZero = false) => {
  const n = Number(value);
  if (!Number.isSafeInteger(n) || n < (allowZero ? 0 : 1))
    fail(
      `${label} must be a ${allowZero ? "non-negative" : "positive"} integer`,
    );
  return n;
};

const { values: opts } = parseArgs({
  options: {
    emulators: { type: "string" },
    harness: { type: "string", multiple: true },
    workloads: { type: "string" },
    boot: { type: "string", default: "skip-bios" },
    bios: { type: "string" },
    baseline: { type: "string" },
    rounds: { type: "string", default: "5" },
    warmups: { type: "string", default: "1" },
    frames: { type: "string" },
    cpu: { type: "string", default: "auto" },
    "work-dir": { type: "string" },
    list: { type: "boolean" },
    help: { type: "boolean" },
  },
});
if (opts.help) {
  console.log(USAGE);
  process.exit(0);
}
if (opts.list) {
  for (const w of WORKLOADS)
    console.log(
      `workload  ${w.name.padEnd(12)} ${String(w.frames).padStart(5)} frames  ${w.stresses}`,
    );
  for (const [name, e] of Object.entries(EMULATORS))
    console.log(
      `emulator  ${name.padEnd(12)} ${e.title}; boots: ${e.boots.join(", ")}`,
    );
  process.exit(0);
}

// Plan: everything is decided before the first run.
const prebuilt = Object.fromEntries(
  (opts.harness ?? []).map((entry) => {
    const [name, path] = entry.split(/=(.*)/s);
    if (!name || !path) fail(`--harness expects NAME=PATH, got ${entry}`);
    if (!existsSync(path)) fail(`harness not found: ${path}`);
    return [name, resolve(path)];
  }),
);
const emulators = opts.emulators
  ? opts.emulators.split(",").filter(Boolean)
  : [
      ...Object.keys(EMULATORS),
      ...Object.keys(prebuilt).filter((name) => !EMULATORS[name]),
    ];
for (const name of emulators)
  if (!EMULATORS[name] && !prebuilt[name])
    fail(`unknown emulator ${name}; pass --harness ${name}=PATH`);
if (new Set(emulators).size !== emulators.length || emulators.length === 0)
  fail("--emulators must list distinct emulators");
const baseline = opts.baseline ?? emulators[0];
if (!emulators.includes(baseline))
  fail(`baseline ${baseline} is not among the emulators`);

const boot = opts.boot;
if (boot !== "skip-bios" && boot !== "hle")
  fail("--boot must be skip-bios or hle");
if (boot === "hle" && opts.bios) fail("--bios applies to --boot skip-bios");
for (const name of emulators) {
  if (EMULATORS[name] && !EMULATORS[name].boots.includes(boot)) {
    fail(
      `${name} does not support --boot ${boot} (supports ${EMULATORS[name].boots.join(", ")})`,
    );
  }
}
let bios = null;
if (boot === "skip-bios") {
  const path = resolve(ROOT, opts.bios ?? "refs/cult-gba-bios/bios.bin");
  if (!existsSync(path))
    fail(
      `BIOS image not found: ${path}${opts.bios ? "" : "; run tools/setup-refs.sh cult-gba-bios"}`,
    );
  const bytes = readFileSync(path);
  if (bytes.length !== 16384)
    fail(`BIOS image ${path} is ${bytes.length} bytes, expected 16384`);
  bios = {
    path,
    sha256: sha256(bytes),
    source: opts.bios
      ? "user-supplied"
      : `refs/cult-gba-bios ${git(["rev-parse", "HEAD"], join(ROOT, "refs/cult-gba-bios"))}`,
  };
}

const selected = opts.workloads
  ? opts.workloads.split(",").filter(Boolean)
  : WORKLOADS.map((w) => w.name);
const frames =
  opts.frames === undefined ? null : positive(opts.frames, "--frames");
const workloads = selected.map((name) => {
  const workload = WORKLOADS.find((w) => w.name === name);
  if (!workload) fail(`unknown workload ${name}; see --list`);
  return { ...workload, frames: frames ?? workload.frames };
});
const rounds = positive(opts.rounds, "--rounds");
const warmups = positive(opts.warmups, "--warmups", true);

if (!detectTaskset()) fail("taskset is required to pin runs to one CPU");

// Same host-wide lane as tools/mgba-bench, so benchmarks never overlap.
if (!process.env.ZIGBA_LANE_HELD) {
  const lock =
    process.env.ZIGBA_LANE_LOCK ??
    `/tmp/zigba-benchmark-${process.getuid()}.lock`;
  mkdirSync(dirname(lock), { recursive: true });
  const child = spawnSync(
    "flock",
    [lock, process.execPath, ...process.argv.slice(1)],
    {
      stdio: "inherit",
      env: { ...process.env, ZIGBA_LANE_HELD: "1" },
    },
  );
  if (child.error || child.status === null)
    fail(`benchmark lane failed: ${child.error?.message ?? child.signal}`);
  process.exit(child.status);
}

let cpuId, cpuSelection;
try {
  ({ cpuId, note: cpuSelection } = await pickCpu(opts.cpu));
} catch (error) {
  fail(error.message);
}
const parent = resolve(opts["work-dir"] ?? join(ROOT, "target/bench/runs"));
mkdirSync(parent, { recursive: true });
const workDir = mkdtempSync(
  join(parent, `${new Date().toISOString().replace(/[:.]/g, "-")}-`),
);
const reportPath = join(workDir, "result.json");

// Builds happen before any measurement. Library and object caches persist
// between runs; each run's harness binaries and ROMs are its own copies.
const buildDir = join(workDir, "build");
mkdirSync(buildDir);
const romPath = buildRoms(join(buildDir, "roms"));
const harnesses = {};
for (const name of emulators) {
  console.log(
    `emu-bench: ${prebuilt[name] ? "using prebuilt" : "building"} ${name} harness`,
  );
  let built;
  try {
    built = prebuilt[name]
      ? { binary: prebuilt[name], source: "prebuilt (caller-supplied)" }
      : EMULATORS[name].build(buildDir);
  } catch (error) {
    fail(error.message);
  }
  harnesses[name] = {
    ...built,
    title: EMULATORS[name]?.title ?? name,
    binarySha256: sha256(readFileSync(built.binary)),
  };
}

const report = {
  schema: 1,
  status: "running",
  plan: {
    argv: process.argv.slice(2),
    startedAt: new Date().toISOString(),
    cpu: cpuId,
    cpuSelection,
    boot,
    bios,
    rounds,
    warmups,
    baseline,
    emulators,
    order:
      "emulator order rotates each round and workload; each observation is a fresh process",
    metric: "loop wall seconds for FRAMES * 280896 guest cycles from power-on",
    workloads: workloads.map((w) => ({
      ...w,
      romSha256: sha256(readFileSync(romPath(w))),
    })),
    harnesses,
    source: {
      revision: git(["rev-parse", "HEAD"]),
      dirty: Boolean(
        git(["status", "--porcelain", "--", "systems/gba", "tools/bench"]),
      ),
    },
    runnerSha256: sha256(readFileSync(new URL(import.meta.url))),
  },
  host: {
    cpu: cpus()[cpuId]?.model ?? null,
    kernel: release(),
    node: process.version,
    loadBefore: loadavg(),
  },
  observations: [],
};
const save = () =>
  writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`);
save();

console.log(
  `emu-bench: ${workloads.length} workloads x ${emulators.length} emulators, ${rounds} rounds + ${warmups} warmup, CPU ${cpuId} (${cpuSelection}), boot ${boot}${bios ? ` (${bios.source})` : ""}`,
);
for (let round = -warmups; round < rounds; round++) {
  for (const [index, workload] of workloads.entries()) {
    for (const emulator of roundOrder(emulators, round + warmups, index)) {
      const args = [
        "-c",
        String(cpuId),
        harnesses[emulator].binary,
        romPath(workload),
        String(workload.frames),
      ];
      if (bios) args.push(`--bios=${bios.path}`);
      const result = spawnSync("taskset", args, {
        encoding: "utf8",
        maxBuffer: 16 << 20,
      });
      const output = `${result.stdout ?? ""}${result.stderr ?? ""}`;
      let line;
      try {
        if (result.error) throw result.error;
        if (result.status !== 0)
          throw new Error(`exit ${result.status ?? result.signal}`);
        line = parseObservation(output, { frames: workload.frames, boot });
      } catch (error) {
        report.status = "failed";
        report.failure = {
          emulator,
          workload: workload.name,
          round,
          message: error.message,
          output,
        };
        save();
        fail(
          `${emulator} on ${workload.name}: ${error.message}\n${output.trim()}\nreport: ${reportPath}`,
        );
      }
      report.observations.push({
        round: round < 0 ? round + warmups : round,
        warmup: round < 0,
        workload: workload.name,
        emulator,
        seconds: line.seconds,
        cycle: line.cycle,
        framebufferHash: line.framebufferHash,
        line,
      });
      save();
    }
  }
  console.log(
    `emu-bench: ${round < 0 ? `warmup ${round + warmups + 1}/${warmups}` : `round ${round + 1}/${rounds}`} done`,
  );
}

report.summary = summarize({
  observations: report.observations,
  workloads,
  emulators,
  baseline,
});
report.status = "complete";
report.host.loadAfter = loadavg();
save();

console.log(`\n${formatTable(report.summary, emulators)}\n`);
console.log(
  `Frames per second of guest time (median of ${rounds} rounds) and loop time relative to ${baseline}`,
);
console.log(
  '(geometric mean of per-round ratios; lower is faster). "image" compares the final frames.',
);
const unstable = report.summary.rows.flatMap((row) =>
  emulators
    .filter((e) => !row.results[e].deterministic)
    .map((e) => `${e}/${row.workload}`),
);
if (unstable.length)
  console.log(
    `warning: final image changed between runs for ${unstable.join(", ")}`,
  );
console.log(`report: ${reportPath}`);
