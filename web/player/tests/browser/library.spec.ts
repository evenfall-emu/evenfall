import { expect, test, type Page } from "@playwright/test";
import type { Emulator } from "@evenfall/gba";

// The Library shows static catalog data (src/pages/library/fixtures.ts) until
// it reads the catalog API and this device.
const catalogOrder = [
  "Elite Redux",
  "Emerald Seaglass",
  "Fire Emblem: Vision Quest",
  "Golden Sun: Reloaded",
  "Pokémon Unbound",
];

async function openLibrary(page: Page) {
  await page.goto("/");
  await page.getByRole("button", { name: "Browse library" }).click();
  await expect(page.getByRole("main", { name: "Library" })).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
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

const loop = (marker: number, name: string) => ({
  name: `${name}.gba`,
  mimeType: "application/octet-stream",
  buffer: Buffer.from([marker, 0, 160, 227, 254, 255, 255, 234]),
});

async function openGame(page: Page) {
  await page.goto("/");
  await page.getByRole("button", { name: "Enter player" }).click();
  await expect(page.getByLabel("Open ROM", { exact: true })).toBeEnabled();
  await page
    .getByLabel("Open ROM", { exact: true })
    .setInputFiles(loop(42, "Birch"));
  await expect(
    page.getByRole("heading", { name: "Birch", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Library", exact: true }).click();
}

const frames = (page: Page) =>
  page.evaluate(async () =>
    Number(
      (await (window as unknown as { emulator: Emulator }).emulator.inspect())
        .frames,
    ),
  );

test("Library lists the catalog with each game's state and filters it", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await openLibrary(page);
  await expect(
    page.getByRole("button", { name: "Library", exact: true }),
  ).toHaveAttribute("aria-current", "page");
  expect(await titles(page)).toEqual(catalogOrder);
  await expect(page.getByText("On this device")).toHaveCount(2);
  await expect(
    page.getByRole("button", { name: "Install Fire Emblem: Vision Quest" }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", {
      name: "Add Golden Sun: The Lost Age (US) for Golden Sun: Reloaded",
    }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Play Golden Sun: Reloaded" }),
  ).toHaveCount(0);

  const details = page.getByRole("region", { name: "Game details" });
  await expect(
    details.getByRole("heading", { level: 2, name: "Elite Redux" }),
  ).toBeVisible();
  await expect(details).toContainText("Available locally");
  await expect(details).toContainText("v2.6 available");
  for (const [run, status] of [
    ["Main adventure", "Cloud synced"],
    ["Randomizer run", "Local changes not backed up"],
    ["Challenge run", "Cloud backup · Not downloaded here"],
  ])
    await expect(
      details.getByRole("listitem").filter({ hasText: run }),
    ).toContainText(status);
  await page.screenshot({ path: "../artifacts/library/desktop.png" });

  await page.getByRole("button", { name: "View Golden Sun: Reloaded" }).click();
  await expect(details).toContainText("Needs Golden Sun: The Lost Age (US)");
  await expect(
    details.getByRole("button", { name: "New playthrough" }),
  ).toHaveCount(0);

  await page.getByRole("searchbox", { name: "Search games" }).fill("sea");
  expect(await titles(page)).toEqual(["Emerald Seaglass"]);
  // The selection is the URL's game: filtering the list does not replace it.
  await expect(
    details.getByRole("heading", { level: 2, name: "Golden Sun: Reloaded" }),
  ).toBeVisible();
  await page.getByRole("searchbox", { name: "Search games" }).fill("");

  await choose(page, "Show", "Ready");
  expect(await titles(page)).toEqual([
    "Fire Emblem: Vision Quest",
    "Pokémon Unbound",
  ]);
  await choose(page, "Show", "Installed");
  expect(await titles(page)).toEqual(catalogOrder.slice(0, 2));
  await choose(page, "Show", "All");
  await choose(page, "Sort", "Popularity");
  expect(await titles(page)).toEqual([
    "Pokémon Unbound",
    "Elite Redux",
    "Fire Emblem: Vision Quest",
    "Emerald Seaglass",
    "Golden Sun: Reloaded",
  ]);
  await choose(page, "Sort", "Played");
  await choose(page, "Franchise", "Fire Emblem");
  expect(await titles(page)).toEqual(["Fire Emblem: Vision Quest"]);
  await choose(page, "Franchise", "All");

  await page.getByRole("combobox", { name: "Tags" }).fill("diff");
  await page.keyboard.press("Enter");
  expect(await titles(page)).toEqual(["Elite Redux", "Golden Sun: Reloaded"]);
  await page.getByRole("combobox", { name: "Tags" }).fill("rand");
  await page.getByRole("option", { name: "Randomizer" }).click();
  expect(await titles(page)).toEqual(["Elite Redux"]);
  await page.getByRole("button", { name: "Remove Difficulty Hack" }).click();
  await page.getByRole("button", { name: "Remove Randomizer" }).click();
  expect(await titles(page)).toEqual(catalogOrder);

  await page.getByRole("searchbox", { name: "Search games" }).fill("zzz");
  await expect(
    page.getByRole("heading", { name: "No matching games" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Clear filters" }).click();
  expect(await titles(page)).toEqual(catalogOrder);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
});

test("Update and progress choices stay disabled until the risk is acknowledged", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await openLibrary(page);

  await page.getByRole("button", { name: "Update…" }).click();
  const update = page.getByRole("dialog", { name: "Elite Redux · v2.6" });
  await update.getByRole("combobox", { name: "Playthrough to update" }).click();
  await expect(page.getByRole("option")).toHaveText([
    "Main adventure · v2.5",
    "Randomizer run · v2.5",
    "Challenge run · v2.4",
  ]);
  await page.getByRole("option", { name: "Challenge run · v2.4" }).click();
  await expect(update).toContainText("Save compatibility not verified");
  const copy = update.getByRole("button", { name: "Copy save & play" });
  await expect(copy).toBeDisabled();
  await update
    .getByRole("checkbox", {
      name: "I understand compatibility is unverified.",
    })
    .click();
  await update.screenshot({ path: "../artifacts/library/update.png" });
  await copy.click();
  await expect(update).toHaveCount(0);
  await expect(page.locator(".console-notice")).toContainText(
    "Library preview",
  );

  await page.getByRole("button", { name: "Play Randomizer run" }).click();
  const progress = page.getByRole("dialog", { name: "Choose progress" });
  await expect(progress).toContainText("Elite Redux · Randomizer run · v2.5");
  const use = progress.getByRole("button", { name: "Use selected progress" });
  await expect(use).toBeDisabled();
  await progress.getByRole("radio", { name: /Cloud backup/ }).click();
  await expect(use).toBeEnabled();
  await progress.screenshot({ path: "../artifacts/library/progress.png" });
  await page.keyboard.press("Escape");
  await expect(progress).toHaveCount(0);

  await page.getByRole("button", { name: "Play Main adventure" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);

  await page.getByRole("button", { name: "New playthrough" }).click();
  const create = page.getByRole("dialog", { name: "New playthrough" });
  await create.getByRole("textbox", { name: "Name" }).fill("  ");
  await expect(
    create.getByRole("button", { name: "Create & play" }),
  ).toBeDisabled();
  await create.getByRole("textbox", { name: "Name" }).fill("Nuzlocke");
  await create.getByRole("button", { name: "Create & play" }).click();
  await expect(create).toHaveCount(0);
});

test("mobile details replace the list, and Back restores its scroll and focus", async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 700 });
  await openLibrary(page);
  const list = page.getByRole("region", { name: "Games" });
  const details = page.getByRole("region", { name: "Game details" });
  await expect(details).toBeHidden();
  await page.screenshot({
    path: "../artifacts/library/mobile-list.png",
    fullPage: true,
  });

  const pick = page.getByRole("button", { name: "View Pokémon Unbound" });
  await pick.scrollIntoViewIfNeeded();
  const scrolled = await page.evaluate(() => scrollY);
  expect(scrolled).toBeGreaterThan(0);
  await pick.click();
  await expect(list).toBeHidden();
  const heading = details.getByRole("heading", {
    level: 2,
    name: "Pokémon Unbound",
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

  await page.getByRole("button", { name: "View Elite Redux" }).click();
  await expect(
    details.getByRole("heading", { level: 2, name: "Elite Redux" }),
  ).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(list).toBeVisible();
  await expect(
    page.getByRole("button", { name: "View Elite Redux" }),
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
