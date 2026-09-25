import { expect, test, type Page } from "@playwright/test";
import { build } from "vite";
import { fixtureBuild } from "./support";
import { resolve } from "node:path";
import { rm } from "node:fs/promises";

const outDir = "../../artifacts/tooltips-fixture";
test.beforeAll(async () => {
  await build({
    ...fixtureBuild,
    build: {
      outDir,
      emptyOutDir: true,
      lib: {
        entry: resolve("tests/browser/tooltips.fixture.tsx"),
        formats: ["es"],
        fileName: () => "fixture.js",
        cssFileName: "fixture",
      },
    },
  });
});
test.afterAll(() => rm(outDir, { recursive: true, force: true }));
async function openFixture(page: Page) {
  // No player/debug entrypoint or styles: only the fixture's own console CSS.
  await page.route("**/tooltip-fixture", (route) =>
    route.fulfill({
      contentType: "text/html",
      body: "<!doctype html><html><head></head><body></body></html>",
    }),
  );
  await page.goto("/tooltip-fixture");
  await page.addStyleTag({ path: resolve(outDir, "fixture.css") });
  await page.addScriptTag({
    type: "module",
    path: resolve(outDir, "fixture.js"),
  });
  await expect(
    page.getByRole("button", { name: "Ability: Adaptability" }),
  ).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
}

test("info tips support hover transfer, focus, Escape and outside dismissal", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1200, height: 800 });
  await openFixture(page);
  const trigger = page.getByRole("button", { name: "Ability: Adaptability" });
  const tip = page.getByRole("tooltip");
  await trigger.hover();
  await expect(tip).toContainText("Powers up moves");
  const anchor = await page
    .locator("[data-tooltip-anchor]")
    .first()
    .boundingBox();
  const box = await tip.boundingBox();
  expect(box!.x).toBeCloseTo(anchor!.x + anchor!.width + 12, 0);
  await tip.hover();
  await expect(tip).toBeVisible();
  await page.screenshot({
    path: `../../artifacts/tooltips/${process.env.TOOLTIP_CAPTURE ?? "after"}-desktop.png`,
  });
  await page.keyboard.press("Escape");
  await expect(tip).toHaveCount(0);
  await page.keyboard.press("Tab");
  await expect(trigger).toBeFocused();
  await expect(tip).toBeVisible();
  await expect(trigger).toHaveAccessibleDescription(/Powers up moves/);
  await page.keyboard.press("Escape");
  await expect(tip).toHaveCount(0);
  await trigger.click();
  await expect(tip).toBeVisible();
  for (const width of [390, 768, 1200]) {
    await page.setViewportSize({ width, height: 800 });
    await expect(tip).toBeVisible();
    await expect
      .poll(async () => {
        const bounds = await tip.boundingBox();
        return (
          !!bounds && bounds.x >= 11 && bounds.x + bounds.width <= width - 11
        );
      })
      .toBe(true);
  }
  await page.getByRole("button", { name: "Outside", exact: true }).click();
  await expect(tip).toHaveCount(0);
});

test("touch opens long details at the viewport edge and permits content scrolling", async ({
  browser,
}) => {
  const context = await browser.newContext({
    hasTouch: true,
    viewport: { width: 390, height: 844 },
  });
  const page = await context.newPage();
  await openFixture(page);
  await page.getByRole("button", { name: "Long move details" }).tap();
  const tip = page.getByRole("tooltip");
  await expect(tip).toBeVisible();
  const bounds = await tip.boundingBox();
  expect(bounds!.x).toBeGreaterThanOrEqual(11);
  expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(379);
  expect(bounds!.y).toBeGreaterThanOrEqual(11);
  expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(833);
  await tip.locator(".info-tooltip-body").evaluate((element) => {
    element.scrollTop = element.scrollHeight;
  });
  await expect(
    tip.getByText("Move effect 40:", { exact: false }),
  ).toBeInViewport();
  await expect(tip).toBeVisible();
  await page.screenshot({
    path: `../../artifacts/tooltips/${process.env.TOOLTIP_CAPTURE ?? "after"}-mobile.png`,
  });
  await page.touchscreen.tap(2, 2);
  await expect(tip).toHaveCount(0);
  const ability = page.getByRole("button", { name: "Ability: Adaptability" });
  await ability.hover();
  await page.mouse.move(2, 2);
  await ability.tap();
  await expect(tip).toContainText("Powers up moves");
  // A pending mouse hover-close must not dismiss a newer touch-open.
  await page.waitForTimeout(200);
  await expect(tip).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(tip).toHaveCount(0);
  await context.close();
});
