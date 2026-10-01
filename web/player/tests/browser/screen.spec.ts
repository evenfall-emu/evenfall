import { expect, test, type Page } from "@playwright/test";
import type { Player } from "../../src/player";
import { chromiumLaunch, SOFTWARE_WEBGPU } from "./launch";

const RED = 0x001f;
const GREEN = 0x03e0;
const BLUE = 0x7c00;
const WHITE = 0x7fff;

/**
 * A ROM that shows a mode-3 bitmap: red, green, blue and white quadrants.
 * Its code sets DISPCNT, copies the bitmap after it into VRAM and loops.
 */
function quadrants() {
  const code = [
    0xe3a00404, // mov r0, #0x04000000
    0xe3a01b01, // mov r1, #0x400
    0xe3811003, // orr r1, r1, #3      @ mode 3, BG2 on
    0xe1c010b0, // strh r1, [r0]
    0xe3a00406, // mov r0, #0x06000000
    0xe28f1014, // add r1, pc, #0x14   @ the bitmap, right after the code
    0xe3a02b4b, // mov r2, #0x12c00    @ 240 * 160 * 2 bytes
    0xe4913004, // ldr r3, [r1], #4
    0xe4803004, // str r3, [r0], #4
    0xe2522004, // subs r2, r2, #4
    0x1afffffb, // bne the ldr
    0xeafffffe, // b .
  ];
  const buffer = Buffer.alloc(code.length * 4 + 240 * 160 * 2);
  code.forEach((word, i) => buffer.writeUInt32LE(word, i * 4));
  for (let y = 0; y < 160; y++)
    for (let x = 0; x < 240; x++) {
      const color = y < 80 ? (x < 120 ? RED : GREEN) : x < 120 ? BLUE : WHITE;
      buffer.writeUInt16LE(color, code.length * 4 + (y * 240 + x) * 2);
    }
  return {
    name: "Quadrants.gba",
    mimeType: "application/octet-stream",
    buffer,
  };
}

const screenState = (page: Page) =>
  page.evaluate(
    () => (window as unknown as { player: Player }).player.state.screen,
  );

async function open(page: Page) {
  await page.goto("/");
  await page.getByRole("button", { name: "Open player" }).click();
  await page.getByLabel("Open ROM", { exact: true }).setInputFiles(quadrants());
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as unknown as { player: Player }).player.state.playing,
      ),
    )
    .toBe(true);
  await expect
    .poll(async () => (await screenState(page)).renderer)
    .not.toBe("starting");
}

/** What the WebGPU screen draws, read back through WebGPU itself. */
async function capture(page: Page) {
  const shot = await page.evaluate(async () => {
    const result = await (
      window as unknown as { player: Player }
    ).player.captureScreen();
    if (!result) return null;
    let binary = "";
    for (let i = 0; i < result.rgba.length; i += 0x8000)
      binary += String.fromCharCode(...result.rgba.subarray(i, i + 0x8000));
    return { width: result.width, height: result.height, rgba: btoa(binary) };
  });
  expect(shot).not.toBeNull();
  const { width, height } = shot!;
  const rgba = Buffer.from(shot!.rgba, "base64");
  const at = (x: number, y: number) => [
    ...rgba.subarray((y * width + x) * 4, (y * width + x) * 4 + 3),
  ];
  return { width, height, at };
}

const QUADRANT_COLORS = [
  [255, 0, 0],
  [0, 255, 0],
  [0, 0, 255],
  [255, 255, 255],
];
const isQuadrantColor = (rgb: number[]) =>
  QUADRANT_COLORS.some((color) => color.every((v, i) => v === rgb[i]));

async function chooseInMenu(page: Page, name: RegExp) {
  await page.getByRole("button", { name: "More options" }).click();
  await page.getByRole("button", { name }).click();
  await page.keyboard.press("Escape");
}

// Chromium's software WebGPU for the whole file. The 2D test hides it.
test.use({ launchOptions: chromiumLaunch(SOFTWARE_WEBGPU) });

/**
 * Reads the presenter's output through WebGPU, because no capture path
 * (screenshots, drawImage, the X display) shows a SwiftShader WebGPU
 * canvas. Expected values come from the quadrants' layout: exact colours
 * at quadrant centres, no blends with Pixel, one or two blended columns
 * at an edge with Vanilla+, and a black border around the largest whole
 * multiple with integer scaling.
 */
