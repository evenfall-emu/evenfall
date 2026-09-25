import { defaultClientConditions, defaultServerConditions } from "vite";
import { defineConfig } from "vitest/config";

// Resolve workspace packages to their TypeScript sources (see vite.config.ts).
const conditions = ["zigba-source"];
export default defineConfig({
  resolve: { conditions: [...conditions, ...defaultClientConditions] },
  ssr: { resolve: { conditions: [...conditions, ...defaultServerConditions] } },
  test: { include: ["tests/*.test.{ts,tsx}"] },
});
