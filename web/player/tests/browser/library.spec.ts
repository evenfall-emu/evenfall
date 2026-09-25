import { expect, test, type Page } from "@playwright/test";
import type { Emulator } from "@evenfall/gba";

async function openGames(page: Page) {
  await page.goto("/");
  await page.getByRole("button", { name: "Enter player" }).click();
  const open = page
    .locator(".app-header")
    .getByRole("button", { name: "Add game", exact: true });
  await expect(open).toBeEnabled();
  for (const [index, name] of ["Zeta", "Aurora", "Birch"].entries()) {
    await page.getByLabel("Open ROM", { exact: true }).setInputFiles({
      name: `${name}.gba`,
      mimeType: "application/octet-stream",
      buffer: Buffer.from([42 + index, 0, 160, 227, 254, 255, 255, 234]),
    });
    await expect(
      page.getByRole("heading", { name, exact: true }),
    ).toBeVisible();
    await expect(open).toBeEnabled();
  }
  await page.getByRole("button", { name: "Library", exact: true }).click();
}

test("Library lists real imports, searches and sorts them, and opens a selected game", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await openGames(page);
  await expect(
    page.getByRole("heading", { name: "Library", exact: true }),
  ).toBeVisible();
  await expect(page.getByRole("dialog", { name: "Your games" })).toHaveCount(0);
  const titles = page
    .getByRole("list", { name: "Games" })
    .getByRole("heading", { level: 3 });
  await expect(titles).toHaveText(["Birch", "Aurora", "Zeta"]);
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({
    path: "../artifacts/library-implementation/desktop-local.png",
  });
  await expect(page.getByRole("combobox", { name: "Sort games" })).toHaveValue(
    "recent",
  );
  await page
    .getByRole("combobox", { name: "Sort games" })
    .selectOption("title");
  await expect(titles).toHaveText(["Aurora", "Birch", "Zeta"]);
  const search = page.getByRole("searchbox", { name: "Search games" });
  await search.focus();
  await expect(search).toHaveCSS("outline-style", "none");
  await expect(page.locator(".library-search")).toHaveCSS(
    "outline-style",
    "solid",
  );
  await page.locator(".library-controls").screenshot({
    path: "../artifacts/library-implementation/search-focus-desktop.png",
  });
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(search).toBeFocused();
  await page.locator(".library-controls").screenshot({
    path: "../artifacts/library-implementation/search-focus-mobile.png",
  });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await search.fill("  zEtA  ");
  await expect(titles).toHaveText(["Zeta"]);
  await page.getByRole("button", { name: "View Zeta", exact: true }).click();
  const detail = page.getByRole("region", { name: "Game details" });
  await expect(
    detail.getByRole("heading", { name: "Zeta", exact: true }),
  ).toBeVisible();
  await expect(detail).toContainText("Installed");
  await expect(page.getByText(/new runs started/)).toHaveCount(0);
  await detail
    .getByRole("button", { name: "Continue Zeta", exact: true })
    .click();
  await expect(page.getByLabel(/^Game screen/)).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Zeta", exact: true }),
  ).toBeVisible();
  const frames = () =>
    page.evaluate(async () =>
      Number(
        (await (window as unknown as { emulator: Emulator }).emulator.inspect())
          .frames,
      ),
    );
  const started = await frames();
  await expect.poll(frames).toBeGreaterThan(started);
});

test("mobile details replace the list and Back restores filters, scroll and focus", async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 480 });
  await openGames(page);
  const detail = page.getByRole("region", { name: "Game details" });
  await expect(detail).toBeHidden();
  await page
    .getByRole("combobox", { name: "Show games" })
    .selectOption("installed");
  await page.getByRole("searchbox", { name: "Search games" }).fill("r");
  const row = page.getByRole("button", { name: "View Aurora", exact: true });
  await row.scrollIntoViewIfNeeded();
  const pane = page.locator(".library-list-pane");
  const scroll = await pane.evaluate((element) => element.scrollTop);
  await row.click();
  await expect(detail).toBeVisible();
  await expect(page.getByRole("list", { name: "Games" })).toBeHidden();
  await expect(detail.getByRole("heading", { name: "Aurora" })).toBeFocused();
  await page.getByRole("button", { name: "Back to library" }).click();
  await expect(row).toBeFocused();
  await expect(detail).toBeHidden();
  await expect(
    page.getByRole("searchbox", { name: "Search games" }),
  ).toHaveValue("r");
  await expect(page.getByRole("combobox", { name: "Show games" })).toHaveValue(
    "installed",
  );
  expect(await pane.evaluate((element) => element.scrollTop)).toBe(scroll);
  for (const width of [360, 390, 768]) {
    await page.setViewportSize({ width, height: 844 });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
  }
});

