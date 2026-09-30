import { defineConfig } from "vite-plus";

export default defineConfig({
  fmt: {
    printWidth: 80,
    proseWrap: "preserve",
    sortImports: false,
    sortPackageJson: false,
    ignorePatterns: [
      ".amp/**",
      "refs/**",
      "**/Cargo.toml",
      "web/gba/bridge/**",
    ],
  },
  lint: {
    ignorePatterns: [".amp/**", "refs/**", "web/gba/bridge/**"],
    jsPlugins: [{ name: "vite-plus", specifier: "vite-plus/oxlint-plugin" }],
    rules: { "vite-plus/prefer-vite-plus-imports": "error" },
    options: { typeAware: true, typeCheck: true },
  },
  test: {
    projects: [
      "web/gba",
      { test: { name: "bench", root: "tools/bench", include: ["*.test.mjs"] } },
    ],
  },
});
