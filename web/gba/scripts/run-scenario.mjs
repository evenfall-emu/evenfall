#!/usr/bin/env node
import { readFile, writeFile, mkdir } from "node:fs/promises";
import { resolve, dirname, join } from "node:path";
import { WasmCore } from "../core.js";
import {
  runScenario,
  sha1,
  sha256,
  validateScenario,
} from "../devtools/scenario.js";

const args = process.argv.slice(2);
if (args[0] === "--example") {
  const dir = resolve(args[1] ?? "scenario-example");
  await mkdir(dir, { recursive: true });
  // Independent ARM fixture: MOV r0,#42; B .
  const rom = new Uint8Array([42, 0, 160, 227, 254, 255, 255, 234]);
  await writeFile(join(dir, "loop.gba"), rom);
  const manifest = {
    version: 2,
    name: "ARM constant loop",
    assets: { rom: { path: "loop.gba", sha1: await sha1(rom) } },
    config: {
      boot: "hle",
      saveType: "none",
      rtc: false,
      rtcEpoch: "0",
      sampleRate: 48000,
    },
    limits: {
      maxCycles: "280896",
      maxRuns: 100,
      chunkCycles: "16384",
      maxAudioBytes: 1048576,
    },
    inputs: [
      { at: "100", keys: 1 },
      { at: "200", keys: 0 },
    ],
    checkpoints: [
      {
        name: "loop",
        at: "280896",
        expect: { cpu: { registers: { 0: 42 } } },
        memory: [
          {
            name: "instructions",
            address: 134217728,
            length: 8,
            sha256: await sha256(rom),
          },
        ],
      },
    ],
  };
  await writeFile(
    join(dir, "scenario.json"),
    JSON.stringify(manifest, null, 2) + "\n",
  );
  console.log(join(dir, "scenario.json"));
} else {
  let core;
  try {
    if (!args[0] || args.length > 3)
      throw new Error(
        "Usage: node web/gba/scripts/run-scenario.mjs MANIFEST [OUTPUT_DIR] [WASM_PATH]\n       node web/gba/scripts/run-scenario.mjs --example [DIRECTORY]",
      );
    const path = resolve(args[0]);
    const manifest = validateScenario(JSON.parse(await readFile(path, "utf8")));
    const assets = {};
    for (const [name, descriptor] of Object.entries(manifest.assets)) {
      if (!descriptor.path)
        throw new Error(`Asset ${name} requires path in CLI`);
      assets[name] = await readFile(resolve(dirname(path), descriptor.path));
    }
    const out = resolve(args[1] ?? `scenario-result-${Date.now()}`);
    // Refuse an existing output directory to preserve earlier evidence.
    await mkdir(out);
    const wasm = await readFile(
      args[2]
        ? resolve(args[2])
        : new URL("../evenfall-gba.wasm", import.meta.url),
    );
    core = await WasmCore.create(wasm);
    const result = await runScenario(core, manifest, assets, {
      build: { wasmSha256: await sha256(wasm) },
      onArtifact: (name, bytes) => writeFile(join(out, name), bytes),
    });
    await writeFile(
      join(out, "result.json"),
      JSON.stringify(result, null, 2) + "\n",
    );
    console.log(
      JSON.stringify({
        status: result.status,
        result: join(out, "result.json"),
        failures: result.failures,
      }),
    );
    if (result.status !== "passed") process.exitCode = 1;
  } catch (error) {
    console.error(JSON.stringify({ status: "error", error: error.message }));
    process.exitCode = 2;
  } finally {
    core?.dispose();
  }
}
