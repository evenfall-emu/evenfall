import { defineConfig } from "@playwright/test";
import { chromiumLaunch } from "./tests/browser/launch";
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
    launchOptions: chromiumLaunch(),
  },
  webServer: deploymentUrl
    ? undefined
    : {
        command: `python3 scripts/serve-web.py --port ${port} --directory dist --validation`,
        url: `http://127.0.0.1:${port}`,
        reuseExistingServer: true,
      },
});
