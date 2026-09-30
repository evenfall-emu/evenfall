import { frame, type Camera, type Framing, type Plate } from "./camera";
import { FRAME_SECONDS } from "./motion";
import { loadImage, observeDeviceSize, type DeviceSize } from "./pixel-canvas";
import { drawSurface, SEA, SKY, SURFACE, SURFACE_ART } from "./surface";

// The surface plate alone in one canvas: the Player's screen before a game
// is open. It moves like Home's surface, at the same whole-art-pixel steps.

/**
 * The surface down to its last row of open sea. The waterline below it only
 * leads into Home's descent, so a lone view ends on the sea instead.
 */
export const OPEN_SEA: Plate = { ...SURFACE, height: SEA.bottom + 1 };

/** Art columns a view aims to show; its width sets the preferred scale. */
const COLUMNS = 280;

/**
 * The framing for a view whose text ends `textBottom` CSS pixels below its
 * top: the sun centered across, whole and below the text, at a scale that
 * shows about `COLUMNS` art columns but never under two CSS pixels per art
 * pixel. The sky continues above the plate as flat color, and the sea may
 * run past the view's bottom.
 */
export function sunsetFraming(textBottom: number, size: DeviceSize): Framing {
  return {
    ratio: size.ratio,
    pixel: Math.max(2, Math.round(size.width / size.ratio / COLUMNS)),
    focus: { x: 0.5, y: 0.8 },
    padding: 12,
    safe: { top: textBottom },
    openTop: true,
  };
}

export interface Sunset {
  /** Frames the plate again, after the text above it changes size. */
  layout(): void;
  /** Whether the clock runs; a stopped scene is a complete still. */
  setRunning(running: boolean): void;
  dispose(): void;
}

/**
 * Draws the open sea into `canvas`, which stays sized to its device-pixel
 * box. `textBottom` reports where the text over the canvas ends, in CSS
 * pixels from the canvas top. When the text leaves no room below it, the
 * sun sits behind the text rather than the scene disappearing.
 */
export async function createSunset(
  canvas: HTMLCanvasElement,
  textBottom: () => number,
): Promise<Sunset> {
  const image = await loadImage(SURFACE_ART);
  let size: DeviceSize | null = null;
  let camera: Camera | null = null;
  let seconds = 0;
  let running = false;
  let disposed = false;
  let request = 0;
  let previous = 0;
  let sinceDraw = 0;

  function draw() {
    const context = canvas.getContext("2d", { alpha: false })!;
    context.imageSmoothingEnabled = false;
    context.fillStyle = SKY;
    context.fillRect(0, 0, canvas.width, canvas.height);
    if (camera) drawSurface(context, image, camera, seconds, OPEN_SEA.height);
  }

  function layout() {
    camera = size
      ? (frame(size, OPEN_SEA, sunsetFraming(textBottom(), size)) ??
        frame(size, OPEN_SEA, sunsetFraming(0, size)))
      : null;
    Object.assign(canvas.dataset, {
      scale: camera?.scale ?? "none",
      x: camera?.x ?? "",
      y: camera?.y ?? "",
      ratio: size?.ratio.toFixed(3) ?? "",
    });
    draw();
  }

  const stop = observeDeviceSize(canvas, (next) => {
    size = next;
    layout();
  });
  canvas.dataset.ready = "true";

  function tick(now: number) {
    if (!running || disposed) return;
    const step = previous ? Math.min((now - previous) / 1000, 0.1) : 0;
    previous = now;
    seconds += step;
    sinceDraw += step;
    if (sinceDraw >= FRAME_SECONDS) {
      sinceDraw %= FRAME_SECONDS;
      draw();
      canvas.dataset.sceneTime = seconds.toFixed(2);
    }
    request = requestAnimationFrame(tick);
  }

  return {
    layout,
    setRunning(next) {
      running = next && !disposed;
      canvas.dataset.motion = running ? "playing" : "still";
      if (running && request === 0) {
        previous = 0;
        request = requestAnimationFrame(tick);
      } else if (!running && request !== 0) {
        cancelAnimationFrame(request);
        request = 0;
      }
    },
    dispose() {
      disposed = true;
      running = false;
      cancelAnimationFrame(request);
      request = 0;
      stop();
    },
  };
}
