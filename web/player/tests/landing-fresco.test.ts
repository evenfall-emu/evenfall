import { describe, expect, it } from "vite-plus/test";
import { frame } from "../src/pages/landing/camera";
import { PLATES, surfaceFraming } from "../src/pages/landing/fresco";

// The real surface plate and framing, at a tablet-shaped view where the
// hero text spans nearly the whole width: 768 × 900 CSS pixels at ratio 1,
// text ending 728 pixels in and 496 pixels down (measured in the browser).
const size = { width: 768, height: 900, ratio: 1 };
const text = { right: 728, bottom: 496 };

describe("landing surface framing", () => {
  it("finds no room for the sun beside text that fills the width", () => {
    expect(frame(size, PLATES.surface, surfaceFraming(text, size))).toBeNull();
  });

  it("frames the sun below the text instead, whole and clear of it", () => {
    const camera = frame(
      size,
      PLATES.surface,
      surfaceFraming(text, size, true),
    );
    expect(camera).not.toBeNull();
    const { scale, x, y } = camera!;
    const sun = PLATES.surface.keep;
    // The sun lies inside the view, below the text plus 12 px of padding.
    expect(x + sun.left * scale).toBeGreaterThanOrEqual(12);
    expect(x + sun.right * scale).toBeLessThanOrEqual(768 - 12);
    expect(y + sun.top * scale).toBeGreaterThanOrEqual(496 + 12);
    expect(y + sun.bottom * scale).toBeLessThanOrEqual(900 - 12);
    // The plate's last row rests on the view's bottom edge.
    expect(y + PLATES.surface.height * scale).toBe(900);
  });
});
