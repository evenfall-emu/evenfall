#!/usr/bin/env node
import { readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { WasmCore } from "../core.js";
import {
  jsonSafe,
  sha1,
  sha256,
  validateScenario,
} from "../devtools/scenario.js";
import { replayFailureBundle } from "../devtools/capture.js";

let core;
try {
  const args = process.argv.slice(2);
  if (args.length < 2 || args.length > 3) {
    throw new Error(
      "Usage: node web/gba/scripts/replay-capture.mjs RESULT_OR_BUNDLE_JSON MANIFEST_JSON [WASM_PATH]",
    );
  }
  const stored = JSON.parse(await readFile(resolve(args[0]), "utf8"));
  const bundle = stored.capture ?? stored;
  const manifestPath = resolve(args[1]);
  const manifest = validateScenario(
    JSON.parse(await readFile(manifestPath, "utf8")),
  );
  const assets = {};
  for (const [name, descriptor] of Object.entries(manifest.assets)) {
    if (!descriptor.path) throw new Error(`Asset ${name} requires path in CLI`);
    assets[name] = await readFile(
      resolve(dirname(manifestPath), descriptor.path),
    );
    if ((await sha1(assets[name])) !== descriptor.sha1)
      throw new Error(`${name} SHA-1 mismatch`);
  }
  const wasm = await readFile(
    args[2]
      ? resolve(args[2])
      : new URL("../evenfall-gba.wasm", import.meta.url),
  );
  const identity = {
    manifest,
    config: manifest.config,
    biosSha1: manifest.assets.bios?.sha1,
    build: { wasmSha256: await sha256(wasm) },
  };
  core = await WasmCore.create(wasm);
  core.load(assets.rom, {
    zipped: false,
    bios: assets.bios,
    save: assets.save,
    saveType: manifest.config.saveType,
    rtc: manifest.config.rtc,
    rtcEpoch: BigInt(manifest.config.rtcEpoch),
    sampleRate: manifest.config.sampleRate,
  });
  const replay = replayFailureBundle(core, bundle, { identity });
  console.log(
    JSON.stringify(
      jsonSafe({
        status: "reproduced",
        reason: bundle.reason,
        inspection: replay.inspection,
        operationsReplayed: replay.results.length,
        audioReplay: bundle.history.audioReplay,
      }),
    ),
  );
} catch (error) {
  console.error(
    JSON.stringify({ status: "error", error: error.message ?? String(error) }),
  );
  process.exitCode = 2;
} finally {
  core?.dispose();
}
