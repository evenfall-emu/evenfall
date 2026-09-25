import { defineConfig } from "@playwright/test";
import { existsSync } from "node:fs";

// Runs against the built player (`zig build web`), like the frontend suite.
const port = process.env.ZIGBA_TEST_PORT ?? "8765";
const deploymentUrl = process.env.ZIGBA_TEST_URL;
export default defineConfig({
  testDir: "tests/browser",
  timeout: 60000,
  workers: 1,
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
        command: `python3 ../../tools/serve-web.py --port ${port} --directory ../../zig-out/web --validation`,
        url: `http://127.0.0.1:${port}`,
        reuseExistingServer: true,
      },
});
