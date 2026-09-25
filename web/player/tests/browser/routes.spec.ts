import { expect, test, type Page } from "@playwright/test";
import type { Emulator } from "@evenfall/gba";

const frames = (page: Page) =>
  page.evaluate(async () =>
    Number(
      (await (window as unknown as { emulator: Emulator }).emulator.inspect())
        .frames,
    ),
  );

test("each page has a URL that survives a reload", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await page.getByRole("button", { name: "Browse library" }).click();
  await expect(page).toHaveURL(/\/library$/);
  await page.getByRole("button", { name: "View Emerald Seaglass" }).click();
  await expect(page).toHaveURL(/\/library\/emerald-seaglass$/);

  await page.reload();
  const details = page.getByRole("region", { name: "Game details" });
  await expect(
    details.getByRole("heading", { level: 2, name: "Emerald Seaglass" }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "View Emerald Seaglass" }),
  ).toHaveAttribute("aria-current", "true");

  // A fresh page has no running game: /player opens the Library.
  await page.goto("/player");
  await expect(page).toHaveURL(/\/library$/);
  await page.getByRole("button", { name: "Player", exact: true }).click();
  await expect(page).toHaveURL(/\/player$/);
  await expect(
    page.getByRole("button", { name: "Player", exact: true }),
  ).toHaveAttribute("aria-current", "page");
  await page.getByRole("button", { name: "evenfall home" }).click();
  await expect(page).toHaveURL(/\/$/);
  await expect(
    page.getByRole("button", { name: "Enter player" }),
  ).toBeVisible();
});

test("a shared game link opens its details on a phone, and Back returns to the list", async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/library/pokemon-unbound");
  // An unknown ID falls back to the first game rather than an empty page.
  await expect(
    page.getByRole("region", { name: "Game details" }).getByRole("heading", {
      level: 2,
    }),
  ).toHaveText("Elite Redux");
  await page.goto("/library/unbound");
  await expect(
    page
      .getByRole("region", { name: "Game details" })
      .getByRole("heading", { level: 2, name: "Pokémon Unbound" }),
  ).toBeFocused();
  await expect(page.getByRole("region", { name: "Games" })).toBeHidden();
  await page.getByRole("button", { name: "Back to Library" }).click();
  await expect(page).toHaveURL(/\/library$/);
  await expect(page.getByRole("region", { name: "Games" })).toBeVisible();
  await expect(
    page.getByRole("button", { name: "View Pokémon Unbound" }),
  ).toBeFocused();
});

test("browser Back and Forward pause and resume the player like the tabs", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Enter player" }).click();
  await expect(page).toHaveURL(/\/player$/);
  await page.getByLabel("Open ROM", { exact: true }).setInputFiles({
    name: "Loop.gba",
    mimeType: "application/octet-stream",
    buffer: Buffer.from([42, 0, 160, 227, 254, 255, 255, 234]),
  });
  await expect(page.getByLabel(/^Game screen/)).toBeVisible();
  const started = await frames(page);
  await expect.poll(() => frames(page)).toBeGreaterThan(started);

  await page.getByRole("button", { name: "Library", exact: true }).click();
  await expect(page).toHaveURL(/\/library$/);
  await page.waitForTimeout(150);
  const paused = await frames(page);
  await page.waitForTimeout(150);
  expect(await frames(page)).toBe(paused);

  await page.goBack();
  await expect(page).toHaveURL(/\/player$/);
  await expect.poll(() => frames(page)).toBeGreaterThan(paused);

  await page.goForward();
  await expect(page).toHaveURL(/\/library$/);
  await expect(page.getByRole("main", { name: "Library" })).toBeVisible();
  await page.waitForTimeout(150);
  const pausedAgain = await frames(page);
  await page.waitForTimeout(150);
  expect(await frames(page)).toBe(pausedAgain);
});

test("only the player's pages load the player; other paths are 404s", async ({
  page,
}) => {
  for (const path of ["/", "/library", "/library/elite-redux", "/player"])
    expect((await page.request.get(path)).status(), path).toBe(200);
  for (const path of ["/libraries", "/library/a/b", "/player/1"])
    expect((await page.request.get(path)).status(), path).toBe(404);
});
