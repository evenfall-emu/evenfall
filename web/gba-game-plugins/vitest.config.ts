import { defaultClientConditions, defaultServerConditions } from "vite-plus";
import { defineConfig } from "vite-plus";

// Resolve workspace packages (@evenfall/gba) to their TypeScript sources.
const conditions = ["evenfall-source"];
export default defineConfig({
  resolve: { conditions: [...conditions, ...defaultClientConditions] },
  ssr: { resolve: { conditions: [...conditions, ...defaultServerConditions] } },
  test: {
    name: "gba-game-plugins",
    // Vitest v4 compatibility: preserve mock call history.
    // Remove after tests no longer rely on calls from setup or earlier tests.
    // https://viteplus.dev/guide/vitest-v5#remove-unneeded-compatibility-settings
    // https://vitest.dev/guide/migration/#clearmocks-is-enabled-by-default
    clearMocks: false,
    // Same 30s budget as the root config: file-based projects do not inherit
    // it, and Cloudflare build runners throttle the CPU across every isolate.
    testTimeout: 30_000,
    include: ["tests/*.test.{ts,tsx}"],
  },
});