test("the presenter draws at device size and the menu switches presets and integer scaling", async ({
  page,
}) => {
  await open(page);
  if ((await screenState(page)).renderer !== "webgpu") {
    expect(
      process.env.EVENFALL_REQUIRE_GPU,
      "EVENFALL_REQUIRE_GPU is set, but Chromium offered no WebGPU",
    ).toBeUndefined();
    test.skip(true, "Chromium offered no WebGPU here");
  }
  const canvas = page.getByLabel(/^Game screen/);
  const box = await canvas.evaluate((element: HTMLCanvasElement) => ({
    width: element.width,
    css: element.getBoundingClientRect().width * devicePixelRatio,
  }));
  expect(Math.abs(box.width - box.css)).toBeLessThanOrEqual(1);
  // A fractional scale, so Vanilla+ has edges to blend.
  expect(box.width % 240).not.toBe(0);

  const pixel = await capture(page);
  expect(pixel.width).toBe(box.width);
  const [qx, qy] = [pixel.width / 4, pixel.height / 4].map(Math.floor);
  expect(pixel.at(qx, qy)).toEqual([255, 0, 0]);
  expect(pixel.at(3 * qx, qy)).toEqual([0, 255, 0]);
  expect(pixel.at(qx, 3 * qy)).toEqual([0, 0, 255]);
  expect(pixel.at(3 * qx, 3 * qy)).toEqual([255, 255, 255]);
  const blendedInRow = (shot: typeof pixel) =>
    Array.from({ length: shot.width }, (_, x) => shot.at(x, qy)).filter(
      (rgb) => !isQuadrantColor(rgb),
    ).length;
  expect(blendedInRow(pixel)).toBe(0);

  await chooseInMenu(page, /^Vanilla\+/);
  expect((await screenState(page)).settings.preset).toBe("vanilla-plus");
  const vanilla = await capture(page);
  expect(vanilla.at(qx, qy)).toEqual([255, 0, 0]);
  expect(vanilla.at(3 * qx, 3 * qy)).toEqual([255, 255, 255]);
  expect(blendedInRow(vanilla)).toBeGreaterThanOrEqual(1);
  expect(blendedInRow(vanilla)).toBeLessThanOrEqual(2);

  await chooseInMenu(page, /^Whole-number scaling/);
  const integer = await capture(page);
  const multiple = Math.min(
    Math.floor(integer.width / 240),
    Math.floor(integer.height / 160),
  );
  const left = Math.floor((integer.width - 240 * multiple) / 2);
  const top = Math.floor((integer.height - 160 * multiple) / 2);
  expect(left).toBeGreaterThan(0);
  expect(integer.at(left - 1, top + 1)).toEqual([0, 0, 0]);
  expect(integer.at(left, top)).toEqual([255, 0, 0]);
  expect(integer.at(left + 240 * multiple - 1, top)).toEqual([0, 255, 0]);
  expect(integer.at(left + 240 * multiple, top)).toEqual([0, 0, 0]);

  // While paused, a smaller window still resizes the drawing buffer.
  await page.evaluate(() =>
    (window as unknown as { player: Player }).player.pause(),
  );
  await page.setViewportSize({ width: 900, height: 700 });
  await expect
    .poll(() => canvas.evaluate((element: HTMLCanvasElement) => element.width))
    .toBeLessThan(box.width);
  expect((await capture(page)).width).toBeLessThan(box.width);

  // The choices persist across visits.
  await page.reload();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as unknown as { player?: Player }).player?.state.screen
            .settings,
      ),
    )
    .toEqual({ preset: "vanilla-plus", integerScale: true });
});

/** The 2D fallback keeps the canvas at native size and disables filters. */
test("without WebGPU the screen draws in 2D and the filters are disabled", async ({
  page,
}) => {
  await page.addInitScript(() => {
    Object.defineProperty(Navigator.prototype, "gpu", { get: () => undefined });
  });
  await open(page);
  expect((await screenState(page)).renderer).toBe("canvas");
  expect(
    await page
      .getByLabel(/^Game screen/)
      .evaluate((element: HTMLCanvasElement) => [
        element.width,
        element.height,
      ]),
  ).toEqual([240, 160]);
  await page.getByRole("button", { name: "More options" }).click();
  await expect(page.getByRole("button", { name: /^Vanilla\+/ })).toBeDisabled();
  await expect(page.getByText(/Screen filters need WebGPU/)).toBeVisible();
});
