import { describe, expect, it } from "vite-plus/test";
import { createMemoryReader, ref } from "@evenfall/gba/bridge";
import {
  gen3BoxPokemon,
  gen3PartyRecord,
  type Gen3RecordFormat,
} from "../src/gen3-record";
import { decodePokemonText } from "../src/text";

const orders = [
  "GAEM",
  "GAME",
  "GEAM",
  "GEMA",
  "GMAE",
  "GMEA",
  "AGEM",
  "AGME",
  "AEGM",
  "AEMG",
  "AMGE",
  "AMEG",
  "EGAM",
  "EGMA",
  "EAGM",
  "EAMG",
  "EMGA",
  "EMAG",
  "MGAE",
  "MGEA",
  "MAGE",
  "MAEG",
  "MEGA",
  "MEAG",
];

const stock: Gen3RecordFormat = {
  speciesLimit: 412,
  eggMask: 0x40000000,
  ivShift: 0,
  abilityOffset: 4,
  abilityShift: 31,
  hiddenNature: false,
};
const hns: Gen3RecordFormat = {
  speciesLimit: 456,
  eggMask: 0x80000000,
  ivShift: 1,
  abilityOffset: 8,
  abilityShift: 0,
  hiddenNature: true,
};

function fixture(
  order: number,
  format: Gen3RecordFormat,
  options: { egg?: boolean; nature?: number } = {},
): Uint8Array {
  const bytes = new Uint8Array(100).fill(0xff, 8, 18);
  const view = new DataView(bytes.buffer);
  const personality = 0x60000000 + order;
  const trainerId = 0x12345678;
  view.setUint32(0, personality, true);
  view.setUint32(4, trainerId, true);
  bytes.set([0xbb, 0xd5, 0xff], 8); // Aa
  bytes[18] = 2;
  bytes[19] = options.egg ? 6 : 2;
  bytes.set([0xbc, 0xd6, 0xff, 0xff, 0xff, 0xff, 0xff], 20); // Bb

  const blocks = Object.fromEntries(
    ["G", "A", "E", "M"].map((name) => [name, new Uint8Array(12)]),
  ) as Record<string, Uint8Array>;
  const growth = new DataView(blocks.G.buffer);
  growth.setUint16(0, 25, true);
  growth.setUint16(2, 0xf123, true);
  growth.setUint32(4, 0xfedcba98, true);
  blocks.G[8] = 0xe4;
  blocks.G[9] = 201;
  blocks.G[10] = options.nature ?? 26;
  const attacks = new DataView(blocks.A.buffer);
  [500, 600, 700, 800].forEach((move, index) =>
    attacks.setUint16(index * 2, move, true),
  );
  blocks.A.set([41, 42, 43, 44], 8);
  blocks.E.set([1, 3, 5, 7, 9, 11]);
  const misc = new DataView(blocks.M.buffer);
  const ivs = [2, 4, 6, 8, 10, 12];
  let ivWord = 0;
  ivs.forEach((iv, index) => (ivWord |= iv << (format.ivShift + index * 5)));
  if (options.egg) ivWord |= format.eggMask;
  if (format.abilityOffset === 4) ivWord |= 1 << format.abilityShift;
  misc.setUint32(4, ivWord >>> 0, true);
  if (format.abilityOffset === 8)
    misc.setUint32(8, 1 << format.abilityShift, true);

  const plain = new Uint8Array(48);
  [...orders[order]].forEach((kind, slot) =>
    plain.set(blocks[kind], slot * 12),
  );
  let checksum = 0;
  const plainView = new DataView(plain.buffer);
  for (let offset = 0; offset < 48; offset += 2)
    checksum = (checksum + plainView.getUint16(offset, true)) & 0xffff;
  view.setUint16(28, checksum, true);
  const key = personality ^ trainerId;
  for (let offset = 0; offset < 48; offset += 4)
    view.setUint32(
      32 + offset,
      (plainView.getUint32(offset, true) ^ key) >>> 0,
      true,
    );

  view.setUint32(80, 0x1234, true);
  bytes[84] = 50;
  view.setUint16(86, 0, true);
  [101, 103, 107, 109, 113, 127].forEach((stat, index) =>
    view.setUint16(88 + index * 2, stat, true),
  );
  return bytes;
}

function decode(bytes: Uint8Array, format = stock, party = false) {
  const layout = party ? gen3PartyRecord(format) : gen3BoxPokemon(format);
  return createMemoryReader([{ address: 0x2000000, bytes }]).read(
    ref(0x2000000, layout),
  );
}

