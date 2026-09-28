import { describe, expect, it } from "vitest";
import { frame, type Plate } from "../src/pages/landing/camera";

// An off-center sun, so a solver that centers the plate cannot pass.
const plate: Plate = {
  width: 800,
  height: 300,
  keep: { left: 500, top: 150, right: 560, bottom: 210 },
  anchor: { x: 530, y: 210 },
};

describe("landing camera", () => {
  it("places the sun at its desktop target at the preferred scale", () => {
    // Preferred scale 2. x = round(0.74 × 1000 − 2 × 530) = −320, within
    // [max(1000 − 1600, 16 − 1000), min(0, 1000 − 16 − 1120)] = [−600, −136].
    // The horizon target 0.6 × 500 − 420 = −120 is below the covering bound
    // 500 − 600 = −100, so y stops at −100.
    expect(
      frame({ width: 1000, height: 500 }, plate, {
        ratio: 1,
        pixel: 2,
        focus: { x: 0.74, y: 0.6 },
        padding: 16,
      }),
    ).toEqual({ scale: 2, x: -320, y: -100 });
  });

  it("works in device pixels, centering the sun on a phone at ratio 3", () => {
    // 390 × 420 CSS pixels at ratio 3: a 1170 × 1260 device view, scale 6.
    // x = round(585 − 3180) = −2595, within [−3630, −2238] ∩ [−3630, 0];
    // padding 48 device pixels: [48 − 3000, 1170 − 48 − 3360] = [−2952, −2238].
    // y = round(756 − 1260) = −504, within [−540, −48].
    expect(
      frame({ width: 1170, height: 1260 }, plate, {
        ratio: 3,
        pixel: 2,
        focus: { x: 0.5, y: 0.6 },
        padding: 16,
      }),
    ).toEqual({ scale: 6, x: -2595, y: -504 });
  });

  it("shrinks the scale when the sun would not fit at the preferred one", () => {
    const wide: Plate = {
      width: 800,
      height: 300,
      keep: { left: 100, top: 50, right: 300, bottom: 150 },
      anchor: { x: 200, y: 150 },
    };
    // At scale 2 the 200-pixel sun is 400 device pixels in a 300 view.
    // Scale 1: x = round(150 − 200) = −50 within [−84, −16]; y is exactly 0.
    expect(
      frame({ width: 300, height: 300 }, wide, {
        ratio: 1,
        pixel: 2,
        focus: { x: 0.5, y: 0.6 },
        padding: 16,
      }),
    ).toEqual({ scale: 1, x: -50, y: 0 });
  });

  it("enlarges past the preferred scale when the plate would not cover", () => {
    // A 3000-pixel view needs scale 4 to cover 800 art pixels; preferred 2.
    const camera = frame({ width: 3000, height: 1000 }, plate, {
      ratio: 1,
      pixel: 2,
      focus: { x: 0.74, y: 0.6 },
      padding: 16,
    });
    expect(camera?.scale).toBe(4);
    expect(camera!.x).toBeLessThanOrEqual(0);
    expect(camera!.x + 4 * plate.width).toBeGreaterThanOrEqual(3000);
  });

  it("reports an impossible composition instead of clipping the sun", () => {
    const edge: Plate = {
      width: 200,
      height: 100,
      keep: { left: 0, top: 20, right: 40, bottom: 60 },
      anchor: { x: 20, y: 60 },
    };
    // The sun touches the plate's left edge, so no offset leaves 16 px.
    expect(
      frame({ width: 100, height: 100 }, edge, {
        ratio: 1,
        pixel: 1,
        focus: { x: 0.5, y: 0.5 },
        padding: 16,
      }),
    ).toBeNull();
  });

  it("keeps the sun below text that covers the top of the view", () => {
    // Text covers the top 300 CSS pixels, so the sun (rows 150–210) must lie
    // within [316, 484]: y ∈ [max(500 − 300s, 316 − 150s), min(0, 484 − 210s)].
    // Scale 2 gives [16, 0], scale 3 [−134, −146], scale 4 [−284, −356]:
    // all empty, and scale 1 cannot cover the view.
    expect(
      frame({ width: 1000, height: 500 }, plate, {
        ratio: 1,
        pixel: 2,
        focus: { x: 0.5, y: 0.6 },
        padding: 16,
        safe: { top: 300 },
      }),
    ).toBeNull();
    // With the text covering only the top 150 pixels, scale 2 works:
    // y ∈ [max(−100, 166 − 300), min(0, 500 − 16 − 420)] = [−100, 0];
    // the target 300 − 420 = −120 clamps to −100, sun top at 200 ≥ 166.
    expect(
      frame({ width: 1000, height: 500 }, plate, {
        ratio: 1,
        pixel: 2,
        focus: { x: 0.5, y: 0.6 },
        padding: 16,
        safe: { top: 150 },
      }),
    ).toEqual({ scale: 2, x: -560, y: -100 });
  });

  it("keeps the sun right of a text column, converting insets to device pixels", () => {
    // Ratio 2: the 300-CSS-pixel column is 600 device pixels, padding 32.
    // Scale 4 (pixel 2 × ratio 2, which also covers 1000 / 300 rows).
    // x = round(1480 − 2120) = −640, within [max(2000 − 3200, 632 − 2000),
    // min(0, 1968 − 2240)] = [−1200, −272]. y target 600 − 840 = −240 clamps
    // to the covering bound 1000 − 1200 = −200.
    expect(
      frame({ width: 2000, height: 1000 }, plate, {
        ratio: 2,
        pixel: 2,
        focus: { x: 0.74, y: 0.6 },
        padding: 16,
        safe: { left: 300 },
      }),
    ).toEqual({ scale: 4, x: -640, y: -200 });
    // A 900-CSS-pixel column (1800 device) leaves 2000 − 1832 − 32 = 136
    // device pixels for a 60-art-pixel sun: it needs scale ≤ 2, but covering
    // the view needs scale ≥ 4, so no framing exists.
    expect(
      frame({ width: 2000, height: 1000 }, plate, {
        ratio: 2,
        pixel: 2,
        focus: { x: 0.74, y: 0.6 },
        padding: 16,
        safe: { left: 900 },
      }),
    ).toBeNull();
  });

  it("pins the plate's bottom row to the view's bottom edge", () => {
    // Scale 2: the only pinned offset is 500 − 600 = −100, and the sun
    // (rows 150–210 → 200–320) clears the 150 + 16 inset.
    expect(
      frame({ width: 1000, height: 500 }, plate, {
        ratio: 1,
        pixel: 2,
        focus: { x: 0.74, y: 0.2 },
        padding: 16,
        safe: { top: 150 },
        pinBottom: true,
      }),
    ).toEqual({ scale: 2, x: -320, y: -100 });
    // A 250-pixel inset needs the sun top at 266 or lower. Pinned at scale 2
    // it sits at 200; at scale 3 (y = −400) at 50; so pinning fails where the
    // unpinned framing would have moved the plate down and cropped its bottom.
    const unpinned = frame({ width: 1000, height: 500 }, plate, {
      ratio: 1,
      pixel: 2,
      focus: { x: 0.74, y: 0.2 },
      padding: 16,
      safe: { top: 250 },
    });
    expect(unpinned).not.toBeNull();
    expect(unpinned!.y).toBeGreaterThan(500 - unpinned!.scale * 300);
    expect(
      frame({ width: 1000, height: 500 }, plate, {
        ratio: 1,
        pixel: 2,
        focus: { x: 0.74, y: 0.2 },
        padding: 16,
        safe: { top: 250 },
        pinBottom: true,
      }),
    ).toBeNull();
  });

  it("lets an open-topped plate sit below the view's top edge", () => {
    // Without openTop, covering 800 rows of a 300-row plate needs scale 3:
    // x = round(740 − 1590) = −850 in [−1400, −696]; pinned y = 800 − 900.
    const framing = {
      ratio: 1,
      pixel: 2,
      focus: { x: 0.74, y: 0.5 },
      padding: 16,
      pinBottom: true,
    };
    expect(frame({ width: 1000, height: 800 }, plate, framing)).toEqual({
      scale: 3,
      x: -850,
      y: -100,
    });
    // With it, the preferred scale 2 covers the width, and the pinned plate
    // starts 800 − 600 = 200 pixels down, leaving flat color above.
    expect(
      frame({ width: 1000, height: 800 }, plate, { ...framing, openTop: true }),
    ).toEqual({ scale: 2, x: -320, y: 200 });
  });

  it("frames plates without a protected region by coverage alone", () => {
    const band: Plate = { width: 480, height: 88, anchor: { x: 240, y: 88 } };
    // Scale 3 (pixel 3, ratio 1); bottom-anchored: y = 264 − 264 = 0.
    // x = round(700 − 720) = −20, within [1400 − 1440, 0] = [−40, 0].
    expect(
      frame({ width: 1400, height: 264 }, band, {
        ratio: 1,
        pixel: 3,
        focus: { x: 0.5, y: 1 },
      }),
    ).toEqual({ scale: 3, x: -20, y: 0 });
  });
});