test("Library pauses and blocks game keys; returning preserves the running or paused session", async ({
  page,
}) => {
  await openGames(page);
  const frames = () =>
    page.evaluate(async () =>
      Number(
        (await (window as unknown as { emulator: Emulator }).emulator.inspect())
          .frames,
      ),
    );
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
  const stopped = await frames();
  await page.getByRole("button", { name: "Player", exact: true }).click();
  await expect.poll(frames).toBeGreaterThan(stopped);
  await page.getByLabel(/^Game screen/).focus();
  await page.keyboard.press("Space");
  await page.waitForTimeout(100);
  const paused = await bytes();
  await page.getByRole("button", { name: "Library", exact: true }).click();
  await page.getByRole("button", { name: "Home", exact: true }).click();
  await page.getByRole("button", { name: "Resume adventure" }).click();
  await page.waitForTimeout(100);
  expect(await bytes()).toEqual(paused);
  await page.getByRole("button", { name: "Library", exact: true }).click();
  const resumedFrom = await frames();
  await page
    .getByRole("region", { name: "Game details" })
    .getByRole("button", { name: "Continue Birch", exact: true })
    .click();
  await expect.poll(frames).toBeGreaterThan(resumedFrom);
});

test("empty Library imports through its native picker and reloads the installed collection", async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/");
  await page.getByRole("button", { name: "Enter player" }).click();
  await page.getByRole("button", { name: "Library", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Your next adventure starts here." }),
  ).toBeVisible();
  await page.screenshot({
    path: "../artifacts/library-implementation/mobile-empty.png",
  });
  const picker = page.waitForEvent("filechooser");
  await page
    .locator(".app-header")
    .getByRole("button", { name: "Add game", exact: true })
    .click();
  await (
    await picker
  ).setFiles({
    name: "Pocket Journey.gba",
    mimeType: "application/octet-stream",
    buffer: Buffer.from([42, 0, 160, 227, 254, 255, 255, 234]),
  });
  await expect(
    page.getByRole("heading", { name: "Pocket Journey", exact: true }),
  ).toBeVisible();
  await page.reload();
  await page.getByRole("button", { name: "Browse library" }).click();
  await expect(
    page.getByRole("button", { name: "View Pocket Journey" }),
  ).toBeVisible();
  await page.screenshot({
    path: "../artifacts/library-implementation/mobile-local.png",
  });
  await page
    .getByRole("combobox", { name: "Show games" })
    .selectOption("not-installed");
  await expect(
    page.getByRole("heading", { name: "No matching games" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Clear filters" }).click();
  await page.getByRole("button", { name: "View Pocket Journey" }).click();
  await page.screenshot({
    path: "../artifacts/library-implementation/mobile-local-detail.png",
  });
  await page
    .getByRole("button", { name: /^(Play|Continue) Pocket Journey$/ })
    .click();
  await expect(page.getByLabel(/^Game screen/)).toBeVisible();
});

test("rapid navigation waits for a pending resume before pausing again", async ({
  page,
}) => {
  await openGames(page);
  // Delay acknowledgement at the public SDK boundary to expose play/pause races.
  await page.evaluate(() => {
    const emulator = (window as unknown as { emulator: Emulator }).emulator;
    const play = emulator.play.bind(emulator);
    emulator.play = async () => {
      await play();
      await new Promise((resolve) => setTimeout(resolve, 250));
    };
  });
  const frames = () =>
    page.evaluate(async () =>
      Number(
        (await (window as unknown as { emulator: Emulator }).emulator.inspect())
          .frames,
      ),
    );
  const before = await frames();
  await page.getByRole("button", { name: "Player", exact: true }).click();
  await expect.poll(frames).toBeGreaterThan(before);
  await page.getByRole("button", { name: "Library", exact: true }).click();
  await page.waitForTimeout(400);
  const paused = await frames();
  await page.waitForTimeout(100);
  expect(await frames()).toBe(paused);
  await page.getByRole("button", { name: "Player", exact: true }).click();
  await expect.poll(frames).toBeGreaterThan(paused);
});
