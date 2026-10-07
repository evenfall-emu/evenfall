// Main-player layouts. Loaded on demand by the player page, with their styles.
import "./dashboard.css";
import { fireRedUpgrade } from "../src/cfru";
import { eliteRedux } from "../src/elite-redux";
import { heartNSoul } from "../src/heart-n-soul";
import type { GameLayouts } from "./layout";
import {
  EliteReduxDashboard,
  FireRedUpgradeDashboard,
  HeartNSoulDashboard,
} from "./pokemon-dashboard";

export const dashboards: GameLayouts = {
  [heartNSoul.id]: HeartNSoulDashboard,
  [eliteRedux.id]: EliteReduxDashboard,
  [fireRedUpgrade.id]: FireRedUpgradeDashboard,
};
