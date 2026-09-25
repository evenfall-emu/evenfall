import { expect, test, type Page } from "@playwright/test";
import { build } from "vite";
import { resolve } from "node:path";
import { rm, writeFile } from "node:fs/promises";
import { SUNSET_CLOUD_CYCLE_SECONDS } from "../../src/sunset-renderer";

const outDir = "../artifacts/about-test";

test.beforeAll(async () => {
  await build({
    configFile: false,
    logLevel: "error",
    define: { "process.env.NODE_ENV": '"development"' },
    build: {
      outDir,
      emptyOutDir: true,
      lib: {
        entry: resolve("tests/browser/about.fixture.tsx"),
        formats: ["es"],
        fileName: () => "fixture.js",
        cssFileName: "fixture",
      },
    },
  });
});
test.afterAll(() => rm(outDir, { recursive: true, force: true }));

async function openFixture(page: Page) {
  await page.goto("/debug.html");
  await page.addStyleTag({ path: resolve(outDir, "fixture.css") });
  await page.addScriptTag({
    type: "module",
    path: resolve(outDir, "fixture.js"),
  });
  await expect(
    page.getByRole("heading", {
      name: "The GBA you remember. A new horizon.",
    }),
  ).toBeVisible();
  await expect(page.locator(".scene-canvas")).toHaveAttribute(
    "data-ready",
    "true",
  );
  await page.evaluate(() => document.fonts.ready);
}

async function pixels(page: Page, box: [number, number, number, number]) {
  return page.locator(".scene-canvas").evaluate((element, bounds) => {
    const context = (element as HTMLCanvasElement).getContext("2d")!;
    return Array.from(context.getImageData(...bounds).data);
  }, box);
}

test("headerless landing uses the full viewport and Enter opens the player", async ({
  page,
}) => {
  await page.setViewportSize({ width: 2118, height: 1080 });
  await openFixture(page);
  await expect(page.getByRole("main")).toHaveCount(1);
  await expect(page.locator("header")).toHaveCount(0);
  await expect(page.getByText(/Made by/)).toHaveCount(0);
  await expect(page.getByRole("link", { name: "Made by bear" })).toHaveCount(0);
  await expect(
    page.getByText("not affiliated with or endorsed by Nintendo", {
      exact: false,
    }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Pause scene" }).click();

  for (const viewport of [
    { width: 2118, height: 1080, capture: "large-2118x1080.png" },
    { width: 3440, height: 1440, capture: "ultrawide-3440x1440.png" },
    { width: 390, height: 844, capture: "mobile-390x844.png" },
  ]) {
    await page.setViewportSize(viewport);
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await expect(
      page.getByRole("button", { name: "Enter player" }),
    ).toBeVisible();
    const screen = await page.locator(".about-screen").boundingBox();
    expect(screen).not.toBeNull();
    expect(screen!.y).toBeLessThanOrEqual(14);
    expect(screen!.y + screen!.height).toBeLessThanOrEqual(viewport.height);
    await page.screenshot({
      path: `../artifacts/sunset-scene/${viewport.capture}`,
    });
  }
  await page.setViewportSize({ width: 390, height: 600 });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollHeight > window.innerHeight,
    ),
  ).toBe(true);
  await expect(
    page.getByRole("button", { name: "Enter player" }),
  ).toBeVisible();
  const shortScreen = await page.locator(".about-screen").boundingBox();
  expect(shortScreen).not.toBeNull();
  expect(shortScreen!.y).toBeLessThanOrEqual(12);
  expect(shortScreen!.height).toBeGreaterThan(600);
  await page.getByRole("button", { name: "Enter player" }).press("Enter");
  await expect(
    page.getByRole("heading", { name: "Player restored" }),
  ).toBeVisible();
  await expect(page.locator(".scene-canvas")).toHaveCount(0);
});

test("waves roll, the bear's raft bobs, and both sky cameos cross the scene", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1704, height: 1100 });
  await openFixture(page);
  const waveBox: [number, number, number, number] = [0, 390, 836, 75];
  const bearBox: [number, number, number, number] = [500, 350, 18, 18];
  const shipBox: [number, number, number, number] = [0, 66, 836, 40];
  const waveBefore = await pixels(page, waveBox);
  const bearBefore = await pixels(page, bearBox);
  const shipBefore = await pixels(page, shipBox);
  await page.waitForTimeout(950);
  expect(await pixels(page, waveBox)).not.toEqual(waveBefore);
  expect(await pixels(page, bearBox)).not.toEqual(bearBefore);
  expect(await pixels(page, shipBox)).not.toEqual(shipBefore);

  // The lone star begins its own pass after the initial airship has left.
  await expect
    .poll(
      async () =>
        Number(
          await page.locator(".scene-canvas").getAttribute("data-scene-time"),
        ),
      { timeout: 15000 },
    )
    .toBeGreaterThan(10);
  const starBox: [number, number, number, number] = [0, 30, 836, 35];
  const starBefore = await pixels(page, starBox);
  await page.waitForTimeout(950);
  expect(await pixels(page, starBox)).not.toEqual(starBefore);
});

