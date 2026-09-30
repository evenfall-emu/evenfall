import { test, expect, type Browser, type Page } from "@playwright/test";
import { spawn } from "node:child_process";
import { once } from "node:events";
import {
  cp,
  mkdtemp,
  readdir,
  readFile,
  rename,
  rm,
  writeFile,
} from "node:fs/promises";
import { createServer, type AddressInfo } from "node:net";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

// Each test serves its own copy of the production build, so it can deploy new
// versions into it.
test.skip(
  Boolean(process.env.EVENFALL_TEST_URL),
  "Deploys new versions into a local copy of the build",
);

const rom = {
  name: "Loop.gba",
  mimeType: "application/octet-stream",
  buffer: Buffer.from([42, 0, 160, 227, 254, 255, 255, 234]),
};

async function freePort() {
  const server = createServer().listen(0, "127.0.0.1");
  await once(server, "listening");
  const { port } = server.address() as AddressInfo;
  server.close();
  await once(server, "close");
  return port;
}

async function site(browser: Browser) {
  const directory = await mkdtemp(join(tmpdir(), "evenfall-update-"));
  await cp(resolve("dist"), directory, { recursive: true });
  const port = await freePort();
  const server = spawn(
    "python3",
    [
      resolve("scripts/serve-web.py"),
      "--port",
      String(port),
      "--directory",
      directory,
    ],
    { stdio: "ignore" },
  );
  const url = `http://127.0.0.1:${port}`;
  await expect
    .poll(() =>
      fetch(url).then(
        (response) => response.ok,
        () => false,
      ),
    )
    .toBe(true);
  const context = await browser.newContext({ baseURL: url });
  // The static server has no API. Answer at once: pending cloud requests
  // hold back the first update check until they time out.
  await context.route("**/api/**", (route) => route.fulfill({ status: 404 }));
  return {
    url,
    context,
    /**
     * Publishes a build labelled `label`: a new service worker and cache
     * name, a marked index.html and a renamed SDK worker whose old name the
     * server no longer has, like a real deployment. Returns the old path.
     */
    async deploy(label: string) {
      const script = join(directory, "service-worker.js");
      const worker = (await readFile(script, "utf8")).match(
        /assets\/worker-[^"]+\.js/,
      )![0];
      const renamed = `assets/worker-${label}.js`;
      await rename(join(directory, worker), join(directory, renamed));
      for (const file of await readdir(directory, { recursive: true })) {
        if (!/\.(html|js)$/.test(file)) continue;
        const path = join(directory, file);
        let text = (await readFile(path, "utf8")).replaceAll(worker, renamed);
        if (file === "index.html")
          text = text.replace(
            /<meta name="evenfall-test-build"[^>]*>|(?=<\/head>)/,
            `<meta name="evenfall-test-build" content="${label}">`,
          );
        if (file === "service-worker.js")
          text = text.replace(
            /const CACHE = "[^"]+";/,
            `const CACHE = "evenfall-player-${label}";`,
          );
        await writeFile(path, text);
      }
      return `/${worker}`;
    },
    async close() {
      await context.close();
      server.kill();
      await rm(directory, { recursive: true, force: true });
    },
  };
}

const build = (page: Page) => page.locator('meta[name="evenfall-test-build"]');
const notice = (page: Page) => page.locator(".update-notice");
/**
 * Waits until the first version controls `page` and has finished its first
 * update check, which takes Chromium about two seconds after an installation;
 * later checks start at once.
 */
async function installed(page: Page) {
  await expect
    .poll(() =>
      page.evaluate(() => Boolean(navigator.serviceWorker.controller)),
    )
    .toBe(true);
  await page.evaluate(async () => {
    await (await navigator.serviceWorker.getRegistration())!.update();
  });
}
/** What the player's hourly and on-focus checks do. */
const checkForUpdate = (page: Page) =>
  page.evaluate(() => {
    void navigator.serviceWorker
      .getRegistration()
      .then((registration) => registration?.update());
  });
const hide = (page: Page) =>
  page.evaluate(() => {
    Object.defineProperty(document, "hidden", {
      configurable: true,
      get: () => true,
    });
    document.dispatchEvent(new Event("visibilitychange"));
  });
const playerCaches = (page: Page) =>
  page.evaluate(async () =>
    (await caches.keys()).filter((name) => name.startsWith("evenfall-player-")),
  );
const storedCycle = (page: Page, hash: string) =>
  page.evaluate(async (hash) => {
    const request = indexedDB.open("evenfall-local");
    const db = await new Promise<IDBDatabase>((resolve) => {
      request.onsuccess = () => resolve(request.result);
    });
    try {
      const read = db
        .transaction("resumes")
        .objectStore("resumes")
        .get([hash, "default"]);
      return await new Promise<string | undefined>((resolve) => {
        read.onsuccess = () => resolve(read.result?.cycle);
      });
    } finally {
      db.close();
    }
  }, hash);

