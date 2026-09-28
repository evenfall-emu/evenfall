// On-screen controls: which GBA keys a touch presses, and where each control
// sits. Pure, so it can be tested without a player.

/** KEYINPUT bits, as `player.touch` expects them. */
export const KEY = {
  a: 1,
  b: 2,
  select: 4,
  start: 8,
  right: 16,
  left: 32,
  up: 64,
  down: 128,
  r: 256,
  l: 512,
} as const;

/** Controls that hold a GBA key while touched; a finger can slide between them. */
export const gameButtons = ["a", "b", "l", "r", "select", "start"] as const;
export type GameButton = (typeof gameButtons)[number];

/**
 * Every on-screen control. `fast` fast-forwards while held; `save`, `load`
 * and `rewind` act on a tap (quick slot 1 and the rewind timeline).
 */
export type ControlId =
  | "dpad"
  | GameButton
  | "fast"
  | "save"
  | "load"
  | "rewind";

export function isGameButton(
  control: string | undefined,
): control is GameButton {
  return (gameButtons as readonly string[]).includes(control ?? "");
}

// tan 60°: each direction owns 60° around its axis, leaving diagonals 30°, so
// a thumb that drifts a little off an axis does not press two directions.
const CARDINAL = Math.sqrt(3);

/**
 * The directions a touch at (dx, dy) from the pad's centre presses, with y
 * pointing down. Touches within a fifth of `radius` of the centre press
 * nothing.
 */
export function dpadKeys(dx: number, dy: number, radius: number) {
  if (Math.hypot(dx, dy) < radius * 0.2) return 0;
  const x = Math.abs(dx);
  const y = Math.abs(dy);
  return (
    (y < x * CARDINAL ? (dx > 0 ? KEY.right : KEY.left) : 0) |
    (x < y * CARDINAL ? (dy > 0 ? KEY.down : KEY.up) : 0)
  );
}

/** A control's centre, in percent of the control surface's width and height. */
export interface ControlPlacement {
  readonly x: number;
  readonly y: number;
}
export type ControlLayout = Readonly<Record<ControlId, ControlPlacement>>;

/**
 * Default placements for 20:9-class phones. Portrait controls fill the area
 * under the screen; landscape controls cover the whole page and sit in the
 * gutters beside the screen. Sizes come from the component, scaled to the
 * surface, so these stay usable on neighbouring aspect ratios.
 */
export const defaultControlLayouts = {
  portrait: {
    l: { x: 15, y: 8 },
    save: { x: 34, y: 8 },
    load: { x: 44.7, y: 8 },
    rewind: { x: 55.3, y: 8 },
    fast: { x: 66, y: 8 },
    r: { x: 85, y: 8 },
    dpad: { x: 27, y: 50 },
    b: { x: 64, y: 58 },
    a: { x: 85, y: 45 },
    select: { x: 39, y: 88 },
    start: { x: 61, y: 88 },
  },
  landscape: {
    rewind: { x: 81.5, y: 7 },
    fast: { x: 86.5, y: 7 },
    l: { x: 10, y: 24 },
    r: { x: 90, y: 24 },
    dpad: { x: 10.5, y: 61 },
    b: { x: 83.5, y: 66 },
    a: { x: 93.5, y: 50 },
    select: { x: 6, y: 92 },
    save: { x: 15.5, y: 92 },
    load: { x: 84.5, y: 92 },
    start: { x: 94, y: 92 },
  },
} as const satisfies Record<"portrait" | "landscape", ControlLayout>;
