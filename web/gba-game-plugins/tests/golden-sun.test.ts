import { readFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { beforeAll, describe, expect, it } from "vite-plus/test";
import {
  createMemoryReader,
  decodeGame,
  detectFamily,
  locateBuild,
  locateGame,
  registerBuild,
  unregisterBuild,
  type MemoryRegion,
} from "@evenfall/gba/bridge";
import { WasmCore } from "@evenfall/gba/core";
import {
  elementalAffinity,
  estimatedTurnOrder,
  goldenAge1,
  goldenSun,
  goldenSunFamily,
  standbyDjinn,
  type GoldenSunUnit,
} from "../src/golden-sun";

const sample = {
  romHash: goldenAge1.romHash,
  timeline: "t",
  sequence: 1,
  cycle: 1n,
};
const EWRAM = 0x02000000;
const IWRAM = 0x03000000;

/** A blank snapshot with a writer for little-endian fields. */
function snapshot() {
  const ewram = new Uint8Array(0x40000);
  const iwram = new Uint8Array(0x8000);
  const region = (address: number) =>
    address >= IWRAM
      ? { bytes: iwram, base: IWRAM }
      : { bytes: ewram, base: EWRAM };
  const put = (address: number, value: number, size: 1 | 2 | 4) => {
    const { bytes, base } = region(address);
    for (let i = 0; i < size; i++)
      bytes[address - base + i] = (value >>> (8 * i)) & 0xff;
  };
  const unit = (
    address: number,
    name: string,
    fields: Record<number, [number, 1 | 2 | 4]>,
  ) => {
    for (let i = 0; i < name.length; i++)
      put(address + i, name.charCodeAt(i), 1);
    const base: Record<number, [number, 1 | 2 | 4]> = {
      0x0f: [10, 1],
      0x34: [100, 2],
      0x36: [20, 2],
      0x38: [100, 2],
      0x3a: [20, 2],
      0x40: [50, 2],
    };
    for (const [offset, [value, size]] of Object.entries({
      ...base,
      ...fields,
    }))
      put(address + Number(offset), value, size);
  };
  const decode = () =>
    decodeGame(
      goldenSun,
      sample,
      createMemoryReader([
        { address: EWRAM, bytes: ewram },
        { address: IWRAM, bytes: iwram },
      ]),
      {},
    );
  return { put, unit, decode };
}

it("lists the party in its stored order, from the joined characters' records", () => {
  const { put, unit, decode } = snapshot();
  put(0x02000040, 0b1001, 1); // Isaac and Mia have joined
  put(0x02000438, 3, 1); // Mia leads
  put(0x02000439, 0, 1);
  put(0x0200043a, 2, 1); // beyond the member count: ignored
  put(0x02000250, 123456, 4);
  unit(0x02000500, "Isaac", {
    0x48: [109, 2],
    0x4a: [127, 2],
    0x100: [0b101, 4],
    0x110: [0b001, 4],
  });
  unit(0x02000500 + 3 * 0x14c, "Mia", {});
  const party = decode().party;
  expect(party.status).toBe("ready");
  if (party.status !== "ready") return;
  expect(party.data.coins).toBe(123456);
  expect(party.data.members.map((m) => [m.slot, m.name, m.element])).toEqual([
    [3, "Mia", "mercury"],
    [0, "Isaac", "venus"],
  ]);
  const isaac = party.data.members[1];
  expect(isaac.elements[0]).toEqual({ power: 109, resist: 127 });
  // Mars (the third element) holds two djinn, one of them Set.
  expect(isaac.djinn[2]).toEqual({ owned: 2, set: 1 });
});

it("reports no party until someone joins, and rejects an order naming an absent character", () => {
  const { put, decode } = snapshot();
  expect(decode().party.status).toBe("unavailable");
  put(0x02000040, 0b0001, 1);
  put(0x02000438, 1, 1); // Garet has not joined
  expect(decode().party).toMatchObject({ status: "unavailable" });
});

it("reads stacked statuses with their turns and signed stages", () => {
  const { put, unit, decode } = snapshot();
  put(0x02000040, 1, 1);
  unit(0x02000500, "Isaac", {
    0x131: [2, 1], // venom
    0x132: [3, 1],
    0x133: [0xfe, 1], // attack down 2 stages for 3 turns
    0x134: [5, 1], // defense turns without a stage: no change
    0x138: [4, 1],
    0x13c: [2, 1],
    0x141: [3, 1],
  });
  const party = decode().party;
  if (party.status !== "ready") throw new Error(party.status);
  expect(party.data.members[0].statuses).toEqual([
    { kind: "venom", turns: null },
    { kind: "attack", turns: 3, stage: -2 },
    { kind: "delusion", turns: 4 },
    { kind: "sleep", turns: 2 },
    { kind: "doom", turns: 3 },
  ]);
});

it("is in battle while the enemy block pointer is set, keeping defeated enemies and skipping empty slots", () => {
  const { put, unit, decode } = snapshot();
  expect(decode().battle).toMatchObject({
    status: "ready",
    data: { active: false },
  });
  put(0x03001f28, 0x02030878, 4);
  unit(0x02030878, "Slime1", { 0x38: [0, 2] });
  unit(0x02030878 + 2 * 0x14c, "Slime3", {});
  const battle = decode().battle;
  expect(battle).toMatchObject({ status: "ready", data: { active: true } });
  if (battle.status !== "ready" || !battle.data.active) return;
  expect(battle.data.enemies.map((e) => [e.slot, e.name, e.hp])).toEqual([
    [0, "Slime1", 0],
    [2, "Slime3", 100],
  ]);
});

const unit = (name: string, agility: number, hp = 1): GoldenSunUnit => ({
  slot: 0,
  name,
  level: 1,
  hp,
  maxHp: 1,
  pp: 0,
  maxPp: 0,
  attack: 0,
  defense: 0,
  agility,
  luck: 0,
  elements: [],
  djinn: [],
  statuses: [],
});

it("orders living units by agility, keeping allies before enemies on ties", () => {
  const order = estimatedTurnOrder(
    [unit("Garet", 90), unit("Isaac", 151), unit("Mia", 200, 0)],
    [unit("Orc1", 90), unit("Orc2", 120)],
  );
  expect(order.map(({ unit, side }) => `${side}:${unit.name}`)).toEqual([
    "party:Isaac",
    "enemy:Orc2",
    "party:Garet",
    "enemy:Orc1",
  ]);
});

it("marks the elements a unit resists or is weak to against its own mean", () => {
  const orc = {
    ...unit("Orc", 1),
    elements: [100, 72, 72, 48].map((resist) => ({ power: 85, resist })),
  };
  expect(elementalAffinity(orc)).toEqual([
    "resists",
    "neutral",
    "neutral",
    "weak",
  ]);
});

it("counts the party's djinn on Standby per element", () => {
  const djinn = (pairs: [number, number][]) =>
    pairs.map(([owned, set]) => ({ owned, set }));
  expect(
    standbyDjinn([
      {
        ...unit("Isaac", 1),
        djinn: djinn([
          [0, 0],
          [0, 0],
          [3, 2],
          [3, 3],
        ]),
      },
      {
        ...unit("Mia", 1),
        djinn: djinn([
          [1, 0],
          [4, 4],
          [0, 0],
          [0, 0],
        ]),
      },
    ]),
  ).toEqual([1, 0, 1, 0]);
});

// Optional user-owned fixtures (hashes in docs/golden-sun.md): the
// shared battle state, the same battle after Rally and after a delusion with
// a djinni on Standby, and the field after winning it.
const romPath = process.env.EVENFALL_GOLDEN_SUN_ROM;
const statePaths = {
  battle: process.env.EVENFALL_GOLDEN_SUN_BATTLE,
  buff: process.env.EVENFALL_GOLDEN_SUN_BUFF,
  delusion: process.env.EVENFALL_GOLDEN_SUN_DELUSION,
  field: process.env.EVENFALL_GOLDEN_SUN_FIELD,
};
const stateHashes = {
  battle: "d1206cddf38d88d72b9f0690ba50d2b3ed70d73d618cc9aad42ee544214670c1",
  buff: "3dcb61f486a002cf5f1c385363deb73d982f15d754672084087eab59e13923e2",
  delusion: "7946eff1ae3c687666c17804d0a41936355af0be5247589a9d0d06d4d1fd835e",
  field: "4659517551017f8bb9ceacd58c33364a6b9debc02312aeb0b071462f6320706c",
};
describe.skipIf(!romPath || Object.values(statePaths).some((path) => !path))(
  "private Golden Age 1 build",
  () => {
    let rom: Uint8Array;
    const regions = {} as Record<
      keyof typeof statePaths,
      readonly MemoryRegion[]
    >;
    beforeAll(async () => {
      const wasm = await readFile(
        new URL("../../gba/evenfall-gba.wasm", import.meta.url),
      );
      rom = new Uint8Array(await readFile(romPath!));
      expect(createHash("sha1").update(rom).digest("hex")).toBe(
        goldenAge1.romHash,
      );
      const { instance } = (await WebAssembly.instantiate(
        wasm,
      )) as unknown as WebAssembly.WebAssemblyInstantiatedSource;
      const core = new WasmCore(instance.exports);
      try {
        core.load(rom);
        for (const key of Object.keys(
          statePaths,
        ) as (keyof typeof statePaths)[]) {
          const state = await readFile(statePaths[key]!);
          expect(createHash("sha256").update(state).digest("hex")).toBe(
            stateHashes[key],
          );
          core.loadState(state);
          regions[key] = core.captureMemory().regions;
        }
      } finally {
        core.dispose();
      }
    });
    const decode = (key: keyof typeof statePaths) =>
      decodeGame(goldenSun, sample, createMemoryReader(regions[key]), {});

    it("locates every reviewed address from the ROM's code", () => {
      expect(detectFamily(goldenSunFamily, rom).score).toBe(1);
      const located = locateBuild(goldenSunFamily, rom);
      expect(
        Object.fromEntries(located.sources.map((r) => [r.source, r.address])),
      ).toEqual(goldenAge1.addresses);
      expect(located.sources.every((r) => r.support >= 2 && !r.inferred)).toBe(
        true,
      );
    });

    it("decodes the shared battle: four adepts against two Orc Brutes", () => {
      const { party, battle } = decode("battle");
      if (
        party.status !== "ready" ||
        battle.status !== "ready" ||
        !battle.data.active
      )
        throw new Error("not ready");
      expect(party.data.coins).toBe(60612);
      expect(
        party.data.members.map((m) => [
          m.name,
          m.level,
          m.hp,
          m.maxHp,
          m.pp,
          m.maxPp,
        ]),
      ).toEqual([
        ["Isaac", 21, 400, 400, 105, 126],
        ["Garet", 20, 302, 337, 150, 150],
        ["Ivan", 20, 277, 283, 174, 174],
        ["Mia", 20, 174, 200, 133, 133],
      ]);
      const [isaac] = party.data.members;
      expect([isaac.attack, isaac.defense, isaac.agility, isaac.luck]).toEqual([
        400, 140, 151, 5,
      ]);
      expect(isaac.elements).toEqual([
        { power: 109, resist: 127 },
        { power: 78, resist: 121 },
        { power: 91, resist: 124 },
        { power: 92, resist: 110 },
      ]);
      expect(isaac.djinn.map((d) => [d.owned, d.set])).toEqual([
        [0, 0],
        [0, 0],
        [3, 3],
        [3, 3],
      ]);
      expect(
        battle.data.enemies.map((e) => [
          e.slot,
          e.name,
          e.level,
          e.hp,
          e.attack,
        ]),
      ).toEqual([
        [0, "Orc Brute1", 21, 226, 248],
        [1, "Orc Brute2", 21, 226, 248],
      ]);
      expect(battle.data.enemies[0].elements.map((e) => e.resist)).toEqual([
        100, 72, 72, 48,
      ]);
      expect(battle.data.enemies.every((e) => e.statuses.length === 0)).toBe(
        true,
      );
    });

    it("shows Rally's attack boost with its turns, already applied to Attack", () => {
      const { battle } = decode("buff");
      if (battle.status !== "ready" || !battle.data.active)
        throw new Error("not ready");
      const brute2 = battle.data.enemies.find((e) => e.name === "Orc Brute2")!;
      expect(brute2.attack).toBe(310);
      expect(brute2.statuses).toEqual([{ kind: "attack", turns: 7, stage: 2 }]);
    });

    it("shows a delusion and the djinni Isaac unleashed on Standby", () => {
      const { party, battle } = decode("delusion");
      if (
        party.status !== "ready" ||
        battle.status !== "ready" ||
        !battle.data.active
      )
        throw new Error("not ready");
      const brute2 = battle.data.enemies.find((e) => e.name === "Orc Brute2")!;
      expect(brute2.statuses).toEqual([{ kind: "delusion", turns: 7 }]);
      expect(party.data.members[0].djinn[2]).toEqual({ owned: 3, set: 2 });
      expect(standbyDjinn(party.data.members)).toEqual([0, 0, 1, 0]);
    });

    it("leaves battle once it is won, keeping the party and the reward", () => {
      const { party, battle } = decode("field");
      expect(battle).toMatchObject({
        status: "ready",
        data: { active: false },
      });
      expect(party).toMatchObject({ status: "ready", data: { coins: 60896 } });
    });
  },
);

// The Lost Age shares the record and its routines at other addresses: no
// reviewed manifest describes it, so the player locates it.
const tlaRomPath = process.env.EVENFALL_GOLDEN_SUN_TLA_ROM;
const tlaFieldPath = process.env.EVENFALL_GOLDEN_SUN_TLA_FIELD;
const tlaHash = "b500663220cb9bf56b9f8e8c0c544f1d6fa3a824";
describe.skipIf(!tlaRomPath || !tlaFieldPath)(
  "private The Lost Age build",
  () => {
    it("is located from its own code, and reads Jenna alone in the opening", async () => {
      const wasm = await readFile(
        new URL("../../gba/evenfall-gba.wasm", import.meta.url),
      );
      const rom = new Uint8Array(await readFile(tlaRomPath!));
      const state = await readFile(tlaFieldPath!);
      expect(createHash("sha1").update(rom).digest("hex")).toBe(tlaHash);
      expect(createHash("sha256").update(state).digest("hex")).toBe(
        "1b1fc38b2bdedf2128bdf208d9a20cb150376d1a29bdc4810a848dfdde5d0081",
      );
      const outcome = await locateGame(goldenSun, rom, {
        romHash: tlaHash,
        yieldControl: async () => {},
      });
      if (outcome.status !== "located")
        throw new Error(
          outcome.status === "rejected" ? outcome.reason : outcome.status,
        );
      expect(outcome.build.review).toEqual([]);
      expect(outcome.build.manifest.addresses).toEqual({
        party: 0x02000520,
        partyFlags: 0x02000040,
        partyOrder: 0x02000458,
        coins: 0x02000250,
        enemies: 0x030000d8,
      });
      const { instance } = (await WebAssembly.instantiate(
        wasm,
      )) as unknown as WebAssembly.WebAssemblyInstantiatedSource;
      const core = new WasmCore(instance.exports);
      let regions: readonly MemoryRegion[];
      try {
        core.load(rom);
        core.loadState(state);
        regions = core.captureMemory().regions;
      } finally {
        core.dispose();
      }
      registerBuild(goldenSun, outcome.build.manifest);
      try {
        const { party, battle } = decodeGame(
          goldenSun,
          { ...sample, romHash: tlaHash },
          createMemoryReader(regions),
          {},
        );
        expect(battle).toMatchObject({
          status: "ready",
          data: { active: false },
        });
        if (party.status !== "ready") throw new Error(party.status);
        expect(party.data.coins).toBe(200);
        expect(
          party.data.members.map((m) => [
            m.name,
            m.element,
            m.level,
            m.hp,
            m.pp,
          ]),
        ).toEqual([["Jenna", "mars", 5, 65, 43]]);
      } finally {
        unregisterBuild(goldenSun, tlaHash);
      }
    }, 30_000);
  },
);
