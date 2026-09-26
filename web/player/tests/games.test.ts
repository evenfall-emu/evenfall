import { describe, expect, it, vi } from "vitest";
import { defineGame, locatable, ref, u8 } from "@evenfall/gba/bridge";
import {
  createGameRegistry,
  watchBuild,
  type BuildCache,
  type GamePlugin,
  type ViewStore,
} from "../src/games";
import {
  reviewedHash,
  runtimeFamily,
  runtimeGame,
  runtimeRom,
} from "./locate-fixture";

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

const first = game("fixture/first", "a".repeat(40));
const second = game("fixture/second", "b".repeat(40));
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
    expect(registry.layoutForRom("dashboards", "a".repeat(40))).toBeNull();
    await registry.load("dashboards");
    expect(registry.layoutForRom("dashboards", "a".repeat(40))).toEqual({
      game: first,
      Layout: FirstDashboard,
    });
    expect(registry.layoutForRom("dashboards", "b".repeat(40))?.Layout).toBe(
      SecondDashboard,
    );
    expect(registry.layoutForRom("dashboards", "c".repeat(40))).toBeNull();
    expect(registry.layoutForRom("dashboards", undefined)).toBeNull();
    expect(registry.gameForRom("b".repeat(40))).toBe(second);
  });

  it("leaves the generic player for a plugin without layouts", async () => {
    const third = game("fixture/third", "d".repeat(40));
    const registry = createGameRegistry([
      ...plugins,
      { id: "headless-package", games: [third] },
    ]);
    await registry.load("dashboards");
    expect(registry.gameForRom("d".repeat(40))).toBe(third);
    expect(registry.layoutForRom("dashboards", "d".repeat(40))).toBeNull();
    expect(registry.layoutForRom("dashboards", "a".repeat(40))?.Layout).toBe(
      FirstDashboard,
    );
  });

  it("works with no installed plugins", async () => {
    const registry = createGameRegistry([]);
    await registry.load("dashboards");
    expect(registry.layoutForRom("dashboards", "a".repeat(40))).toBeNull();
  });

  it("rejects ambiguous builds and layouts for foreign games", async () => {
    const clash = game("fixture/clash", "a".repeat(40));
    const ambiguous = createGameRegistry([
      ...plugins,
      { id: "clash", games: [clash] },
    ]);
    expect(() => ambiguous.gameForRom("a".repeat(40))).toThrow(
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

function memoryCache() {
  const entries = new Map<string, unknown>();
  const cache: BuildCache = {
    get: async (key) => entries.get(key),
    set: async (key, value) => void entries.set(key, value),
    delete: async (key) => void entries.delete(key),
  };
  return { entries, cache };
}

describe("build recognition", () => {
  const childHash = "f".repeat(40);
  const child = runtimeRom(0x02000100, 0x40);
  const plugin = (game = runtimeGame()): GamePlugin => ({
    id: "runtime-package",
    games: [game],
  });

  it("locates a build no reviewed manifest matches, and caches it", async () => {
    const game = runtimeGame();
    const registry = createGameRegistry([plugin(game), ...plugins]);
    const { entries, cache } = memoryCache();
    const onLocating = vi.fn();
    expect(registry.gameForRom(childHash)).toBeNull();
    expect(
      await registry.recognize(childHash, async () => child, {
        cache,
        onLocating,
      }),
    ).toEqual({ status: "located", game });
    expect(onLocating).toHaveBeenCalledWith(game);
    expect(registry.gameForRom(childHash)).toBe(game);
    expect(registry.buildInfo(childHash)).toEqual({
      provenance: "located",
      review: [],
    });
    expect(registry.buildInfo(reviewedHash)).toEqual({
      provenance: "reviewed",
      review: [],
    });
    expect([...entries.keys()]).toEqual([
      `${runtimeFamily.id}:${childHash}:${locatable(game)!.version}`,
    ]);
    // Known builds answer without reading the ROM.
    const unread = () => Promise.reject(new Error("read"));
    expect(await registry.recognize(childHash, unread)).toEqual({
      status: "located",
      game,
    });
    expect(await registry.recognize(reviewedHash, unread)).toEqual({
      status: "reviewed",
      game,
    });
  });

  it("recognizes a cached build in a later session without reading the ROM", async () => {
    const { cache } = memoryCache();
    await createGameRegistry([plugin()]).recognize(
      childHash,
      async () => child,
      {
        cache,
      },
    );
    const game = runtimeGame();
    const registry = createGameRegistry([plugin(game)]);
    const readRom = vi.fn(async () => child);
    expect(await registry.recognize(childHash, readRom, { cache })).toEqual({
      status: "cached",
      game,
    });
    expect(readRom).not.toHaveBeenCalled();
    expect(registry.gameForRom(childHash)).toBe(game);
  });

  it("drops a cached build its game no longer accepts, and locates it again", async () => {
    const game = runtimeGame();
    const { entries, cache } = memoryCache();
    const key = `${runtimeFamily.id}:${childHash}:${locatable(game)!.version}`;
    entries.set(key, { manifest: { family: "other" }, review: [] });
    expect(
      await createGameRegistry([plugin(game)]).recognize(
        childHash,
        async () => child,
        { cache },
      ),
    ).toEqual({ status: "located", game });
    expect(entries.get(key)).toMatchObject({
      manifest: { family: runtimeFamily.id },
    });
  });

  it("forgets a failing build and does not locate it again this session", async () => {
    const game = runtimeGame();
    const registry = createGameRegistry([plugin(game)]);
    const { entries, cache } = memoryCache();
    await registry.recognize(childHash, async () => child, { cache });
    await registry.forget(childHash, cache);
    await registry.forget(reviewedHash, cache);
    expect(registry.gameForRom(childHash)).toBeNull();
    expect(registry.gameForRom(reviewedHash)).toBe(game);
    expect(entries.size).toBe(0);
    const readRom = vi.fn(async () => child);
    expect(await registry.recognize(childHash, readRom, { cache })).toEqual({
      status: "unsupported",
    });
    expect(readRom).not.toHaveBeenCalled();
  });

  it("leaves other ROMs to the generic player", async () => {
    const registry = createGameRegistry([plugin(), ...plugins]);
    const { entries, cache } = memoryCache();
    const onLocating = vi.fn();
    expect(
      await registry.recognize(
        childHash,
        async () => runtimeRom(0x02000100, 0x40, "AXVE"),
        { cache, onLocating },
      ),
    ).toEqual({ status: "unsupported" });
    expect(onLocating).not.toHaveBeenCalled();
    expect(entries.size).toBe(0);
    expect(registry.buildInfo(childHash)).toBeUndefined();
    // Without locatable games there is nothing to read.
    const readRom = vi.fn(async () => child);
    expect(
      await createGameRegistry(plugins).recognize(childHash, readRom),
    ).toEqual({ status: "unsupported" });
    expect(readRom).not.toHaveBeenCalled();
  });
});

describe("located build watch", () => {
  function store() {
    const listeners = new Set<() => void>();
    let snapshot: Record<string, { status: string }> = {};
    const views: ViewStore = {
      subscribe: (listener) => {
        listeners.add(listener);
        return () => listeners.delete(listener);
      },
      getSnapshot: () => snapshot,
    };
    const publish = (next: Record<string, string>) => {
      snapshot = Object.fromEntries(
        Object.entries(next).map(([key, status]) => [key, { status }]),
      );
      listeners.forEach((listener) => listener());
    };
    return { views, publish, listeners };
  }

  it("fails once when a view errors on consecutive samples without ever decoding", async () => {
    const { views, publish, listeners } = store();
    const fail = vi.fn();
    watchBuild(views, fail, 3);
    publish({ party: "loading", media: "loading" });
    publish({ party: "error", media: "ready" });
    publish({ party: "unavailable", media: "ready" });
    publish({ party: "error", media: "ready" });
    publish({ party: "error", media: "ready" });
    await Promise.resolve();
    expect(fail).not.toHaveBeenCalled();
    publish({ party: "error", media: "ready" });
    publish({ party: "error", media: "ready" });
    await Promise.resolve();
    expect(fail).toHaveBeenCalledTimes(1);
    expect(listeners.size).toBe(0);
  });

  it("trusts a view that has decoded", async () => {
    const { views, publish } = store();
    const fail = vi.fn();
    const stop = watchBuild(views, fail, 2);
    publish({ party: "ready" });
    for (let i = 0; i < 4; i++) publish({ party: "error" });
    await Promise.resolve();
    expect(fail).not.toHaveBeenCalled();
    stop();
    stop();
  });
});
