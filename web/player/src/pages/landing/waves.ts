// Wave motion for the painted sea, combining two motions:
//
// - Sway: each art row of water shifts sideways by whole art pixels along two
//   slow swells, rows out of phase so crests roll.
// - Swell: each row samples a nearby row of the plate, a few rows above or
//   below along a wave travelling down the sea, so crests visibly rise and
//   sink toward the viewer (the original Home's sampling, on the art grid).
//
// Both grow from almost nothing at the horizon to their full reach near the
// viewer. Rows are drawn from the plate itself, so the painted foam, crests
// and sun reflection move without new pixels being invented.

import { drawShiftedRow } from "./sky";

export interface Sea {
  /** First art row of water (the horizon) and its last moving row. */
  top: number;
  bottom: number;
  /** Rows above this, near the horizon, only drift a little. */
  swell: number;
}

const depthOf = (sea: Sea, row: number) =>
  (row - sea.top) / (sea.bottom - sea.top);

/**
 * The sideways offset of one art row of water at `seconds`, in whole art
 * pixels. Rows near the horizon barely move; rows near the viewer sway up to
 * `amplitude` pixels. Neighboring rows are out of phase, so crests roll.
 */
export function waveOffset(
  sea: Sea,
  row: number,
  seconds: number,
  amplitude = 3,
): number {
  if (row < sea.top || row > sea.bottom) return 0;
  const reach = row < sea.swell ? 0.25 : 0.35 + depthOf(sea, row) * 0.65;
  const phase = row * 0.37;
  const motion =
    Math.sin(seconds * 0.9 - phase) * 0.7 +
    Math.sin(seconds * 0.43 + phase * 0.6 + 1.3) * 0.3;
  return Math.round(motion * amplitude * reach);
}

/**
 * The plate row drawn at `row` at `seconds`: the row itself, or one a few
 * rows away along a swell travelling toward the viewer, clamped to the sea.
 */
export function waveSource(
  sea: Sea,
  row: number,
  seconds: number,
  amplitude = 2,
): number {
  if (row < sea.swell || row > sea.bottom) return row;
  const depth = depthOf(sea, row);
  const rise = Math.round(
    Math.sin(depth * 9 - seconds * 1.2) * depth * amplitude,
  );
  return Math.min(sea.bottom, Math.max(sea.swell, row + rise));
}

/**
 * Draws the plate's water rows at their offsets and sources for `seconds`.
 * Each row is copied from the plate with a whole-art-pixel shift; a row that
 * shifts is padded from its own opposite end, so the sea never shows a gap.
 * Rows left of `shore` are held still, keeping the painted cliff foot fixed.
 */
export function drawSea(
  context: CanvasRenderingContext2D,
  image: CanvasImageSource,
  plate: { width: number },
  sea: Sea,
  camera: { scale: number; x: number; y: number },
  seconds: number,
  shore: (row: number) => number,
) {
  for (let row = sea.top; row <= sea.bottom; row++) {
    const shift = waveOffset(sea, row, seconds);
    const source = waveSource(sea, row, seconds);
    if (shift === 0 && source === row) continue;
    drawShiftedRow(
      context,
      image,
      plate.width,
      camera,
      row,
      source,
      shift,
      shore(row),
    );
  }
}
