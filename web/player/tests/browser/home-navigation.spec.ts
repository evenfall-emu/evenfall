import { expect, test, type Page } from "@playwright/test";
import type { Emulator } from "@evenfall/gba";

const loop = {
  name: "Loop.gba",
  mimeType: "application/octet-stream",
  buffer: Buffer.from([42, 0, 160, 227, 254, 255, 255, 234]),
};

async function ready(page: Page) {
  await page.goto("/");
  await expect(page.getByRole("button", { name: "Open player" })).toBeVisible();
}

// The shared header; Home has its own.
const header = (page: Page) =>
  page.locator("header").filter({
    has: page.getByRole("button", { name: "evenfall home" }),
  });

const frames = (page: Page) =>
  page.evaluate(async () =>
    Number(
      (await (window as unknown as { emulator: Emulator }).emulator.inspect())
        .frames,
    ),
  );

test("Home is the default and preserves a playing session across navigation", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await ready(page);

  await expect(page.locator(".landing")).toBeVisible();
  await expect(header(page)).toHaveCount(0);
  await expect(page.locator(".player-tools")).toBeHidden();
  // Home's own navigation, without the shared header's tabs.
  const nav = page.getByRole("navigation", { name: "Main navigation" });
  await expect(nav.getByRole("link", { name: "Features" })).toBeVisible();
  await expect(nav.getByRole("button", { name: "Player" })).toHaveCount(0);
  await expect(page.getByText(/Made by/)).toHaveCount(0);
  // The hero scene fills the first view.
  expect(
    await page
      .getByRole("region", {
        name: "The games you love. Worlds you haven’t met.",
      })
      .evaluate((element) => {
        const rect = element.getBoundingClientRect();
        return rect.top === 0 && rect.bottom === innerHeight;
      }),
  ).toBe(true);
  await expect(page.getByRole("button", { name: "About" })).toHaveCount(0);
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({
    path: "../artifacts/home-desktop.png",
    fullPage: true,
  });

  await page.getByRole("button", { name: "Open player" }).click();
  await expect(header(page)).toHaveCount(1);
  await expect(page.locator(".player-tools")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Player", exact: true }),
  ).toHaveAttribute("aria-current", "page");
  await expect(page.getByLabel("Open ROM", { exact: true })).toBeEnabled();
  await page.getByLabel("Open ROM", { exact: true }).setInputFiles(loop);
  const canvas = page.getByLabel(/^Game screen/);
  await expect(canvas).toBeVisible();
  const started = await frames(page);
  await expect.poll(() => frames(page)).toBeGreaterThan(started);

  await page.getByRole("button", { name: "evenfall home" }).click();
  await expect(page.locator(".landing")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Resume adventure" }),
  ).toBeVisible();
  await page.waitForTimeout(150);
  const paused = await frames(page);
  await page.waitForTimeout(150);
  expect(await frames(page)).toBe(paused);

  await expect(header(page)).toHaveCount(0);
  await expect(page.locator(".player-tools")).toBeHidden();
  await page.getByRole("button", { name: "Resume adventure" }).click();
  await expect(canvas).toBeVisible();
  await expect.poll(() => frames(page)).toBeGreaterThan(paused);
});

test("Home navigation remains usable on mobile", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await ready(page);

  await expect(header(page)).toHaveCount(0);
  await expect(page.locator(".player-tools")).toBeHidden();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBe(true);
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({
    path: "../artifacts/home-mobile.png",
    fullPage: true,
  });

  await page.getByRole("button", { name: "Open player" }).click();
  await expect(page.getByLabel(/^Game screen/)).toBeVisible();
  await expect(page.getByRole("button", { name: "Library" })).toBeVisible();
  await page.getByRole("button", { name: "Library", exact: true }).click();
  await expect(header(page)).toHaveCount(1);
  await expect(
    page.getByRole("button", { name: "Library", exact: true }),
  ).toHaveAttribute("aria-current", "page");
  await expect(page.locator(".player-tools")).toBeHidden();
  await page.getByRole("button", { name: "evenfall home" }).click();
  await expect(page.locator(".landing")).toBeVisible();
  await expect(header(page)).toHaveCount(0);
  await expect(page.locator(".player-tools")).toBeHidden();
});

test("shared navigation fits mobile when pixel fonts cannot load", async ({
  page,
}) => {
  await page.setViewportSize({ width: 360, height: 800 });
  await page.route("**/*.ttf", (route) => route.abort());
  await ready(page);
  await page.getByRole("button", { name: "Open player" }).click();
  await expect(header(page)).toBeVisible();
  await page.evaluate(() => document.fonts.ready);

  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(
    360,
  );
  for (const button of await header(page).getByRole("button").all()) {
    const bounds = await button.boundingBox();
    expect(bounds!.x).toBeGreaterThanOrEqual(0);
    expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(360);
  }
  await page.screenshot({
    path: "../artifacts/home-mobile-fallback.png",
    fullPage: true,
  });
  await page.getByRole("button", { name: "Library", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Library", exact: true }),
  ).toHaveAttribute("aria-current", "page");
});