test("painted waterline foam stays attached to the raft through both bob extremes", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.clock.install({ time: new Date("2026-01-01T00:00:00Z") });
  await openFixture(page);
  await page.clock.pauseAt(new Date("2026-01-01T00:01:00Z"));
  // The opaque mint foam is part of the artwork, even before motion starts.
  const foamShape = (data: number[]) =>
    data
      .filter((_, i) => i % 4 === 0)
      .map(
        (red, i) =>
          red === 187 && data[i * 4 + 1] === 238 && data[i * 4 + 2] === 216,
      );
  const still = foamShape(await pixels(page, [450, 398, 112, 26]));
  expect(still.filter(Boolean).length).toBeGreaterThan(20);
  await page.getByRole("button", { name: "Play scene" }).click();
  await page.clock.runFor(950);
  // The foam moves down with the logs, rather than staying on the sea or
  // separating into particles. Translating the sampling box restores its shape.
  expect(foamShape(await pixels(page, [450, 400, 112, 26]))).toEqual(still);
  expect(foamShape(await pixels(page, [450, 398, 112, 26]))).not.toEqual(still);
  await page.clock.runFor(1900);
  expect(foamShape(await pixels(page, [450, 396, 112, 26]))).toEqual(still);
  await page.getByRole("button", { name: "Pause scene" }).click();
  const paused = await pixels(page, [450, 396, 112, 26]);
  await page.clock.runFor(2000);
  expect(await pixels(page, [450, 396, 112, 26])).toEqual(paused);
});

test("pause freezes every pixel and resume continues without resetting the scene", async ({
  page,
}) => {
  await openFixture(page);
  await page.getByRole("button", { name: "Pause scene" }).click();
  const canvas = page.locator(".scene-canvas");
  await expect(canvas).toHaveAttribute("data-motion", "paused");
  const stoppedAt = Number(await canvas.getAttribute("data-scene-time"));
  const before = await pixels(page, [0, 0, 836, 471]);
  await page.waitForTimeout(250);
  expect(await pixels(page, [0, 0, 836, 471])).toEqual(before);
  expect(Number(await canvas.getAttribute("data-scene-time"))).toBe(stoppedAt);
  await page.getByRole("button", { name: "Play scene" }).click();
  await expect(canvas).toHaveAttribute("data-motion", "playing");
  await expect
    .poll(async () => Number(await canvas.getAttribute("data-scene-time")))
    .toBeGreaterThan(stoppedAt);
});

test("clouds use the two-minute cycle and the lone star leaves a small fading particle trail", async ({
  page,
}) => {
  expect(SUNSET_CLOUD_CYCLE_SECONDS).toBe(120);
  await page.setViewportSize({ width: 1704, height: 1100 });
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.clock.install({ time: new Date("2026-01-01T00:00:00Z") });
  await openFixture(page);
  await page.clock.pauseAt(new Date("2026-01-01T00:01:00Z"));
  await page.getByRole("button", { name: "Play scene" }).click();
  await page.clock.runFor(100);
  const cloudBox: [number, number, number, number] = [660, 125, 160, 25];
  const coastBox: [number, number, number, number] = [10, 260, 50, 60];
  const trailBox: [number, number, number, number] = [20, 40, 60, 26];
  const clearSky = await pixels(page, trailBox);
  const cloudsBefore = await pixels(page, cloudBox);
  const coastBefore = await pixels(page, coastBox);
  await page.clock.runFor(11000);
  expect(await pixels(page, cloudBox)).not.toEqual(cloudsBefore);
  expect(await pixels(page, coastBox)).toEqual(coastBefore);

  // This sky region is behind the star, outside the sprite and ship paths.
  const trail = await pixels(page, trailBox);
  let sparkles = 0;
  for (let i = 0; i < trail.length; i += 4) {
    if (
      trail[i] !== clearSky[i] ||
      trail[i + 1] !== clearSky[i + 1] ||
      trail[i + 2] !== clearSky[i + 2]
    )
      sparkles++;
  }
  expect(sparkles).toBeGreaterThan(1);
  expect(sparkles).toBeLessThan(35);
  await page.getByRole("button", { name: "Pause scene" }).click();
  await page.clock.runFor(2500);
  expect(await pixels(page, trailBox)).toEqual(trail);
  await page
    .locator(".scene-canvas")
    .screenshot({ path: "../artifacts/sunset-scene/particles.png" });
  await page.getByRole("button", { name: "Play scene" }).click();
  await page.clock.runFor(2500);
  expect(await pixels(page, trailBox)).toEqual(clearSky);
});

