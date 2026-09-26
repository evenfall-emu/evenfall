import { expect, test, type Locator, type Page } from "@playwright/test";
import type { Emulator } from "@evenfall/gba";
import { loop, maplePatch, mockCatalog, sha1 } from "./catalog.fixture";

async function openLibrary(page: Page) {
  await page.goto("/");
  await page.getByRole("button", { name: "Browse library" }).click();
  await expect(page.getByRole("main", { name: "Library" })).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
}

/** Opens loop ROMs in the Player, which stores each with a playthrough. */
async function importRoms(page: Page, roms: [number, string][]) {
  await page.goto("/");
  await page.getByRole("button", { name: "Enter player" }).click();
  for (const [marker, name] of roms) {
    await expect(page.getByLabel("Open ROM", { exact: true })).toBeEnabled();
    await page
      .getByLabel("Open ROM", { exact: true })
      .setInputFiles(loop(marker, name));
    await expect(
      page.getByRole("heading", { name, exact: true }),
    ).toBeVisible();
  }
  await expect(page.getByLabel("Open ROM", { exact: true })).toBeEnabled();
  await page.getByRole("button", { name: "Library", exact: true }).click();
  await expect(page.getByRole("main", { name: "Library" })).toBeVisible();
}

const titles = (page: Page) =>
  page
    .getByRole("list", { name: "Games" })
    .getByRole("button", { name: /^View / })
    .evaluateAll((buttons) =>
      buttons.map((button) =>
        button.getAttribute("aria-label")!.replace(/^View /, ""),
      ),
    );

async function choose(page: Page, select: string, option: string) {
  await page.getByRole("combobox", { name: select, exact: true }).click();
  await page.getByRole("option", { name: option, exact: true }).click();
}

async function chooseFile(
  page: Page,
  button: string,
  file: ReturnType<typeof loop>,
) {
  const picker = page.waitForEvent("filechooser");
  await page.getByRole("button", { name: button }).click();
  await (await picker).setFiles(file);
}

/**
 * Reads records from the player's local database: the one at `key`, or with
 * `prefix`, every record whose array key starts with it.
 */
function stored(
  page: Page,
  store: string,
  key: string | string[],
  prefix = false,
): Promise<Record<string, unknown>[]> {
  return page.evaluate(
    async ({ store, key, prefix }) => {
      const request = indexedDB.open("zigba-local");
      const db = await new Promise<IDBDatabase>((resolve, reject) => {
        request.onsuccess = () => resolve(request.result);
        request.onerror = () => reject(request.error);
      });
      const result = await new Promise<unknown>((resolve, reject) => {
        const get = db
          .transaction(store)
          .objectStore(store)
          .getAll(prefix ? IDBKeyRange.bound([key], [key, []]) : key);
        get.onsuccess = () => resolve(get.result);
        get.onerror = () => reject(get.error);
      });
      db.close();
      return JSON.parse(
        JSON.stringify(result, (_, value) =>
          value instanceof Uint8Array ? Array.from(value) : value,
        ),
      );
    },
    { store, key, prefix },
  );
}

/** The playthrough rows in the details pane. */
const runsIn = (details: Locator) =>
  details.getByRole("listitem").filter({ hasText: "Last played" });

async function openGame(page: Page) {
  await importRoms(page, [[42, "Birch"]]);
}

const frames = (page: Page) =>
  page.evaluate(async () =>
    Number(
      (await (window as unknown as { emulator: Emulator }).emulator.inspect())
        .frames,
    ),
  );

