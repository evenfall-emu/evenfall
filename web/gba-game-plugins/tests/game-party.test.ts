import { describe, expect, it } from "vitest";
import {
  decodeGame,
  defineGame,
  array,
  createMemoryReader,
  ref,
  u8,
} from "@evenfall/gba/bridge";
import { gameParty, stockPartyPokemon } from "../src/game-party";
import { heartNSoul } from "../src/heart-n-soul";
import { pokemonResources } from "./fixtures/pokemon";

// Physical block orders, independently transcribed from the inverse of
// pret/pokefirered's GetSubstruct table (revision cited by the codec).
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

function record(
  order: number,
  isEgg = false,
  species = 25,
  hns = false,
): Uint8Array {
  const bytes = new Uint8Array(100);
  const view = new DataView(bytes.buffer);
  const personality = 0x7ffffff8 + order; // Base is divisible by 24; crosses bit 31.
  const owner = 0x12345678;
  view.setUint32(0, personality, true);
  view.setUint32(4, owner, true);
  bytes[0x13] = isEgg ? 6 : 2; // hasSpecies, optionally egg, never bad egg
  const blocks: Record<string, number[]> = {
    G: [species, 15, 0x3040, 0x1020, 0x4433, 0x6655],
    A: [100, 200, 300, 400, 0x0f0e, 0x1110],
    E: [0x0102, 0x0304, 0x0506, 0x0708, 0x090a, 0x0b0c],
    // Deliberately disagree on bits 30/31 in HnS records. Reusing the stock
    // egg mask must fail for BOTH an egg and an ordinary Pokémon.
    M: [
      0x1213,
      0x1415,
      0x5634,
      hns ? (isEgg ? 0x8123 : 0x4123) : isEgg ? 0xc123 : 0x8123,
      0x1819,
      0x1a1b,
    ],
  };
  const plain = new DataView(new ArrayBuffer(48));
  let checksum = 0;
  for (const [slot, kind] of [...orders[order]].entries()) {
    blocks[kind].forEach((word, i) => {
      plain.setUint16(slot * 12 + i * 2, word, true);
      checksum += word;
    });
  }
  view.setUint16(0x1c, checksum & 0xffff, true);
  for (let i = 0; i < 48; i += 4) {
    view.setUint32(
      0x20 + i,
      plain.getUint32(i, true) ^ personality ^ owner,
      true,
    );
  }
  view.setUint32(0x50, 0x000008, true);
  bytes[0x54] = 26;
  view.setUint16(0x56, 53, true);
  view.setUint16(0x58, 62, true);
  return bytes;
}

const profile = {
  romHash: "a".repeat(40),
  source: {
    repository: "synthetic",
    revision: "fixture",
    variant: "test",
    compiler: "none",
    options: [],
  },
  symbols: {
    gPlayerPartyCount: ref(0x02000000, u8),
    gPlayerParty: array(0x02000010, stockPartyPokemon, 6),
  },
};
const game = defineGame({
  id: "pokemon/fixture",
  profiles: [profile],
  bridges: { gameParty },
});
const sample = {
  romHash: profile.romHash,
  timeline: "fixture",
  sequence: 1,
  cycle: 1000n,
};

function hnsMemory(pokemon: Uint8Array, settings?: Uint8Array) {
  const pointer = new Uint8Array(4);
  new DataView(pointer.buffer).setUint32(0, 0x02030000, true);
  return createMemoryReader([
    { address: 0x0201d9e9, bytes: new Uint8Array([1]) },
    { address: 0x0201d9ec, bytes: pokemon },
    { address: 0x03003740, bytes: pointer },
    { address: 0x02030000, bytes: settings ?? new Uint8Array(0x3db8) },
  ]);
}

describe("Heart & Soul 1.2.1 party view", () => {
  it.each([false, true])(
    "uses HnS species, addresses and egg bits (egg=%s)",
    (isEgg) => {
      const memory = hnsMemory(record(17, isEgg, 412, true));
      const stamp = {
        ...sample,
        romHash: "a4be81f8330d7f223a3ab847c3da822fe6f8a2e0",
      };
      expect(
        decodeGame(heartNSoul, stamp, memory, pokemonResources()).party,
      ).toMatchObject({
        status: "ready",
        sample: stamp,
        data: {
          party: [
            {
              slot: 0,
              personality: 0x80000009,
              species: 412,
              level: 26,
              hp: 53,
              maxHp: 62,
              status: 8,
              isEgg,
            },
          ],
        },
      });
      expect(
        decodeGame(heartNSoul, sample, memory, pokemonResources()).party.status,
      ).toBe("unsupported");
    },
  );

  it.each([
    [455, "ready"],
    [456, "unavailable"],
    [461, "unavailable"],
    [462, "unavailable"],
  ] as const)(
    "bounds HnS species %i including reserved and egg sentinels",
    (species, status) => {
      const memory = hnsMemory(record(7, false, species, true));
      expect(
        decodeGame(
          heartNSoul,
          { ...sample, romHash: heartNSoul.profiles[0].romHash },
          memory,
          pokemonResources(),
        ).party.status,
      ).toBe(status);
    },
  );

  it("reads runtime flags through the actual HnS settings pointer", () => {
    const settings = new Uint8Array(0x3db8);
    settings[0x3d9d] = 0xa0; // randomized types and abilities
    settings[0x3da0] = 0x20; // base-stat equalizer
    settings[0x3da4] = 0x20; // shiny mode 2
    const result = decodeGame(
      heartNSoul,
      { ...sample, romHash: heartNSoul.profiles[0].romHash },
      hnsMemory(record(7, false, 25, true), settings),
      pokemonResources(),
    ).party;
    expect(result).toMatchObject({
      status: "ready",
      data: {
        party: [
          {
            stats: { base: null },
            types: null,
            abilities: null,
            warnings: expect.arrayContaining([
              "HnS randomized types are not resolved yet",
              "HnS randomized abilities are not resolved yet",
              "HnS base-stat equalizer is active; actual stats remain authoritative",
            ]),
          },
        ],
      },
    });
  });
});

