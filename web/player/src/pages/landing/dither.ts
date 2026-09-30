// The fresco's ordered dither: an 8 × 8 Bayer matrix on the art grid, the
// one scripts/landing-art/build.py uses for the plates' painted fades, so fades
// drawn at runtime match them. Every pixel stays a palette color.

import type { Camera } from "./camera";

const BAYER = [
  [0, 32, 8, 40, 2, 34, 10, 42],
  [48, 16, 56, 24, 50, 18, 58, 26],
  [12, 44, 4, 36, 14, 46, 6, 38],
  [60, 28, 52, 20, 62, 30, 54, 22],
  [3, 35, 11, 43, 1, 33, 9, 41],
  [51, 19, 59, 27, 49, 17, 57, 25],
  [15, 47, 7, 39, 13, 45, 5, 37],
  [63, 31, 55, 23, 61, 29, 53, 21],
];

/** The threshold of the art pixel in column `x`, row `y`, in [0, 1). */
const threshold = (x: number, y: number) =>
  BAYER[((y % 8) + 8) % 8][((x % 8) + 8) % 8] / 64;

/** Whether the art pixel at (x, y) is lit at coverage `amount` (0 to 1). */
export const shown = (x: number, y: number, amount: number) =>
  threshold(x, y) < amount;

export type Edge = "top" | "bottom";

/**
 * Dissolves the `rows` art rows along a view's top or bottom edge into
 * `color`: all `color` in the row the edge cuts through, none past the
 * band. The art stays where its threshold is at least its coverage's
 * complement, as in build.py's fades toward deep water, so over a plate's
 * painted fade the sparser of the two wins instead of the patterns
 * cancelling. The mask is painted once, one pixel per art pixel; the
 * returned function draws it at `camera`'s scale over what the view already
 * shows.
 */
export function fadeEdge(
  camera: Camera,
  size: { width: number; height: number },
  edge: Edge,
  rows: number,
  color: string,
) {
  const k = camera.scale;
  const left = Math.floor(-camera.x / k);
  const columns = Math.ceil((size.width - camera.x) / k) - left;
  // The band's first art row: the one the top edge cuts through, or `rows`
  // above the one the bottom edge cuts through.
  const first =
    edge === "top"
      ? Math.floor(-camera.y / k)
      : Math.ceil((size.height - camera.y) / k) - 1 - rows;
  const mask = document.createElement("canvas");
  mask.width = columns;
  mask.height = rows + 1;
  const paint = mask.getContext("2d")!;
  paint.fillStyle = color;
  for (let i = 0; i <= rows; i++) {
    const row = first + i;
    // Coverage at the row's side nearest the edge, from none in the row the
    // edge cuts through to all `rows` rows away.
    const distance =
      edge === "top"
        ? camera.y + row * k
        : size.height - (camera.y + (row + 1) * k);
    const amount = distance / (rows * k);
    for (let c = 0; c < columns; c++)
      if (threshold(left + c, row) < 1 - amount) paint.fillRect(c, i, 1, 1);
  }
  return (context: CanvasRenderingContext2D) => {
    context.imageSmoothingEnabled = false;
    context.drawImage(
      mask,
      camera.x + left * k,
      camera.y + first * k,
      columns * k,
      mask.height * k,
    );
  };
}