test("Library joins the catalog with this device's ROMs and filters it", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await mockCatalog(page);
  await importRoms(page, [
    [42, "Birch"],
    [40, "maple-dump"],
  ]);
  await expect(
    page.getByRole("button", { name: "Library", exact: true }),
  ).toHaveAttribute("aria-current", "page");
  // A catalog base shows under the catalog's name, not its file name.
  const order = ["Maple (US)", "Birch Quest", "Cedar Saga", "Maple Remix"];
  await expect.poll(() => titles(page)).toEqual(order);
  await expect(page.getByText("On this device", { exact: true })).toHaveCount(
    2,
  );
  await expect(
    page.getByRole("button", {
      name: "Add the Cedar Saga v1 ROM for Cedar Saga",
    }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Play Cedar Saga" }),
  ).toHaveCount(0);

  await page.getByRole("button", { name: "View Birch Quest" }).click();
  await expect(page).toHaveURL(/\/library\/birch-quest$/);
  const details = page.getByRole("region", { name: "Game details" });
  await expect(details).toContainText("Available locally");
  await expect(details).toContainText("v2 available");
  await expect(
    details.getByRole("listitem").filter({ hasText: "Playthrough 1" }),
  ).toContainText("v1 · Last played just now");
  await expect(details).toContainText("On this device only");
  await page.screenshot({ path: "../artifacts/library/desktop.png" });

  await page.getByRole("button", { name: "View Maple Remix" }).click();
  await expect(details).toContainText("Your Maple (US) is on this device");

  await choose(page, "Show", "Ready");
  expect(await titles(page)).toEqual(["Maple Remix"]);
  await choose(page, "Show", "Installed");
  expect(await titles(page)).toEqual(order.slice(0, 2));
  await choose(page, "Show", "All");
  await choose(page, "Sort", "Popularity");
  expect(await titles(page)).toEqual([
    "Cedar Saga",
    "Birch Quest",
    "Maple Remix",
    "Maple (US)",
  ]);
  await choose(page, "Franchise", "Trees");
  expect(await titles(page)).toEqual(["Cedar Saga", "Birch Quest"]);
  await choose(page, "Franchise", "All");
  await choose(page, "Sort", "Played");
  await page.getByRole("combobox", { name: "Tags" }).fill("har");
  await page.keyboard.press("Enter");
  expect(await titles(page)).toEqual(["Birch Quest", "Maple Remix"]);
  await page.getByRole("button", { name: "Remove Hard" }).click();
  await page.getByRole("searchbox", { name: "Search games" }).fill("map");
  expect(await titles(page)).toEqual(["Maple (US)", "Maple Remix"]);
  await page.getByRole("searchbox", { name: "Search games" }).fill("zzz");
  await expect(
    page.getByRole("heading", { name: "No matching games" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Clear filters" }).click();
  expect(await titles(page)).toEqual(order);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
});

test("Library keeps the last catalog it read for offline use", async ({
  page,
}) => {
  await mockCatalog(page);
  await openLibrary(page);
  await expect.poll(() => titles(page)).toHaveLength(3);
  await page.unroute("**/api/games");
  await page.route("**/api/games", (route) => route.abort());
  await openLibrary(page);
  expect(await titles(page)).toEqual([
    "Birch Quest",
    "Cedar Saga",
    "Maple Remix",
  ]);
  await expect(page.locator(".console-notice")).toContainText(
    "Catalog unavailable",
  );
});

test("Install patches the base ROM and plays the verified release", async ({
  page,
}) => {
  await mockCatalog(page);
  await importRoms(page, [[40, "Maple base"]]);
  await page.getByRole("button", { name: "Play Maple Remix" }).click();
  await expect(
    page.getByRole("heading", { name: "Maple Remix", exact: true }),
  ).toBeVisible();
  await expect(page.getByLabel(/^Game screen/)).toBeVisible();
  expect(
    await page.evaluate(
      async () =>
        (await (window as unknown as { emulator: Emulator }).emulator.inspect())
          .romHash,
    ),
  ).toBe(sha1(41));

  await page.getByRole("button", { name: "Library", exact: true }).click();
  await page.getByRole("button", { name: "View Maple Remix" }).click();
  await expect(
    page.getByRole("region", { name: "Game details" }),
  ).toContainText("Available locally");
});

test("Install stores nothing when the patch is not the catalog's", async ({
  page,
}) => {
  await mockCatalog(page);
  await importRoms(page, [[40, "Maple base"]]);
  await page.route("https://example.invalid/maple.bps", (route) =>
    route.fulfill({
      body: Buffer.alloc(maplePatch.length),
      headers: { "access-control-allow-origin": "*" },
    }),
  );
  await page.getByRole("button", { name: "Install Maple Remix" }).click();
  await expect(page.getByRole("alert")).toContainText(
    "This is not a BPS patch.",
  );
  expect(await stored(page, "games", sha1(41))).toEqual([]);
  await page.getByRole("button", { name: "Dismiss error" }).click();

  await page.route("https://example.invalid/maple.bps", (route) =>
    route.fulfill({ status: 404 }),
  );
  await page.getByRole("button", { name: "Install Maple Remix" }).click();
  await expect(page.getByRole("alert")).toContainText("(HTTP 404)");
});

test("Choose file stores only a ROM the game wants, then plays it", async ({
  page,
}) => {
  await mockCatalog(page);
  await openLibrary(page);
  await page.getByRole("button", { name: "View Cedar Saga" }).click();
  const details = page.getByRole("region", { name: "Game details" });
  await expect(details).toContainText("Needs the Cedar Saga v1 ROM");
  await chooseFile(page, "Choose file…", loop(45, "Wrong"));
  await expect(page.getByRole("alert")).toContainText(
    "That file isn’t Cedar Saga.",
  );
  expect(await stored(page, "games", sha1(45))).toEqual([]);

  await chooseFile(page, "Choose file…", loop(44, "cedar-dump"));
  await expect(details).toContainText("Available locally");
  await expect(details).toContainText("No playthroughs yet");
  expect(await stored(page, "playthroughs", [sha1(44), "default"])).toEqual([]);

  await details.getByRole("button", { name: "New playthrough" }).click();
  const create = page.getByRole("dialog", { name: "New playthrough" });
  await create.getByRole("textbox", { name: "Name" }).fill("  ");
  await expect(
    create.getByRole("button", { name: "Create & play" }),
  ).toBeDisabled();
  await create.getByRole("textbox", { name: "Name" }).fill("Hard mode");
  await create.getByRole("button", { name: "Create & play" }).click();
  await expect(
    page.getByRole("heading", { name: "Cedar Saga", exact: true }),
  ).toBeVisible();
  await expect(page.getByLabel(/^Game screen/)).toBeVisible();

  await page.getByRole("button", { name: "Library", exact: true }).click();
  await expect(
    details.getByRole("listitem").filter({ hasText: "Hard mode" }),
  ).toBeVisible();
});

test("Update adds the latest ROM, then copies the in-game save to a new run", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await mockCatalog(page);
  await importRoms(page, [[42, "Birch"]]);
  // A 32 KiB SRAM image.
  const save = Array.from({ length: 32768 }, (_, index) => index & 0xff);
  await page.evaluate(
    async ({ hash, save }) => {
      const request = indexedDB.open("zigba-local");
      const db = await new Promise<IDBDatabase>((resolve, reject) => {
        request.onsuccess = () => resolve(request.result);
        request.onerror = () => reject(request.error);
      });
      const tx = db.transaction("saves", "readwrite");
      tx.objectStore("saves").put(
        { data: new Uint8Array(save), updatedAt: Date.now() },
        [hash, "default"],
      );
      await new Promise((resolve) => (tx.oncomplete = resolve));
      db.close();
    },
    { hash: sha1(42), save },
  );

  await page.getByRole("button", { name: "View Birch Quest" }).click();
  await page.getByRole("button", { name: "Update…" }).click();
  const update = page.getByRole("dialog", { name: "Birch Quest · v2" });
  await expect(update).toContainText("Add the v2 ROM to this device first");
  await chooseFile(page, "Choose file…", loop(43, "birch-v2"));
  const copy = update.getByRole("button", { name: "Copy save & play" });
  await expect(copy).toBeDisabled();
  await update
    .getByRole("checkbox", {
      name: "I understand compatibility is unverified.",
    })
    .click();
  await update.screenshot({ path: "../artifacts/library/update.png" });
  await copy.click();
  await expect(page.getByLabel(/^Game screen/)).toBeVisible();

  const runs = await stored(page, "playthroughs", sha1(43), true);
  expect(runs).toMatchObject([{ name: "Playthrough 1" }]);
  expect(
    await stored(page, "saves", [sha1(43), runs[0].id as string]),
  ).toMatchObject([{ data: save }]);
  expect(
    await page.evaluate(async () =>
      Array.from(
        await (
          window as unknown as { emulator: Emulator }
        ).emulator.exportSave(),
      ),
    ),
  ).toEqual(save);

  await page.getByRole("button", { name: "Library", exact: true }).click();
  const details = page.getByRole("region", { name: "Game details" });
  await expect(runsIn(details)).toHaveCount(2);
  await expect(runsIn(details).first()).toContainText("v2 ·");
  await expect(runsIn(details).last()).toContainText("v1 ·");
});

test("playthrough menus rename, and delete after confirmation", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await mockCatalog(page);
  await importRoms(page, [[42, "Birch"]]);
  await page.getByRole("button", { name: "View Birch Quest" }).click();
  const details = page.getByRole("region", { name: "Game details" });

  await page
    .getByRole("button", { name: "More options for Playthrough 1" })
    .click();
  await page.getByRole("menuitem", { name: "Rename…" }).click();
  const rename = page.getByRole("dialog", { name: "Rename playthrough" });
  await expect(rename.getByRole("button", { name: "Rename" })).toBeDisabled();
  await rename.getByRole("textbox", { name: "Name" }).fill("Main");
  await rename.getByRole("button", { name: "Rename" }).click();
  await expect(rename).toHaveCount(0);
  await expect(runsIn(details)).toHaveText([/^Main/]);

  await details.getByRole("button", { name: "New playthrough" }).click();
  const create = page.getByRole("dialog", { name: "New playthrough" });
  await create.getByRole("textbox", { name: "Name" }).fill("Second");
  await create.getByRole("button", { name: "Create & play" }).click();
  await expect(page.getByLabel(/^Game screen/)).toBeVisible();
  await page.getByRole("button", { name: "Library", exact: true }).click();
  await expect(runsIn(details)).toHaveCount(2);

  // Main is no longer the running playthrough.
  await page.getByRole("button", { name: "More options for Main" }).click();
  await page.getByRole("menuitem", { name: "Delete…" }).click();
  const remove = page.getByRole("dialog", { name: "Delete playthrough?" });
  await remove.getByRole("button", { name: "Cancel" }).click();
  await expect(runsIn(details)).toHaveCount(2);
  await page.getByRole("button", { name: "More options for Main" }).click();
  await page.getByRole("menuitem", { name: "Delete…" }).click();
  await remove.getByRole("button", { name: "Delete" }).click();
  await expect(remove).toHaveCount(0);
  await expect(runsIn(details)).toHaveText([/^Second/]);
  expect(await stored(page, "playthroughs", sha1(42), true)).toMatchObject([
    { name: "Second" },
  ]);
});

test("mobile details replace the list, and Back restores its scroll and focus", async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 700 });
  await mockCatalog(page, 8);
  await openLibrary(page);
  await expect.poll(() => titles(page)).toHaveLength(11);
  const list = page.getByRole("region", { name: "Games" });
  const details = page.getByRole("region", { name: "Game details" });
  await expect(details).toBeHidden();
  await page.screenshot({
    path: "../artifacts/library/mobile-list.png",
    fullPage: true,
  });

  const pick = page.getByRole("button", { name: "View Oak 8" });
  await pick.scrollIntoViewIfNeeded();
  const scrolled = await page.evaluate(() => scrollY);
  expect(scrolled).toBeGreaterThan(0);
  await pick.click();
  await expect(list).toBeHidden();
  const heading = details.getByRole("heading", {
    level: 2,
    name: "Oak 8",
  });
  await expect(heading).toBeFocused();
  await expect(
    page.getByRole("navigation", { name: "Main navigation" }),
  ).toBeHidden();
  await page.screenshot({
    path: "../artifacts/library/mobile-details.png",
    fullPage: true,
  });

  await page.getByRole("button", { name: "Back to Library" }).click();
  await expect(list).toBeVisible();
  await expect(pick).toBeFocused();
  expect(await page.evaluate(() => scrollY)).toBe(scrolled);

  await page.getByRole("button", { name: "View Birch Quest" }).click();
  await expect(
    details.getByRole("heading", { level: 2, name: "Birch Quest" }),
  ).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(list).toBeVisible();
  await expect(
    page.getByRole("button", { name: "View Birch Quest" }),
  ).toBeFocused();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
});

