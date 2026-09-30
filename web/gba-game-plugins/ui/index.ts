// `@evenfall/gba-pokemon/ui`: the player plugin plus the React panels it renders.
// Unofficial; see ../README.md for supported builds and the disclaimer.
import { eliteRedux } from "../src/elite-redux";
import { heartNSoul } from "../src/heart-n-soul";

export type {
  BuildInfo,
  GameDock,
  GameLayoutProps,
  GameLayouts,
  GamePanel,
  HandheldHud,
} from "./layout";

/**
 * Registration for the Evenfall player's game registry (web/player/src/games.ts).
 * Layout modules and their CSS load on demand, in their own chunks.
 */
export const pokemonPlugin = {
  id: "@evenfall/gba-pokemon",
  games: [heartNSoul, eliteRedux],
  dashboards: () => import("./dashboards").then((module) => module.dashboards),
};
