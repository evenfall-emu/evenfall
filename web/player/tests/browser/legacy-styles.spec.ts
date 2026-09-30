import { expect, test } from "@playwright/test";
import { build } from "vite-plus";
import tailwindcss from "@tailwindcss/vite";
import { resolve } from "node:path";
import { rm } from "node:fs/promises";

const outDir = "../../artifacts/legacy-styles-test";

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
        entry: resolve("tests/browser/legacy-styles.fixture.tsx"),
        formats: ["es"],
        fileName: () => "fixture.js",
        cssFileName: "fixture",
      },
    },
  });
});
test.afterAll(() => rm(outDir, { recursive: true, force: true }));

test("legacy pages keep their text metrics while Tailwind roots keep Tailwind's", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1200, height: 800 });
  await page.route("**/legacy-styles-fixture", (route) =>
    route.fulfill({
      contentType: "text/html",
      body: "<!doctype html><html><head></head><body></body></html>",
    }),
  );
  await page.goto("/legacy-styles-fixture");
  await page.addStyleTag({ path: resolve(outDir, "fixture.css") });
  await page.addScriptTag({
    type: "module",
    path: resolve(outDir, "fixture.js"),
  });
  await expect(page.locator("#nested")).toBeVisible();
  const text = (selector: string) =>
    page.locator(selector).evaluate((element) => {
      const style = getComputedStyle(element);
      return {
        color: style.color,
        fontSize: style.fontSize,
        lineHeight: style.lineHeight,
      };
    });

  // Plugin dashboards and legacy panels were laid out on these; Tailwind's
  // 1.5 pushed their fixed-height rows into each other.
  expect(await text("#legacy")).toMatchObject({
    color: "rgb(255, 240, 200)",
    lineHeight: "normal",
  });
  const content = "rgb(227, 234, 240)";
  expect(await text("#sized")).toEqual({
    color: content,
    fontSize: "16px",
    lineHeight: "24px",
  });
  expect(await text("#nested")).toEqual({
    color: content,
    fontSize: "16px",
    lineHeight: "24px",
  });
  // An unsized popup takes the fluid root size, clamped to 19px here.
  expect(await text("#portal")).toEqual({
    color: content,
    fontSize: "19px",
    lineHeight: "28.5px",
  });
});
