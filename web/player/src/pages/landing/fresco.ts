import descentArt from "../../assets/landing/descent.png";
import seabedArt from "../../assets/landing/seabed.png";
import { createAbyss } from "./abyss";
import {
  ambient,
  burst,
  BURST_SECONDS,
  drawBubbles,
  type Bubble,
} from "./bubbles";
import { frame, type Camera, type Framing, type Plate } from "./camera";
import { fadeEdge, type Edge } from "./dither";
import { FRAME_SECONDS } from "./motion";
import { loadImage, observeDeviceSize, type DeviceSize } from "./pixel-canvas";
import { drawSurface, SKY, SUN_X, SURFACE, SURFACE_ART } from "./surface";

// The fresco's plates share one 480-art-pixel-wide grid and one sun column.
// The surface is framed first; every lower canvas reuses its scale and
// horizontal offset, so their columns line up down the whole page.
// Coordinates are art pixels within each plate; see the assets' README.
const WIDTH = SURFACE.width;

export const PLATES = {
  surface: SURFACE,
  descent: { width: WIDTH, height: 98, anchor: { x: SUN_X, y: 0 } },
  seabed: { width: WIDTH, height: 128, anchor: { x: SUN_X, y: 128 } },
} satisfies Record<string, Plate>;

type PlateName = keyof typeof PLATES;
export type LayerName = PlateName | "abyss";

/** Each layer's edge color, filling a view beyond its plate. */
const FILL: Record<LayerName, string> = {
  surface: SKY,
  descent: "#012d53",
  abyss: "#012d53",
  seabed: "#012d53",
};

/**
 * The canvas edge where each plate meets the abyss. Both plates paint a fade
 * into deep water there, but a canvas shorter than its plate crops it, most
 * on views wide enough to raise the scale, and the art would stop at a
 * straight line. So the last `EDGE_FADE` art rows at that edge always
 * dissolve into deep water; over a whole painted fade they change little.
 */
const MEETS_ABYSS = { descent: "bottom", seabed: "top" } as const satisfies {
  [name in PlateName]?: Edge;
};
type MeetsAbyss = keyof typeof MEETS_ABYSS;
const EDGE_FADE = 28;

/** Preferred on-screen size of one art pixel, in CSS pixels. */
const PIXEL = 4;
/** Views narrower than this, in CSS pixels, center the sun below the text.
 * It matches the page's `md` breakpoint, where the hero layout changes. */
const NARROW = 768;
const BUBBLE = "#bcd8af";

/** Where the hero text sits over the surface canvas, in CSS pixels. */
export interface TextBox {
  /** Distance from the canvas top to the text's bottom edge. */
  bottom: number;
  /** Distance from the canvas left to the text's right edge. */
  right: number;
}

/**
 * The surface framing. The plate's bottom row sits on the canvas's bottom
 * edge, so the water continues into the descent below, and the sun stays
 * clear of the hero text: beside it on wide views, below it on narrow ones.
 * Wide views put the sun at 72% across; narrow ones near the middle.
 * `below` asks for the narrow framing on a wide view, when the text leaves
 * no room beside it.
 */
export function surfaceFraming(
  text: TextBox,
  size: DeviceSize,
  below = false,
): Framing {
  const narrow = below || size.width / size.ratio < NARROW;
  return {
    ratio: size.ratio,
    pixel: PIXEL,
    focus: { x: narrow ? 0.52 : 0.72, y: 0.8 },
    padding: 12,
    safe: narrow ? { top: text.bottom } : { left: text.right },
    pinBottom: true,
    openTop: true,
  };
}

/**
 * A lower layer at the surface's scale and horizontal offset. The descent
 * and abyss hang from their canvas's top. The seabed rests on its canvas's
 * bottom, so its sand ends with the page, and its deep-water top rows meet
 * the abyss above; a canvas taller than the plate shows deep water above it.
 * A canvas shorter than its plate crops the edge that meets the abyss; see
 * `MEETS_ABYSS`.
 * All canvases are the page's full width, so they share offsets.
 */
