import type { ComponentType, ReactNode } from "react";
import { resolveGame, type GameIdentity } from "@evenfall/gba/bridge";
// Installed game packages. Each one is an optional dependency: removing its
// import, its entry in `installed` and its package.json dependency leaves the
// generic player. See frontend/README.md#game-plugins.
import { pokemonPlugin } from "@evenfall/gba-pokemon/ui";

/** What the player gives a game layout: playback state and the player itself. */
export interface GameLayoutProps {
  readonly playing: boolean;
  /** The self-contained player (canvas, controls); render it exactly once. */
  readonly children: ReactNode;
}

/** Layout components keyed by the `id` of one of the plugin's games. */
export type GameLayouts = Readonly<
  Record<string, ComponentType<GameLayoutProps>>
>;

export type GameLayoutKind = "dashboards" | "huds";

/**
 * A game package's contribution to the player. Games are headless,
 * exact-build definitions from `@evenfall/gba/bridge`; layouts load on demand
 * with their styles, so each page only pays for what it renders.
 */
export interface GamePlugin {
  readonly id: string;
  readonly games: readonly GameIdentity[];
  /** Main player layouts, rendered around the player for a matching ROM. */
  readonly dashboards?: () => Promise<GameLayouts>;
  /** Dev / Debug HUD layouts (`/debug.html#/hud`). */
  readonly huds?: () => Promise<GameLayouts>;
}

export interface GameLayout {
  readonly game: GameIdentity;
  readonly Layout: ComponentType<GameLayoutProps>;
}

export function createGameRegistry(plugins: readonly GamePlugin[]) {
  const games = plugins.flatMap((plugin) => plugin.games);
  const loaded: Record<GameLayoutKind, GameLayout[]> = {
    dashboards: [],
    huds: [],
  };
  // Exact build identity selects a game; titles and filenames never do.
  const gameForRom = (romHash: string | undefined) =>
    romHash ? resolveGame(games, romHash) : null;
  return {
    games,
    gameForRom,
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
    /** The first loaded layout, for diagnostics on unsupported ROMs. */
    anyLayout(kind: GameLayoutKind): GameLayout | null {
      return loaded[kind][0] ?? null;
    },
  };
}

const installed: readonly GamePlugin[] = [pokemonPlugin];

export const gameRegistry = createGameRegistry(installed);
