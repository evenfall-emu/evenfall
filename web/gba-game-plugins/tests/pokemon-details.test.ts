import { describe, expect, it } from "vite-plus/test";
import {
  createMemoryReader,
  pointer,
  ref,
  type DecodeResult,
  type MemoryLayout,
} from "@evenfall/gba/bridge";
import { createGen3Catalog } from "../src/catalog";
import {
  gameStorage,
  pokemonDetails,
  pokemonStorageLayout,
} from "../src/bridges";
import type { BoxPokemon } from "../src/model";
import { pokemonResources, pokemonRules } from "./fixtures/pokemon";

const baseRecord: BoxPokemon = {
  personality: 128,
  trainerId: 0,
  species: 1,
  nickname: "Test",
  trainerName: "Owner",
  language: 2,
  textWarnings: [],
  heldItem: 0,
  experience: 550,
  friendship: 70,
  hiddenNature: null,
  moveIds: [1, 2, 3, 0],
  pp: [4, 5, 6, 0],
  ppBonuses: 0b11_10_01,
  ivs: {
    hp: 1,
    attack: 2,
    defense: 3,
    speed: 4,
    specialAttack: 5,
    specialDefense: 6,
  },
  evs: {
    hp: 6,
    attack: 5,
    defense: 4,
    speed: 3,
    specialAttack: 2,
    specialDefense: 1,
  },
  isEgg: false,
  abilitySlot: 1,
};

const details = (record: Partial<BoxPokemon> = {}, rules = pokemonRules) =>
  pokemonDetails(
    { ...baseRecord, ...record },
    pokemonResources().catalog,
    rules,
    "fixture",
  );

describe("Pokémon detail enrichment", () => {
  it("uses asymmetric legacy stats with per-stat zero fallback", () => {
    expect(
      details({}, { ...pokemonRules, legacyStats: true }).stats.base,
    ).toEqual({
      hp: 10,
      attack: 21,
      defense: 30,
      speed: 41,
      specialAttack: 50,
      specialDefense: 61,
    });
  });

  it("keeps move category, slot, and the slot-specific PP bonus", () => {
    expect(
      details().moves.map(
        (move) => move && [move.slot, move.category, move.maxPp],
      ),
    ).toEqual([
      [0, "special", 12],
      [1, "status", 28],
      [2, "physical", 48],
      null,
    ]);
  });

  it("honors hidden nature zero and otherwise derives nature", () => {
    expect(details({ hiddenNature: 0, personality: 24 }).nature).toMatchObject({
      id: 0,
    });
    expect(
      details({ hiddenNature: null, personality: 24 }).nature,
    ).toMatchObject({ id: 24 });
  });

  it("applies gender ratio boundaries", () => {
    expect(details({ personality: 127 }).gender).toBe("female");
    expect(details({ personality: 128 }).gender).toBe("male");
    expect(details({ species: 2 }).gender).toBe("female");
    expect(details({ species: 3 }).gender).toBe("genderless");
  });

  it("matches legacy abilities by both species and ability slot", () => {
    const rules = {
      ...pokemonRules,
      legacyAbilities: [{ species: 1, slot: 1 }],
    };
    expect(details({}, rules).abilities?.[0]).toMatchObject({
      id: 9,
      slot: 0,
      kind: "ability",
      state: "available",
    });
    expect(details({ abilitySlot: 0 }, rules).abilities?.[0].id).toBe(6);
    expect(details({ species: 2 }, rules).abilities?.[0].id).toBe(7);
  });

  it("uses the strict shiny threshold and personality-zero rule", () => {
    const rules = {
      ...pokemonRules,
      shinyThreshold: 8,
      shinyRequiresPersonality: true,
    };
    expect(details({ personality: 7 }, rules).shiny).toBe(true);
    expect(details({ personality: 8 }, rules).shiny).toBe(false);
    expect(details({ personality: 0 }, rules).shiny).toBe(false);
  });
});