describe("gameParty", () => {
  it.each(orders.map((order, index) => [order, index] as const))(
    "decodes encrypted party basics for physical block order %s",
    (_order, index) => {
      // Only the active slot is captured. Reading all six slots would fail.
      const bytes = new Uint8Array(116);
      bytes[0] = 1;
      bytes.set(record(index), 16);
      const result = decodeGame(
        game,
        sample,
        createMemoryReader([{ address: 0x02000000, bytes }]),
      );
      expect(result.gameParty).toEqual({
        status: "ready",
        sample,
        data: {
          party: [
            {
              slot: 0,
              personality: 0x7ffffff8 + index,
              species: 25,
              level: 26,
              hp: 53,
              maxHp: 62,
              status: 8,
              isEgg: false,
            },
          ],
        },
      });
      expect(bytes).toEqual(
        Uint8Array.from([1, ...new Array(15).fill(0), ...record(index)]),
      );
    },
  );

  it("distinguishes an empty party, six active slots and an invalid count", () => {
    const bytes = new Uint8Array(616);
    for (let slot = 0; slot < 6; slot++) {
      const pokemon = record(slot + 17, slot === 0);
      pokemon[0x56] = 53 - slot * 7;
      bytes.set(pokemon, 16 + slot * 100);
    }
    const decode = () =>
      decodeGame(
        game,
        sample,
        createMemoryReader([{ address: 0x02000000, bytes }]),
      ).gameParty;
    expect(decode()).toMatchObject({ status: "ready", data: { party: [] } });
    bytes[0] = 6;
    const full = decode();
    expect(full.status).toBe("ready");
    if (full.status === "ready") {
      expect(
        full.data.party.map((pokemon) => [
          pokemon.slot,
          pokemon.hp,
          pokemon.isEgg,
        ]),
      ).toEqual([
        [0, 53, true],
        [1, 46, false],
        [2, 39, false],
        [3, 32, false],
        [4, 25, false],
        [5, 18, false],
      ]);
    }
    bytes[0] = 7;
    expect(decode()).toMatchObject({
      status: "unavailable",
      reason: "Party count is outside its declared capacity",
    });
  });

  it.each([
    [0, "unavailable"],
    [1, "ready"],
    [100, "ready"],
    [101, "unavailable"],
  ] as const)(
    "validates level %i without rejecting fainted Pokémon",
    (level, status) => {
      const bytes = record(17);
      bytes[0x54] = level;
      bytes[0x56] = 0;
      const memory = createMemoryReader([{ address: 0x02000010, bytes }]);
      expect(memory.read(ref(0x02000010, stockPartyPokemon))).toMatchObject(
        status === "ready" ? { status, data: { hp: 0, level } } : { status },
      );
    },
  );

  it.each([
    [0, "unavailable"],
    [1, "ready"],
    [411, "ready"],
    [412, "unavailable"],
    [0xffff, "unavailable"],
  ] as const)(
    "validates stock internal species %i even with a valid checksum",
    (species, status) => {
      const memory = createMemoryReader([
        { address: 0x02000010, bytes: record(17, false, species) },
      ]);
      expect(memory.read(ref(0x02000010, stockPartyPokemon))).toMatchObject(
        status === "ready" ? { status, data: { species } } : { status },
      );
    },
  );

  it.each([
    [0x4f, 1, "Pokémon checksum mismatch"],
    [0x13, 1, "Empty or bad-egg record"],
    [0x13, 2, "Empty or bad-egg record"],
    [0x13, 4, "Party record is inconsistent or being updated"],
    [0x56, 0xff, "Party record is inconsistent or being updated"],
    [0x58, 62, "Party record is inconsistent or being updated"],
  ] as const)(
    "rejects corrupted bytes at %i (xor %i) rather than publishing partial data",
    (offset, xor, reason) => {
      const bytes = new Uint8Array(216);
      bytes[0] = 2;
      bytes.set(record(7), 16);
      bytes.set(record(17), 116);
      bytes[116 + offset] ^= xor;
      const result = decodeGame(
        game,
        sample,
        createMemoryReader([{ address: 0x02000000, bytes }]),
      );
      expect(result.gameParty).toEqual({
        status: "unavailable",
        sample,
        reason: `Party slot 2: ${reason}`,
      });
    },
  );
});
