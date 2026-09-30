import { defineConfig } from "@playwright/test";
import { existsSync } from "node:fs";
const port = process.env.EVENFALL_TEST_PORT ?? "8765";
const deploymentUrl = process.env.EVENFALL_TEST_URL;
export default defineConfig({
  testDir: "tests/browser",
  timeout: 60000,
  // Parallelize files while retaining CPU headroom for realtime browser tests.
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
        command: `python3 scripts/serve-web.py --port ${port} --directory dist --validation`,
        url: `http://127.0.0.1:${port}`,
        reuseExistingServer: true,
      },
});
