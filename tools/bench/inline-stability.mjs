#!/usr/bin/env node
// Checks that the hot paths' inlining does not depend on LLVM's inline
// threshold: builds the native release-lto harness and the simd128 web
// module at thresholds 200, 250 (LLVM's -O3 default) and 300, and compares
// the sizes of the out-of-line functions matching --filter. A function whose
// size or existence changes has an inline decision near its threshold, which
// an unrelated edit can flip; pin it with #[inline(always)] or
// #[inline(never)]. Exits 1 when anything differs.
import { execFileSync, spawn } from "node:child_process";
import { mkdirSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { parseArgs } from "node:util";
import { ROOT } from "./common.mjs";

const HOT =
  "cpu::engine|bus::kernel|Bus>::(elapse|advance_step|advance_eager|advance_slow|flush_device_batch|dma_bulk|run_dma|write_iwram|write_ewram)$|Apu>::(elapse_mixed|flush_square_noise)$|compose|render";
const { values: opts } = parseArgs({
  options: {
    filter: { type: "string", default: HOT },
    targets: { type: "string", default: "native,wasm" },
    help: { type: "boolean", short: "h" },
  },
});
if (opts.help) {
  console.log(
    "usage: node tools/bench/inline-stability.mjs [--filter REGEX] [--targets native,wasm]",
  );
  process.exit(0);
}

const THRESHOLDS = [200, 250, 300];
// Output caches that compile LTO profiles incrementally change the code, so
// bypass wrappers as build-times.sh does.
const cargo = join(homedir(), ".cargo/bin/cargo");
const env = { ...process.env };
delete env.RUSTC_WRAPPER;
delete env.CARGO_BUILD_RUSTC_WRAPPER;
const LINK =
  "-C link-arg=--initial-memory=16777216 -C link-arg=--max-memory=536870912 -C link-arg=-zstack-size=8388608 -C link-arg=--export-memory";

const TARGETS = {
  native: {
    args: [
      "--profile",
      "release-lto",
      "-p",
      "evenfall-bench",
      "--bin",
      "evenfall-harness",
    ],
    flagsVar: "RUSTFLAGS",
    extraFlags: "",
    output: "release-lto/evenfall-harness",
    functions: nativeFunctions,
  },
  wasm: {
    args: [
      "--config",
      "profile.wasm-release.strip=false",
      "--target",
      "wasm32-unknown-unknown",
      "--profile",
      "wasm-release",
      "-p",
      "evenfall-gba-web",
    ],
    flagsVar: "CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS",
    extraFlags: `${LINK} -C target-feature=+simd128`,
    output: "wasm32-unknown-unknown/wasm-release/evenfall_gba_web.wasm",
    functions: wasmFunctions,
  },
};

function build(kind, threshold) {
  const target = TARGETS[kind];
  const dir = join(ROOT, "target/inline-stability", `${kind}-${threshold}`);
  mkdirSync(dir, { recursive: true });
  const flags =
    `${target.extraFlags} -C llvm-args=-inline-threshold=${threshold}`.trim();
  return new Promise((resolve, reject) => {
    const child = spawn(
      cargo,
      ["build", "-q", "--locked", ...target.args, "--target-dir", dir],
      {
        cwd: ROOT,
        env: { ...env, [target.flagsVar]: flags },
        stdio: ["ignore", "inherit", "inherit"],
      },
    );
    child.on("exit", (code) =>
      code === 0
        ? resolve(join(dir, target.output))
        : reject(new Error(`${kind} build at ${threshold} failed`)),
    );
  });
}

const demangle = (names) =>
  execFileSync("c++filt", { input: names.join("\n"), maxBuffer: 1 << 28 })
    .toString()
    .split("\n")
    .map((n) =>
      n.replace(/\[[0-9a-f]{16}\]/g, "").replace(/::h[0-9a-f]{16}$/, ""),
    );

function nativeFunctions(path) {
  const table = new Map();
  const out = execFileSync("nm", ["-S", "-C", path], {
    maxBuffer: 1 << 28,
  }).toString();
  for (const line of out.split("\n")) {
    const parts = line.split(" ");
    if (parts.length < 4 || !"tT".includes(parts[2])) continue;
    const name = parts
      .slice(3)
      .join(" ")
      .replace(/::h[0-9a-f]{16}$/, "");
    table.set(name, (table.get(name) ?? 0) + parseInt(parts[1], 16));
  }
  return table;
}

// Body sizes from the code section, named by the "name" custom section.
function wasmFunctions(path) {
  const b = readFileSync(path);
  let p = 8;
  const leb = () => {
    let r = 0,
      s = 0,
      x;
    do {
      x = b[p++];
      r += (x & 0x7f) * 2 ** s;
      s += 7;
    } while (x & 0x80);
    return r;
  };
  let imports = 0;
  const sizes = [],
    names = new Map();
  while (p < b.length) {
    const id = b[p++],
      end = leb() + p;
    if (id === 2) {
      for (let n = leb(); n > 0; n--) {
        p += leb();
        p += leb();
        if (b[p++] === 0) {
          leb();
          imports++;
        } else throw new Error("unexpected non-function import");
      }
    } else if (id === 10) {
      for (let n = leb(); n > 0; n--) {
        const s = leb();
        sizes.push(s);
        p += s;
      }
    } else if (id === 0) {
      const length = leb();
      if (b.toString("utf8", p, p + length) === "name") {
        p += length;
        while (p < end) {
          const sub = b[p++],
            subEnd = leb() + p;
          if (sub === 1) {
            for (let n = leb(); n > 0; n--) {
              const index = leb(),
                l = leb();
              names.set(index, b.toString("utf8", p, p + l));
              p += l;
            }
          }
          p = subEnd;
        }
      }
    }
    p = end;
  }
  const demangled = demangle(
    sizes.map((_, i) => names.get(i + imports) ?? `#${i}`),
  );
  const table = new Map();
  sizes.forEach((s, i) =>
    table.set(demangled[i], (table.get(demangled[i]) ?? 0) + s),
  );
  return table;
}

const filter = new RegExp(opts.filter);
let unstable = 0;
for (const kind of opts.targets.split(",")) {
  const paths = await Promise.all(THRESHOLDS.map((t) => build(kind, t)));
  const tables = paths.map((path) => TARGETS[kind].functions(path));
  const base = tables[1];
  for (const i of [0, 2]) {
    const other = tables[i];
    const names = new Set(
      [...base.keys(), ...other.keys()].filter((n) => filter.test(n)),
    );
    const changed = [...names]
      .filter((n) => base.get(n) !== other.get(n))
      .sort();
    console.log(
      `${kind}: threshold 250 -> ${THRESHOLDS[i]}: ${changed.length} of ${names.size} functions change`,
    );
    for (const n of changed)
      console.log(
        `  ${base.get(n) ?? "inlined"} -> ${other.get(n) ?? "inlined"}  ${n}`,
      );
    unstable += changed.length;
  }
}
process.exit(unstable ? 1 : 0);
