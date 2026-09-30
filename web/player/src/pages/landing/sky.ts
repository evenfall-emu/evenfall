// Sky motion for the surface plate, carried over from the original Home's
// scene and kept on the art grid:
//
// - Clouds: the painted cloud rows drift slowly sideways over a two-minute
//   cycle, whole art pixels at a time, strongest in the middle of the cloud
//   band and still at its edges, so the sun, cliffs and horizon never move.
// - Beacon: the lighthouse lamp pulses by swapping its painted colors for
//   lighter ones in whole steps, not by blending.

type Camera = { scale: number; x: number; y: number };

export interface Clouds {
  /** Art rows the drift applies to; it tapers to nothing at both ends. */
  top: number;
  bottom: number;
  /** The first column that drifts in a row; the cliff left of it stays. */
  left: (row: number) => number;
  /** The largest drift, in art pixels. */
  reach: number;
  /** Seconds for one full drift cycle. */
  cycle: number;
}

/** The sideways drift of one cloud row at `seconds`, in whole art pixels. */
export function cloudShift(clouds: Clouds, row: number, seconds: number) {
  if (row < clouds.top || row >= clouds.bottom) return 0;
  const span = clouds.bottom - clouds.top;
  const edge = Math.min(
    1,
    (row - clouds.top) / (span * 0.25),
    (clouds.bottom - row) / (span * 0.3),
  );
  const weight = edge * edge * (3 - 2 * edge);
  return Math.round(
    Math.sin((seconds * 2 * Math.PI) / clouds.cycle) * clouds.reach * weight,
  );
}

/**
 * Copies one plate row to itself shifted by `shift` whole art pixels within
 * columns [start, width), wrapping, from source row `source`.
 */
export function drawShiftedRow(
  context: CanvasRenderingContext2D,
  image: CanvasImageSource,
  plateWidth: number,
  camera: Camera,
  row: number,
  source: number,
  shift: number,
  start: number,
) {
  const { scale: k, x, y } = camera;
  const width = plateWidth - start;
  const s = ((shift % width) + width) % width;
  const dy = y + row * k;
  context.drawImage(
    image,
    start,
    source,
    width - s,
    1,
    x + (start + s) * k,
    dy,
    (width - s) * k,
    k,
  );
  if (s > 0)
    context.drawImage(
      image,
      start + width - s,
      source,
      s,
      1,
      x + start * k,
      dy,
      s * k,
      k,
    );
}

/** Draws the cloud rows at their drift for `seconds`. */
export function drawClouds(
  context: CanvasRenderingContext2D,
  image: CanvasImageSource,
  plate: { width: number },
  clouds: Clouds,
  camera: Camera,
  seconds: number,
) {
  for (let row = clouds.top; row < clouds.bottom; row++) {
    const shift = cloudShift(clouds, row, seconds);
    if (shift !== 0)
      drawShiftedRow(
        context,
        image,
        plate.width,
        camera,
        row,
        row,
        shift,
        clouds.left(row),
      );
  }
}

/** A point light: art pixels and the colors they step through as it pulses. */
export interface Glow {
  pixels: readonly (readonly [number, number])[];
  /** Colors through one pulse; the first is the painted color, left as is. */
  ramp: readonly string[];
  /** Seconds per pulse, and an offset into it as a fraction. */
  period: number;
  phase?: number;
}

/** The ramp step a light shows at `seconds`: 0 at rest, rising and falling. */
export function glowStep(glow: Glow, seconds: number) {
  const cycle = (seconds / glow.period + (glow.phase ?? 0)) % 1;
  const wave = (1 - Math.cos(cycle * 2 * Math.PI)) / 2;
  return Math.min(glow.ramp.length - 1, Math.floor(wave * glow.ramp.length));
}

/** Draws a pulsing light at `seconds`. */
export function drawGlow(
  context: CanvasRenderingContext2D,
  glow: Glow,
  camera: Camera,
  seconds: number,
) {
  const step = glowStep(glow, seconds);
  if (step === 0) return;
  context.fillStyle = glow.ramp[step];
  for (const [px, py] of glow.pixels)
    context.fillRect(
      camera.x + px * camera.scale,
      camera.y + py * camera.scale,
      camera.scale,
      camera.scale,
    );
}
