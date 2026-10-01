# @evenfall/present

Draws Evenfall's emulated frames into a canvas through WebGPU, with the aspect-correct scaling presets of [`crates/present`](../../crates/present/src/lib.rs).
The module loads only in browsers that offer WebGPU, and `Presenter.create` resolves to null elsewhere, leaving the canvas free for a 2D context.

```js
import { Presenter } from "@evenfall/present";

const presenter = await Presenter.create(canvas); // a 240x160 source by default
if (presenter) {
  presenter.configure({ preset: "vanilla-plus", integerScale: false });
  presenter.resize(canvas.width, canvas.height); // the drawing buffer, in device pixels
  presenter.setFrame(pixels); // Uint16Array, 240x160, 15-bit BGR
  presenter.draw();
}
```

Build the module with `npm run build:wasm -w @evenfall/present`.
[`docs/design/presentation.md`](../../docs/design/presentation.md#in-the-browser) explains how it works and how it is checked.
