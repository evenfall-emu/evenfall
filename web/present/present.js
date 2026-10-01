// The WebGPU presenter's module is loaded on first use, and only by browsers
// that expose WebGPU, so other browsers never download it.
let glue;
function load() {
  if (!glue) {
    glue = import("./pkg/evenfall_present_web.js").then(async (module) => {
      await module.default();
      return module;
    });
    // A failed download may succeed on a later attempt.
    glue.catch(() => {
      glue = undefined;
    });
  }
  return glue;
}

export const PRESETS = Object.freeze(["pixel", "vanilla-plus"]);

export class Presenter {
  #inner;

  /** @private Use `Presenter.create`. */
  constructor(inner) {
    this.#inner = inner;
  }

  static async create(canvas, { sourceWidth = 240, sourceHeight = 160 } = {}) {
    if (typeof navigator === "undefined" || !navigator.gpu) return null;
    try {
      const module = await load();
      const inner = await module.Presenter.create(
        canvas,
        sourceWidth,
        sourceHeight,
        canvas.width,
        canvas.height,
      );
      return new Presenter(inner);
    } catch {
      return null;
    }
  }

  setFrame(pixels) {
    this.#live().setFrame(pixels);
  }

  configure({ preset = "pixel", integerScale = false } = {}) {
    this.#live().configure(preset, integerScale);
  }

  resize(width, height) {
    this.#live().resize(width, height);
  }

  draw() {
    return this.#live().draw();
  }

  capture() {
    return this.#live().capture();
  }

  destroy() {
    this.#inner?.destroy();
    this.#inner = undefined;
  }

  #live() {
    if (!this.#inner) throw new Error("The presenter was destroyed.");
    return this.#inner;
  }
}
