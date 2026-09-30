/**
 * Client-side half of the catalog install flow, run against locally supplied
 * media. The catalog normalizer writes one record shape for each collection.
 * Every patch is applied with the player's own applier and checked against
 * its recorded output hash; a sample is booted in the WASM core.
 *
 * Skips unless the media is present; see docs/testing.md#local-base-roms.
 */
import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { basename, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { crc32 } from "node:zlib";
import { afterAll, describe, expect, it, vi } from "vite-plus/test";
import { CYCLES_PER_FRAME, WasmCore } from "@evenfall/gba/core";
import { patchRelease } from "../src/pages/library/install";
import type { CatalogVersion } from "../src/pages/library/catalog";
import seed from "../../server/src/romhacks.json";

const sha1 = (bytes: Uint8Array) =>
  createHash("sha1").update(bytes).digest("hex");
const crc = (bytes: Uint8Array) =>
  crc32(Buffer.from(bytes)).toString(16).padStart(8, "0");

const basesDir = resolve(
  process.env.ZIGBA_BASES_DIR ??
    fileURLToPath(new URL("../../bases", import.meta.url)),
);
const packs = process.env.ZIGBA_ROMPACK_DIR
  ? [resolve(process.env.ZIGBA_ROMPACK_DIR)]
  : [
      "../../.amp/in/rom-hacks",
      "../../.amp/in/rom-hacks-golden-sun-fire-emblem/golden-sun",
      "../../.amp/in/rom-hacks-golden-sun-fire-emblem/fire-emblem",
    ].map((path) => fileURLToPath(new URL(path, import.meta.url)));
const wasmPath = new URL("../../web/zigba.wasm", import.meta.url);

/** A release record usable here: a patch, its base dump and an output hash. */
type Record = {
  name?: string;
  family?: string;
  collection?: string;
  baseFile: string;
  baseSha1: string;
  patch: string;
  patchSha1: string;
  outputSha1: string;
  outputCrc32?: string | null;
};

function records(): { usable: Record[]; skipped: string[] } {
  const usable: Record[] = [];
  const skipped: string[] = [];
  for (const pack of packs) {
    const path = join(pack, "normalized.json");
    if (!existsSync(path)) continue;
    const parsed = JSON.parse(readFileSync(path, "utf8"));
    for (const entry of parsed.entries as unknown[]) {
      if (typeof entry !== "object" || entry === null) continue;
      const candidate = entry as Partial<Record>;
      if (!candidate.patch) continue;
      const complete =
        typeof candidate.baseFile === "string" &&
        typeof candidate.baseSha1 === "string" &&
        typeof candidate.patch === "string" &&
        typeof candidate.patchSha1 === "string" &&
        typeof candidate.outputSha1 === "string";
      if (complete) usable.push(candidate as Record);
      else skipped.push(candidate.name ?? pack);
    }
  }
  return { usable, skipped };
}

const found = records();
const ready =
  found.usable.length > 0 && existsSync(basesDir) && existsSync(wasmPath);

/** Boots a ROM and reports the frame's colour count; a fault fails the test. */
async function launch(wasm: Uint8Array, rom: Uint8Array) {
  const core = await WasmCore.create(wasm);
  try {
    core.load(rom, { rtc: false });
    core.setVideoInterval(16);
    // The core bounds one call's work, so advance one frame from wherever the
    // load left the clock rather than assuming it starts at zero.
    let reason = "executed";
    for (let frame = 0; frame < 600; frame += 1) {
      const run = await core.runUntil(
        core.currentCycle() + BigInt(CYCLES_PER_FRAME),
      );
      reason = run.reason;
      expect(reason, "the patched ROM faulted").not.toBe("faulted");
    }
    const colours = new Set(core.frame());
    return { reason, colours: colours.size };
  } finally {
    core.dispose();
  }
}

describe("romhack install", () => {
  afterAll(() => vi.unstubAllGlobals());

  it.skipIf(!ready)(
    "applies every release patch to its base",
    { timeout: 300_000 },
    async () => {
      const mismatched: string[] = [];
      let applied = 0;
      for (const record of found.usable) {
        const basePath = join(basesDir, basename(record.baseFile));
        if (!existsSync(basePath)) continue;
        const base = new Uint8Array(readFileSync(basePath));
        if (sha1(base) !== record.baseSha1) {
          mismatched.push(`${record.name ?? record.baseFile} (base dump)`);
          continue;
        }
        const patch = new Uint8Array(
          readFileSync(resolve(record.collection!, record.patch)),
        );
        if (sha1(patch) !== record.patchSha1) {
          mismatched.push(`${record.name ?? record.patch} (patch bytes)`);
          continue;
        }
        const format = record.patch.slice(record.patch.lastIndexOf(".") + 1);
        expect(["bps", "ups", "ips"], "unsupported patch format").toContain(
          format,
        );
        const version = seed.versions.find(
          (item) => item.outputSha1 === record.outputSha1,
        );
        expect(version, `catalog entry for ${record.name}`).toBeDefined();
        expect(version?.patch).toEqual({
          url: `https://patches.evenfall-emu.org/${record.patchSha1}.${format}`,
          format,
          size: patch.length,
        });
        expect(
          seed.bases.find((item) => item.id === version?.requiredBaseId)?.sha1,
        ).toBe(record.baseSha1);
        vi.stubGlobal(
          "fetch",
          async () =>
            new Response(patch, {
              status: 200,
              headers: { "content-type": "application/octet-stream" },
            }),
        );
        const rom = await patchRelease(
          version as unknown as CatalogVersion,
          base,
        );
        expect(sha1(rom), `${record.name ?? record.patch} output`).toBe(
          record.outputSha1,
        );
        if (record.outputCrc32)
          expect(crc(rom), `${record.name ?? record.patch} output CRC-32`).toBe(
            record.outputCrc32,
          );
        applied += 1;
      }
      expect(mismatched, "media that does not match its record").toEqual([]);
      expect(found.skipped, "incomplete release records").toEqual([]);
      expect(applied, "releases patched").toBe(found.usable.length);
    },
  );

  it.skipIf(!ready)(
    "boots patched releases in the core",
    { timeout: 180_000 },
    async () => {
      const wasm = new Uint8Array(readFileSync(wasmPath));
      const booted: string[] = [];
      const selected = [
        ...new Set(found.usable.map((record) => record.family)),
      ].map((family) =>
        found.usable.find((record) => record.family === family)!,
      );
      for (const record of selected) {
        const basePath = join(basesDir, basename(record.baseFile));
        if (!existsSync(basePath)) continue;
        const patch = new Uint8Array(
          readFileSync(resolve(record.collection!, record.patch)),
        );
        vi.stubGlobal(
          "fetch",
          async () =>
            new Response(patch, {
              status: 200,
              headers: { "content-type": "application/octet-stream" },
            }),
        );
        const format = record.patch.slice(record.patch.lastIndexOf(".") + 1);
        const rom = await patchRelease(
          {
            patch: {
              url: `https://example.invalid/${basename(record.patch)}`,
              format,
              size: patch.length,
            },
            outputSha1: record.outputSha1,
          } as CatalogVersion,
          new Uint8Array(readFileSync(basePath)),
        );
        const result = await launch(wasm, rom);
        expect(
          result.colours,
          "patched ROM rendered a blank frame",
        ).toBeGreaterThan(1);
        booted.push(
          `${record.name ?? basename(record.patch)}: ${result.reason}, ${result.colours} colours`,
        );
      }
      expect(booted.length, "families booted").toBe(selected.length);
      console.log(
        `booted ${booted.length} patched releases:\n  ${booted.join("\n  ")}`,
      );
    },
  );
});

describe("romhack install media", () => {
  it("reports what it needs when the media is absent", () => {
    if (ready) {
      console.log(
        `${found.usable.length} release records across ${packs.length} collection(s), bases in ${basesDir}`,
      );
      return;
    }
    console.log(
      "Run node tools/catalog/romhacks.mjs build and keep the base dumps in " +
        `${basesDir} to run the install flow; skipped otherwise.`,
    );
    expect(true).toBe(true);
  });
});