export function follow(
  surface: Camera,
  name: Exclude<LayerName, "surface">,
  height: number,
): Camera {
  const y =
    name === "seabed" ? height - surface.scale * PLATES.seabed.height : 0;
  return { scale: surface.scale, x: surface.x, y };
}

export type FrescoCanvases = Record<LayerName, HTMLCanvasElement>;

export interface Fresco {
  /** Which layers are on screen; only those animate. */
  setVisible(name: LayerName, visible: boolean): void;
  /** Whether the clock runs at all: false when hidden or reduced motion. */
  setRunning(running: boolean): void;
  /** Starts the one-time dive burst, if it has not played yet. */
  dive(): void;
  dispose(): void;
}

/**
 * Bubbles for a plate at `seconds`, centered on the page rather than on the
 * sun: `middle` is the art column at the view's center and `span` the number
 * of art columns visible. Descent bubbles rise through the whole water column
 * and the dive burst fans out from the middle; seabed bubbles rise from the
 * ruins on both sides.
 */
function bubblesFor(
  name: PlateName,
  seconds: number,
  diveAge: number | null,
  middle: number,
  span: number,
): Bubble[] {
  const width = Math.min(span, WIDTH);
  if (name === "descent")
    return [
      ...ambient(
        {
          x: middle,
          spread: Math.round(width * 0.7),
          bottom: 96,
          top: 4,
          count: Math.max(6, Math.round(width / 22)),
          rise: 9,
        },
        seconds,
        1,
      ),
      ...(diveAge === null
        ? []
        : burst({ x: middle, y: 30 }, diveAge, 34, Math.round(width * 0.55))),
    ];
  if (name === "seabed")
    return [-1, 1].flatMap((side, i) =>
      ambient(
        {
          x: middle + side * Math.round(width * 0.3),
          spread: Math.round(width * 0.14),
          bottom: 78,
          top: 0,
          count: 4,
          rise: 7 + i * 0.5,
        },
        seconds,
        7 + i * 2,
      ),
    );
  return [];
}

/**
 * Draws the fresco's layers, each in its own canvas in the page flow, and
 * animates the ones on screen: waves on the surface, bubbles below it, and
 * the abyss behind the product sections. The clock only advances while
 * running; a stopped fresco is a complete still.
 */