test("the explorer's scarf flutters without moving the bear's head, and the lantern flickers", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1704, height: 1100 });
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.clock.install({ time: new Date("2026-01-01T00:00:00Z") });
  await openFixture(page);
  await page.clock.pauseAt(new Date("2026-01-01T00:01:00Z"));
  await page.getByRole("button", { name: "Play scene" }).click();
  await page.clock.runFor(400);
  const scarfBox: [number, number, number, number] = [470, 361, 28, 12];
  const headBox: [number, number, number, number] = [506, 356, 10, 8];
  const lanternBox: [number, number, number, number] = [546, 384, 5, 5];
  const redShape = (data: number[]) =>
    data
      .filter((_, i) => i % 4 === 0)
      .map(
        (red, i) =>
          red > 90 && red > data[i * 4 + 1] * 2.8 && red > data[i * 4 + 2] * 2,
      );
  const scarfBefore = redShape(await pixels(page, scarfBox));
  expect(scarfBefore.filter(Boolean).length).toBeGreaterThan(50);
  const headBefore = await pixels(page, headBox);
  const lanternBefore = await pixels(page, lanternBox);
  const saveFrame = async (name: string) => {
    const url = await page
      .locator(".scene-canvas")
      .evaluate((canvas) => (canvas as HTMLCanvasElement).toDataURL());
    await writeFile(
      `../artifacts/sunset-scene/${name}.png`,
      Buffer.from(url.split(",")[1], "base64"),
    );
  };
  await saveFrame("scarf-before");
  // These two points in the wave cycle have the same rounded bob height.
  // Red fabric changes shape while the opaque bear's head stays exactly fixed.
  await page.clock.runFor(1200);
  await saveFrame("scarf-after");
  expect(await pixels(page, headBox)).toEqual(headBefore);
  expect(await pixels(page, lanternBox)).not.toEqual(lanternBefore);
  const scarfAfter = redShape(await pixels(page, scarfBox));
  const movingClothColumns = new Set<number>();
  for (let i = 0; i < scarfAfter.length; i++) {
    const column = i % scarfBox[2];
    if (column >= 6 && scarfAfter[i] !== scarfBefore[i])
      movingClothColumns.add(column);
  }
  // Movement must reach the broad red fabric, beyond the far-left tip.
  expect(movingClothColumns.size).toBeGreaterThanOrEqual(6);
  // The last three scarf columns remain attached to the neck.
  for (let i = 0; i < scarfAfter.length; i++) {
    if (i % scarfBox[2] >= 25) expect(scarfAfter[i]).toBe(scarfBefore[i]);
  }
  await page.getByRole("button", { name: "Pause scene" }).click();
  const paused = await pixels(page, [450, 347, 112, 74]);
  await page.clock.runFor(2000);
  expect(await pixels(page, [450, 347, 112, 74])).toEqual(paused);
});

test("offscreen scene stops rendering and resumes when visible", async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 500 });
  await openFixture(page);
  await page.evaluate(() => {
    const spacer = document.createElement("div");
    spacer.style.height = "1000px";
    document.body.append(spacer);
    scrollTo(0, document.documentElement.scrollHeight);
  });
  const canvas = page.locator(".scene-canvas");
  await expect(canvas).toHaveAttribute("data-motion", "paused");
  const time = await canvas.getAttribute("data-scene-time");
  await page.waitForTimeout(250);
  expect(await canvas.getAttribute("data-scene-time")).toBe(time);
  await canvas.scrollIntoViewIfNeeded();
  await expect(canvas).toHaveAttribute("data-motion", "playing");
});

test("reduced motion starts with a complete still scene and can be explicitly resumed", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.setViewportSize({ width: 1704, height: 1100 });
  await openFixture(page);
  const canvas = page.locator(".scene-canvas");
  await expect(canvas).toHaveAttribute("data-motion", "paused");
  const before = await pixels(page, [0, 0, 836, 471]);
  await page.waitForTimeout(250);
  expect(await pixels(page, [0, 0, 836, 471])).toEqual(before);
  await canvas.screenshot({
    path: "../artifacts/sunset-scene/reduced-motion-test.png",
  });
  await page
    .locator(".about-screen")
    .screenshot({ path: "../artifacts/sunset-scene/desktop-final.png" });
  await page.getByRole("button", { name: "Play scene" }).click();
  await expect(canvas).toHaveAttribute("data-motion", "playing");
});
