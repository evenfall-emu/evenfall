import { expect, test, type Page } from "@playwright/test";
import { build } from "vite-plus";
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
  // No player entrypoint or styles: only the fixture's own console CSS.
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

test("only unconfirmed auto-detected HUDs show an icon with accessible help", async ({
  browser,
}, testInfo) => {
  const context = await browser.newContext({
    hasTouch: true,
    viewport: { width: 1200, height: 800 },
  });
  const page = await context.newPage();
  await openFixture(page);
  const label = "HUD compatibility warning";
  for (const name of ["Fully located HUD", "Reviewed HUD"])
    await expect(
      page.getByRole("region", { name }).getByRole("button", { name: label }),
    ).toHaveCount(0);
  await expect(page.getByText("Detected build · not reviewed")).toHaveCount(0);
  const heading = page.getByRole("region", { name: "Partially located HUD" });
  const warning = heading.getByRole("button", { name: label });
  await expect(warning).toBeVisible();
  await expect(warning).toHaveText("");
  const glyph = await warning.locator("svg").boundingBox();
  expect(glyph!.width).toBe(16);
  expect(glyph!.height).toBe(16);
  const fullHeading = await page
    .getByRole("region", { name: "Fully located HUD" })
    .boundingBox();
  expect((await heading.boundingBox())!.height).toBe(fullHeading!.height);
  await heading.screenshot({ path: testInfo.outputPath("warning-icon.png") });
  await warning.hover();
  expect(
    await warning.evaluate((element) => {
      const style = getComputedStyle(element);
      return [style.backgroundColor, style.borderWidth];
    }),
  ).toEqual(["rgba(0, 0, 0, 0)", "0px"]);
  const tip = page.getByRole("tooltip");
  await expect(tip).toContainText(
    "This HUD was auto-detected with some unconfirmed values and may not work perfectly. Please report any bugs you find.",
  );
  await page.screenshot({ path: testInfo.outputPath("warning-hover.png") });
  await page.keyboard.press("Escape");
  await expect(tip).toHaveCount(0);
  await warning.focus();
  await expect(tip).toBeVisible();
  await expect(warning).toHaveAccessibleDescription(/Please report any bugs/);
  await page.screenshot({ path: testInfo.outputPath("warning-focus.png") });
  await page.keyboard.press("Escape");
  await page.getByRole("button", { name: "Outside", exact: true }).click();
  await page.setViewportSize({ width: 390, height: 844 });
  const target = await warning.boundingBox();
  // Meet the 24px inline target minimum without inflating the heading.
  expect(target!.width).toBeGreaterThanOrEqual(24);
  expect(target!.height).toBeGreaterThanOrEqual(24);
  expect((await heading.boundingBox())!.height).toBe(
    (await page
      .getByRole("region", { name: "Fully located HUD" })
      .boundingBox())!.height,
  );
  // The invisible hit target remains usable outside the 16px glyph.
  await page.touchscreen.tap(target!.x + 3, target!.y + 3);
  await expect(tip).toBeVisible();
  const bounds = await tip.boundingBox();
  expect(bounds!.x).toBeGreaterThanOrEqual(11);
  expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(379);
  await page.screenshot({ path: testInfo.outputPath("warning-touch.png") });
  await context.close();
});
