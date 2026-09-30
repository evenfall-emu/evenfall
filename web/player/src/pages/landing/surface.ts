import surfaceArt from "../../assets/landing/surface.png";
import type { Camera, Plate } from "./camera";
import { drawClouds, drawGlow, type Clouds, type Glow } from "./sky";
import { drawSea, type Sea } from "./waves";

// The surface plate: starry sky over the coastal sunset, the sea, and the
// waterline where Home's fresco dives below it. Home and the Player's empty
// screen both draw it. Coordinates are art pixels; see the assets' README.

export const SURFACE_ART = surfaceArt;

export const SUN = { left: 297, top: 159, right: 346, bottom: 197 };
export const SUN_X = Math.round((SUN.left + SUN.right) / 2);

export const SURFACE = {
  width: 480,
  height: 252,
  keep: SUN,
  anchor: { x: SUN_X, y: SUN.bottom },
} satisfies Plate;

/** The flat sky of the plate's top row, filling a view above the plate. */
export const SKY = "#072957";

/** The painted sea, from the horizon down to where the waterline (the
 * surface seen from below, the plate's last rows) begins. */
export const SEA: Sea = { top: 198, bottom: 241, swell: 204 };
/** The painted cliff foot reaches into the sea's first rows; hold it still. */
const shore = (row: number) =>
  row < 200 ? 290 : row < 202 ? 90 : row < 204 ? 70 : 0;
/**
 * The painted cloud band drifts over two minutes, from above its highest
 * cloud to just above the sun (row 159), so the disk never shears. Each row
 * starts right of the headland, which rises into the band from row 123.
 */
const CLOUDS: Clouds = {
  top: 94,
  bottom: 157,
  left: (row) => (row < 125 ? 0 : row < 146 ? 14 : 40),
  reach: 6,
  cycle: 120,
};
/** The lighthouse lamp pulses. The painted stars stay still and lit. */
const BEACON: Glow = {
  pixels: [
    [103, 165],
    [104, 165],
    [105, 165],
    [102, 166],
    [103, 166],
    [104, 166],
    [105, 166],
    [102, 167],
    [103, 167],
    [104, 167],
    [103, 168],
    [104, 168],
  ],
  ramp: ["#55bca4", "#8cdbbe", "#bcd8af", "#fff0c8"],
  period: 5,
};

/**
 * Draws the plate's first `rows` rows at `camera` as they are at `seconds`:
 * drifting clouds, rolling sea and the pulsing lamp. The caller fills the
 * view first; nothing is drawn below the last row.
 */
export function drawSurface(
  context: CanvasRenderingContext2D,
  image: CanvasImageSource,
  camera: Camera,
  seconds: number,
  rows: number = SURFACE.height,
) {
  const { width } = SURFACE;
  context.drawImage(
    image,
    0,
    0,
    width,
    rows,
    camera.x,
    camera.y,
    width * camera.scale,
    rows * camera.scale,
  );
  drawClouds(context, image, SURFACE, CLOUDS, camera, seconds);
  drawSea(context, image, SURFACE, SEA, camera, seconds, shore);
  drawGlow(context, BEACON, camera, seconds);
}
