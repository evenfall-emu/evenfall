/** How source pixels become canvas pixels. */
export type Preset = "pixel" | "vanilla-plus";

/** Every preset, the default first. */
export const PRESETS: readonly Preset[];

export interface PresentSettings {
  /** `pixel` (nearest neighbour, the default) or `vanilla-plus` (sharp bilinear). */
  preset?: Preset;
  /** Scale only by whole multiples, leaving a black border. Default false. */
  integerScale?: boolean;
}

/** Draws emulated frames into one canvas through WebGPU. */
export class Presenter {
  private constructor();
  /**
   * Resolves to a presenter for `canvas`, or to null when this browser has no
   * usable WebGPU. The canvas gets its WebGPU context only once a device
   * exists, so after null it can usually still get a 2D context.
   * The source defaults to the GBA's 240x160.
   */
  static create(
    canvas: HTMLCanvasElement,
    source?: { sourceWidth?: number; sourceHeight?: number },
  ): Promise<Presenter | null>;
  /** Uploads a completed frame: row-major, one 15-bit BGR pixel per entry. */
  setFrame(pixels: Uint16Array): void;
  configure(settings: PresentSettings): void;
  /** Follows the canvas's drawing buffer, in device pixels. */
  resize(width: number, height: number): void;
  /**
   * Draws the last frame. Returns false when the canvas had no texture this
   * time, and throws once the device is lost or reported an error.
   */
  draw(): boolean;
  /**
   * Draws the last frame as `draw` would, into an offscreen texture of the
   * canvas's size, and resolves to its pixels as RGBA bytes, row-major.
   * Browsers don't let pages read a WebGPU canvas back reliably, so this is
   * how a page or a test sees what the screen shows.
   */
  capture(): Promise<{ width: number; height: number; rgba: Uint8Array }>;
  /** Releases the device. The canvas keeps its WebGPU context. */
  destroy(): void;
}
