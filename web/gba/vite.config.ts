import { defineConfig } from "vite-plus";

export default defineConfig({
  // Compiles the TypeScript bridge (src/bridge) to bridge/*.js + .d.ts for
  // the published package, one output file per source file. Workspace
  // consumers read the sources directly through the "evenfall-source" export
  // condition; see docs/sdk.md.
  pack: {
    entry: ["src/bridge/*.ts"],
    unbundle: true,
    outDir: "bridge",
    format: "esm",
    platform: "neutral",
    target: "es2022",
    dts: true,
    tsconfig: "tsconfig.json",
    // Keep package imports (react, @evenfall/gba's own types) as written.
    deps: { neverBundle: true },
  },
  // The native/WASM parity test builds and runs a release Rust binary, with
  // its own 120-second limits on each step.
  test: { testTimeout: 300_000, hookTimeout: 300_000 },
});
