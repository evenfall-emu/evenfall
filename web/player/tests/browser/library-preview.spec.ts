import { expect, test, type Page } from "@playwright/test";
import { build } from "vite";
import { resolve } from "node:path";
import { rm } from "node:fs/promises";

const outDir = "../artifacts/library-fixture";
test.beforeAll(async () => {
  await build({
    configFile: false,
    logLevel: "error",
    define: { "process.env.NODE_ENV": '"development"' },
    build: {
      outDir,
      emptyOutDir: true,
      lib: {
        entry: resolve("tests/browser/library.fixture.tsx"),
        formats: ["es"],
        fileName: () => "fixture.js",
        cssFileName: "fixture",
      },
    },
  });
});
test.afterAll(() => rm(outDir, { recursive: true, force: true }));
async function openFixture(page: Page) {
  await page.route("**/library-fixture", (route) =>
    route.fulfill({
      contentType: "text/html",
      body: "<!doctype html><html><head></head><body></body></html>",
    }),
  );
  await page.goto("/library-fixture");
  await page.addStyleTag({ path: resolve(outDir, "fixture.css") });
  await page.addScriptTag({
    type: "module",
    path: resolve(outDir, "fixture.js"),
  });
  await expect(
    page.getByRole("heading", { name: "Library", exact: true }),
  ).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
}

test("catalog preview separates installed, ready and missing-base states without claiming installation", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await openFixture(page);
  const titles = page
    .getByRole("list", { name: "Games" })
    .getByRole("heading", { level: 3 });
  await expect(titles).toHaveText([
    "Heart & Soul",
    "Pokémon Emerald",
    "My imported game",
    "Elite Redux",
    "Emerald Seaglass",
    "Gaia",
    "Radical Red",
    "Unbound",
  ]);
  await page
    .getByRole("combobox", { name: "Show games" })
    .selectOption("not-installed");
  await expect(titles).toHaveText([
    "Elite Redux",
    "Emerald Seaglass",
    "Gaia",
    "Radical Red",
    "Unbound",
  ]);
  await page.getByRole("button", { name: "View Unbound", exact: true }).click();
  const detail = page.getByRole("region", { name: "Game details" });
  await expect(detail).toContainText("Base ROM required");
  await expect(detail).toContainText("Pokémon FireRed · US 1.0");
  await expect(page.getByText(/new runs started/)).toHaveCount(0);
  await page.getByRole("combobox", { name: "Show games" }).selectOption("all");
  const status = page
    .getByRole("listitem")
    .filter({
      has: page.getByRole("heading", { name: "Unbound", exact: true }),
    })
    .getByRole("button", { name: "Base ROM required" });
  await status.hover();
  await expect(page.getByRole("tooltip")).toBeVisible();
  await page.screenshot({
    path: "../artifacts/library-implementation/desktop-preview.png",
  });
  await page.keyboard.press("Escape");
  await expect(page.getByRole("tooltip")).toHaveCount(0);
  await status.focus();
  await expect(page.getByRole("tooltip")).toHaveText("Base ROM required");
  await expect(status).toHaveAccessibleDescription("Base ROM required");
  const picker = page.waitForEvent("filechooser");
  await detail
    .getByRole("button", { name: "Choose file", exact: true })
    .click();
  await (
    await picker
  ).setFiles({
    name: "base.gba",
    mimeType: "application/octet-stream",
    buffer: Buffer.from([1, 2, 3]),
  });
  await expect(
    detail.getByRole("button", { name: "Choose file", exact: true }),
  ).toBeDisabled();
  await expect(page.getByRole("tooltip")).toHaveCount(0);
  await expect(page.getByRole("alert")).toContainText("Preview only");
  await expect(detail).toContainText("Base ROM required");
  await page.screenshot({
    path: "../artifacts/library-implementation/preview-error.png",
  });
  const transfer = await page.evaluateHandle(() => {
    const data = new DataTransfer();
    data.items.add(new File([new Uint8Array([3, 2, 1])], "dropped.gba"));
    return data;
  });
  await page
    .locator(".library-base-drop")
    .dispatchEvent("drop", { dataTransfer: transfer });
  await expect(page.getByRole("alert")).toHaveCount(0);
  await expect(page.getByRole("alert")).toContainText("Preview only");
  await page.getByRole("button", { name: "View Elite Redux" }).click();
  await expect(detail).toContainText("Base ROM ready");
  await expect(
    detail.getByRole("button", { name: "Install & play Elite Redux" }),
  ).toBeVisible();
  await page.mouse.move(0, 0);
  await page.screenshot({
    path: "../artifacts/library-implementation/preview-ready.png",
  });
  await detail
    .getByRole("button", { name: "Install & play Elite Redux" })
    .click();
  await expect(page.getByRole("alert")).toContainText("Preview only");
  await expect(detail).toContainText("Base ROM ready");
});

test("status hints support touch without selecting a game", async ({
  browser,
}) => {
  const context = await browser.newContext({
    hasTouch: true,
    viewport: { width: 390, height: 844 },
  });
  const page = await context.newPage();
  await openFixture(page);
  const status = page
    .getByRole("button", { name: "Base ROM ready", exact: true })
    .first();
  await status.tap();
  const tip = page.getByRole("tooltip");
  await expect(tip).toHaveText("Base ROM ready");
  await expect(
    page.getByRole("button", { name: "Back to library" }),
  ).not.toBeVisible();
  await page.screenshot({
    path: "../artifacts/tooltips/after-library-mobile.png",
  });
  await page.touchscreen.tap(2, 2);
  await expect(tip).toHaveCount(0);
  await context.close();
});

test("mobile catalog details use a native picker and restore a scrolled list", async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await openFixture(page);
  await page.screenshot({
    path: "../artifacts/library-implementation/mobile-preview-list.png",
  });
  const row = page.getByRole("button", { name: "View Unbound", exact: true });
  await row.scrollIntoViewIfNeeded();
  const pane = page.locator(".library-list-pane");
  const scroll = await pane.evaluate((element) => element.scrollTop);
  expect(scroll).toBeGreaterThan(0);
  await row.click();
  await page.screenshot({
    path: "../artifacts/library-implementation/mobile-preview-detail.png",
  });
  const picker = page.waitForEvent("filechooser");
  await page
    .getByRole("button", { name: "Choose base ROM", exact: true })
    .click();
  await (await picker).setFiles([]);
  await expect(page.getByRole("alert")).toHaveCount(0);
  await page.getByRole("button", { name: "Back to library" }).click();
  await expect(row).toBeFocused();
  expect(await pane.evaluate((element) => element.scrollTop)).toBe(scroll);
  for (const width of [360, 390, 768, 900, 1100, 1440]) {
    await page.setViewportSize({ width, height: 844 });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
  }
});