test("Add game opens the native picker and plays the chosen ROM", async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await openLibrary(page);
  const picker = page.waitForEvent("filechooser");
  await page.getByRole("button", { name: "Add game" }).click();
  await (await picker).setFiles(loop(42, "Pocket Journey"));
  await expect(
    page.getByRole("heading", { name: "Pocket Journey", exact: true }),
  ).toBeVisible();
  await expect(page.getByLabel(/^Game screen/)).toBeVisible();
});

test("Library pauses and blocks game keys; returning preserves the running or paused session", async ({
  page,
}) => {
  await openGame(page);
  const bytes = () =>
    page.evaluate(async () =>
      Array.from(
        await (
          window as unknown as { emulator: Emulator }
        ).emulator.saveState(),
      ),
    );
  const before = await bytes();
  await page.keyboard.press("Space");
  await page.keyboard.press("r");
  await page.keyboard.press("Control+Digit1");
  expect(await bytes()).toEqual(before);
  const stopped = await frames(page);
  await page.getByRole("button", { name: "Player", exact: true }).click();
  await expect.poll(() => frames(page)).toBeGreaterThan(stopped);
  await page.getByLabel(/^Game screen/).focus();
  await page.keyboard.press("Space");
  await page.waitForTimeout(100);
  const paused = await bytes();
  await page.getByRole("button", { name: "Library", exact: true }).click();
  await page.getByRole("button", { name: "evenfall home" }).click();
  await page.getByRole("button", { name: "Resume adventure" }).click();
  await page.waitForTimeout(100);
  expect(await bytes()).toEqual(paused);
  await page.getByRole("button", { name: "Library", exact: true }).click();
  await page.getByRole("button", { name: "Player", exact: true }).click();
  await page.waitForTimeout(100);
  expect(await bytes()).toEqual(paused);
});

test("rapid navigation waits for a pending resume before pausing again", async ({
  page,
}) => {
  await openGame(page);
  // Delay acknowledgement at the public SDK boundary to expose play/pause races.
  await page.evaluate(() => {
    const emulator = (window as unknown as { emulator: Emulator }).emulator;
    const play = emulator.play.bind(emulator);
    emulator.play = async () => {
      await play();
      await new Promise((resolve) => setTimeout(resolve, 250));
    };
  });
  const before = await frames(page);
  await page.getByRole("button", { name: "Player", exact: true }).click();
  await expect.poll(() => frames(page)).toBeGreaterThan(before);
  await page.getByRole("button", { name: "Library", exact: true }).click();
  await page.waitForTimeout(400);
  const paused = await frames(page);
  await page.waitForTimeout(100);
  expect(await frames(page)).toBe(paused);
  await page.getByRole("button", { name: "Player", exact: true }).click();
  await expect.poll(() => frames(page)).toBeGreaterThan(paused);
});
