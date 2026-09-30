import { test, expect, type Page } from "@playwright/test";
import { readFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import type { Emulator } from "@evenfall/gba";

test.use({ hasTouch: true });

const loop = {
  name: "Loop.gba",
  mimeType: "application/octet-stream",
  buffer: Buffer.from([42, 0, 160, 227, 254, 255, 255, 234]),
};
const timelineRom = process.env.EVENFALL_TIMELINE_ROM ?? loop;
const timelineFrames = process.env.EVENFALL_TIMELINE_ROM ? 240 : 24;
const sha = (bytes: Uint8Array) =>
  createHash("sha256").update(bytes).digest("hex");
async function ready(page: Page) {
  await page.goto("/");
  await page.getByRole("button", { name: "Open player" }).click();
  await expect(page.getByLabel("Open ROM", { exact: true })).toBeEnabled();
}
async function importState(page: Page, path: string) {
  await page.getByRole("button", { name: "More options" }).click();
  await expect(page.getByLabel("Import state", { exact: true })).toBeEnabled();
  await page.getByLabel("Import state", { exact: true }).setInputFiles(path);
  await expect(
    page.getByRole("dialog", { name: "Saves and options" }),
  ).toHaveCount(0);
}
async function snapshotHash(page: Page) {
  return page.evaluate(async () => {
    const emulator = (window as unknown as { emulator: Emulator }).emulator;
    const bytes = await emulator.saveState();
    return Array.from(
      new Uint8Array(
        await crypto.subtle.digest("SHA-256", Uint8Array.from(bytes)),
      ),
    )
      .map((v) => v.toString(16).padStart(2, "0"))
      .join("");
  });
}
async function runOneFrame(page: Page) {
  await page.evaluate(async () => {
    const emulator = (window as unknown as { emulator: Emulator }).emulator;
    const cpu = await emulator.inspect();
    await emulator.runUntil(cpu.cycle + 280896n);
  });
}

test("sound volume opens on hover or focus and persists independently of mute", async ({
  page,
}, testInfo) => {
  await ready(page);
  const sound = page.getByRole("button", { name: "Mute sound" });
  const slider = page.getByRole("slider", { name: "Volume" });
  await expect(slider).toBeHidden();
  await sound.hover();
  await expect(slider).toBeVisible();
  await slider.focus();
  await page.mouse.move(0, 0);
  await expect(slider).toBeVisible();
  await slider.press("Home");
  await slider.press("ArrowRight");
  await expect(slider).toHaveValue("1");
  await expect(page.locator(".volume-panel output")).toHaveText("1%");
  expect(await page.evaluate(() => localStorage.getItem("zigba-volume"))).toBe(
    "0.01",
  );
  await sound.click();
  await expect(
    page.getByRole("button", { name: "Unmute sound" }),
  ).toHaveAttribute("aria-pressed", "true");
  await expect(slider).toHaveValue("1");
  await page.goto("/");
  await page.getByRole("button", { name: "Open player" }).click();
  await page.getByRole("button", { name: "Mute sound" }).hover();
  await expect(slider).toHaveValue("1");
  await expect(slider).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({
    path: testInfo.outputPath("volume-hover-desktop.png"),
  });
  await page.setViewportSize({ width: 390, height: 844 });
  // Phones keep the toolbar in the player menu.
  await page.getByRole("button", { name: "Player menu" }).click();
  await page.getByRole("button", { name: "Mute sound" }).hover();
  await expect(slider).toBeInViewport();
  await page.screenshot({
    path: testInfo.outputPath("volume-hover-mobile.png"),
  });
});

test("paused game shows a resume hint over the screen, not while playing or empty", async ({
  page,
}) => {
  await ready(page);
  const overlay = page.locator(".pause-overlay");
  await expect(overlay).toHaveCount(0);
  await page.getByLabel("Open ROM", { exact: true }).setInputFiles(loop);
  await expect
    .poll(() => page.evaluate(() => window.player.state.playing))
    .toBe(true);
  await expect(overlay).toHaveCount(0);
  await page.getByLabel(/^Game screen/).focus();
  await page.keyboard.press("Space");
  await expect(overlay).toBeVisible();
  await expect(overlay).toContainText("PAUSED");
  await expect(overlay).toContainText("Space");
  await page.evaluate(() => document.fonts.ready);
  await page.locator(".game-display").screenshot({
    path: "test-results/paused-desktop.png",
  });
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(
    overlay.getByRole("button", { name: "Resume game" }),
  ).toBeInViewport();
  await page.locator(".game-display").screenshot({
    path: "test-results/paused-mobile.png",
  });
  await overlay.getByRole("button", { name: "Resume game" }).click();
  await expect(overlay).toHaveCount(0);
  await page.keyboard.press("Space");
  await expect(overlay).toBeVisible();
  await page.keyboard.press("Space");
  await expect(overlay).toHaveCount(0);
});

/** Whether the page shows the phone player (see src/handheld.tsx). */
const phone = ({ width, height }: { width: number; height: number }) =>
  width < height ? width <= 650 : height <= 500;

async function screenFillsColumn(page: Page) {
  const viewport = page.viewportSize()!;
  // Crossing portrait/landscape also changes the handheld's own layout.
  await expect(page.locator(".handheld")).toHaveCount(phone(viewport) ? 1 : 0);
  if (phone(viewport))
    await expect(page.locator(".handheld")).toHaveAttribute(
      "data-orientation",
      viewport.width < viewport.height ? "portrait" : "landscape",
    );
  const geometry = await page.locator(".console-player").evaluate((player) => {
    const display = player.querySelector(".game-display")!;
    const canvas = display.querySelector("canvas")!;
    const container = player.querySelector(".game-viewport")!;
    const style = getComputedStyle(container);
    return {
      available:
        container.clientWidth -
        parseFloat(style.paddingLeft) -
        parseFloat(style.paddingRight),
      availableHeight:
        container.clientHeight -
        parseFloat(style.paddingTop) -
        parseFloat(style.paddingBottom),
      display: display.getBoundingClientRect().width,
      displayHeight: display.getBoundingClientRect().height,
      canvas: canvas.getBoundingClientRect().width,
      ratio:
        canvas.getBoundingClientRect().width /
        canvas.getBoundingClientRect().height,
    };
  });
  expect(geometry.display).toBeLessThanOrEqual(geometry.available + 1);
  expect(geometry.displayHeight).toBeLessThanOrEqual(
    geometry.availableHeight + 1,
  );
  expect(geometry.canvas).toBeGreaterThan(0);
  expect(geometry.ratio).toBeCloseTo(1.5, 2);
  if (phone(viewport)) {
    // No frame: the screen spans the width in portrait, the height in
    // landscape.
    expect(geometry.display - geometry.canvas).toBeCloseTo(0, 0);
    if (viewport.width < viewport.height)
      expect(geometry.canvas).toBeCloseTo(viewport.width, 0);
    else expect(geometry.displayHeight).toBeCloseTo(viewport.height, 0);
  } else expect(geometry.display - geometry.canvas).toBeCloseTo(12, 0);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollHeight <= innerHeight,
    ),
  ).toBe(true);
  for (const element of await page
    .locator(
      '.game-display, .quick-bar:visible, .touch-pad, [aria-label="Touch controls"]',
    )
    .all()) {
    const box = await element.boundingBox();
    expect(box!.y).toBeGreaterThanOrEqual(0);
    expect(box!.y + box!.height).toBeLessThanOrEqual(
      page.viewportSize()!.height + 1,
    );
  }
}

