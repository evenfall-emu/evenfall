import {
  defaultClientConditions,
  defaultServerConditions,
  defineConfig,
} from "vite";
import react from "@vitejs/plugin-react";
import { resolve } from "node:path";
import { offlinePlayer } from "./offline";

// Workspace packages export their TypeScript sources under this condition,
// so the player needs no separate SDK bridge build. See docs/sdk.md.
export const sourceConditions = ["zigba-source"];

export default defineConfig({
  plugins: [react(), offlinePlayer()],
  base: "./",
  resolve: { conditions: [...sourceConditions, ...defaultClientConditions] },
  ssr: {
    resolve: { conditions: [...sourceConditions, ...defaultServerConditions] },
  },
  server: {
    port: 5173,
    strictPort: true,
    proxy: { "/api": "http://127.0.0.1:8787" },
  },
  // The SDK's worker, WASM and audio worklet are emitted as hashed assets.
  publicDir: false,
  build: {
    outDir: process.env.ZIGBA_PLAYER_DIR ?? "../zig-out/player",
    emptyOutDir: true,
    rollupOptions: {
      input: {
        player: resolve(import.meta.dirname, "index.html"),
        validation: resolve(import.meta.dirname, "validation.html"),
      },
    },
  },
});
