import type { ComponentType, ReactNode } from "react";
import {
  buildProvenance,
  locatable,
  locateGame,
  registerBuild,
  resolveGame,
  unregisterBuild,
  type GameIdentity,
  type Manifest,
} from "@evenfall/gba/bridge";
// Installed game packages. Each one is an optional dependency: removing its
// import, its entry in `installed` and its package.json dependency leaves the
// generic player. See docs/player.md#game-plugins.
import { pokemonPlugin } from "@evenfall/gba-pokemon/ui";

/** How the player knows the loaded build. */
export interface BuildInfo {
  /** Reviewed: a committed manifest. Located: recognized from the game's locators. */
  readonly provenance: "reviewed" | "located";
  /** For a located build, values found only by inference or kept from the template. */
  readonly review: readonly string[];
}

/** What the player gives a game layout: playback state and the player itself. */
export interface GameLayoutProps {
  readonly playing: boolean;
  /** The loaded build's provenance, so a layout can mark unreviewed builds. */
  readonly build?: BuildInfo;
  /** The self-contained player (canvas, controls); render it exactly once. */
  readonly children: ReactNode;
  /**
   * Set on phones, where the player owns the whole page. A layout then
   * returns `handheld(hud)` instead of placing `children`: the player places
   * the screen and controls, and shows the layout's HUD around them.
   */
  readonly handheld?: (hud: HandheldHud) => ReactNode;
  /**
   * Set on landscape pages narrower than 17:10 that are not phones, where
   * columns on both sides of the screen would narrow it. A layout then
   * returns `docked(dock)` instead of placing `children`: the player places
   * the heading and the screen in one column and the panels as tabs in one
   * column beside it.
   */
  readonly docked?: (dock: GameDock) => ReactNode;
}

/** One of a game layout's panels, shown as a tab. */
export interface GamePanel {
  readonly id: string;
  readonly label: string;
  readonly content: ReactNode;
}

/** A game layout's contribution to the phone player. */
export interface HandheldHud {
  /** One non-interactive line under the screen; tapping it opens `panels`. */
  readonly summary: ReactNode;
  /** Tabs of the sheet (portrait) or drawer (landscape), in order. */
  readonly panels: readonly GamePanel[];
}

/** A game layout's contribution to the two-column player. */
export interface GameDock {
  /** Above the screen, such as where the game takes place. */
  readonly heading?: ReactNode;
  /** Tabs of the column beside the screen, in order. */
  readonly panels: readonly GamePanel[];
  /**
   * The panel the game's state calls for, such as the battle while one
   * runs. The dock opens it whenever it changes; in between, the player's
   * own choice of tab stands.
   */
  readonly focus?: string;
}

/** Where recognized builds are kept between sessions (the library's `builds`). */
export interface BuildCache {
  get(key: string): Promise<unknown>;
  set(key: string, value: unknown): Promise<void>;
  delete(key: string): Promise<void>;
}

export type Recognition =
  | {
      readonly status: "reviewed" | "located" | "cached";
      readonly game: GameIdentity;
    }
  | { readonly status: "unsupported" };

/** A cached located build: its manifest and what to review in it. */
interface CachedBuild {
  readonly manifest: Manifest<any, any>;
  readonly review: readonly string[];
}

/** Layout components keyed by the `id` of one of the plugin's games. */
export type GameLayouts = Readonly<
  Record<string, ComponentType<GameLayoutProps>>
>;

export type GameLayoutKind = "dashboards";

/**
 * A game package's contribution to the player. Games are headless,
 * exact-build definitions from `@evenfall/gba/bridge`; layouts load on demand
 * with their styles.
 */
export interface GamePlugin {
  readonly id: string;
  readonly games: readonly GameIdentity[];
  /** Main player layouts, rendered around the player for a matching ROM. */
  readonly dashboards?: () => Promise<GameLayouts>;
}

export interface GameLayout {
  readonly game: GameIdentity;
  readonly Layout: ComponentType<GameLayoutProps>;
}

