import { expect, test, type Page } from "@playwright/test";
import { build } from "vite-plus";
import tailwindcss from "@tailwindcss/vite";
import { resolve } from "node:path";
import { rm } from "node:fs/promises";

// The two-column player that game layouts get on pages narrower than 17:10
// (src/docked.tsx), around the Player's screen markup.
const outDir = "../artifacts/docked-test";

test.beforeAll(async () => {
  await build({
    configFile: false,
    logLevel: "error",
    plugins: [tailwindcss()],
    define: { "process.env.NODE_ENV": '"development"' },
    build: {
      outDir,
      emptyOutDir: true,
      lib: {
        entry: resolve("tests/browser/docked.fixture.tsx"),
        formats: ["es"],
        fileName: () => "fixture.js",
        cssFileName: "fixture",
      },
    },
  });
});
test.afterAll(() => rm(outDir, { recursive: true, force: true }));

async function open(page: Page) {
  await page.route("**/docked-fixture", (route) =>
    route.fulfill({
      contentType: "text/html",
      body: "<!doctype html><html><head></head><body></body></html>",
    }),
  );
  await page.goto("/docked-fixture");
  await page.addStyleTag({ path: resolve(outDir, "fixture.css") });
  await page.addScriptTag({
    type: "module",
    path: resolve(outDir, "fixture.js"),
  });
  await expect(page.getByRole("tablist")).toBeVisible();
}

test("pages narrower than 17:10 in landscape get the dock", async ({
  page,
}) => {
  await open(page);
  for (const [width, height, docked] of [
    // A 3:2 laptop at 200%, less the browser's own bars.
    [1560, 956, true],
    [1024, 700, true],
    // 16:10 at 125%, and 16:9: side columns leave the screen enough width.
    [1536, 880, false],
    [1920, 960, false],
    [1280, 720, false],
    // Portrait tablets keep the game layout's own stacked panels.
    [900, 1200, false],
  ] as const) {
    await page.setViewportSize({ width, height });
    await expect(page.locator("#docked"), `${width}×${height}`).toHaveAttribute(
      "data-docked",
      String(docked),
    );
  }
});

test("the screen takes the column's height beside a dock of tabs", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1560, height: 956 });
  await open(page);
  const box = (selector: string) =>
    page.locator(selector).evaluate((element) => {
      const { left, right, top, bottom, width, height } =
        element.getBoundingClientRect();
      return { left, right, top, bottom, width, height };
    });
  const dock = await box(".docked-panels");
  const viewport = await box(".game-viewport");
  const display = await box(".game-display");
  const canvas = await box(".game-display canvas");
  // The dock is 30% of the page, between 340 and 480 px: 468 here.
  expect(dock.width).toBeCloseTo(468, 0);
  expect(display.right).toBeLessThan(dock.left);
  // Height, not width, bounds the screen: the display fills its viewport's
  // height, keeps 3:2 and has width to spare.
  expect(display.height).toBeCloseTo(viewport.height - 8, 0);
  expect(canvas.width / canvas.height).toBeCloseTo(1.5, 2);
  expect(display.width).toBeLessThan(viewport.width - 8);
  expect(await page.evaluate(() => document.documentElement.scrollHeight)).toBe(
    956,
  );
});

test("the dock opens the panel the game calls for, and keeps a chosen tab until then", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1560, height: 956 });
  await open(page);
  const tab = (name: string) => page.getByRole("tab", { name });
  const search = page.getByRole("textbox", { name: "Search team" });
  await expect(tab("Battle")).toHaveAttribute("aria-selected", "true");
  await expect(page.getByText("Wild Pidgey")).toBeVisible();

  await tab("Team").click();
  await expect(tab("Team")).toHaveAttribute("aria-selected", "true");
  await search.fill("Pyrox");
  // Re-rendering with the same focus leaves the chosen tab alone.
  await page.evaluate(() => window.dockFixture.rerender());
  await expect(tab("Team")).toHaveAttribute("aria-selected", "true");

  // A new focus opens its tab; hidden panels keep their state.
  await page.evaluate(() => window.dockFixture.focus("team"));
  await page.evaluate(() => window.dockFixture.focus("battle"));
  await expect(tab("Battle")).toHaveAttribute("aria-selected", "true");
  await expect(search).toBeHidden();
  await tab("Team").click();
  await expect(search).toHaveValue("Pyrox");

  // Without a focus, the chosen tab stays.
  await page.evaluate(() => window.dockFixture.focus(undefined));
  await expect(tab("Team")).toHaveAttribute("aria-selected", "true");
});
