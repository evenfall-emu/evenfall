import { defineConfig } from "@playwright/test";
import { existsSync } from "node:fs";

// Runs against the built player (`npm run build -w @evenfall/player`), like
// the player suite.
const port = process.env.EVENFALL_TEST_PORT ?? "8765";
const deploymentUrl = process.env.EVENFALL_TEST_URL;
export default defineConfig({
  testDir: "tests/browser",
  timeout: 60000,
  // Per-file fixture directories stay owned by one worker.
  workers: 2,
  use: {
    baseURL: deploymentUrl ?? `http://127.0.0.1:${port}`,
    headless: true,
    launchOptions: {
      executablePath:
        process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE ??
        (existsSync("/usr/bin/chromium") ? "/usr/bin/chromium" : undefined),
      args: ["--no-sandbox", "--disable-audio-output"],
    },
  },
  webServer: deploymentUrl
    ? undefined
    : {
        command: `python3 ../player/scripts/serve-web.py --port ${port} --directory ../player/dist --validation`,
        url: `http://127.0.0.1:${port}`,
        reuseExistingServer: true,
      },
});