describe("Gen III catalog", () => {
  it("reads fork fields at their declared asymmetric offsets and checks bounds", () => {
    const bytes = new Uint8Array(0x800).fill(0xff);
    const view = new DataView(bytes.buffer);
    const name = (offset: number) => bytes.set([0xbb, 0xff], offset); // A
    name(0x000);
    name(0x100);
    name(0x300);
    name(0x320);
    name(0x340);
    name(0x360);
    bytes.fill(0, 0x180, 0x18c);
    bytes[0x187] = 0xf9; // Signed -7, not unsigned 249.
    bytes[0x189] = 2;
    view.setUint32(0x360 + 20, 0x700, true);
    bytes.set([0xbb, 0xfe, 0xbc, 0xff], 0x700); // A newline B; terminator at bound.
    bytes.set([1, 2, 3, 4, 5, 6], 0x200);
    bytes.set([11, 12, 13, 14, 15, 16], 0x200 + 26);
    bytes.set([1, 2], 0x206);
    bytes.set([6, 7], 0x216);
    bytes[0x210] = 128;
    bytes[0x213] = 0;
    bytes.set([3, 4], 0x200 + 32);
    bytes.set([8, 9], 0x200 + 34);
    bytes.set([5, 6], 0x200 + 36);
    view.setUint32(0x400, 0x300, true);
    const table = (address: number, count: number, stride: number) => ({
      address,
      count,
      stride,
    });
    const catalog = createGen3Catalog(
      createMemoryReader([{ address: 0, bytes }]),
      {
        speciesNames: table(0, 1, 8),
        species: table(0x200, 1, 40),
        moveNames: table(0x100, 1, 8),
        moves: table(0x180, 1, 12),
        abilities: table(0x320, 1, 8),
        types: table(0x340, 1, 8),
        natures: table(0x400, 1, 4),
        items: table(0x360, 1, 44),
        experience: table(0x500, 1, 404),
        categoryOffset: 9,
        priorityOffset: 7,
        itemDescriptionOffset: 20,
        itemDescriptionMaxLength: 4,
        legacyStatsOffset: 26,
        legacyTypesOffset: 32,
        legacyAbilitiesOffset: 34,
        alternateTypesOffset: 36,
      },
    );
    expect(catalog.species(0)).toMatchObject({
      base: { hp: 1, specialDefense: 6 },
      legacyBase: { hp: 11, specialDefense: 16 },
      legacyTypes: [3, 4],
      legacyAbilities: [8, 9],
      alternateTypes: [5, 6],
    });
    expect(catalog.move(0)).toMatchObject({ priority: -7, category: "status" });
    expect(catalog.item(0)).toMatchObject({
      description: "A\nB",
      warnings: [],
    });
    expect(() => catalog.species(1)).toThrow(RangeError);
    expect(() => catalog.move(-1)).toThrow(RangeError);
    expect(() => catalog.nature(1)).toThrow(RangeError);
    expect(() => catalog.experience(0, 101)).toThrow(RangeError);
  });

  it("distinguishes zero-based abilities from one-based moves and bounds description text", () => {
    const bytes = new Uint8Array(0x200);
    const view = new DataView(bytes.buffer);
    bytes.set([0xbb, 0xff, 0xbc, 0xff, 0xbd, 0xff]); // Names A, B, C.
    [0x180, 0x184, 0x186].forEach((address, id) =>
      view.setUint32(0x100 + id * 4, address, true),
    );
    [0x190, 0x194].forEach((address, id) =>
      view.setUint32(0x120 + id * 4, address, true),
    );
    bytes.set([0xbb, 0xfe, 0xbc, 0xff], 0x180); // Terminates exactly at the bound.
    bytes.set([0xfd, 0xff], 0x184); // Unsupported control.
    bytes.set([0xbb, 0xbc, 0xbd, 0xbe, 0xff], 0x186); // Terminator beyond the bound.
    bytes.set([0xbd, 0xff], 0x190);
    bytes.set([0xbe, 0xb4, 0xff], 0x194);
    const names = { address: 0, count: 3, stride: 2 };
    const config = {
      speciesNames: names,
      species: names,
      moveNames: names,
      moves: { address: 0x40, count: 3, stride: 12 },
      abilities: names,
      types: names,
      natures: names,
      items: names,
      experience: names,
      categoryOffset: null,
      abilityDescriptions: {
        address: 0x100,
        count: 3,
        stride: 4,
        firstId: 0,
        maxLength: 4,
      },
      moveDescriptions: {
        address: 0x120,
        count: 2,
        stride: 4,
        firstId: 1,
        maxLength: 4,
      },
    };
    const reader = () => createMemoryReader([{ address: 0, bytes }]);
    const catalog = createGen3Catalog(reader(), config);
    expect(catalog.ability(0)).toMatchObject({
      description: "A\nB",
      warnings: [],
    });
    expect(catalog.ability(1)).toMatchObject({
      description: null,
      warnings: ["Unsupported Gen III control code 0xFD"],
    });
    expect(catalog.ability(2)).toMatchObject({
      description: null,
      warnings: ["Text exceeds verified 4-byte bound"],
    });
    expect(catalog.move(0).description).toBeNull();
    expect(catalog.move(1).description).toBe("C");
    expect(catalog.move(2).description).toBe("D’");
    expect(() => catalog.move(3)).toThrow(RangeError);
    expect(
      createGen3Catalog(reader(), {
        ...config,
        abilityDescriptions: undefined,
      }).ability(0),
    ).toMatchObject({ description: null });
    view.setUint32(0x120, 0x200, true);
    expect(() => createGen3Catalog(reader(), config).move(1)).toThrow();
  });
});