test("player and controls fit wide, short, and portrait viewports", async ({
  page,
}) => {
  await ready(page);
  await page.evaluate(() => document.fonts.ready);
  for (const [width, height] of [
    [2118, 1080],
    [3440, 1440],
    [1366, 768],
    [1024, 600],
    [390, 844],
    [390, 600],
    [844, 390],
  ]) {
    await page.setViewportSize({ width, height });
    await screenFillsColumn(page);
    await expect(
      page.getByLabel("Open game", { exact: true }),
    ).toBeInViewport();
    await page.screenshot({
      path: `../../artifacts/ui-refresh/player-empty-${width}x${height}.png`,
    });
  }
  await page.setViewportSize({ width: 1366, height: 768 });
  await page.getByLabel("Open ROM", { exact: true }).setInputFiles(loop);
  await expect(
    page.getByRole("heading", { name: "Loop", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "How to play", exact: true }).click();
  await page.getByRole("button", { name: "Show touch controls" }).click();
  await page.getByRole("button", { name: "Close dialog" }).click();
  for (const [width, height] of [
    [2118, 1080],
    [390, 600],
    [844, 390],
  ]) {
    await page.setViewportSize({ width, height });
    await screenFillsColumn(page);
    await expect(
      page.getByLabel("Touch controls", { exact: true }),
    ).toBeInViewport();
    const onPhone = phone({ width, height });
    // Phones keep the toolbar in the player menu, which scrolls.
    if (onPhone)
      await page.getByRole("button", { name: "Player menu" }).click();
    await page.getByRole("button", { name: "More options" }).click();
    const menu = page.getByRole("dialog", { name: "Saves and options" });
    if (onPhone) {
      const last = menu.locator(".slot-row").last();
      await last.scrollIntoViewIfNeeded();
      await expect(last).toBeInViewport();
    } else {
      const box = await menu.boundingBox();
      expect(box!.y + box!.height).toBeLessThanOrEqual(height);
    }
    await page.getByRole("button", { name: "More options" }).click();
    if (onPhone) await page.getByRole("button", { name: "Close menu" }).click();
    await page.screenshot({
      path: `../../artifacts/ui-refresh/player-touch-${width}x${height}.png`,
    });
  }
  await expect(page.getByRole("button", { name: /^Rewind/ })).toBeEnabled();
});

/**
 * The empty screen's sunset: its scale and where its sun and text sit, in
 * CSS pixels. The sun covers art pixels 297–346 across and 159–197 down
 * (see src/assets/landing/README.md).
 */
function emptyScreen(page: Page) {
  return page.locator(".screen-empty canvas").evaluate((element) => {
    const canvas = element as HTMLCanvasElement;
    const { scale, x, y, ratio } = canvas.dataset;
    const k = Number(scale) / Number(ratio);
    const box = canvas.getBoundingClientRect();
    const text = canvas.nextElementSibling!.getBoundingClientRect();
    return {
      scale: Number(scale),
      backing: [canvas.width, canvas.height],
      device: [
        Math.round(box.width * Number(ratio)),
        Math.round(box.height * Number(ratio)),
      ],
      textBottom: text.bottom - box.top,
      width: box.width,
      height: box.height,
      sun: {
        left: Number(x) / Number(ratio) + 297 * k,
        right: Number(x) / Number(ratio) + 347 * k,
        top: Number(y) / Number(ratio) + 159 * k,
        bottom: Number(y) / Number(ratio) + 198 * k,
      },
    };
  });
}

test("the empty screen draws the sunset at whole pixels, the sun whole below the text", async ({
  page,
}) => {
  await ready(page);
  await page.evaluate(() => document.fonts.ready);
  for (const [width, height] of [
    [1920, 1080],
    [1366, 768],
    [1024, 600],
    [390, 844],
    [360, 780],
    [844, 390],
  ]) {
    await page.setViewportSize({ width, height });
    const canvas = page.locator(".screen-empty canvas");
    await expect(canvas).toHaveAttribute("data-scale", /^\d+$/);
    // Let the resize settle into a new framing before measuring it.
    await page.waitForTimeout(100);
    const screen = await emptyScreen(page);
    const at = `${width}×${height}`;
    // One canvas pixel per device pixel, and a whole number of them per
    // art pixel, at no less than two CSS pixels on these screens.
    expect(screen.backing, at).toEqual(screen.device);
    expect(screen.scale, at).toBeGreaterThanOrEqual(2);
    expect(screen.sun.top, at).toBeGreaterThanOrEqual(screen.textBottom);
    expect(screen.sun.bottom, at).toBeLessThanOrEqual(screen.height);
    expect(screen.sun.left, at).toBeGreaterThanOrEqual(0);
    expect(screen.sun.right, at).toBeLessThanOrEqual(screen.width);
  }

  // It moves while the page has focus, like Home's scene.
  await page.bringToFront();
  const canvas = page.locator(".screen-empty canvas");
  await expect(canvas).toHaveAttribute("data-motion", "playing");
  const pixels = () =>
    canvas.evaluate((element: HTMLCanvasElement) =>
      Array.from(
        element
          .getContext("2d")!
          .getImageData(0, 0, element.width, element.height).data,
      ).join(),
    );
  const before = await pixels();
  await expect.poll(pixels).not.toBe(before);
  await page.emulateMedia({ reducedMotion: "reduce" });
  await expect(canvas).toHaveAttribute("data-motion", "still");

  // Opening a game replaces it.
  await page.getByLabel("Open game", { exact: true }).setInputFiles(loop);
  await expect(
    page.getByRole("heading", { name: "Loop", exact: true }),
  ).toBeVisible();
  await expect(page.locator(".screen-empty")).toHaveCount(0);
});

test("console autoplays, routes keys outside the canvas, and keeps help and typing isolated", async ({
  page,
}) => {
  await ready(page);
  await page.getByLabel("Open ROM", { exact: true }).setInputFiles(loop);
  const canvas = page.getByLabel(/^Game screen/);
  const frames = () =>
    page.evaluate(async () =>
      Number(
        (await (window as unknown as { emulator: Emulator }).emulator.inspect())
          .frames,
      ),
    );
  const initial = await frames();
  await expect.poll(frames).toBeGreaterThan(initial);
  await canvas.focus();
  await page.mouse.move(0, 0);
  await expect(page.locator(".screen-overlay")).toHaveCount(0);
  await page.getByRole("button", { name: "evenfall home" }).focus();
  await page.keyboard.down("Tab");
  await expect(page.getByRole("button", { name: "Playback speed" })).toHaveText(
    "16×",
  );
  await expect(
    page.getByRole("button", { name: "evenfall home" }),
  ).toBeFocused();
  await page.keyboard.up("Tab");
  await expect(page.getByRole("button", { name: "Playback speed" })).toHaveText(
    "1×",
  );
  const keys = () =>
    page.evaluate(async () => {
      const bytes = await (
        window as unknown as { emulator: Emulator }
      ).emulator.readMemory(0x04000130, 2);
      return 1023 ^ (new DataView(bytes.buffer).getUint16(0, true) & 1023);
    });
  await page.keyboard.down("KeyX");
  await expect.poll(keys).toBe(1);
  // Moving focus into an editor must release a key even before its keyup.
  await page.evaluate(() => {
    const input = document.createElement("input");
    input.setAttribute("aria-label", "Typing fixture");
    document.body.append(input);
    input.focus();
  });
  await expect.poll(keys).toBe(0);
  await page.keyboard.up("KeyX");
  await page.getByLabel("Typing fixture").pressSequentially("xzar");
  await expect(page.getByLabel("Typing fixture")).toHaveValue("xzar");
  await expect.poll(keys).toBe(0);
  await page.keyboard.down("Tab");
  await expect(page.getByRole("button", { name: "Playback speed" })).toHaveText(
    "1×",
  );
  await page.keyboard.up("Tab");
  await page.getByLabel("Typing fixture").evaluate((node) => node.remove());
  await page.getByRole("button", { name: "How to play", exact: true }).click();
  const help = page.getByRole("dialog", { name: "How to play", exact: true });
  await expect(help).toBeVisible();
  await expect(help).toContainText("Rewind");
  await expect(help).toContainText("Backspace");
  const paused = await frames();
  await page.keyboard.press("KeyX");
  await page.keyboard.press("KeyR");
  await page.keyboard.press("Shift+Digit4");
  await expect.poll(keys).toBe(0);
  expect(await frames()).toBe(paused);
  await page.evaluate(() => document.fonts.ready);
  await page.setViewportSize({ width: 1440, height: 1000 });
  await help.screenshot({
    path: "../../artifacts/hud/player-help-desktop.png",
  });
  await page.setViewportSize({ width: 390, height: 844 });
  await help.screenshot({ path: "../../artifacts/hud/player-help-mobile.png" });
  // Phones show touch controls by default when the pointer is coarse.
  const toggle = help.getByRole("button", { name: /^(Show|Hide) touch/ });
  await toggle.scrollIntoViewIfNeeded();
  if (await page.evaluate(() => matchMedia("(pointer: coarse)").matches)) {
    await expect(toggle).toHaveText("Hide touch controls");
    await toggle.click();
  }
  await expect(toggle).toHaveText("Show touch controls");
  await expect(
    help.getByRole("button", { name: "Close dialog" }),
  ).toBeInViewport();
  await help.screenshot({
    path: "../../artifacts/hud/player-help-mobile-bottom.png",
  });
  expect(
    await help.evaluate((node) => node.scrollWidth <= node.clientWidth),
  ).toBe(true);
  await help.getByRole("button", { name: "Show touch controls" }).click();
  await page.keyboard.press("Escape");
  await expect(help).toHaveCount(0);
  await expect(
    page.getByLabel("Touch controls", { exact: true }),
  ).toBeVisible();
  await expect.poll(frames).toBeGreaterThan(paused);
});

test("R steps once while Shift+R previews and commits exact checkpoints", async ({
  page,
}) => {
  await ready(page);
  await page.getByLabel("Open ROM", { exact: true }).setInputFiles(timelineRom);
  const canvas = page.getByLabel(/^Game screen/);
  const frames = () =>
    page.evaluate(async () =>
      Number(
        (await (window as unknown as { emulator: Emulator }).emulator.inspect())
          .frames,
      ),
    );
  await expect.poll(() => frames().catch(() => -1)).toBeGreaterThanOrEqual(0);
  const started = await frames();
  await expect.poll(frames).toBeGreaterThan(started);
  await canvas.focus();
  await page.keyboard.press("Space");
  await page.waitForTimeout(100);
  const paused = await frames();
  await page.waitForTimeout(100);
  expect(await frames()).toBe(paused);
  await page.evaluate(async (frameCount) => {
    const emulator = (window as unknown as { emulator: Emulator }).emulator;
    // Every frame is a checkpoint; the live-frame entry is excluded from steps.
    await emulator.setRewindConfig(undefined, 1);
    for (let index = 0; index < frameCount; index++) {
      const cpu = await emulator.inspect();
      await emulator.runUntil(cpu.cycle + 280896n);
    }
  }, timelineFrames);
  const before = await page.evaluate(async () => {
    const emulator = (window as unknown as { emulator: Emulator }).emulator;
    return {
      state: await emulator.saveState(),
      sample: await emulator.captureMemory(),
      depth: await emulator.rewindDepth(),
      timeline: await emulator.rewindCheckpoints(),
    };
  });
  expect(before.timeline.checkpoints.length).toBeGreaterThan(2);

  await page.keyboard.press("Shift+KeyR");
  const dialog = page.getByRole("dialog", { name: "Rewind timeline" });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByText(/checkpoints available/)).toBeVisible();
  await expect(dialog.locator("canvas")).toHaveCount(
    Math.min(3, before.timeline.checkpoints.length),
  );
  await expect(dialog.getByText("Loading…")).toHaveCount(0);
  const range = dialog.getByRole("slider", { name: "Rewind checkpoint" });
  await expect(range).toBeFocused();
  await page.keyboard.press("ArrowLeft");
  await expect(range).toHaveValue(
    String(before.timeline.checkpoints.length - 2),
  );
  const midpoint = String(Math.floor(before.timeline.checkpoints.length / 2));
  await range.fill(midpoint);
  const selected = before.timeline.checkpoints[0];
  const previewed = await page.evaluate(async () => {
    const emulator = (window as unknown as { emulator: Emulator }).emulator;
    return {
      state: await emulator.saveState(),
      sample: await emulator.captureMemory(),
      depth: await emulator.rewindDepth(),
    };
  });
  expect(Array.from(previewed.state)).toEqual(Array.from(before.state));
  expect(previewed.sample!.cycle).toBe(before.sample!.cycle);
  expect(previewed.sample!.timeline).toBe(before.sample!.timeline);
  expect(previewed.depth).toBe(before.depth);
  await page.evaluate(() => document.fonts.ready);
  await page.setViewportSize({ width: 1280, height: 900 });
  await dialog.screenshot({
    path: "../../artifacts/rewind/timeline-desktop.png",
  });
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(
    page.locator('.handheld[data-orientation="portrait"]'),
  ).toBeVisible();
  await expect(range).toHaveValue(midpoint);
  await dialog.screenshot({
    path: "../../artifacts/rewind/timeline-mobile.png",
  });
  expect(
    await dialog.evaluate((node) => node.scrollWidth <= node.clientWidth),
  ).toBe(true);
  await range.fill("0");
  await expect(dialog.locator(".rewind-thumbnail").first()).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await dialog.getByRole("button", { name: "Rewind here" }).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.locator(".console-notice")).toContainText(
    "Rewind checkpoint restored",
  );
  const committed = await page.evaluate(async () => {
    const emulator = (window as unknown as { emulator: Emulator }).emulator;
    return {
      cpu: await emulator.inspect(),
      sample: await emulator.captureMemory(),
    };
  });
  expect(committed.cpu.cycle).toBe(selected.cycle);
  expect(committed.sample!.timeline).not.toBe(before.sample!.timeline);

  // Rebuild history and exercise both sides of the 300 ms debounce boundary.
  await page.evaluate(async () => {
    const emulator = (window as unknown as { emulator: Emulator }).emulator;
    await emulator.setRewindConfig(undefined, 60);
    for (let index = 0; index < 6; index++) {
      const cpu = await emulator.inspect();
      await emulator.runUntil(cpu.cycle + 60n * 280896n);
    }
  });
  await canvas.focus();
  const debounceStart = await page.evaluate(() =>
    (window as unknown as { emulator: Emulator }).emulator.inspect(),
  );
  await page.keyboard.press("KeyR");
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as unknown as { emulator: Emulator }).emulator.inspect(),
      ),
    )
    .not.toEqual(debounceStart);
  const debounced = await page.evaluate(() =>
    (window as unknown as { emulator: Emulator }).emulator.inspect(),
  );
  await page.keyboard.press("KeyR");
  await page.waitForTimeout(100);
  expect(
    await page.evaluate(() =>
      (window as unknown as { emulator: Emulator }).emulator.inspect(),
    ),
  ).toEqual(debounced);
  await page.waitForTimeout(210);
  await page.keyboard.press("KeyR");
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as unknown as { emulator: Emulator }).emulator.inspect(),
      ),
    )
    .not.toEqual(debounced);

  // New forward history makes held-key repeat protection independently visible.
  await page.evaluate(async () => {
    const emulator = (window as unknown as { emulator: Emulator }).emulator;
    for (let index = 0; index < 4; index++) {
      const cpu = await emulator.inspect();
      await emulator.runUntil(cpu.cycle + 60n * 280896n);
    }
  });
  await page.waitForTimeout(310);
  await page.keyboard.down("KeyX");
  const heldStart = await page.evaluate(() =>
    (window as unknown as { emulator: Emulator }).emulator.inspect(),
  );
  await page.keyboard.down("KeyR");
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as unknown as { emulator: Emulator }).emulator.inspect(),
      ),
    )
    .not.toEqual(heldStart);
  const oneStep = await page.evaluate(() =>
    (window as unknown as { emulator: Emulator }).emulator.inspect(),
  );
  expect(
    await page.evaluate(async () => {
      const bytes = await (
        window as unknown as { emulator: Emulator }
      ).emulator.readMemory(0x04000130, 2);
      return new DataView(bytes.buffer).getUint16(0, true) & 1023;
    }),
  ).toBe(1023);
  await page.keyboard.up("KeyX");
  await page.waitForTimeout(450);
  expect(
    await page.evaluate(() =>
      (window as unknown as { emulator: Emulator }).emulator.inspect(),
    ),
  ).toEqual(oneStep);

  // Focus loss can swallow keyup. It must release the UI latch as well as the
  // emulated key mask, so returning to the page does not permanently disable R.
  await page.evaluate(async () => {
    const emulator = (window as unknown as { emulator: Emulator }).emulator;
    for (let index = 0; index < 3; index++) {
      const cpu = await emulator.inspect();
      await emulator.runUntil(cpu.cycle + 60n * 280896n);
    }
    window.dispatchEvent(new Event("blur"));
  });
  await page.waitForTimeout(310);
  const blurred = await page.evaluate(() =>
    (window as unknown as { emulator: Emulator }).emulator.inspect(),
  );
  await page.evaluate(() =>
    window.dispatchEvent(
      new KeyboardEvent("keydown", { code: "KeyR", key: "r" }),
    ),
  );
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as unknown as { emulator: Emulator }).emulator.inspect(),
      ),
    )
    .not.toEqual(blurred);
  await page.evaluate(() =>
    window.dispatchEvent(
      new KeyboardEvent("keyup", { code: "KeyR", key: "r" }),
    ),
  );
  await page.keyboard.up("KeyR");
});

