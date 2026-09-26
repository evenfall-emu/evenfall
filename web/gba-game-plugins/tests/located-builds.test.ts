import { readFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { describe, expect, it } from "vitest";
import {
  locateGame,
  relocateManifest,
  resolveGame,
  type Manifest,
} from "@evenfall/gba/bridge";
import {
  heartNSoul,
  heartNSoul121,
  heartNSoulFamily,
} from "../src/heart-n-soul";
import {
  eliteRedux,
  eliteReduxBeta2,
  eliteReduxFamily,
} from "../src/elite-redux";

// Builds no committed manifest describes: the player locates them at runtime.
// Each check compares locating with relocating the reviewed build, two
// independent methods, value for value.
const builds = [
  {
    name: "Heart & Soul 1.2.0",
    game: heartNSoul,
    family: heartNSoulFamily,
    reviewed: heartNSoul121 as Manifest<any, any>,
    parent: process.env.ZIGBA_HNS_ROM,
    child: process.env.ZIGBA_HNS_120_ROM,
    romHash: "b8a4b491ca67a76f079a8e7c19fd5512a2147dd2",
    review: ["rom.catalog.moveDescriptions.count"],
  },
  {
    name: "Elite Redux 2.65 Beta2 debug",
    game: eliteRedux,
    family: eliteReduxFamily,
    reviewed: eliteReduxBeta2 as Manifest<any, any>,
    parent: process.env.ZIGBA_REDUX_ROM,
    child: process.env.ZIGBA_REDUX_DEBUG_ROM,
    romHash: "299165fb56e19ac8afe5d66f83dd5bada823d58c",
    review: [],
  },
];

const valueAt = (manifest: Manifest<any, any>, path: string) =>
  path
    .split(".")
    .reduce<unknown>(
      (node, key) => (node as Record<string, unknown>)[key],
      manifest,
    );

for (const build of builds)
  describe.skipIf(!build.parent || !build.child)(
    `private located ${build.name}`,
    () => {
      it("is located at runtime where relocation puts it", async () => {
        const parent = new Uint8Array(await readFile(build.parent!));
        const child = new Uint8Array(await readFile(build.child!));
        expect(createHash("sha1").update(parent).digest("hex")).toBe(
          build.reviewed.romHash,
        );
        expect(createHash("sha1").update(child).digest("hex")).toBe(
          build.romHash,
        );
        expect(resolveGame([build.game], build.romHash)).toBeNull();
        const outcome = await locateGame(build.game, child);
        if (outcome.status !== "located")
          throw new Error(
            outcome.status === "rejected" ? outcome.reason : outcome.status,
          );
        const { manifest, review } = outcome.build;
        expect(manifest.romHash).toBe(build.romHash);
        expect(manifest.provenance).toEqual({
          method: "located",
          parent: build.reviewed.romHash,
        });
        expect(review).toEqual(build.review);
        const { evidence } = relocateManifest(
          build.family,
          { rom: parent, manifest: build.reviewed },
          { rom: child, romHash: build.romHash },
        );
        // Every RAM source and every ROM address, relocated with evidence.
        expect(
          evidence
            .filter((e) => e.path.startsWith("addresses."))
            .map((e) => e.path)
            .sort(),
        ).toEqual(
          Object.keys(build.family.sources)
            .map((name) => `addresses.${name}`)
            .sort(),
        );
        expect(evidence.every((e) => e.method !== "unresolved")).toBe(true);
        for (const { path, to } of evidence)
          expect([path, valueAt(manifest, path)]).toEqual([path, to]);
      }, 30_000);
    },
  );
