// The deep water behind the product sections, drawn procedurally on the
// fresco's grid. A still layer (rock walls along both sides and faint light
// shafts) is painted once per size at one device pixel per art pixel and then
// enlarged by the fresco's whole-number scale. Animated details are drawn on
// top each frame: marine snow, kelp swaying on the walls' ledges,
// bioluminescent specks, and bubbles rising beside the text column.
//
// The walls and shafts fade in at the top and out at the bottom through an
// ordered dither, so the abyss meets the descent above and the seabed below
// on flat deep water, without a seam.

import { ambient, drawBubbles } from "./bubbles";
import type { Camera } from "./camera";
import { shown } from "./dither";

const DEEP = "#012d53";
const SHAFT = "#01345a";
const SNOW = ["#0a546d", "#035777", "#0a546d", "#146884"];
const ROCK = "#011e3c";
const ROCK_EDGE = "#01274a";
const STEM = "#0a546d";
const LEAF = "#146884";
const GLOW = ["#60c5a1", "#146884"];
const BUBBLE = "#bcd8af";
/** Rows over which walls and shafts fade in at the top and out below. */
const FADE = 28;

// A small deterministic hash, so the scene varies without randomness.
const hash = (n: number) => {
  let h = Math.imul(n ^ 0x2545f491, 0x9e3779b1);
  h ^= h >>> 15;
  h = Math.imul(h, 0x85ebca77);
  return ((h ^ (h >>> 13)) >>> 0) / 4294967296;
};

export interface AbyssLayout {
  camera: Camera;
  /** The view in device pixels. */
  size: { width: number; height: number };
  /** The art column the light comes from. */
  sunX: number;
  /** Width of the centered text column, in art pixels. */
  textWidth: number;
}

interface Ledge {
  x: number;
  y: number;
  side: -1 | 1;
  height: number;
}

/**
 * The abyss for one layout: its still layer and the kelp ledges on it. Call
 * `draw` each frame with the scene clock.
 */