test("R skips sub-second checkpoints across repeated taps", async ({
  page,
}) => {
  await ready(page);
  await page.getByLabel("Open ROM", { exact: true }).setInputFiles(loop);
  await expect(
    page.getByRole("button", { name: "How to play", exact: true }),
  ).toBeEnabled();
  const canvas = page.getByLabel(/^Game screen/);
  await canvas.focus();
  await page.keyboard.press("Space");
  const checkpoints = await page.evaluate(async () => {
    const emulator = (window as unknown as { emulator: Emulator }).emulator;
    await emulator.setRewindConfig(undefined, 60);
    for (let index = 0; index < 4; index++) {
      const cpu = await emulator.inspect();
      await emulator.runUntil(cpu.cycle + 60n * 280896n);
    }
    const cpu = await emulator.inspect();
    await emulator.runUntil(cpu.cycle + 6n * 280896n);
    return (await emulator.rewindCheckpoints()).checkpoints;
  });
  expect(checkpoints).toHaveLength(4);
  const cycle = () =>
    page.evaluate(
      async () =>
        (await (window as unknown as { emulator: Emulator }).emulator.inspect())
          .cycle,
    );
  await page.keyboard.press("KeyR");
  await expect.poll(cycle).toBe(checkpoints[2].cycle);

  // Playing a short distance after restore must not keep returning to the
  // checkpoint we just restored. Use guest time, not a wall-clock sleep.
  await page.evaluate(async () => {
    const emulator = (window as unknown as { emulator: Emulator }).emulator;
    const cpu = await emulator.inspect();
    await emulator.runUntil(cpu.cycle + 6n * 280896n);
  });
  await page.waitForTimeout(350);
  await page.keyboard.press("KeyR");
  await expect.poll(cycle).toBe(checkpoints[1].cycle);
});

