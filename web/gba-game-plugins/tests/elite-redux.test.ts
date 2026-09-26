import { readFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { beforeAll, describe, expect, it } from "vitest";
import {
  createMemoryReader,
  ref,
  createGame,
  decodeGame,
  resolveGame,
  type MemoryRegion,
} from "@evenfall/gba/bridge";
import { WasmCore } from "@evenfall/gba/core";
import {
  eliteRedux,
  eliteReduxFamily,
  reduxTrainerContext,
  eliteReduxBeta2,
  reduxAbilities,
} from "../src/elite-redux";
import {
  reduxBox,
  reduxParty,
  reduxBattle,
  reduxSettings,
} from "../src/redux-record";
import type { ReduxCatalog } from "../src/redux-catalog";
import { pokemonResources } from "./fixtures/pokemon";

function recordBytes() {
  const bytes = new Uint8Array(0x4c);
  const v = new DataView(bytes.buffer);
  v.setUint32(0, 0x12345678, true);
  v.setUint32(4, 0xabcdef12, true);
  v.setUint32(8, (10000 << 11) | 777, true);
  v.setUint32(12, (219 << 22) | (513 << 11) | 420, true);
  v.setUint32(16, (19 << 27) | (1001 << 16) | 996, true);
  v.setUint32(
    20,
    (2 << 30) |
      (3 << 28) |
      (2 << 26) |
      (17 << 19) |
      (2 << 16) |
      (1 << 15) |
      (24 << 10) |
      850,
    true,
  );
  bytes.set([1, 23, 45, 67, 89, 101], 24);
  bytes.set([0xbb, 0xff], 31);
  bytes.set([0xbc, 0xff], 38);
  bytes[50] = 32;
  bytes.set([8, 12, 3, 19], 52);
  v.setUint32(56, 16, true);
  bytes[60] = 20;
  v.setUint16(62, 17, true);
  [62, 41, 29, 33, 25, 30].forEach((n, i) => v.setUint16(64 + i * 2, n, true));
  return bytes;
}
const read = (bytes: Uint8Array) => createMemoryReader([{ address: 0, bytes }]);
describe("Redux packed ABI", () => {
  it("separates overlapping move/experience/friendship/shiny fields and reads asymmetric stats", () => {
    const mon = read(recordBytes()).read(ref(0, reduxParty));
    expect(mon).toMatchObject({
      species: 996,
      personality: 0x12345678,
      trainerId: 0xabcdef12,
      moveIds: [777, 420, 513, 1001],
      experience: 10000,
      friendship: 219,
      heldItem: 850,
      hiddenNature: 24,
      abilitySlot: 2,
      isEgg: true,
      shinyTier: 2,
      language: 2,
      trainerName: "A",
      nickname: "B",
      pp: [8, 12, 3, 19],
      level: 20,
      hp: 17,
      maxHp: 62,
      calculatedStats: {
        hp: 62,
        attack: 41,
        defense: 29,
        speed: 33,
        specialAttack: 25,
        specialDefense: 30,
      },
      ivs: { hp: 31, speed: 0 },
      evs: {
        hp: 1,
        attack: 23,
        defense: 45,
        speed: 67,
        specialAttack: 89,
        specialDefense: 101,
      },
    });
  });
  it("reads settings bit boundaries without conflating randomizers or difficulty", () => {
    const bytes = new Uint8Array(0xc8d),
      view = new DataView(bytes.buffer);
    view.setUint16(0xc88, 0x2fff, true);
    view.setUint16(0xc8a, 0x128, true);
    bytes[0xc8c] = 0xfb;
    expect(read(bytes).read(ref(0, reduxSettings))).toEqual({
      difficulty: 2,
      evsEnabled: false,
      randomizedAbilities: true,
      randomizedInnates: true,
      randomizedTypes: false,
    });
    view.setUint16(0xc8a, 0x80, true);
    bytes[0xc8c] = 4;
    expect(read(bytes).read(ref(0, reduxSettings))).toMatchObject({
      evsEnabled: true,
      randomizedAbilities: false,
      randomizedInnates: false,
      randomizedTypes: true,
    });
  });
  it("reads signed stage deltas, live PP and all seven possible ability slots", () => {
    const bytes = new Uint8Array(0x68),
      v = new DataView(bytes.buffer);
    v.setUint16(0x36, 29, true);
    v.setUint16(0x38, 61, true);
    bytes[0x4b] = 19;
    bytes.set([6, 0, 12, 1, 11, 5, 7, 6], 0x3c);
    bytes.set([8, 4, 2, 1], 0x47);
    [314, 575, 85, 492, 500, 0, 1033].forEach((id, i) =>
      v.setUint16(0x28 + i * 2, id, true),
    );
    expect(read(bytes).read(ref(0, reduxBattle))).toMatchObject({
      abilityIds: [314, 575, 85, 492, 500, 0, 1033],
      pp: [8, 4, 2, 1],
      statStages: {
        attack: -6,
        defense: 6,
        speed: -5,
        specialAttack: 5,
        specialDefense: -1,
        accuracy: 1,
        evasion: 0,
      },
    });
    bytes[0x3d] = 13;
    expect(() => read(bytes).read(ref(0, reduxBattle))).toThrow("stage");
  });
  it("rejects impossible HP, nature and empty occupied records", () => {
    const bytes = recordBytes();
    bytes[62] = 63;
    expect(() => read(bytes).read(ref(0, reduxParty))).toThrow("HP");
    bytes[21] |= 0x7c;
    expect(() => read(bytes).read(ref(0, reduxBox))).toThrow("nature");
    expect(() => read(new Uint8Array(52)).read(ref(0, reduxBox))).toThrow(
      "identity",
    );
  });
  it("bounds the entire relocated storage record and rejects unaligned pointers and unsupported box text", () => {
    const bytes = new Uint8Array(0x9f78);
    bytes[0] = 25;
    bytes.set(recordBytes().subarray(0, 0x34), 4 + (25 * 30 + 29) * 0x34);
    bytes.set([0xbb, 0xff], 0x9e74 + 25 * 9);
    const pointerBytes = new Uint8Array([0, 0, 0, 2]);
    const decodeStorage = (data = bytes) => {
      const memory = createMemoryReader([
        {
          address: eliteRedux.profiles[0].symbols.storage.address,
          bytes: pointerBytes,
        },
        { address: 0x02000000, bytes: data },
      ]);
      return memory.read(memory.read(eliteRedux.profiles[0].symbols.storage)!);
    };
    const storage = decodeStorage();
    expect(storage.currentBox).toBe(25);
    expect(storage.boxes[25]).toMatchObject({
      name: "A",
      pokemon: [{ slot: 29, record: { species: 996, nickname: "B" } }],
    });
    expect(() => decodeStorage(bytes.subarray(0, bytes.length - 1))).toThrow(
      "Uncaptured memory",
    );
    pointerBytes[0] = 1;
    expect(() => decodeStorage()).toThrow("Unaligned pointer");
    pointerBytes[0] = 0;
    bytes[0x9e74 + 25 * 9] = 0xfc;
    expect(() => decodeStorage()).toThrow(
      "Redux box 26: Unsupported Gen III control code 0xFC",
    );
  });
  it("distinguishes selected ability from alternatives, and both sides of innate unlocks", () => {
    const base = pokemonResources().catalog;
    const catalog: ReduxCatalog = {
      ...base,
      species: (id) => ({
        ...base.species(id),
        abilities: [11, 22, 33],
        innates: [44, 55, 66],
      }),
      abilityFlags: () => ({ unsuppressable: false, blocksSuppression: false }),
      holdEffect: () => 0,
    };
    const record = {
      ...read(recordBytes()).read(ref(0, reduxBox)),
      species: 1,
    };
    const settings = {
      difficulty: 2,
      evsEnabled: true,
      randomizedAbilities: false,
      randomizedInnates: false,
      randomizedTypes: false,
    };
    const values = (level: number, difficulty = 2, enemy = false) =>
      reduxAbilities(
        record,
        level,
        catalog,
        { ...settings, difficulty },
        enemy,
      )!;
    expect(values(16).map((a) => [a.id, a.state])).toEqual([
      [33, "available"],
      [44, "available"],
      [55, "locked"],
      [66, "locked"],
    ]);
    expect(values(17).map((a) => a.state)).toEqual([
      "available",
      "available",
      "available",
      "locked",
    ]);
    expect(values(24).every((a) => a.state === "available")).toBe(true);
    expect(values(9, 3)[1].state).toBe("locked");
    expect(values(10, 3)[1].state).toBe("available");
    expect(values(1, 2, true).every((a) => a.state === "available")).toBe(true);
    expect(values(1, 1).every((a) => a.state === "available")).toBe(true);
    expect(
      reduxAbilities(
        record,
        20,
        catalog,
        { ...settings, randomizedInnates: true },
        false,
      ),
    ).toBeNull();
  });
  it("registers compatible builds by exact hash without broadening to unknown revisions", () => {
    const second: typeof eliteReduxBeta2 = {
      ...eliteReduxBeta2,
      romHash: "b".repeat(40),
      source: { ...eliteReduxBeta2.source, variant: "synthetic second build" },
      addresses: { ...eliteReduxBeta2.addresses, playerCount: 0 },
      rom: {
        ...eliteReduxBeta2.rom,
        catalog: { ...eliteReduxBeta2.rom.catalog, abilityCount: 1 },
      },
    };
    const game = createGame(eliteReduxFamily, [eliteReduxBeta2, second]);
    expect(resolveGame([game], second.romHash)).toBe(game);
    expect(resolveGame([game], "c".repeat(40))).toBeNull();
    const resources = game.prepare!({
      rom: read(new Uint8Array()),
      romHash: second.romHash,
    });
    // Links belong to the game, so every build gets them.
    expect(resources.linksFactory).toBeDefined();
    expect(() => resources.catalog.ability(1)).toThrow(
      "Invalid Redux catalog ID",
    );
    expect(
      decodeGame(
        game,
        { romHash: second.romHash, timeline: "second", sequence: 1, cycle: 0n },
        read(new Uint8Array(1)),
        resources,
        new Set(["party"]),
      ).party,
    ).toMatchObject({ status: "ready", data: { party: [] } });
    expect(() =>
      game.prepare!({ rom: read(new Uint8Array()), romHash: "c".repeat(40) }),
    ).toThrow("No pokemon/elite-redux manifest");
  });
});

const romPath = process.env.ZIGBA_REDUX_ROM,
  statePath = process.env.ZIGBA_REDUX_STATE;
describe.skipIf(!romPath || !statePath)("private Redux release/state", () => {
  let regions: readonly MemoryRegion[];
  let resources: ReturnType<NonNullable<typeof eliteRedux.prepare>>;
  const sample = {
    romHash: eliteReduxBeta2.romHash,
    timeline: "redux-test",
    sequence: 1,
    cycle: 12852115585n,
  };
  beforeAll(async () => {
    const rom = new Uint8Array(await readFile(romPath!));
    const state = new Uint8Array(await readFile(statePath!));
    expect(createHash("sha1").update(rom).digest("hex")).toBe(sample.romHash);
    expect(createHash("sha256").update(state).digest("hex")).toBe(
      "bf403592558c7a6cdc0deb493f8ab26f8796dfec829f0a2031fcbbc44af66d23",
    );
    const instance = await WebAssembly.instantiate(
      await WebAssembly.compile(
        Uint8Array.from(
          await readFile(
            new URL("../../../zig-out/web/zigba.wasm", import.meta.url),
          ),
        ),
      ),
    );
    const core = new WasmCore(instance.exports);
    try {
      core.load(rom);
      core.loadState(state);
      regions = core.captureMemory().regions;
    } finally {
      core.dispose();
    }
    resources = eliteRedux.prepare!({
      rom: createMemoryReader([{ address: 0x08000000, bytes: rom }]),
      romHash: sample.romHash,
    });
  });
  const decode = (input = regions) =>
    decodeGame(eliteRedux, sample, createMemoryReader(input), resources);
  const trainerContext = (input = regions) =>
    reduxTrainerContext({
      memory: createMemoryReader(input),
      symbols: eliteRedux.profiles[0].symbols,
    });
  function changed(address: number, mutate: (bytes: Uint8Array) => void) {
    return regions.map((region) => {
      const bytes = region.bytes.slice();
      if (address >= region.address && address < region.address + bytes.length)
        mutate(bytes.subarray(address - region.address));
      return { ...region, bytes };
    });
  }
  it("binds trainer resolution to coherent settings and the highest non-egg level, including fainted and sixth-slot Pokémon", () => {
    expect(trainerContext()).toMatchObject({
      status: "ready",
      data: {
        difficulty: 0,
        evsEnabled: true,
        highestLevel: 20,
        hellLevelBonus: 0,
      },
    });
    const memory = createMemoryReader(regions);
    const sb1 = memory.u32(0x0300340c),
      sb2 = memory.u32(0x03003410);
    const input = changed(0x02000000, (bytes) => {
      const view = new DataView(
        bytes.buffer,
        bytes.byteOffset,
        bytes.byteLength,
      );
      const first = 0x39908;
      bytes.set(bytes.slice(first, first + 0x4c), first + 5 * 0x4c);
      bytes[first + 5 * 0x4c + 0x3c] = 57;
      view.setUint16(first + 5 * 0x4c + 0x3e, 0, true); // Fainted still sets scaling level.
      bytes[first + 0x4c + 0x3c] = 99;
      view.setUint32(
        first + 0x4c + 20,
        view.getUint32(first + 0x4c + 20, true) | 0x8000,
        true,
      ); // Egg must not.
      bytes[sb2 - 0x02000000 + 0xc89] |= 0x30;
      bytes[sb2 - 0x02000000 + 0xc8c] &= ~4;
      bytes[sb1 - 0x02000000 + 0x1ae1] |= 8;
    });
    const result = trainerContext(input);
    expect(result).toMatchObject({
      status: "ready",
      data: {
        difficulty: 3,
        evsEnabled: false,
        highestLevel: 57,
        hellLevelBonus: 5,
      },
    });
    if (result.status !== "ready") throw new Error("Expected ready context");
    const trainers = resources.searchTrainers("roxanne initial", result.data);
    if (trainers.status !== "ready") throw new Error(trainers.reason);
    // The trainers view binds the same search to the live context.
    const view = decode(input).trainers;
    if (view?.status !== "ready") throw new Error("Expected trainer search");
    expect(view.data("roxanne initial")).toEqual(trainers);
    expect(trainers.data[0].averageLevel).toBe(62);
    expect(Object.values(trainers.data[0].team[0].stats.evs)).toEqual([
      0, 0, 0, 0, 0, 0,
    ]);
    const portrait = resources.artwork(trainers.data[0].portrait!);
    expect([portrait.width, portrait.height]).toEqual([64, 64]);
    expect(portrait.pixels.some((value) => value !== 0)).toBe(true);
    expect(trainerContext()).toMatchObject({
      status: "ready",
      data: { highestLevel: 20, difficulty: 0 },
    });
  });
  it("reads the real party, trainer battle, ROM descriptions and artwork", () => {
    const result = decode();
    expect(
      result.battle.status,
      result.battle.status === "error"
        ? result.battle.message
        : "battle is ready",
    ).toBe("ready");
    expect(result.party).toMatchObject({
      status: "ready",
      data: {
        party: [
          { speciesName: "Frigibax", hp: 62, maxHp: 62, level: 20 },
          { speciesName: "Klink" },
          { speciesName: "Aron" },
          { speciesName: "Goomy" },
        ],
      },
    });
    expect(result.battle).toMatchObject({
      status: "ready",
      data: {
        active: true,
        kind: "trainer",
        playerParty: [
          { species: 996 },
          { species: 599 },
          { species: 304 },
          { species: 704 },
        ],
        opponentParty: [{ species: 258, hp: 58 }, { species: 704 }],
        battlers: [
          {
            hp: 62,
            maxHp: 62,
            abilities: [{ id: 314 }, { id: 575 }, { id: 85 }, { id: 492 }],
          },
          { speciesName: "Mudkip", hp: 58, maxHp: 58 },
        ],
      },
    });
    expect(result.storage.status).toBe("ready");
    if (result.party.status === "ready")
      for (const mon of result.party.data.party) {
        expect(mon.warnings).toEqual([]);
        expect(resources.artwork(mon.artwork!.front).pixels).toHaveLength(
          64 * 64 * 4,
        );
        expect(resources.artwork(mon.artwork!.icon).pixels).toHaveLength(
          32 * 32 * 4,
        );
      }
    const masterBall = resources.catalog.item(1);
    expect(masterBall).toMatchObject({
      id: 1,
      name: "Master Ball",
      artwork: { kind: "item", item: 1, romHash: sample.romHash },
    });
    const itemImage = resources.artwork(masterBall.artwork!);
    expect(itemImage).toMatchObject({ width: 24, height: 24 });
    expect(
      itemImage.pixels.some((alpha, index) => index % 4 === 3 && alpha),
    ).toBe(true);
    expect(resources.catalog.ability(575).description).toContain("Fire");
    expect(resources.catalog.move(1)).toMatchObject({
      name: "Pound",
      pp: 20,
      power: 40,
      category: "physical",
    });
  });
  it("does not report stale battle RAM when gMain says overworld", () => {
    expect(
      decode(changed(0x03003859, (bytes) => (bytes[0] &= ~2))).battle,
    ).toMatchObject({ status: "ready", data: { active: false } });
  });
  it("reads the final storage slot with absent current PP, and retains authoritative live abilities in randomized mode", () => {
    const modified = changed(0x02000000, (bytes) => {
      const box = bytes.slice(0x39908, 0x39908 + 0x34);
      bytes.set(box, 0x2dc08 + 4 + (25 * 30 + 29) * 0x34);
      bytes[0x29c48 + 0xc8a] |= 0xa8;
    });
    const result = decode(modified);
    expect(result.storage.status).toBe("ready");
    if (result.storage.status === "ready")
      expect(result.storage.data.boxes[25].pokemon.at(-1)).toMatchObject({
        slot: 29,
        species: 996,
        abilities: null,
        types: null,
        stats: { actual: null },
        moves: [{ pp: null }, { pp: null }, { pp: null }, null],
      });
    expect(result.party).toMatchObject({
      status: "ready",
      data: {
        party: [
          { abilities: null, types: null },
          { abilities: null },
          { abilities: null },
          { abilities: null },
        ],
      },
    });
    expect(result.battle.status).toBe("ready");
    if (result.battle.status === "ready" && result.battle.data.active) {
      expect(
        result.battle.data.battlers[0].abilities!.map((a) => a.id),
      ).toEqual([314, 575, 85, 492]);
      expect(result.battle.data.battlers[0].warnings).toEqual([]);
    }
  });
  it("distinguishes persistent suppression, shield effect and Embargo", () => {
    const states = (input: readonly MemoryRegion[]) => {
      const battle = decode(input).battle;
      expect(battle.status).toBe("ready");
      if (battle.status !== "ready" || !battle.data.active)
        throw new Error("No battle");
      return battle.data.battlers[0].abilities!.map((a) => a.state);
    };
    expect(states(regions)).toEqual([
      "available",
      "available",
      "available",
      "available",
    ]);
    const scenario = (
      acid: boolean,
      gas: boolean,
      item: number,
      embargo: boolean,
    ) =>
      changed(0x02000000, (bytes) => {
        const v = new DataView(bytes.buffer, bytes.byteOffset);
        v.setUint32(
          0x1c7ac,
          (acid ? 1 << 16 : 0) | (embargo ? 1 << 17 : 0),
          true,
        );
        bytes[0x1cb69] = gas ? 1 : 0;
        v.setUint16(0x1c50c + 0x3a, item, true);
      });
    expect(states(scenario(true, false, 0, false))).toEqual(
      Array(4).fill("suppressed"),
    );
    expect(states(scenario(false, true, 0, false))).toEqual(
      Array(4).fill("suppressed"),
    );
    expect(states(scenario(true, true, 411, false))).toEqual(
      Array(4).fill("available"),
    );
    expect(states(scenario(true, false, 411, true))).toEqual(
      Array(4).fill("suppressed"),
    );
    const immune = scenario(true, false, 0, true).map((region) => {
      const bytes = region.bytes.slice();
      if (region.address === 0x02000000)
        new DataView(bytes.buffer).setUint16(0x1c50c + 0x28, 121, true); // Multitype is unsuppressable.
      return { ...region, bytes };
    });
    expect(states(immune)).toEqual([
      "available",
      "suppressed",
      "suppressed",
      "suppressed",
    ]);
  });
  it.each([
    [0x20, "Freeze"],
    [0x1000, "Frostbite"],
    [0x2000, "Bleed"],
  ] as const)("distinguishes Redux ailment %s as %s", (status, name) => {
    const battle = decode(
      changed(0x0201c50c + 8, (bytes) =>
        new DataView(bytes.buffer, bytes.byteOffset).setUint32(0, status, true),
      ),
    ).battle;
    expect(battle.status).toBe("ready");
    if (battle.status === "ready" && battle.data.active)
      expect(battle.data.battlers[0].ailments).toEqual([name]);
  });
  it("rejects broken linkage and illegal catalog IDs instead of using stale data", () => {
    expect(
      decode(changed(0x0201c50c + 4, (bytes) => (bytes[0] ^= 1))).battle.status,
    ).toBe("error");
    expect(
      decode(
        changed(0x0201c50c + 0x28, (bytes) => {
          bytes[0] = 0xff;
          bytes[1] = 0xff;
        }),
      ).battle.status,
    ).toBe("error");
    expect(
      decode(changed(0x02039905, (bytes) => (bytes[0] = 7))).party.status,
    ).toBe("error");
  });
});
