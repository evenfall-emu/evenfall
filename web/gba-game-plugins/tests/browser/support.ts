// Shared setup for fixtures bundled by these Playwright specs. Specs run
// from games/pokemon against the player built by `zig build web`.
import { resolve } from "node:path";
import { defaultClientConditions, type InlineConfig } from "vite-plus";

/** Base Vite options: resolve workspace packages to their TS sources. */
export const fixtureBuild = {
  configFile: false,
  logLevel: "error",
  resolve: { conditions: ["zigba-source", ...defaultClientConditions] },
  define: { "process.env.NODE_ENV": '"development"' },
} satisfies InlineConfig;

/** The player's page styles, which these panels are drawn on top of. */
export const playerCss = resolve("../../frontend/src/console.css");
/** This package's styles for the main-player dashboards. */
export const dashboardCss = resolve("ui/dashboard.css");
