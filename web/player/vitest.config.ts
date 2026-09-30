import { defaultClientConditions, defaultServerConditions } from "vite-plus";
import { defineConfig } from "vite-plus";

// Resolve workspace packages to their TypeScript sources (see vite.config.ts).
const conditions = ["zigba-source"];
export default defineConfig({
  resolve: { conditions: [...conditions, ...defaultClientConditions] },
  ssr: { resolve: { conditions: [...conditions, ...defaultServerConditions] } },
  test: {
    // Vitest v4 compatibility: preserve mock call history.
    // Remove after tests no longer rely on calls from setup or earlier tests.
    // https://viteplus.dev/guide/vitest-v5#remove-unneeded-compatibility-settings
    // https://vitest.dev/guide/migration/#clearmocks-is-enabled-by-default
    clearMocks: false,
    include: ["tests/*.test.{ts,tsx}"],
  },
});
