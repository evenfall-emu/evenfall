// Bubbles for the underwater scene. Every position is derived from the scene
// clock and the bubble's index, so there is no particle list to grow, and a
// stopped clock freezes every bubble in place.

/** Small authored sprites, one character per art pixel ("." is empty). */
const SPRITES = [["#"], [".#.", "#.#", ".#."], ["##", "##"]] as const;

export interface Bubble {
  x: number;
  y: number;
  sprite: readonly string[];
}

// A small deterministic hash, so bubble paths vary without randomness.
const hash = (n: number) => {
  let h = Math.imul(n ^ 0x9e3779b9, 0x85ebca6b);
  h ^= h >>> 13;
  h = Math.imul(h, 0xc2b2ae35);
  return ((h ^ (h >>> 16)) >>> 0) / 4294967296;
};

export interface Stream {
  /** Art-pixel column the stream rises around, and its full width. */
  x: number;
  spread: number;
  /** Art-pixel rows the bubbles rise between (bottom > top). */
  bottom: number;
  top: number;
  count: number;
  /** Seconds for one bubble to rise from bottom to top. */
  rise: number;
}

/** Ambient bubbles rising through a stream at `seconds`. */
export function ambient(
  stream: Stream,
  seconds: number,
  seed: number,
): Bubble[] {
  const bubbles: Bubble[] = [];
  const travel = stream.bottom - stream.top;
  for (let i = 0; i < stream.count; i++) {
    // Each bubble has its own speed, so a stream never moves in lockstep.
    const rise = stream.rise * (0.75 + hash(seed * 53 + i) * 0.5);
    const phase = (seconds / rise + hash(seed * 97 + i)) % 1;
    const y = stream.bottom - phase * travel;
    // A slow sideways wobble, whole pixels only.
    const wobble = Math.round(
      Math.sin((seconds + i * 1.7) * 1.9 + hash(seed + i) * 6),
    );
    const x =
      stream.x +
      Math.round((hash(seed * 31 + i) - 0.5) * stream.spread) +
      wobble;
    bubbles.push({
      x,
      y: Math.round(y),
      sprite: SPRITES[hash(seed * 7 + i) < 0.55 ? 0 : i % 2 ? 1 : 2],
    });
  }
  return bubbles;
}

export const BURST_SECONDS = 2.4;

/**
 * The one-time dive burst: bubbles fanning up across the water from under
 * `origin`, `age` seconds after it started. Empty once the burst is over.
 */
export function burst(
  origin: { x: number; y: number },
  age: number,
  count = 34,
  spread = 150,
): Bubble[] {
  if (age < 0 || age >= BURST_SECONDS) return [];
  const bubbles: Bubble[] = [];
  for (let i = 0; i < count; i++) {
    const delay = hash(i * 13 + 5) * 0.9;
    const t = age - delay;
    if (t < 0) continue;
    const speed = 18 + hash(i * 7 + 1) * 24;
    const drift = (hash(i * 3 + 2) - 0.5) * spread;
    const x = origin.x + Math.round(drift * Math.min(1, 0.35 + t * 0.9));
    const y = origin.y + Math.round(46 - t * speed);
    if (y < 0) continue;
    bubbles.push({ x, y, sprite: SPRITES[i % 3] });
  }
  return bubbles;
}

/** Draws bubbles at integer art-pixel positions, `scale` device pixels each. */
export function drawBubbles(
  context: CanvasRenderingContext2D,
  bubbles: Bubble[],
  camera: { scale: number; x: number; y: number },
  color: string,
) {
  context.fillStyle = color;
  for (const { x, y, sprite } of bubbles) {
    sprite.forEach((row, dy) => {
      for (let dx = 0; dx < row.length; dx++) {
        if (row[dx] !== "#") continue;
        context.fillRect(
          camera.x + (x + dx) * camera.scale,
          camera.y + (y + dy) * camera.scale,
          camera.scale,
          camera.scale,
        );
      }
    });
  }
}