export function createAbyss({ camera, size, sunX, textWidth }: AbyssLayout) {
  const k = camera.scale;
  const left = Math.ceil(-camera.x / k);
  const right = Math.floor((size.width - camera.x) / k);
  const span = right - left;
  const rows = Math.ceil(size.height / k);
  const margin = Math.max(0, (span - textWidth) / 2);
  // Walls stay within the margins beside the text column.
  const wallMax = Math.max(4, Math.min(22, Math.floor(margin * 0.55)));
  const fade = (y: number) =>
    Math.max(0, Math.min(1, y / FADE, (rows - 1 - y) / FADE));

  const still = document.createElement("canvas");
  still.width = span + 1;
  still.height = rows;
  const paint = still.getContext("2d")!;
  paint.fillStyle = DEEP;
  paint.fillRect(0, 0, still.width, rows);

  // Faint light shafts from the sun's column, widening as they sink and
  // thinning out through the dither, fading in from the top.
  const shafts = [
    { x: sunX - 46, slope: -0.18, width: 7 },
    { x: sunX - 4, slope: 0.02, width: 11 },
    { x: sunX + 34, slope: 0.2, width: 6 },
  ];
  const reach = Math.round(rows * 0.7);
  paint.fillStyle = SHAFT;
  for (const shaft of shafts)
    for (let y = 0; y < reach; y++) {
      const amount = Math.min(y / FADE, 1 - y / reach) * 1.4;
      const w = Math.round(shaft.width + y * 0.03);
      const start = Math.round(shaft.x + y * shaft.slope - w / 2);
      for (let x = start; x < start + w; x++)
        if (shown(x, y, amount)) paint.fillRect(x - left, y, 1, 1);
    }

  // Stepped rock walls along both sides, their depth varying in blocks of
  // three rows, with a lighter rim on the inner edge.
  const ledges: Ledge[] = [];
  const wall = (y: number, side: number) => {
    const block = Math.floor(y / 3);
    const depth =
      0.55 +
      0.25 * Math.sin(block / 9 + side * 2) +
      0.2 * Math.sin(block / 3.1 + side) +
      (hash(block * 7 + side) - 0.5) * 0.25;
    return Math.max(1, Math.round(wallMax * depth));
  };
  for (let y = 0; y < rows; y++) {
    const amount = fade(y);
    for (const side of [-1, 1] as const) {
      const w = wall(y, side);
      for (let i = 0; i < w; i++) {
        const x = side < 0 ? left + i : right - i;
        if (!shown(x, y, amount)) continue;
        paint.fillStyle = i === w - 1 ? ROCK_EDGE : ROCK;
        paint.fillRect(x - left, y, 1, 1);
      }
      // A ledge where the wall juts out, away from the faded ends.
      if (
        amount === 1 &&
        y % 3 === 0 &&
        w > wall(y - 3, side) + 1 &&
        !ledges.some((l) => l.side === side && Math.abs(l.y - y) < 40)
      )
        ledges.push({
          x: side < 0 ? left + w - 1 : right - w + 1,
          y,
          side,
          height: 12 + Math.round(hash(y * 5 + side) * 16),
        });
    }
  }

  const glowCount = Math.round(rows / 45);
  const snowCount = Math.round((span * rows) / 320);

  function draw(context: CanvasRenderingContext2D, seconds: number) {
    context.imageSmoothingEnabled = false;
    context.drawImage(
      still,
      camera.x + left * k,
      camera.y,
      still.width * k,
      rows * k,
    );
    const dot = (x: number, y: number, color: string) => {
      context.fillStyle = color;
      context.fillRect(camera.x + x * k, camera.y + y * k, k, k);
    };

    // Marine snow: sparse specks sinking slowly across the whole view.
    for (let i = 0; i < snowCount; i++) {
      const speed = 0.6 + hash(i * 11) * 1.4;
      const y = Math.floor((hash(i * 7) * rows + seconds * speed) % rows);
      const x =
        left +
        Math.floor(hash(i * 3) * span) +
        Math.round(Math.sin(seconds * 0.3 + i) * 1.5);
      dot(x, y, SNOW[i % SNOW.length]);
    }

    // Kelp growing up from each ledge, swaying more toward its tip.
    ledges.forEach((ledge, i) => {
      for (let r = 1; r <= ledge.height; r++) {
        const tip = (r / ledge.height) ** 1.4;
        const sway = Math.round(
          Math.sin(seconds * 0.8 + r * 0.14 + i * 1.3) * tip * 2.5,
        );
        const x = ledge.x + ledge.side * -1 + sway;
        dot(x, ledge.y - r, STEM);
        if (r > 2 && r % 4 === 0)
          dot(x + (r % 8 === 0 ? 1 : -1), ledge.y - r, LEAF);
      }
    });

    // Bioluminescent specks near the walls, each on its own slow pulse.
    for (let i = 0; i < glowCount; i++) {
      const side = i % 2 ? -1 : 1;
      const offset = wallMax + 2 + Math.floor(hash(i * 5) * 6);
      const x = side < 0 ? left + offset : right - offset;
      const y = Math.round(FADE + hash(i * 9) * (rows - 2 * FADE));
      const phase = (seconds / (4 + hash(i * 13) * 3) + hash(i * 19)) % 1;
      if (phase < 0.1) dot(x, y, GLOW[0]);
      else if (phase < 0.22) dot(x, y, GLOW[1]);
    }

    // Bubbles rise in the margins, when the text column leaves room.
    if (margin > wallMax + 8) {
      const middle = (wallMax + margin) / 2;
      const streams = [left + middle, right - middle].map((x, i) =>
        ambient(
          {
            x: Math.round(x),
            spread: Math.round((margin - wallMax) * 0.6),
            bottom: rows - FADE,
            top: FADE,
            count: Math.max(3, Math.round(rows / 70)),
            rise: rows / 36,
          },
          seconds,
          20 + i,
        ),
      );
      drawBubbles(context, streams.flat(), camera, BUBBLE);
    }
  }

  return { draw };
}
