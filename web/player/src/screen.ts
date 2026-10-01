import { Presenter, type Preset } from "@evenfall/present";
import { draw } from "./media";
import { observeDeviceSize } from "./pages/landing/pixel-canvas";
import { SCREEN_KEY } from "./storage-names";

export interface ScreenSettings {
  preset: Preset;
  integerScale: boolean;
}

/**
 * How the game screen draws: `webgpu` through the shared presenter at the
 * canvas's device size, `canvas` through Canvas 2D at 240x160, scaled by CSS
 * with the Pixel look only. `starting` lasts until WebGPU answers.
 */
export type ScreenRenderer = "starting" | "webgpu" | "canvas";

export const DEFAULT_SCREEN: ScreenSettings = {
  preset: "pixel",
  integerScale: false,
};

export function loadScreenSettings(): ScreenSettings {
  try {
    const stored = JSON.parse(localStorage.getItem(SCREEN_KEY) ?? "null");
    return {
      preset:
        stored?.preset === "vanilla-plus"
          ? "vanilla-plus"
          : DEFAULT_SCREEN.preset,
      integerScale: stored?.integerScale === true,
    };
  } catch {
    return DEFAULT_SCREEN;
  }
}

export function storeScreenSettings(settings: ScreenSettings) {
  try {
    localStorage.setItem(SCREEN_KEY, JSON.stringify(settings));
  } catch {
    // The setting still applies until the page closes.
  }
}

/** Set once a WebGPU device fails, so later screens go straight to 2D. */
let webgpuFailed = false;

/**
 * Shows emulator frames in one canvas. It tries the WebGPU presenter first
 * and falls back to Canvas 2D when the browser has none.
 *
 * A canvas can hold only one kind of context, so frames that arrive before
 * WebGPU answers wait, newest only. If the device fails during play,
 * `onBroken` asks for a fresh canvas, which then draws in 2D.
 */
export class GameScreen {
  renderer: ScreenRenderer = "starting";
  private presenter?: Presenter;
  private waiting?: Uint16Array;
  private stopObserving?: () => void;
  private disposed = false;

  constructor(
    private readonly canvas: HTMLCanvasElement,
    private settings: ScreenSettings,
    private readonly onRenderer: (renderer: ScreenRenderer) => void,
    private readonly onBroken: () => void,
  ) {
    void this.start();
  }

  private async start() {
    const presenter = webgpuFailed
      ? null
      : await Presenter.create(this.canvas).catch(() => null);
    if (this.disposed) {
      presenter?.destroy();
      return;
    }
    if (!presenter) {
      this.useCanvas();
      return;
    }
    this.presenter = presenter;
    this.renderer = "webgpu";
    this.attempt(() => {
      presenter.configure(this.settings);
      if (this.waiting) presenter.setFrame(this.waiting);
      presenter.draw();
    });
    this.waiting = undefined;
    // A first draw that failed has already asked for a fresh canvas.
    if (this.disposed) return;
    this.stopObserving = observeDeviceSize(this.canvas, ({ width, height }) =>
      this.attempt(() => {
        presenter.resize(width, height);
        presenter.draw();
      }),
    );
    this.onRenderer(this.renderer);
  }

  private useCanvas() {
    this.renderer = "canvas";
    if (this.waiting) this.drawCanvas(this.waiting);
    this.waiting = undefined;
    this.onRenderer(this.renderer);
  }

  private drawCanvas(pixels: Uint16Array) {
    // A canvas that already holds a WebGPU context has no 2D one.
    if (this.canvas.getContext("2d", { alpha: false }))
      draw(this.canvas, pixels);
    else this.fail();
  }

  /** Shows a frame. The caller may reuse `pixels` once this returns. */
  show(pixels: Uint16Array) {
    if (this.renderer === "starting") {
      (this.waiting ??= new Uint16Array(pixels.length)).set(pixels);
    } else if (this.renderer === "canvas") {
      this.drawCanvas(pixels);
    } else {
      const presenter = this.presenter!;
      this.attempt(() => {
        presenter.setFrame(pixels);
        presenter.draw();
      });
    }
  }

  configure(settings: ScreenSettings) {
    this.settings = settings;
    const presenter = this.presenter;
    if (presenter)
      this.attempt(() => {
        presenter.configure(settings);
        presenter.draw();
      });
  }

  /** What the WebGPU screen shows, as RGBA bytes; undefined in 2D. */
  capture() {
    return this.presenter?.capture();
  }

  dispose() {
    this.disposed = true;
    this.stopObserving?.();
    this.presenter?.destroy();
    this.presenter = undefined;
  }

  private attempt(work: () => void) {
    if (this.disposed) return;
    try {
      work();
    } catch {
      this.fail();
    }
  }

  private fail() {
    webgpuFailed = true;
    this.dispose();
    this.onBroken();
  }
}