export async function createFresco(
  canvases: FrescoCanvases,
  text: () => TextBox,
  textColumn: () => number,
): Promise<Fresco> {
  const images = await Promise.all([
    loadImage(SURFACE_ART),
    loadImage(descentArt),
    loadImage(seabedArt),
  ]).then(([surface, descent, seabed]) => ({ surface, descent, seabed }));
  const names = Object.keys(canvases) as LayerName[];
  const sizes: Partial<Record<LayerName, DeviceSize>> = {};
  const cameras: Partial<Record<LayerName, Camera | null>> = {};
  const visible = new Set<LayerName>();
  let seconds = 0;
  let diveStart: number | null = null;
  let dived = false;
  let running = false;
  let disposed = false;
  let request = 0;
  let previous = 0;
  let sinceDraw = 0;
  let abyss: ReturnType<typeof createAbyss> | null = null;
  let fades: Partial<Record<LayerName, ReturnType<typeof fadeEdge>>> = {};

  function draw(name: LayerName) {
    const canvas = canvases[name];
    const camera = cameras[name];
    const size = sizes[name];
    const context = canvas.getContext("2d", { alpha: false })!;
    context.imageSmoothingEnabled = false;
    if (!camera || !size) {
      context.fillStyle = FILL[name];
      context.fillRect(0, 0, canvas.width, canvas.height);
      return;
    }
    if (name === "abyss") {
      abyss?.draw(context, seconds);
      return;
    }
    context.fillStyle = FILL[name];
    context.fillRect(0, 0, canvas.width, canvas.height);
    if (name === "surface") {
      drawSurface(context, images.surface, camera, seconds);
      return;
    }
    const plate = PLATES[name];
    context.drawImage(
      images[name],
      camera.x,
      camera.y,
      plate.width * camera.scale,
      plate.height * camera.scale,
    );
    const age = diveStart === null ? null : seconds - diveStart;
    const span = size.width / camera.scale;
    const middle = Math.round((size.width / 2 - camera.x) / camera.scale);
    drawBubbles(
      context,
      bubblesFor(name, seconds, age, middle, Math.round(span)),
      camera,
      BUBBLE,
    );
    // Over the bubbles too, so they dissolve with the art near the edge.
    fades[name]?.(context);
  }

  /** Frames every layer from the current sizes, then redraws them all. */
  function layout() {
    const size = sizes.surface;
    // Beside the text when there is room, else below it.
    const surface = size
      ? (frame(size, PLATES.surface, surfaceFraming(text(), size)) ??
        frame(size, PLATES.surface, surfaceFraming(text(), size, true)))
      : null;
    cameras.surface = surface;
    for (const name of names) {
      if (name === "surface") continue;
      const height = sizes[name]?.height;
      cameras[name] =
        surface && height !== undefined ? follow(surface, name, height) : null;
    }
    const deep = cameras.abyss;
    abyss =
      deep && sizes.abyss
        ? createAbyss({
            camera: deep,
            size: sizes.abyss,
            sunX: SUN_X,
            // The column is measured in CSS pixels; the abyss works in art pixels.
            textWidth: (textColumn() * sizes.abyss.ratio) / deep.scale,
          })
        : null;
    fades = {};
    for (const plate of Object.keys(MEETS_ABYSS) as MeetsAbyss[]) {
      const camera = cameras[plate];
      const view = sizes[plate];
      if (camera && view)
        fades[plate] = fadeEdge(
          camera,
          view,
          MEETS_ABYSS[plate],
          EDGE_FADE,
          FILL[plate],
        );
    }
    for (const name of names) {
      const camera = cameras[name];
      Object.assign(canvases[name].dataset, {
        scale: camera?.scale ?? "none",
        x: camera?.x ?? "",
        y: camera?.y ?? "",
        ratio: sizes[name]?.ratio.toFixed(3) ?? "",
      });
      draw(name);
    }
  }

  const stops = names.map((name) =>
    observeDeviceSize(canvases[name], (size) => {
      sizes[name] = size;
      layout();
    }),
  );
  // The hero text can change size when its font loads without resizing the
  // canvas, so frame the sun again then.
  void document.fonts.ready.then(() => {
    if (!disposed) layout();
  });
  for (const canvas of Object.values(canvases)) canvas.dataset.ready = "true";

  function tick(now: number) {
    if (!running || disposed) return;
    const step = previous ? Math.min((now - previous) / 1000, 0.1) : 0;
    previous = now;
    seconds += step;
    sinceDraw += step;
    if (diveStart !== null && seconds - diveStart >= BURST_SECONDS)
      diveStart = null;
    if (sinceDraw >= FRAME_SECONDS) {
      sinceDraw %= FRAME_SECONDS;
      for (const name of visible) draw(name);
      canvases.surface.dataset.sceneTime = seconds.toFixed(2);
    }
    request = requestAnimationFrame(tick);
  }

  function sync() {
    const next = running && visible.size > 0 && !disposed;
    const active = request !== 0;
    for (const name of names)
      canvases[name].dataset.motion =
        next && visible.has(name) ? "playing" : "still";
    if (next && !active) {
      previous = 0;
      request = requestAnimationFrame(tick);
    } else if (!next && active) {
      cancelAnimationFrame(request);
      request = 0;
    }
  }

  return {
    setVisible(name, on) {
      if (on) visible.add(name);
      else visible.delete(name);
      sync();
    },
    setRunning(next) {
      running = next;
      sync();
    },
    dive() {
      if (dived) return;
      dived = true;
      diveStart = seconds;
    },
    dispose() {
      disposed = true;
      running = false;
      cancelAnimationFrame(request);
      request = 0;
      for (const stop of stops) stop();
    },
  };
}
