import { readFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { describe, expect, it } from "vite-plus/test";
import {
  createMemoryReader,
  detectFamily,
  locateBuild,
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
    parent: process.env.EVENFALL_HNS_ROM,
    child: process.env.EVENFALL_HNS_120_ROM,
    romHash: "b8a4b491ca67a76f079a8e7c19fd5512a2147dd2",
    review: ["rom.catalog.moveDescriptions.count"],
  },
  {
    name: "Elite Redux 2.65 Beta2 debug",
    game: eliteRedux,
    family: eliteReduxFamily,
    reviewed: eliteReduxBeta2 as Manifest<any, any>,
    parent: process.env.EVENFALL_REDUX_ROM,
    child: process.env.EVENFALL_REDUX_DEBUG_ROM,
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

describe.skipIf(!process.env.EVENFALL_REDUX_ROM)(
  "private shared Redux locators on reviewed Beta2",
  () => {
    it("finds the reviewed values, including the ability record layout", async () => {
      const rom = new Uint8Array(
        await readFile(process.env.EVENFALL_REDUX_ROM!),
      );
      expect(createHash("sha1").update(rom).digest("hex")).toBe(
        eliteReduxBeta2.romHash,
      );
      const located = locateBuild(eliteReduxFamily, rom);
      for (const result of located.sources)
        expect([result.source, result.address]).toEqual([
          result.source,
          valueAt(eliteReduxBeta2, `addresses.${result.source}`),
        ]);
      for (const result of located.rom)
        expect([result.source, result.address]).toEqual([
          result.source,
          valueAt(eliteReduxBeta2, `rom.${result.source}`),
        ]);
      const outcome = await locateGame(eliteRedux, rom);
      expect(outcome.status).toBe("located");
    }, 30_000);
  },
);

describe.skipIf(!process.env.EVENFALL_REDUX_26541_ROM)(
  "private located Redux 2.65.4.1b",
  () => {
    it("locates moved globals and the changed ability layout without a version manifest", async () => {
      const rom = new Uint8Array(
        await readFile(process.env.EVENFALL_REDUX_26541_ROM!),
      );
      const romHash = "2f3e17e55c6afe23da97ec0f96a067eeb142ad15";
      expect(createHash("sha1").update(rom).digest("hex")).toBe(romHash);
      expect(detectFamily(eliteReduxFamily, rom).score).toBe(1);
      expect(resolveGame([eliteRedux], romHash)).toBeNull();
      const outcome = await locateGame(eliteRedux, rom);
      if (outcome.status !== "located")
        throw new Error(
          outcome.status === "rejected" ? outcome.reason : outcome.status,
        );
      const { manifest, review } = outcome.build;
      // Literal-pool relocation corroborates these globals. The ROM header
      // corroborates the icon table. Ability indexing/flag accesses in the
      // release code give the base, 196-byte stride and +0xbc flags.
      expect(manifest.addresses).toEqual({
        playerCount: 0x0203990d,
        playerParty: 0x02039910,
        enemyParty: 0x02039ad8,
        settings: 0x03003410,
        thirdBadge: 0x0300340c,
        storage: 0x03003414,
        battleActive: 0x03003859,
        battleFlags: 0x0201c3f4,
        battlerCount: 0x0201c484,
        partyIndexes: 0x0201c488,
        battlers: 0x0201c514,
        absent: 0x0201c6e3,
        statuses3: 0x0201c7b4,
        neutralizingGas: 0x0201cb71,
        enigmaBerries: 0x0201ca30,
      });
      expect(manifest.rom.catalog).toMatchObject({
        abilities: 0x08fd95e8,
        abilityStride: 196,
        abilityFlagsOffset: 0xbc,
        experience: 0x08c02744,
        itemCount: 1023,
      });
      expect(manifest.rom.trainers).toMatchObject({
        hellSkills: 0x0850cd28,
        trainerCount: 932,
      });
      expect(manifest.rom.artwork.iconTable).toBe(0x08e31fa0);
      expect(review).toEqual([]);
      const { catalog } = eliteReduxFamily.resources({
        rom: createMemoryReader([{ address: 0x08000000, bytes: rom }]),
        romHash,
        config: manifest.rom,
      });
      expect(catalog.ability(1).name).toBe("Stench");
      expect(catalog.ability(149).name).toBe("Illusion");
      expect(catalog.abilityFlags(121).unsuppressable).toBe(true);
      expect(catalog.abilityFlags(256).unsuppressable).toBe(true);
      expect(catalog.abilityFlags(149).unsuppressable).toBe(false);
    }, 30_000);
  },
);