test("notifications expire within two seconds even after playback resumes", async ({
  page,
}) => {
  await ready(page);
  await page.getByLabel("Open ROM", { exact: true }).setInputFiles(loop);
  await expect(
    page.getByRole("button", { name: "How to play", exact: true }),
  ).toBeEnabled();
  await page.getByLabel(/^Game screen/).focus();
  await page.keyboard.press("Space");
  await page.keyboard.press("Shift+Digit1");
  await expect(page.locator(".console-notice")).toContainText(
    "Saved slot 1 on this device",
  );
  await page.keyboard.press("Space");
  await expect(page.locator(".console-notice")).toHaveCount(0, {
    timeout: 2500,
  });
});

test("timeline cancellation resumes safely and stale previews are discarded", async ({
  page,
}) => {
  await ready(page);
  await page.getByLabel("Open ROM", { exact: true }).setInputFiles(loop);
  await expect(
    page.getByRole("button", { name: "How to play", exact: true }),
  ).toBeEnabled();
  const canvas = page.getByLabel(/^Game screen/);
  const frames = () =>
    page.evaluate(async () =>
      Number(
        (await (window as unknown as { emulator: Emulator }).emulator.inspect())
          .frames,
      ),
    );

  // Clearing history while playing makes the unavailable path explicit and
  // must resume the play state that Shift+R temporarily paused.
  await page.evaluate(() =>
    (window as unknown as { emulator: Emulator }).emulator.setRewindConfig(0),
  );
  await canvas.focus();
  await page.keyboard.press("Shift+KeyR");
  await expect(
    page.getByRole("dialog", { name: "Rewind timeline" }),
  ).toHaveCount(0);
  await expect(page.locator(".console-notice")).toContainText(
    "No rewind checkpoints",
  );
  const resumed = await frames();
  await expect.poll(frames).toBeGreaterThan(resumed);

  await canvas.focus();
  await page.keyboard.press("Space");
  const state = await page.evaluate(async () => {
    const emulator = (window as unknown as { emulator: Emulator }).emulator;
    await emulator.setRewindConfig(undefined, 1);
    for (let index = 0; index < 4; index++) {
      const cpu = await emulator.inspect();
      await emulator.runUntil(cpu.cycle + 280896n);
    }
    return emulator.saveState();
  });
  await page.evaluate(() => {
    const emulator = (window as unknown as { emulator: Emulator }).emulator;
    const original = emulator.rewindFrame.bind(emulator);
    emulator.rewindFrame = async (index: number) => {
      await new Promise((resolve) => setTimeout(resolve, 150));
      return original(index);
    };
  });
  await canvas.focus();
  await page.keyboard.press("Shift+KeyR");
  const dialog = page.getByRole("dialog", { name: "Rewind timeline" });
  await expect(dialog).toBeVisible();
  await page.evaluate(
    (bytes) =>
      (window as unknown as { emulator: Emulator }).emulator.loadState(
        new Uint8Array(bytes),
      ),
    Array.from(state),
  );
  await page.waitForTimeout(250);
  await expect(dialog).toHaveCount(0);
  await expect(page.locator(".console-notice")).toContainText(
    "Rewind history changed",
  );
  expect(
    await page.evaluate(() =>
      (window as unknown as { emulator: Emulator }).emulator.rewindDepth(),
    ),
  ).toBe(0);
  expect(await snapshotHash(page)).toBe(
    createHash("sha256").update(Buffer.from(state)).digest("hex"),
  );
});

