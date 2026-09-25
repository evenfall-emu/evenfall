import { defaultClientConditions, defaultServerConditions } from "vite";
import { defineConfig } from "vitest/config";

// Resolve workspace packages (@evenfall/gba) to their TypeScript sources.
const conditions = ["zigba-source"];
export default defineConfig({
  resolve: { conditions: [...conditions, ...defaultClientConditions] },
  ssr: { resolve: { conditions: [...conditions, ...defaultServerConditions] } },
  test: { include: ["tests/*.test.{ts,tsx}"] },
});