describe("storage geometry", () => {
  const record: MemoryLayout<DecodeResult<BoxPokemon>> = {
    byteLength: 80,
    decode(memory, address) {
      return memory.u8(address) === 0xee
        ? { status: "unavailable", reason: "corrupt fixture" }
        : {
            status: "ready",
            data: { ...baseRecord, species: memory.u8(address) },
          };
    },
  };
  const layout = pokemonStorageLayout(record, {
    byteLength: 330,
    boxesOffset: 4,
    boxCount: 2,
    capacity: 2,
    namesOffset: 324,
    nameLength: 3,
  });

  it("moves through boxes by record capacity and preserves only occupied slots", () => {
    const bytes = new Uint8Array(330);
    bytes[4 + 0x13] = 2;
    bytes[4] = 11;
    bytes[4 + 3 * 80 + 0x13] = 2;
    bytes[4 + 3 * 80] = 22;
    bytes.set([0xbb, 0xff, 0xff, 0xbc, 0xff, 0xff], 324);
    const result = createMemoryReader([{ address: 0x1000, bytes }]).read(
      ref(0x1000, layout),
    );
    expect(result).toMatchObject({
      status: "ready",
      data: {
        currentBox: 0,
        boxes: [
          {
            box: 0,
            name: "A",
            pokemon: [{ slot: 0, record: { species: 11 } }],
          },
          {
            box: 1,
            name: "B",
            pokemon: [{ slot: 1, record: { species: 22 } }],
          },
        ],
      },
    });
    if (result.status === "ready")
      expect("hp" in result.data.boxes[0].pokemon[0].record).toBe(false);
  });

  it("reports occupied storage without inventing party-only HP", () => {
    const bytes = new Uint8Array(330);
    bytes[4] = 1;
    bytes[4 + 0x13] = 2;
    bytes.set([0xbb, 0xff], 324);
    const roots = new Uint8Array(8);
    const rootsView = new DataView(roots.buffer);
    rootsView.setUint32(0, 0x1000, true);
    rootsView.setUint32(4, 0x2000, true);
    const rulesLayout: MemoryLayout<typeof pokemonRules> = {
      byteLength: 1,
      decode: () => pokemonRules,
    };
    const memory = createMemoryReader([
      { address: 0, bytes: roots },
      { address: 0x1000, bytes },
      { address: 0x2000, bytes: new Uint8Array(1) },
    ]);
    const result = gameStorage.decode({
      memory,
      symbols: {
        storage: ref(0, pointer(layout)),
        rules: ref(4, pointer(rulesLayout)),
      },
      sample: { romHash: "fixture", timeline: "t", sequence: 1, cycle: 1n },
      resources: pokemonResources(),
    });
    expect(result).toMatchObject({
      status: "ready",
      data: { occupied: 1 },
    });
    if (result.status === "ready") {
      expect(result.data.boxes[0].pokemon[0]).toMatchObject({ species: 1 });
      expect("hp" in result.data.boxes[0].pokemon[0]).toBe(false);
    }
  });

  it("rejects current-box overflow and corrupt occupied records", () => {
    const overflow = new Uint8Array(330);
    overflow[0] = 2;
    expect(
      createMemoryReader([{ address: 0, bytes: overflow }]).read(
        ref(0, layout),
      ),
    ).toMatchObject({ status: "unavailable" });
    const corrupt = new Uint8Array(330);
    corrupt[4] = 0xee;
    corrupt[4 + 0x13] = 2;
    expect(
      createMemoryReader([{ address: 0, bytes: corrupt }]).read(ref(0, layout)),
    ).toEqual({
      status: "unavailable",
      reason: "Box 1, slot 1: corrupt fixture",
    });
  });
});