test("Home cancels an in-flight rewind without restoring or restarting play", async ({
  page,
}) => {
  await ready(page);
  await page.getByLabel("Open ROM", { exact: true }).setInputFiles(loop);
  await expect(
    page.getByRole("heading", { name: "Loop", exact: true }),
  ).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as unknown as { emulator: Emulator }).emulator.rewindDepth(),
      ),
    )
    .toBeGreaterThan(0);
  await page.evaluate(() => {
    const host = window as unknown as {
      emulator: Emulator;
      rewindPaused: boolean;
      releaseRewind: () => void;
    };
    const pause = host.emulator.pause.bind(host.emulator);
    let first = true;
    host.emulator.pause = async () => {
      const result = await pause();
      if (first) {
        first = false;
        host.rewindPaused = true;
        await new Promise<void>((resolve) => {
          host.releaseRewind = resolve;
        });
      }
      return result;
    };
  });
  await page.getByLabel(/^Game screen/).focus();
  await page.keyboard.press("KeyR");
  await page.waitForFunction(
    () => (window as unknown as { rewindPaused: boolean }).rewindPaused,
  );
  await page.getByRole("button", { name: "evenfall home" }).click();
  await expect(page.locator(".landing")).toBeVisible();
  const paused = await snapshotHash(page);
  await page.evaluate(() =>
    (window as unknown as { releaseRewind: () => void }).releaseRewind(),
  );
  await page.waitForTimeout(350);
  expect(await snapshotHash(page)).toBe(paused);
});

