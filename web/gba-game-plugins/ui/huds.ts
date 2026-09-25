// Dev / Debug HUD layouts. Loaded on demand by the debug page, with their styles.
import "./hud.css";
import { heartNSoul } from "../src/heart-n-soul";
import type { GameLayouts } from "./layout";
import { HeartNSoulLayout } from "./party-hud";

export const huds: GameLayouts = {
  [heartNSoul.id]: HeartNSoulLayout,
};
