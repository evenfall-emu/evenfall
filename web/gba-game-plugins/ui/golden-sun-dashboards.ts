// The Golden Sun layout, loaded on demand with its styles. It shares the
// page grid, strips and headings of the Pokémon dashboards' stylesheet.
import "./dashboard.css";
import "./golden-sun.css";
import { goldenSun } from "../src/golden-sun";
import type { GameLayouts } from "./layout";
import { GoldenSunDashboard } from "./golden-sun-dashboard";

export const dashboards: GameLayouts = {
  [goldenSun.id]: GoldenSunDashboard,
};
