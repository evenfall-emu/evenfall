import { describe, expect, it } from "vite-plus/test";
import {
  decodeGame,
  defineGame,
  resolveGame,
  array,
  at,
  createMemoryReader,
  ref,
  u8,
  type BuildProfile,
  type BridgeSample,
} from "@evenfall/gba/bridge";

const profile = (romHash: string, address: number) => ({
  romHash,
  source: {
    repository: "synthetic",
    revision: "fixture",
    variant: "test",
    compiler: "none",
    options: [],
  },
  symbols: { count: ref(address, u8) },
});
const first = profile("a".repeat(40), 0x02000000);
const second: BuildProfile<typeof first.symbols> = profile(
  "b".repeat(40),
  0x02000002,
);
const game = defineGame({
  id: "pokemon/fixture",
  profiles: [first, second],
  bridges: {
    gameParty: {
      id: "gameParty",
      decode: ({ memory, symbols }) => ({
        status: "ready",
        data: { count: memory.read(symbols.count) },
      }),
    },
    combat: {
      id: "combat",
      decode: ({ memory, symbols }) => ({
        status: "ready",
        data: { active: memory.read(symbols.count) > 3 },
      }),
    },
  },
});

const sample: BridgeSample = {
  romHash: second.romHash,
  timeline: "fixture-timeline",
  sequence: 2,
  cycle: 1234n,
};

describe("per-game bridge dispatch", () => {
  it("selects one game/build and shares a captured sample across its bridges", () => {
    const memory = createMemoryReader([
      { address: 0x02000000, bytes: new Uint8Array([1, 0xff, 5]) },
    ]);
    expect(resolveGame([game], second.romHash)).toBe(game);
    expect(resolveGame([game], "c".repeat(40))).toBeNull();
    const result = decodeGame(game, sample, memory);
    expect(result.gameParty).toEqual({
      status: "ready",
      sample,
      data: { count: 5 },
    });
    expect(result.combat).toEqual({
      status: "ready",
      sample,
      data: { active: true },
    });
    expect(Object.isFrozen(result.gameParty)).toBe(true);
  });

  it("does not decode unsupported ROMs or silently pick an ambiguous game", () => {
    const memory = createMemoryReader([]);
    expect(
      decodeGame(game, { ...sample, romHash: "c".repeat(40) }, memory),
    ).toEqual({
      gameParty: { status: "unsupported", romHash: "c".repeat(40) },
      combat: { status: "unsupported", romHash: "c".repeat(40) },
    });
    const duplicate = defineGame({ ...game, id: "pokemon/other" });
    expect(() => resolveGame([game, duplicate], first.romHash)).toThrow(
      "multiple game",
    );
    expect(() => resolveGame([game, game], first.romHash)).toThrow(
      "Duplicate game id",
    );
    expect(() => defineGame({ ...game, profiles: [first, first] })).toThrow(
      "Duplicate build hash",
    );
    expect(() =>
      defineGame({ ...game, bridges: { typo: game.bridges.combat } }),
    ).toThrow("key must match");
  });

  it("isolates a failing bridge and publishes owned, deeply frozen data", () => {
    const scratch = { party: [{ hp: 53 }] };
    const collection = defineGame({
      id: "pokemon/isolation",
      profiles: [first],
      bridges: {
        broken: {
          id: "broken",
          decode() {
            throw new Error("Bad decoder");
          },
        },
        waiting: {
          id: "waiting",
          decode: () => ({ status: "unavailable", reason: "Booting" }),
        },
        healthy: {
          id: "healthy",
          decode: () => ({ status: "ready", data: scratch }),
        },
      },
    });
    const result = decodeGame(
      collection,
      { ...sample, romHash: first.romHash },
      createMemoryReader([]),
    );
    expect(result.broken).toMatchObject({
      status: "error",
      message: "Bad decoder",
    });
    expect(result.waiting).toMatchObject({
      status: "unavailable",
      reason: "Booting",
    });
    scratch.party[0].hp = 1;
    expect(result.healthy).toMatchObject({
      status: "ready",
      data: { party: [{ hp: 53 }] },
    });
    if (result.healthy.status === "ready") {
      expect(Object.isFrozen(result.healthy.data.party[0])).toBe(true);
    }
    expect(Object.isFrozen(scratch)).toBe(false);
  });

  it("rejects class instances instead of silently stripping their behavior", () => {
    class Counter {
      count = 3;
    }
    const collection = defineGame({
      id: "pokemon/class-data",
      profiles: [first],
      bridges: {
        counter: {
          id: "counter",
          decode: () => ({ status: "ready", data: new Counter() }),
        },
      },
    });
    expect(
      decodeGame(
        collection,
        { ...sample, romHash: first.romHash },
        createMemoryReader([]),
      ).counter,
    ).toMatchObject({
      status: "error",
      message: "Bridge data must contain plain records and arrays",
    });
  });
});

describe("captured-memory reader", () => {
  it("owns Buffer/subarray inputs and supports unaligned little-endian reads", () => {
    const source = Buffer.from([0xff, 0x12, 0x34, 0x56, 0x78, 0xee]);
    const memory = createMemoryReader([
      { address: 0x02000001, bytes: source.subarray(1, 5) },
    ]);
    source.fill(0);
    expect(memory.u32(0x02000001)).toBe(0x78563412);
    expect(memory.u16(0x02000002)).toBe(0x5634);
    const copied = memory.bytes(0x02000001, 4);
    copied.fill(0);
    expect(memory.u8(0x02000001)).toBe(0x12);
    const root = array(0x02000001, u8, 4);
    expect(memory.read(root)).toEqual([0x12, 0x34, 0x56, 0x78]);
    expect(memory.read(at(root, 3))).toBe(0x78);
    expect(() => at(root, 4)).toThrow("out of bounds");
  });

  it("rejects uncaptured, overlapping and wrapping ranges", () => {
    const memory = createMemoryReader([
      { address: 0x02000000, bytes: new Uint8Array(4) },
    ]);
    expect(() => memory.u16(0x02000003)).toThrow("Uncaptured memory");
    expect(() => memory.u32(0xffffffff)).toThrow("Invalid guest memory range");
    expect(() => memory.bytes(0x02000000, -1)).toThrow(
      "Invalid guest memory range",
    );
    expect(() => array(0xffffffff, u8, 2)).toThrow(
      "Invalid guest memory range",
    );
    expect(() =>
      createMemoryReader([
        { address: 0x02000000, bytes: new Uint8Array(4) },
        { address: 0x02000003, bytes: new Uint8Array(4) },
      ]),
    ).toThrow("Overlapping");
  });
});