describe("Gen III full records", () => {
  it.each(orders.map((order, index) => [order, index] as const))(
    "decodes physical permutation %s",
    (_order, index) => {
      expect(decode(fixture(index, stock))).toMatchObject({
        status: "ready",
        data: {
          personality: 0x60000000 + index,
          trainerId: 0x12345678,
          species: 25,
          nickname: "Aa",
          trainerName: "Bb",
          heldItem: 0xf123,
          experience: 0xfedcba98,
          moveIds: [500, 600, 700, 800],
          pp: [41, 42, 43, 44],
          ivs: {
            hp: 2,
            attack: 4,
            defense: 6,
            speed: 8,
            specialAttack: 10,
            specialDefense: 12,
          },
          evs: {
            hp: 1,
            attack: 3,
            defense: 5,
            speed: 7,
            specialAttack: 9,
            specialDefense: 11,
          },
          abilitySlot: 1,
        },
      });
    },
  );

  it("uses the distinct HnS IV, egg, ability, and hidden-nature fields", () => {
    expect(
      decode(fixture(17, hns, { egg: true, nature: 0 }), hns),
    ).toMatchObject({
      status: "ready",
      data: { isEgg: true, hiddenNature: 0, abilitySlot: 1, ivs: { hp: 2 } },
    });
    expect(decode(fixture(17, hns, { nature: 26 }), hns)).toMatchObject({
      status: "ready",
      data: { hiddenNature: null },
    });
    expect(decode(fixture(17, hns, { nature: 27 }), hns)).toMatchObject({
      status: "unavailable",
      reason: "Hidden nature is outside its verified range",
    });
  });

  it("reads fainted party HP and all five non-HP stat offsets", () => {
    expect(decode(fixture(3, stock), stock, true)).toMatchObject({
      status: "ready",
      data: {
        hp: 0,
        maxHp: 101,
        calculatedStats: {
          hp: 101,
          attack: 103,
          defense: 107,
          speed: 109,
          specialAttack: 113,
          specialDefense: 127,
        },
      },
    });
  });

  it("rejects checksum, empty, bad-egg, and egg-flag inconsistencies", () => {
    const checksum = fixture(0, stock);
    checksum[32] ^= 1;
    expect(decode(checksum)).toMatchObject({ status: "unavailable" });
    for (const flags of [0, 1, 2 | 1]) {
      const bytes = fixture(0, stock);
      bytes[19] = flags;
      expect(decode(bytes)).toMatchObject({ status: "unavailable" });
    }
    const badEggFlag = fixture(0, stock);
    badEggFlag[19] |= 4;
    expect(decode(badEggFlag)).toMatchObject({ status: "unavailable" });
  });
});

describe("Gen III text", () => {
  it("stops at EOS but accepts a full-width field without EOS", () => {
    expect(decodePokemonText(Uint8Array.of(0xbb, 0xd5, 0xff, 0xbc))).toEqual({
      text: "Aa",
      warnings: [],
    });
    expect(decodePokemonText(Uint8Array.of(0xbb, 0xbc))).toEqual({
      text: "AB",
      warnings: [],
    });
  });

  it("replaces unsupported glyphs, controls, and Japanese text with warnings", () => {
    expect(decodePokemonText(Uint8Array.of(0xbb, 0x31, 0xfc))).toEqual({
      text: "A��",
      warnings: [
        "Unsupported Gen III glyph 0x31",
        "Unsupported Gen III control code 0xFC",
      ],
    });
    expect(decodePokemonText(Uint8Array.of(1, 0xff), 1)).toEqual({
      text: "�",
      warnings: ["Japanese Gen III text is unsupported"],
    });
  });
});

describe("Gen III text encoding", () => {
  it("round-trips through the decoder, two-character glyphs included", async () => {
    const { encodePokemonText, decodePokemonText } =
      await import("../src/text");
    for (const text of ["BULBASAUR", "Mr. Mime", "PKMN Center", "Farfetch’d"]) {
      const bytes = encodePokemonText(text);
      expect(bytes.at(-1)).toBe(0xff);
      expect(decodePokemonText(bytes).text).toBe(text);
    }
    expect(encodePokemonText("PK")).toEqual(Uint8Array.from([0x53, 0xff]));
    expect(() => encodePokemonText("日本")).toThrow("No Gen III glyph");
  });
});
