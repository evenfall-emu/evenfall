import { readFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { beforeAll, describe, expect, it } from "vitest";
import { decodeGame, createMemoryReader } from "@evenfall/gba/bridge";
import { WasmCore, type MemorySnapshot } from "@evenfall/gba/core";
import { heartNSoul } from "../src/heart-n-soul";
import { gen3BattlePokemon } from "../src/gen3-record";
import { applyStatStages } from "../src/bridges";

it("applies signed stages with integer flooring, preserves HP and does not mutate raw stats", () => {
  const raw = Object.freeze({
    hp: 83,
    attack: 41,
    defense: 31,
    speed: 7,
    specialAttack: 53,
    specialDefense: 47,
  });
  const stages = {
    attack: 1,
    defense: 1,
    speed: -6,
    specialAttack: 6,
    specialDefense: -5,
    accuracy: 6,
    evasion: -6,
  };
  expect(applyStatStages(raw, stages)).toEqual({
    hp: 83,
    attack: 61,
    defense: 46,
    speed: 1,
    specialAttack: 212,
    specialDefense: 13,
  });
  expect(
    applyStatStages(raw, { ...stages, attack: -1, defense: -3, speed: 0 }),
  ).toMatchObject({
    attack: 27,
    defense: 12,
    speed: 7,
  });
  expect(raw.attack).toBe(41);
});

describe("Gen III live battle layout (synthetic)", () => {
  const layout = gen3BattlePokemon({
    speciesLimit: 456,
    moveLimit: 368,
    typeLimit: 19,
    abilityLimit: 82,
  });
  function fixture() {
    const bytes = new Uint8Array(0x58);
    const view = new DataView(bytes.buffer);
    view.setUint16(0, 455, true);
    [101, 202, 303, 404, 505].forEach((stat, i) =>
      view.setUint16(2 + i * 2, stat, true),
    );
    [367, 3, 0, 19].forEach((move, i) =>
      view.setUint16(0x0c + i * 2, move, true),
    );
    bytes.set([6, 0, 12, 1, 11, 5, 7, 6], 0x18);
    bytes[0x20] = 81;
    bytes.set([18, 2], 0x21);
    bytes.set([9, 8, 0, 6], 0x24);
    view.setUint16(0x28, 257, true);
    bytes[0x2a] = 100;
    view.setUint16(0x2c, 300, true);
    return bytes;
  }
  const decode = (bytes: Uint8Array) =>
    layout.decode(createMemoryReader([{ address: 0, bytes }]), 0);
  it("keeps asymmetric stat order, live PP and both signed-stage boundaries", () => {
    expect(decode(fixture())).toMatchObject({
      status: "ready",
      data: {
        species: 455,
        level: 100,
        hp: 257,
        maxHp: 300,
        stats: {
          hp: 300,
          attack: 101,
          defense: 202,
          speed: 303,
          specialAttack: 404,
          specialDefense: 505,
        },
        moveIds: [367, 3, 0, 19],
        pp: [9, 8, 0, 6],
        ability: 81,
        typeIds: [18, 2],
        statStages: {
          attack: -6,
          defense: 6,
          speed: -5,
          specialAttack: 5,
          specialDefense: -1,
          accuracy: 1,
          evasion: 0,
        },
      },
    });
  });
  it.each([
    ["species upper bound", 0, 456, 2],
    ["move upper bound", 0x0c, 368, 2],
    ["ability upper bound", 0x20, 82, 1],
    ["type upper bound", 0x21, 19, 1],
    ["stage overflow", 0x19, 13, 1],
    ["HP overflow", 0x28, 301, 2],
    ["zero max HP", 0x2c, 0, 2],
    ["zero level", 0x2a, 0, 1],
    ["level overflow", 0x2a, 101, 1],
  ] as const)("rejects %s", (_label, offset, value, width) => {
    const bytes = fixture();
    if (width === 2) new DataView(bytes.buffer).setUint16(offset, value, true);
    else bytes[offset] = value;
    expect(decode(bytes).status).toBe("unavailable");
  });
});

const hash = "a4be81f8330d7f223a3ab847c3da822fe6f8a2e0";
const sample = {
  romHash: hash,
  timeline: "battle-fixture",
  sequence: 1,
  cycle: 55_823_304_776n,
};

let battleRegions: MemorySnapshot["regions"];
let inactiveRegions: MemorySnapshot["regions"];
let resources: NonNullable<ReturnType<NonNullable<typeof heartNSoul.prepare>>>;

// Optional user-owned fixtures: a clean checkout must not require a private ROM.
const romPath = process.env.ZIGBA_HNS_ROM;
const battlePath = process.env.ZIGBA_HNS_BATTLE;
const inactivePath = process.env.ZIGBA_HNS_STATE;
describe.skipIf(!romPath || !battlePath || !inactivePath)(
  "Heart & Soul battle bridge",
  () => {
    beforeAll(async () => {
      const [wasm, rom, battle, inactive] = await Promise.all([
        readFile(new URL("../../../zig-out/web/zigba.wasm", import.meta.url)),
        readFile(romPath!),
        readFile(battlePath!),
        readFile(inactivePath!),
      ]);
      const sha = (bytes: Uint8Array) =>
        createHash("sha256").update(bytes).digest("hex");
      expect(createHash("sha1").update(rom).digest("hex")).toBe(hash);
      expect(sha(battle)).toBe(
        "5515f43a8119632270b2f64a447917fd826bc7cf16655f3276bbb0f3c4d42e54",
      );
      expect(sha(inactive)).toBe(
        "71817992f9053560808493c79c3e65b5acba2b4499375cc52d76806b2107fb5f",
      );
      const { instance } = (await WebAssembly.instantiate(
        wasm,
      )) as unknown as WebAssembly.WebAssemblyInstantiatedSource;
      const core = new WasmCore(instance.exports);
      try {
        core.load(rom);
        core.loadState(battle);
        battleRegions = core.captureMemory().regions;
        core.loadState(inactive);
        inactiveRegions = core.captureMemory().regions;
      } finally {
        core.dispose();
      }
      resources = heartNSoul.prepare!({
        rom: createMemoryReader([{ address: 0x08000000, bytes: rom }]),
        romHash: hash,
      });
    });

    function decode(regions = battleRegions) {
      return decodeGame(
        heartNSoul,
        sample,
        createMemoryReader(regions),
        resources,
      );
    }

    function changed(address: number, mutate: (bytes: Uint8Array) => void) {
      return battleRegions.map((region) => {
        const bytes = region.bytes.slice();
        if (
          address >= region.address &&
          address < region.address + bytes.length
        )
          mutate(bytes.subarray(address - region.address));
        return { address: region.address, bytes };
      });
    }

    it("decodes the supplied wild battle with ROM-backed details and live values", () => {
      const result = decode();
      expect(result.battle).toMatchObject({
        status: "ready",
        data: {
          active: true,
          kind: "wild",
          format: "single",
          typeFlags: 4,
          types: ["master"],
          turn: null,
          battleWeather: null,
          playerParty: [
            {
              slot: 0,
              species: 158,
              speciesName: "TOTODILE",
              level: 10,
              hp: 26,
              maxHp: 33,
            },
            { slot: 1, species: 16, speciesName: "PIDGEY" },
            { slot: 2, species: 187, speciesName: "HOPPIP" },
          ],
          opponentParty: [
            {
              slot: 0,
              species: 187,
              speciesName: "HOPPIP",
              level: 3,
              hp: 16,
              maxHp: 16,
              artwork: {
                icon: { species: 187, kind: "icon" },
                front: { species: 187, kind: "front" },
              },
            },
          ],
          battlers: [
            {
              battler: 0,
              side: "player",
              partySlot: 0,
              speciesName: "TOTODILE",
              hp: 26,
              maxHp: 33,
              stats: {
                actual: {
                  hp: 33,
                  attack: 18,
                  defense: 20,
                  speed: 16,
                  specialAttack: 17,
                  specialDefense: 17,
                },
              },
              statStages: {
                attack: 0,
                defense: 0,
                speed: 0,
                specialAttack: 0,
                specialDefense: 0,
                accuracy: 0,
                evasion: 0,
              },
              moves: [
                { name: "SCRATCH", pp: 30 },
                { name: "LEER", pp: 30 },
                { name: "WATER GUN", pp: 25 },
                null,
              ],
            },
            {
              battler: 1,
              side: "opponent",
              partySlot: 0,
              speciesName: "HOPPIP",
              hp: 16,
              maxHp: 16,
              moves: [{ name: "SPLASH", pp: 40 }, null, null, null],
            },
          ],
        },
      });
      const masterBall = resources.catalog.item(1);
      expect(masterBall).toMatchObject({
        id: 1,
        name: "MASTER BALL",
        artwork: { kind: "item", item: 1, romHash: hash },
      });
      const itemImage = resources.artwork(masterBall.artwork!);
      expect(itemImage).toMatchObject({ width: 24, height: 24 });
      expect(
        itemImage.pixels.some((alpha, index) => index % 4 === 3 && alpha),
      ).toBe(true);
      expect(result.context).toMatchObject({
        status: "ready",
        data: {
          arena: {
            title: "Route 29",
            fields: [
              { id: "battle", label: "Battle", value: "Wild battle" },
              { id: "opponent", label: "Opponent", value: "Wild Pokémon" },
              {
                id: "weather",
                label: "Weather",
                value: null,
                reason: "Live battle weather is not verified for this build",
              },
            ],
          },
          location: {
            mapGroup: 0,
            mapNumber: 11,
            x: 22,
            y: 13,
            regionMapSection: 0x58,
            name: "Route 29",
          },
          weather: { id: 0, name: "None" },
          warnings: [],
        },
      });
      expect(resources.catalog.move(98)).toMatchObject({
        name: "QUICK ATTACK",
        priority: 1,
        description: "An extremely fast attack\nthat always strikes first.",
        warnings: [],
      });
      expect(resources.catalog.ability(67)).toMatchObject({
        name: "TORRENT",
        description: "Ups WATER moves in a pinch.",
        warnings: [],
      });
      expect(resources.catalog.item(1)).toMatchObject({
        name: "MASTER BALL",
        description: "The best BALL that\ncatches a POKéMON\nwithout fail.",
        warnings: [],
      });
    });

    it("uses the verified gMain bit and ignores stale battle records when inactive", () => {
      const result = decode(inactiveRegions);
      expect(result.battle).toMatchObject({
        status: "ready",
        data: { active: false },
      });
      expect(result.context).toMatchObject({
        status: "ready",
        data: {
          arena: {
            fields: [{ id: "weather", label: "Map weather", value: "None" }],
          },
        },
      });
    });

    it("does not turn unverified trainer identity or live weather into map facts", () => {
      const result = decode(
        changed(0x020003a0, (bytes) => {
          bytes[0] = 8;
        }),
      ).context;
      expect(result).toMatchObject({
        status: "ready",
        data: {
          arena: {
            fields: [
              { id: "battle", value: "Trainer battle" },
              {
                id: "opponent",
                value: null,
                reason: expect.stringContaining("identity is not verified"),
              },
              {
                id: "weather",
                value: null,
                reason: expect.stringContaining("Live battle weather"),
              },
            ],
          },
        },
      });
    });

    it("includes both linked opponents in a synthetic double wild battle", () => {
      const result = decode(
        changed(0x02000000, (bytes) => {
          const view = new DataView(bytes.buffer, bytes.byteOffset);
          view.setUint32(0x3a0, 5, true); // Double + master.
          bytes[0x1420] = 4;
          bytes[0x15c8] = 4; // Player partner is absent.
          view.setUint16(0x1422 + 6, 1, true); // Battler 3 uses enemy slot 1.
          bytes.copyWithin(0x1de9c + 100, 0x1de9c, 0x1de9c + 100);
          bytes.copyWithin(0x143c + 3 * 0x58, 0x143c + 0x58, 0x143c + 2 * 0x58);
          view.setUint16(0x143c + 3 * 0x58 + 0x28, 12, true);
        }),
      ).battle;
      expect(result).toMatchObject({
        status: "ready",
        data: {
          active: true,
          kind: "wild",
          format: "double",
          opponentParty: [{ slot: 0 }, { slot: 1 }],
          battlers: [
            { battler: 0, side: "player" },
            { battler: 1, side: "opponent", partySlot: 0, hp: 16 },
            { battler: 3, side: "opponent", partySlot: 1, hp: 12 },
          ],
        },
      });
    });

    it("publishes asymmetric live stats and signed stages rather than party values", () => {
      const result = decode(
        changed(0x0200143c, (bytes) => {
          const view = new DataView(bytes.buffer, bytes.byteOffset);
          view.setUint16(2, 71, true);
          view.setUint16(4, 29, true);
          bytes[0x19] = 12;
          bytes[0x1a] = 1;
          bytes[0x1e] = 9;
          bytes[0x20] = 1;
          bytes[0x21] = 0;
          bytes[0x22] = 1;
        }),
      ).battle;
      expect(result.status).toBe("ready");
      if (result.status === "ready" && result.data.active)
        expect(result.data.battlers[0]).toMatchObject({
          stats: { actual: { attack: 71, defense: 29 } },
          statStages: { attack: 6, defense: -5, accuracy: 3 },
          stageAdjustedStats: { attack: 284, defense: 8 },
          types: [
            { id: 0, name: "NORMAL" },
            { id: 1, name: "FIGHT" },
          ],
          abilities: [
            {
              id: 1,
              name: "STENCH",
              slot: 0,
              kind: "ability",
              state: "available",
            },
          ],
        });
    });

    it("rejects stale party linkage and invalid stage bytes", () => {
      expect(
        decode(
          changed(0x0200143c + 0x48, (bytes) => {
            bytes[0] ^= 1;
          }),
        ).battle,
      ).toMatchObject({
        status: "unavailable",
        reason: "Battler 0 does not match its linked party record",
      });
      expect(
        decode(
          changed(0x0200143c + 0x19, (bytes) => {
            bytes[0] = 13;
          }),
        ).battle,
      ).toMatchObject({
        status: "unavailable",
        reason: "Battler 0: Battle Pokémon is inconsistent or being updated",
      });
    });
  },
);
