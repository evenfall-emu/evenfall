// Frames a pixel-art plate in a canvas at a whole-number scale, so each art
// pixel covers exactly `scale × scale` device pixels. View sizes and offsets
// are device pixels; plate coordinates are art pixels; `Framing` lengths are
// CSS pixels, converted with the measured ratio.

export interface Rect {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

export interface Plate {
  width: number;
  height: number;
  /** A region that must stay fully visible, with padding, when present. */
  keep?: Rect;
  /** The art point placed at `focus` in the view, as nearly as allowed. */
  anchor: { x: number; y: number };
}

export interface Framing {
  /** Device pixels per CSS pixel, measured for this canvas. */
  ratio: number;
  /** Preferred on-screen size of one art pixel, in CSS pixels. */
  pixel: number;
  /** Where the anchor should land, as fractions of the view. */
  focus: { x: number; y: number };
  /** Minimum room around `keep`, in CSS pixels. */
  padding?: number;
  /** Insets of the view that `keep` must avoid, such as text, in CSS pixels. */
  safe?: Partial<Rect>;
  /** Align the plate's bottom row with the view's bottom edge exactly. */
  pinBottom?: boolean;
  /**
   * The plate's top row continues upward as flat color, so the plate need
   * not reach the view's top edge; the caller fills the gap with that color.
   */
  openTop?: boolean;
}

export interface Camera {
  scale: number;
  x: number;
  y: number;
}

/** Integer offsets that keep [0, view] covered (only its far end when
 * `openStart`) and, when given, the art span [low, high] inside
 * [start, end], at `scale`; null when none exist. */
function offsets(
  view: number,
  size: number,
  scale: number,
  keep?: { low: number; high: number; start: number; end: number },
  openStart = false,
): [number, number] | null {
  let first = view - scale * size;
  let last = openStart ? Infinity : 0;
  if (keep) {
    first = Math.max(first, keep.start - scale * keep.low);
    last = Math.min(last, keep.end - scale * keep.high);
  }
  first = Math.ceil(first);
  last = Math.floor(last);
  return first <= last ? [first, last] : null;
}

const clamp = (value: number, [low, high]: [number, number]) =>
  Math.min(high, Math.max(low, value));

/**
 * The integer scale nearest `pixel` CSS pixels per art pixel, and the offsets
 * nearest `focus`, such that the plate covers the view and `keep` stays
 * inside the view minus `safe` and `padding`. Scales are tried outward from
 * the preferred one, up to twice the larger of it and the covering scale.
 * Returns null when none works.
 */
export function frame(
  view: { width: number; height: number },
  plate: Plate,
  framing: Framing,
): Camera | null {
  const { ratio, pixel, focus, padding = 0, safe = {} } = framing;
  const { pinBottom = false, openTop = false } = framing;
  const preferred = Math.max(1, Math.round(pixel * ratio));
  const cover = Math.max(
    1,
    Math.ceil(view.width / plate.width),
    openTop ? 1 : Math.ceil(view.height / plate.height),
  );
  const limit = Math.max(preferred, cover) * 2;
  const scales: number[] = [];
  for (let scale = cover; scale <= limit; scale++) scales.push(scale);
  scales.sort(
    (a, b) => Math.abs(a - preferred) - Math.abs(b - preferred) || a - b,
  );
  const device = (css = 0) => Math.round(css * ratio);
  const pad = device(padding);
  const { keep } = plate;
  for (const scale of scales) {
    const xs = offsets(
      view.width,
      plate.width,
      scale,
      keep && {
        low: keep.left,
        high: keep.right,
        start: device(safe.left) + pad,
        end: view.width - device(safe.right) - pad,
      },
    );
    let ys = offsets(
      view.height,
      plate.height,
      scale,
      keep && {
        low: keep.top,
        high: keep.bottom,
        start: device(safe.top) + pad,
        end: view.height - device(safe.bottom) - pad,
      },
      openTop,
    );
    if (ys && pinBottom) {
      const bottom = view.height - scale * plate.height;
      ys = ys[0] <= bottom && bottom <= ys[1] ? [bottom, bottom] : null;
    }
    if (!xs || !ys) continue;
    return {
      scale,
      x: clamp(Math.round(focus.x * view.width - scale * plate.anchor.x), xs),
      y: clamp(Math.round(focus.y * view.height - scale * plate.anchor.y), ys),
    };
  }
  return null;
}
