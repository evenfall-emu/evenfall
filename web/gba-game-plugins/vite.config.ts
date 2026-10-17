import { defineConfig } from "vite-plus";

export default defineConfig({
  // Compiles the headless definitions (src) to dist/*.js + .d.ts for the
  // published package, one output file per source file. Workspace consumers
  // read the sources directly through the "evenfall-source" export condition,
  // and the dashboard panels in ui/ stay repo-only.
  pack: {
    entry: ["src/*.ts"],
    unbundle: true,
    outDir: "dist",
    format: "esm",
    platform: "neutral",
    target: "es2022",
    dts: true,
    tsconfig: "tsconfig.json",
    // Keep package imports (@evenfall/gba) as written.
    deps: { neverBundle: true },
  },
});