test("the production install manifest launches the library and stays available offline", async ({
  browser,
}) => {
  const host = await site(browser);
  try {
    const page = await host.context.newPage();
    // A nested page must discover the same manifest as the library.
    await page.goto("/library/install-check");
    await installed(page);
    const cdp = await host.context.newCDPSession(page);
    const manifest = await cdp.send("Page.getAppManifest");
    expect(manifest.errors).toEqual([]);
    expect(manifest.data).toBeTruthy();
    const data = JSON.parse(manifest.data!);
    expect(data.name).toBe("Evenfall");
    expect(data.display).toBe("standalone");
    const launch = new URL(data.start_url, manifest.url);
    expect(launch.href).toBe(`${host.url}/library`);
    expect(new URL(data.scope, manifest.url).href).toBe(`${host.url}/`);

    await host.context.setOffline(true);
    await page.goto(launch.href);
    await expect(page.getByPlaceholder("Search games…")).toBeVisible();
    // Fetch through the controlling worker, not Playwright's network client.
    const offline = await page.evaluate(async (url) => {
      const response = await fetch(url);
      const manifest = await response.json();
      const sizes = await Promise.all(
        manifest.icons.map(async (icon: { src: string }) => {
          const image = await fetch(new URL(icon.src, url));
          const bitmap = await createImageBitmap(await image.blob());
          return [bitmap.width, bitmap.height];
        }),
      );
      return sizes;
    }, manifest.url);
    expect(offline).toEqual([
      [192, 192],
      [512, 512],
    ]);
  } finally {
    await host.close();
  }
});

test("an untouched player without a game reloads into a new version, at most once a minute", async ({
  browser,
}) => {
  const host = await site(browser);
  try {
    const page = await host.context.newPage();
    let loads = 0;
    page.on("load", () => loads++);
    await page.goto("/");
    await installed(page);
    // An earlier player's cache, under the legacy prefix.
    await page.evaluate(() => caches.open("zigba-player-0123456789"));
    await host.deploy("v2");
    await checkForUpdate(page);
    await expect(build(page)).toHaveAttribute("content", "v2");
    // Only that reload: the first installation claimed the page without one.
    await expect.poll(() => loads).toBe(2);
    await expect.poll(() => playerCaches(page)).toEqual(["evenfall-player-v2"]);
    expect(
      await page.evaluate(async () =>
        (await caches.keys()).filter((name) => name.startsWith("zigba-")),
      ),
    ).toEqual([]);
    await host.deploy("v3");
    await checkForUpdate(page);
    await expect(notice(page)).toContainText(
      "A new version of Evenfall is ready.",
    );
    await expect(build(page)).toHaveAttribute("content", "v2");
    await notice(page).getByRole("button", { name: "Update now" }).click();
    await expect(build(page)).toHaveAttribute("content", "v3");
    await expect(notice(page)).toHaveCount(0);
    await expect.poll(() => loads).toBe(3);
  } finally {
    await host.close();
  }
});

test("a player the user has touched asks first, then updates while hidden", async ({
  browser,
}) => {
  const host = await site(browser);
  try {
    const page = await host.context.newPage();
    await page.goto("/");
    await installed(page);
    await page.keyboard.press("Shift");
    await host.deploy("v2");
    await checkForUpdate(page);
    await expect(
      notice(page).getByRole("button", { name: "Update now" }),
    ).toBeEnabled();
    await expect(build(page)).toHaveCount(0);
    await page.screenshot({ path: "../../artifacts/update/home-prompt.png" });
    await page.setViewportSize({ width: 390, height: 844 });
    await page.screenshot({
      path: "../../artifacts/update/home-prompt-mobile.png",
    });
    await notice(page).getByRole("button", { name: "Dismiss update" }).click();
    await expect(notice(page)).toHaveCount(0);
    await expect(build(page)).toHaveCount(0);
    await hide(page);
    await expect(build(page)).toHaveAttribute("content", "v2");
  } finally {
    await host.close();
  }
});

test("a running game keeps its version and assets until its player updates, and saves first", async ({
  browser,
}) => {
  const host = await site(browser);
  try {
    const game = await host.context.newPage();
    await game.goto("/");
    await game.getByRole("button", { name: "Open player" }).click();
    await game.getByLabel("Open ROM", { exact: true }).setInputFiles(rom);
    await expect
      .poll(() => game.evaluate(() => window.player.state.playing))
      .toBe(true);
    await installed(game);
    const oldWorker = await host.deploy("v2");
    expect((await fetch(host.url + oldWorker)).status).toBe(404);
    await checkForUpdate(game);
    await expect(notice(game)).toBeVisible();
    await game.screenshot({ path: "../../artifacts/update/player-prompt.png" });
    // A new, untouched tab activates the new version for every tab.
    const other = await host.context.newPage();
    await other.goto("/");
    await expect(build(other)).toHaveAttribute("content", "v2");
    await expect(build(game)).toHaveCount(0);
    await expect(notice(game)).toBeVisible();
    expect(await game.evaluate(() => window.player.state.playing)).toBe(true);
    expect(
      await game.evaluate(
        (path) => fetch(path).then((response) => response.status),
        oldWorker,
      ),
    ).toBe(200);
    const hash = await game.evaluate(() => window.player.state.game!.hash);
    const cycle = await game.evaluate(async () =>
      String((await window.player.emulator.inspect()).cycle),
    );
    await notice(game).getByRole("button", { name: "Update now" }).click();
    await expect(build(game)).toHaveAttribute("content", "v2");
    expect(BigInt((await storedCycle(game, hash)) ?? -1)).toBeGreaterThan(
      BigInt(cycle),
    );
    // The old version's cache goes once a single tab remains.
    expect(await playerCaches(game)).toHaveLength(2);
    await other.close();
    await game.reload();
    await expect.poll(() => playerCaches(game)).toEqual(["evenfall-player-v2"]);
  } finally {
    await host.close();
  }
});