test("new console controls and persistent quick slots work", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await ready(page);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({
    path: "../../artifacts/hud/reboot-empty-mobile.png",
    fullPage: true,
  });
  await page.setViewportSize({ width: 1280, height: 720 });
  await page.getByLabel("Open ROM", { exact: true }).setInputFiles(loop);
  await expect(
    page.getByRole("heading", { name: "Loop", exact: true }),
  ).toBeVisible();
  await expect(page.locator('[data-game-layout="generic"]')).toBeVisible();
  await expect(
    page.getByRole("button", { name: "evenfall home" }),
  ).toBeVisible();
  await expect(page.locator(".console-credit")).toHaveCount(0);
  await expect(page.getByText(/Made by/)).toHaveCount(0);
  await screenFillsColumn(page);
  const canvas = page.getByLabel(/^Game screen/);
  await canvas.focus();
  await page.keyboard.press("Space");
  const saved = await snapshotHash(page);
  await page.keyboard.press("Shift+Digit2");
  await expect(page.locator(".console-notice")).toContainText(
    "Saved slot 2 on this device",
  );
  await runOneFrame(page);
  await expect.poll(() => snapshotHash(page)).not.toBe(saved);
  await page.keyboard.press("Control+Digit2");
  await expect(page.locator(".console-notice")).toContainText("Loaded slot 2");
  expect(await snapshotHash(page)).toBe(saved);
  // The load's checkpoint may still be committing, which ignores the key.
  await expect(async () => {
    await page.keyboard.press("Control+Digit4");
    await expect(page.locator(".console-notice")).toContainText(
      "Quick slot 4 is empty",
      { timeout: 1000 },
    );
  }).toPass();
  expect(await snapshotHash(page)).toBe(saved);
  // Digit0 addresses slot 10.
  await page.keyboard.press("Shift+Digit0");
  await expect(page.locator(".console-notice")).toContainText(
    "Saved slot 10 on this device",
  );
  // Quick-slot keys are ignored until the save finishes committing.
  await expect(async () => {
    await page.keyboard.press("Control+Digit0");
    await expect(page.locator(".console-notice")).toContainText(
      "Loaded slot 10",
      { timeout: 1000 },
    );
  }).toPass();
  expect(await snapshotHash(page)).toBe(saved);
  await page.getByRole("button", { name: "Playback speed" }).click();
  await page
    .getByRole("dialog", { name: "Playback speed" })
    .getByRole("button", { name: "2×" })
    .click();
  await expect(page.getByRole("button", { name: "Playback speed" })).toHaveText(
    "2×",
  );
  await page.getByRole("button", { name: "Mute sound" }).click();
  await expect(
    page.getByRole("button", { name: "Unmute sound" }),
  ).toHaveAttribute("aria-pressed", "true");
  await canvas.focus();
  await page.keyboard.press("Space");
  await page.keyboard.press("Space");
  await page.goto("/");
  await expect(
    page.getByRole("button", { name: "Resume adventure" }),
  ).toBeEnabled();
  await page.getByRole("button", { name: "Resume adventure" }).click();
  await expect(page.getByLabel("Open ROM", { exact: true })).toBeEnabled();
  await page.getByRole("button", { name: "Library", exact: true }).click();
  await page.getByRole("button", { name: "Player", exact: true }).click();
  await expect(page.getByRole("main", { name: "Library" })).toHaveCount(0);
  await canvas.focus();
  await page.keyboard.press("Space");
  await page.keyboard.press("Control+Digit2");
  await expect(page.locator(".console-notice")).toContainText("Loaded slot 2");
  expect(await snapshotHash(page)).toBe(saved);
  await page.getByRole("button", { name: "Library", exact: true }).click();
  await expect(page.getByRole("main", { name: "Library" })).toBeVisible();
  await page.keyboard.press("Shift+Digit4");
  await expect(page.locator(".console-notice")).not.toContainText(
    "Saved slot 4",
  );
  await page.screenshot({ path: "../../artifacts/hud/reboot-library.png" });
  await page.getByRole("button", { name: "Player", exact: true }).click();
  await expect(page.getByRole("main", { name: "Library" })).toHaveCount(0);
  await page.keyboard.press("Control+Digit4");
  await expect(page.locator(".console-notice")).toContainText(
    "Quick slot 4 is empty",
  );
  await page.getByRole("button", { name: "Dismiss notification" }).click();
  await page.getByRole("button", { name: "Fullscreen", exact: true }).click();
  await expect
    .poll(() => page.evaluate(() => document.fullscreenElement?.id))
    .toBe("game-display");
  await page.getByLabel("Player help", { exact: true }).click();
  await expect(page.getByRole("dialog", { name: "How to play" })).toContainText(
    "Quick save / load",
  );
  await page.screenshot({
    path: "../../artifacts/hud/reboot-fullscreen-help.png",
  });
  await page.getByRole("button", { name: "Show touch controls" }).click();
  await page.getByRole("button", { name: "Close dialog" }).click();
  await page.evaluate(() => document.exitFullscreen());
  await expect(
    page.getByLabel("Touch controls", { exact: true }),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByLabel("Touch controls", { exact: true })).toHaveCount(
    0,
  );
  await page.setViewportSize({ width: 390, height: 844 });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({
    path: "../../artifacts/hud/reboot-generic-mobile.png",
    fullPage: true,
  });
  // Phones keep the toolbar in the player menu.
  await page.getByRole("button", { name: "Player menu" }).click();
  await page.getByRole("button", { name: "More options" }).click();
  await expect(
    page.getByRole("dialog", { name: "Saves and options" }),
  ).toBeVisible();
  await page.screenshot({
    path: "../../artifacts/hud/reboot-options-mobile.png",
    fullPage: true,
  });
  expect(errors).toEqual([]);
});
