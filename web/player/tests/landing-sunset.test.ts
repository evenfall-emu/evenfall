import { describe, expect, it } from "vite-plus/test";
import { frame } from "../src/pages/landing/camera";
import { SUN } from "../src/pages/landing/surface";
import { OPEN_SEA, sunsetFraming } from "../src/pages/landing/sunset";

// The Player's empty screen at sizes measured in the browser: the screen's
// box in device pixels and where its text ends, in CSS pixels.

/** The sun's box in the view, in device pixels, for a framed camera. */
function sunIn(camera: { scale: number; x: number; y: number }) {
  return {
    left: camera.x + SUN.left * camera.scale,
    right: camera.x + (SUN.right + 1) * camera.scale,
    top: camera.y + SUN.top * camera.scale,
    bottom: camera.y + (SUN.bottom + 1) * camera.scale,
  };
}

describe("empty screen sunset framing", () => {
  it("ends on open sea, without the waterline that leads into Home's descent", () => {
    // surface.png: the sea's last row is 241; rows 242–251 are the waterline.
    expect(OPEN_SEA.height).toBe(242);
  });

  it("keeps two CSS pixels per art pixel on a 3x phone, the sun below the text", () => {
    // A 390 × 260 portrait screen at three device pixels per CSS pixel,
    // text ending 140 CSS pixels down.
    const size = { width: 1170, height: 780, ratio: 3 };
    const camera = frame(size, OPEN_SEA, sunsetFraming(140, size))!;
    expect(camera.scale).toBe(6);
    // Whole, and clear of the text by its 12 CSS pixels of padding.
    const sun = sunIn(camera);
    expect(sun.top).toBeGreaterThanOrEqual((140 + 12) * 3);
    expect(sun.bottom).toBeLessThanOrEqual(780);
    expect(sun.left).toBeGreaterThanOrEqual(0);
    expect(sun.right).toBeLessThanOrEqual(1170);
    // The sea reaches the screen's bottom edge; the sky fills above.
    expect(camera.y + OPEN_SEA.height * camera.scale).toBeGreaterThanOrEqual(
      780,
    );
  });

  it("centers the sun across a desktop screen at three CSS pixels per art pixel", () => {
    const size = { width: 880, height: 590, ratio: 1 };
    const camera = frame(size, OPEN_SEA, sunsetFraming(260, size))!;
    expect(camera.scale).toBe(3);
    const sun = sunIn(camera);
    expect(Math.abs((sun.left + sun.right) / 2 - 440)).toBeLessThanOrEqual(3);
    expect(sun.top).toBeGreaterThanOrEqual(260 + 12);
  });

  it("drops to a smaller whole scale when the text fills most of the screen", () => {
    // A 360 × 240 screen whose text wraps to end 170 pixels down: at two CSS
    // pixels per art pixel the 39-row sun cannot fit below it, at one it can.
    const size = { width: 360, height: 240, ratio: 1 };
    const camera = frame(size, OPEN_SEA, sunsetFraming(170, size))!;
    expect(camera.scale).toBe(1);
    const sun = sunIn(camera);
    expect(sun.top).toBeGreaterThanOrEqual(170 + 12);
    expect(sun.bottom).toBeLessThanOrEqual(240);
    expect(sun.left).toBeGreaterThanOrEqual(0);
    expect(sun.right).toBeLessThanOrEqual(360);
  });
});
