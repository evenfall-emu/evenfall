import { describe, expect, it } from "vitest";
import { defineGame, ref, u8 } from "@evenfall/gba/bridge";
import { createGameRegistry, type GamePlugin } from "../src/games";

const game = (id: string, romHash: string) =>
  defineGame({
    id,
    profiles: [
      {
        romHash,
        source: {
          repository: "synthetic",
          revision: "fixture",
          variant: "test",
          compiler: "none",
          options: [],
        },
        symbols: { value: ref(0x02000000, u8) },
      },
    ],
    bridges: {
      value: {
        id: "value",
        decode: ({ memory, symbols }) => ({
          status: "ready",
          data: memory.read(symbols.value),
        }),
      },
    },
  });

const first = game("fixture/first", "a".repeat(64));
const second = game("fixture/second", "b".repeat(64));
const FirstDashboard = () => null;
const SecondDashboard = () => null;
const plugins: GamePlugin[] = [
  {
    id: "first-package",
    games: [first],
    dashboards: async () => ({ [first.id]: FirstDashboard }),
  },
  {
    id: "second-package",
    games: [second],
    dashboards: async () => ({ [second.id]: SecondDashboard }),
  },
];

describe("game plugin registry", () => {
  it("selects layouts by exact build across installed plugins", async () => {
    const registry = createGameRegistry(plugins);
    expect(registry.layoutForRom("dashboards", "a".repeat(64))).toBeNull();
    await registry.load("dashboards");
    expect(registry.layoutForRom("dashboards", "a".repeat(64))).toEqual({
      game: first,
      Layout: FirstDashboard,
    });
    expect(registry.layoutForRom("dashboards", "b".repeat(64))?.Layout).toBe(
      SecondDashboard,
    );
    expect(registry.layoutForRom("dashboards", "c".repeat(64))).toBeNull();
    expect(registry.layoutForRom("dashboards", undefined)).toBeNull();
    expect(registry.gameForRom("b".repeat(64))).toBe(second);
  });

  it("leaves the generic player for a plugin without layouts", async () => {
    const third = game("fixture/third", "d".repeat(64));
    const registry = createGameRegistry([
      ...plugins,
      { id: "headless-package", games: [third] },
    ]);
    await registry.load("dashboards");
    expect(registry.gameForRom("d".repeat(64))).toBe(third);
    expect(registry.layoutForRom("dashboards", "d".repeat(64))).toBeNull();
    expect(registry.layoutForRom("dashboards", "a".repeat(64))?.Layout).toBe(
      FirstDashboard,
    );
  });

  it("works with no installed plugins", async () => {
    const registry = createGameRegistry([]);
    await registry.load("dashboards");
    expect(registry.layoutForRom("dashboards", "a".repeat(64))).toBeNull();
  });

  it("rejects ambiguous builds and layouts for foreign games", async () => {
    const clash = game("fixture/clash", "a".repeat(64));
    const ambiguous = createGameRegistry([
      ...plugins,
      { id: "clash", games: [clash] },
    ]);
    expect(() => ambiguous.gameForRom("a".repeat(64))).toThrow(
      "ROM matches multiple game definitions",
    );
    const foreign = createGameRegistry([
      {
        id: "foreign",
        games: [first],
        dashboards: async () => ({ [second.id]: SecondDashboard }),
      },
    ]);
    await expect(foreign.load("dashboards")).rejects.toThrow(
      "foreign has a layout for unknown game fixture/second",
    );
  });
});