export function createGameRegistry(plugins: readonly GamePlugin[]) {
  const games = plugins.flatMap((plugin) => plugin.games);
  const loaded: Record<GameLayoutKind, GameLayout[]> = {
    dashboards: [],
  };
  // Exact build identity selects a game; titles and filenames never do.
  const gameForRom = (romHash: string | undefined) =>
    romHash ? resolveGame(games, romHash) : null;
  // Located builds this session: what to review, and their cache keys.
  const located = new Map<string, { review: readonly string[]; key: string }>();
  // Located builds whose views failed; not located again this session.
  const failed = new Set<string>();
  const cacheKey = (game: GameIdentity, romHash: string, version: string) =>
    `${game.id}:${romHash}:${version}`;
  return {
    games,
    gameForRom,
    /**
     * Recognize a ROM no reviewed build matches: from the cache, else by
     * running each locatable game's locators on its bytes. A recognized build
     * is registered with its game, so `gameForRom` and the bridges see it.
     */
    async recognize(
      romHash: string,
      readRom: () => Promise<Uint8Array>,
      options: {
        readonly cache?: BuildCache;
        readonly signal?: AbortSignal;
        /** The catalog already names the game: try only it, without detection. */
        readonly gameId?: string;
        /** Called when a game's detection matches and locating starts. */
        readonly onLocating?: (game: GameIdentity) => void;
      } = {},
    ): Promise<Recognition> {
      const known = gameForRom(romHash);
      if (known)
        return {
          status:
            buildProvenance(known, romHash) === "located"
              ? "located"
              : "reviewed",
          game: known,
        };
      if (failed.has(romHash)) return { status: "unsupported" };
      const candidates = games.flatMap((game) => {
        const target = locatable(game);
        return target && (!options.gameId || game.id === options.gameId)
          ? [{ game, key: cacheKey(game, romHash, target.version) }]
          : [];
      });
      for (const { game, key } of candidates) {
        const cached = (await options.cache?.get(key)) as
          | CachedBuild
          | undefined;
        if (!cached) continue;
        try {
          registerBuild(game, cached.manifest);
          located.set(romHash, { review: cached.review, key });
          return { status: "cached", game };
        } catch {
          await options.cache?.delete(key);
        }
      }
      if (!candidates.length) return { status: "unsupported" };
      const rom = await readRom();
      for (const { game, key } of candidates) {
        const outcome = await locateGame(game, rom, {
          signal: options.signal,
          romHash,
          known: options.gameId === game.id,
          onDetected: () => options.onLocating?.(game),
        });
        if (outcome.status !== "located") continue;
        registerBuild(game, outcome.build.manifest);
        located.set(romHash, { review: outcome.build.review, key });
        await options.cache?.set(key, {
          manifest: outcome.build.manifest,
          review: outcome.build.review,
        } satisfies CachedBuild);
        return { status: "located", game };
      }
      return { status: "unsupported" };
    },
    /**
     * Drop a located build whose views fail, here and in the cache, and do
     * not locate it again this session. Reviewed builds are kept.
     */
    async forget(romHash: string, cache?: BuildCache) {
      const entry = located.get(romHash);
      if (!entry) return;
      for (const game of games) unregisterBuild(game, romHash);
      located.delete(romHash);
      failed.add(romHash);
      await cache?.delete(entry.key);
    },
    /** The loaded build's provenance, for the layout. */
    buildInfo(romHash: string | undefined): BuildInfo | undefined {
      const game = gameForRom(romHash);
      if (!game || !romHash) return undefined;
      return buildProvenance(game, romHash) === "located"
        ? { provenance: "located", review: located.get(romHash)?.review ?? [] }
        : { provenance: "reviewed", review: [] };
    },
    /** Load every plugin's layouts of one kind; call once before rendering. */
    async load(kind: GameLayoutKind) {
      const layouts = await Promise.all(
        plugins.map(async (plugin) => {
          const byId = (await plugin[kind]?.()) ?? {};
          for (const id of Object.keys(byId))
            if (!plugin.games.some((game) => game.id === id))
              throw new Error(
                `${plugin.id} has a layout for unknown game ${id}`,
              );
          return plugin.games.flatMap((game) => {
            const Layout = byId[game.id];
            return Layout ? [{ game, Layout }] : [];
          });
        }),
      );
      loaded[kind] = layouts.flat();
    },
    layoutForRom(kind: GameLayoutKind, romHash: string | undefined) {
      const game = gameForRom(romHash);
      return loaded[kind].find((layout) => layout.game === game) ?? null;
    },
  };
}

/** What `watchBuild` reads: a bridge store's views and their updates. */
export interface ViewStore {
  subscribe(listener: () => void): () => void;
  getSnapshot(): Readonly<Record<string, { readonly status: string }>>;
}

/**
 * Watch a located build's views and call `fail` once if one of them never
 * decodes: it errors on `limit` consecutive samples without having been ready
 * since the watch began. Returns the unsubscribe function.
 */
export function watchBuild(store: ViewStore, fail: () => void, limit = 20) {
  const ready = new Set<string>();
  const errors = new Map<string, number>();
  let failed = false;
  let stopped = false;
  const stop = () => {
    if (!stopped) unsubscribe();
    stopped = true;
  };
  const unsubscribe = store.subscribe(() => {
    if (failed || stopped) return;
    for (const [key, view] of Object.entries(store.getSnapshot())) {
      if (view.status === "ready") ready.add(key);
      else if (view.status === "error")
        errors.set(key, (errors.get(key) ?? 0) + 1);
      else if (view.status !== "idle" && view.status !== "loading")
        errors.delete(key);
      if (!ready.has(key) && (errors.get(key) ?? 0) >= limit) failed = true;
    }
    // Deferred: a store may publish while `subscribe` is still running.
    if (failed)
      queueMicrotask(() => {
        if (stopped) return;
        stop();
        fail();
      });
  });
  return stop;
}

const installed: readonly GamePlugin[] = [pokemonPlugin];

export const gameRegistry = createGameRegistry(installed);
