import { readFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { beforeAll, describe, expect, it } from "vite-plus/test";
import { createMemoryReader } from "@evenfall/gba/bridge";
import { eliteReduxBeta2 } from "../src/elite-redux";
import { createReduxCatalog } from "../src/redux-catalog";
import { createReduxTrainerSearch } from "../src/redux-trainers";
import { pokemonResources } from "./fixtures/pokemon";

it("resolves signed level offsets, difficulty-selected teams, EV order and starting HP without inventing identity", () => {
  const bytes = new Uint8Array(4096);
  const view = new DataView(bytes.buffer);
  const trainer = 0x100 + 0x38;
  view.setUint32(trainer + 4, 0x08000300, true);
  bytes.set([0xbb, 0xff], 0x300); // A
  bytes.set([0xbc, 0xff], 0x400); // B
  for (const [i, offset] of [0x18, 0x20, 0x28].entries()) {
    bytes[trainer + offset] = 2;
    view.setUint32(trainer + offset + 4, 0x08000800 + i * 0x100, true);
    for (let slot = 0; slot < 2; slot++) {
      const address = 0x800 + i * 0x100 + slot * 0x1c;
      bytes[address] = slot ? 3 : 254; // +3 / -2, not unsigned 254.
      view.setUint16(address + 2, i + 1, true);
      bytes[address + 6] = 1;
      bytes.set([4, 8, 12, 16, 20, 24], address + 0x11);
      bytes[address + 0x17] = 13; // Jolly: +Speed, -SpA.
    }
  }
  const base = pokemonResources().catalog;
  const search = createReduxTrainerSearch(
    createMemoryReader([{ address: 0x08000000, bytes }]),
    {
      ...base,
      species: (id) => ({ ...base.species(id), innates: [11, 12, 13] }),
      abilityFlags: () => ({ unsuppressable: false, blocksSuppression: false }),
      holdEffect: () => 0,
    },
    {
      trainers: 0x08000100,
      trainerCount: 2,
      classes: 0x08000400,
      classCount: 1,
      portraitCount: 1,
      hellSkills: 0x08000600,
    },
    "fixture",
  );
  const context = {
    difficulty: 2,
    highestLevel: 20,
    hellLevelBonus: 0,
    evsEnabled: true,
  };
  const result = search("a", context);
  if (result.status !== "ready") throw new Error(result.reason);
  const team = result.data[0].team;
  expect(result.data[0].averageLevel).toBe(20.5);
  expect(team.map((mon) => [mon.species, mon.level])).toEqual([
    [2, 18],
    [2, 23],
  ]);
  expect(team[0]).toMatchObject({
    hp: 37,
    maxHp: 37,
    stats: {
      actual: {
        hp: 37,
        attack: 18,
        defense: 21,
        speed: 28,
        specialAttack: 26,
        specialDefense: 33,
      },
      evs: {
        hp: 4,
        attack: 8,
        defense: 12,
        specialAttack: 16,
        specialDefense: 20,
        speed: 24,
      },
    },
    abilities: [{ id: 7 }, { id: 11 }, { id: 12 }, { id: 13 }],
  });
  expect(team[0].personality).toBeUndefined();
  const hell = search("a", {
    ...context,
    difficulty: 3,
    highestLevel: 1,
    hellLevelBonus: 5,
  });
  if (hell.status !== "ready") throw new Error(hell.reason);
  expect(hell.data[0].team.map((mon) => [mon.species, mon.level])).toEqual([
    [3, 6],
    [3, 9],
  ]);
  const capped = search("a", {
    ...context,
    difficulty: 3,
    highestLevel: 99,
    hellLevelBonus: 5,
  });
  if (capped.status !== "ready") throw new Error(capped.reason);
  expect(capped.data[0].team.map((mon) => mon.level)).toEqual([100, 100]);
  expect(search("no match", context)).toEqual({ status: "ready", data: [] });
  const disabled = search("a", { ...context, evsEnabled: false });
  if (disabled.status !== "ready") throw new Error(disabled.reason);
  expect(Object.values(disabled.data[0].team[0].stats.evs)).toEqual([
    0, 0, 0, 0, 0, 0,
  ]);
  expect(disabled.data[0].team[0].stats.actual?.speed).toBe(26);
});

it("matches trainer names and classes without case, accents or punctuation", () => {
  const bytes = new Uint8Array(4096);
  const view = new DataView(bytes.buffer);
  const trainer = 0x100 + 0x38;
  view.setUint32(trainer + 4, 0x08000300, true);
  bytes.set([0xca, 0xe3, 0xdf, 0x1b, 0xda, 0xd5, 0xe2, 0xff], 0x300); // Pokéfan
  bytes.set(
    [0xca, 0xe3, 0xdf, 0x1b, 0x00, 0xc7, 0xd5, 0xe2, 0xdd, 0xd5, 0xd7, 0xff],
    0x400,
  ); // Poké Maniac
  for (const [i, offset] of [0x18, 0x20, 0x28].entries()) {
    bytes[trainer + offset] = 1;
    view.setUint32(trainer + offset + 4, 0x08000800 + i * 0x100, true);
    const address = 0x800 + i * 0x100;
    bytes[address] = 254; // -2 levels.
    view.setUint16(address + 2, i + 1, true);
    bytes[address + 6] = 1;
    bytes[address + 0x17] = 13; // Jolly: +Speed, -SpA.
  }
  const base = pokemonResources().catalog;
  const search = createReduxTrainerSearch(
    createMemoryReader([{ address: 0x08000000, bytes }]),
    {
      ...base,
      species: (id) => ({ ...base.species(id), innates: [11, 12, 13] }),
      abilityFlags: () => ({ unsuppressable: false, blocksSuppression: false }),
      holdEffect: () => 0,
    },
    {
      trainers: 0x08000100,
      trainerCount: 2,
      classes: 0x08000400,
      classCount: 1,
      portraitCount: 1,
      hellSkills: 0x08000600,
    },
    "fixture",
  );
  const context = {
    difficulty: 0,
    highestLevel: 20,
    hellLevelBonus: 0,
    evsEnabled: true,
  };
  const names = (query: string) => {
    const result = search(query, context);
    if (result.status !== "ready") throw new Error(result.reason);
    return result.data.map((entry) => entry.name);
  };
  expect(names("pokefan")).toEqual(["Pokéfan"]);
  expect(names("POKÉFAN")).toEqual(["Pokéfan"]);
  expect(names("poke maniac")).toEqual(["Pokéfan"]);
  expect(names("poké")).toEqual(["Pokéfan"]);
  expect(names("maniac zzz")).toEqual([]);
});

const path = process.env.ZIGBA_REDUX_ROM;
describe.skipIf(!path)(
  "Redux trainer previews from the verified release ROM",
  () => {
    let search: ReturnType<typeof createReduxTrainerSearch>;
    beforeAll(async () => {
      const bytes = new Uint8Array(await readFile(path!));
      expect(createHash("sha1").update(bytes).digest("hex")).toBe(
        eliteReduxBeta2.romHash,
      );
      const rom = createMemoryReader([{ address: 0x08000000, bytes }]);
      search = createReduxTrainerSearch(
        rom,
        createReduxCatalog(
          rom,
          eliteReduxBeta2.rom.catalog,
          eliteReduxBeta2.romHash,
        ),
        eliteReduxBeta2.rom.trainers,
        eliteReduxBeta2.romHash,
      );
    });

    it.each([
      {
        name: "May",
        ids: [
          529, 532, 535, 600, 768, 769, 530, 533, 536, 87, 63, 70, 531, 534,
          537, 664, 665, 666,
        ],
      },
      {
        name: "Brendan",
        ids: [
          520, 523, 526, 593, 592, 599, 521, 524, 527, 122, 104, 113, 522, 525,
          528, 661, 662, 663,
        ],
      },
    ])(
      "orders $name by encounter progression, not internal ID or scaled level",
      ({ name, ids }) => {
        for (const difficulty of [0, 1, 2, 3]) {
          const context = {
            difficulty,
            highestLevel: 20,
            hellLevelBonus: 0,
            evsEnabled: true,
          };
          const result = search(name, context);
          if (result.status !== "ready") throw new Error(result.reason);
          expect(
            result.data.slice(0, 18).map((entry) => entry.internalId),
          ).toEqual(ids);
          expect(
            result.data.slice(0, 18).map((entry) => entry.variant),
          ).toEqual(
            [
              "Route 103",
              "Rustboro",
              "Route 110",
              "Meteor Falls · Partner",
              "Route 119",
              "Lilycove / Route 103 rematch",
            ].flatMap((location) =>
              ["Mudkip", "Treecko", "Torchic"].map(
                (starter) => `${location} · Your starter: ${starter}`,
              ),
            ),
          );
          const partners = search(`${name} partner`, context);
          if (partners.status !== "ready") throw new Error(partners.reason);
          expect(partners.data.map((entry) => entry.id)).toEqual(
            result.data.slice(9, 12).map((entry) => entry.id),
          );
        }
      },
    );

    it("resolves Easy zero EVs and Ace applied EVs into the same ordinary party-member contract", () => {
      const context = {
        difficulty: 0,
        highestLevel: 20,
        hellLevelBonus: 0,
        evsEnabled: true,
      };
      const easy = search("roXanNe", context);
      if (easy.status !== "ready") throw new Error(easy.reason);
      expect(easy.data).toHaveLength(5);
      expect(new Set(easy.data.map((entry) => entry.id)).size).toBe(5);
      const initial = easy.data.find((entry) => entry.internalId === 265)!;
      expect(initial).toMatchObject({
        name: "Roxanne",
        variant: "Initial encounter",
        averageLevel: 20,
        portrait: { kind: "trainer", trainerPic: 40 },
      });
      expect(initial.team.map((mon) => mon.species)).toEqual([
        347, 345, 222, 2503, 299,
      ]);
      // Release table: Anorith base 45/95/65/75/40/50, IV31, Jolly. At level20,
      // no-EV totals are 54/49/37/45/24/31; applying 252 Atk EVs raises Atk to61.
      expect(initial.team[0]).toMatchObject({
        hp: 54,
        maxHp: 54,
        level: 20,
        nature: { name: "Jolly" },
        stats: {
          actual: {
            hp: 54,
            attack: 49,
            defense: 37,
            speed: 45,
            specialAttack: 24,
            specialDefense: 31,
          },
          evs: {
            hp: 0,
            attack: 0,
            defense: 0,
            speed: 0,
            specialAttack: 0,
            specialDefense: 0,
          },
        },
      });
      expect(initial.team[0].trainerId).toBeUndefined();
      const ace = search("roxanne", { ...context, difficulty: 1 });
      if (ace.status !== "ready") throw new Error(ace.reason);
      const changed = ace.data.find((entry) => entry.id === initial.id)!;
      expect(changed.team[0].stats.actual?.attack).toBe(61);
      expect(changed.team[0].stats.evs.attack).toBe(252);
      expect(initial.team[0].stats.evs.attack).toBe(0); // Previously returned results stay unchanged.
    });

    it("resolves max PP, Hell skills, and Iron Pill while keeping contextual battle modifiers out of raw totals", () => {
      const context = {
        difficulty: 3,
        highestLevel: 20,
        hellLevelBonus: 5,
        evsEnabled: true,
      };
      const result = search("roxanne", context);
      if (result.status !== "ready") throw new Error(result.reason);
      const initial = result.data.find((entry) => entry.internalId === 265)!;
      expect(initial.team).toHaveLength(6);
      expect(initial.averageLevel).toBe(25);
      expect(
        initial.team[0].abilities
          ?.slice(-3)
          .map((ability) => [ability.slot, ability.id, ability.kind]),
      ).toEqual([
        [4, 46, "extra"],
        [5, 61, "extra"],
        [6, 307, "extra"],
      ]);
      const ironPill = result.data.find((entry) => entry.internalId === 773)!
        .team[0];
      expect(ironPill.stats.ivs).toMatchObject({ attack: 31, speed: 0 });
      // Brave Tyranitar Lv25: Speed base61, EV0, IV0, nature -Speed => floor(35×0.9)=31.
      expect(ironPill.stats.actual?.speed).toBe(31);
      const easy = search("roxanne initial", { ...context, difficulty: 0 });
      if (easy.status !== "ready") throw new Error(easy.reason);
      expect(easy.data).toHaveLength(1);
      expect(
        easy.data[0].team[0].moves.map(
          (move) => move && [move.id, move.pp, move.maxPp],
        ),
      ).toEqual([
        [404, 24, 24],
        [317, 24, 24],
        [334, 24, 24],
        [306, 16, 16],
      ]);
    });

    it("keeps broad partial-name searches usable across every difficulty", () => {
      for (const difficulty of [0, 1, 2, 3]) {
        const result = search("e", {
          difficulty,
          highestLevel: 35,
          hellLevelBonus: 0,
          evsEnabled: true,
        });
        if (result.status !== "ready") throw new Error(result.reason);
        expect(result.data.length).toBeGreaterThan(100);
      }
    });
  },
);
