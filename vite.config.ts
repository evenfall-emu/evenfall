import { defineConfig } from "vite-plus";

export default defineConfig({
  fmt: {
    printWidth: 80,
    proseWrap: "preserve",
    sortImports: false,
    sortPackageJson: false,
    ignorePatterns: [
      ".amp/**",
      ".dist/**",
      "refs/**",
      "**/Cargo.toml",
      "web/gba/bridge/**",
      "web/player/dist/**",
      "web/server/migrations/**",
    ],
  },
  lint: {
    ignorePatterns: [
      ".amp/**",
      ".dist/**",
      "refs/**",
      "web/gba/bridge/**",
      "web/player/dist/**",
    ],
    jsPlugins: [{ name: "vite-plus", specifier: "vite-plus/oxlint-plugin" }],
    rules: { "vite-plus/prefer-vite-plus-imports": "error" },
    options: { typeAware: true, typeCheck: true },
  },
  test: {
    projects: [
      "web/gba",
      "web/player/vitest.config.ts",
      "web/gba-game-plugins/vitest.config.ts",
      {
        test: {
          name: "server",
          root: "web/server",
          include: ["test/*.test.ts", "scripts/catalog/*.test.mjs"],
        },
      },
      { test: { name: "bench", root: "tools/bench", include: ["*.test.mjs"] } },
    ],
  },
});
