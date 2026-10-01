# Presentation

The [`evenfall-present`](../../crates/present/src/lib.rs) crate draws a system's completed frames into a GPU texture for graphical hosts.
It decodes the frame's pixel format, places the picture in the drawable area at the source's aspect ratio, and scales it with a preset.
It runs on [wgpu](https://wgpu.rs), so one implementation and one set of WGSL shaders serve Vulkan, Metal, DX12, GLES and WebGPU.

Presentation sits outside the deterministic core.
It only reads frames, so it can't change emulation, recordings or save states.
The crate doesn't depend on any system, and headless hosts such as the CLI don't depend on it.

[`Renderer`](../../crates/present/src/renderer.rs) owns the GPU pipeline and the last frame.
[`Viewport`](../../crates/present/src/viewport.rs) places the picture.
[`scale.wgsl`](../../crates/present/src/scale.wgsl) decodes and filters.

## How a frame is drawn

1. The host uploads a completed frame with `Renderer::set_frame`.
   The frame becomes a 16-bit integer texture at its native resolution, with no conversion on the CPU.
2. `Renderer::draw` places the viewport in the drawable area, clears the target to black, and draws one triangle over the viewport.
3. For each output pixel, the fragment shader finds the source pixels it covers, decodes them from BGR555 and filters them as the preset says.

The renderer keeps the last frame, so a host can redraw after a resize or a settings change without asking the emulator.
Hosts pass the drawable area in physical pixels, so HiDPI displays draw at full device resolution.
`draw` records into the host's command encoder, so a host can draw its own interface over the picture in the same submission.

## Placing the picture

`Viewport::fit` centres the picture at the source's aspect ratio, assuming square source pixels.
By default the picture fills the drawable area along its limiting axis.
Its other side is rounded to the nearest pixel, halves up.
With `integer_scale`, the picture is the largest whole multiple of the source that fits, and the rest of the area stays black.
An area smaller than the source can't hold one multiple, so integer scaling falls back to fitting there.
When a border has an odd number of pixels, the extra one goes on the right or at the bottom.

Integer scaling keeps every source pixel the same size with any preset.
Turn it on when exact pixel shapes matter more than filling the screen.

## Presets

- **`Pixel`** is nearest neighbour, and the default.
  Every output pixel shows the source pixel under its centre.
  At fractional scales some source pixels come out one output pixel wider or taller than their neighbours, which shows as uneven lines and as shimmer while scrolling.
- **`VanillaPlus`** is sharp bilinear.
  The picture is first enlarged with nearest neighbour by the largest whole multiple k that fits, then scaled bilinearly to the final size.
  Every source pixel keeps the same size.
  Blending stays within 1/k of a source pixel of each edge, which is at most two output pixels, where plain bilinear would blur across a whole source pixel.
  At whole multiples it draws exactly what `Pixel` draws.

Choose `Pixel` for the unfiltered picture.
Choose `VanillaPlus` when the drawable area isn't a whole multiple of the source, as in most windows and on phones.

Each 5-bit channel c decodes to c / 31 of full scale, which an 8-bit target stores as round(c * 255 / 31).
The web player's Canvas 2D path uses the same conversion, so `Pixel` shows the same colours.
No colour correction is applied: the picture shows the values the game wrote.

## Filtering in shader arithmetic

The shaders read texels with `textureLoad` and compute every filter weight themselves, instead of using the GPU's texture sampler.
Samplers compute weights at a precision each GPU chooses, and Vulkan requires only 4 bits of it.
The same sampled picture can therefore differ between GPUs by more than an 8-bit step.
Shader arithmetic runs in 32-bit floats everywhere and stays within one 8-bit step of the f64 definition.
Nearest neighbour uses exact integers instead: floor((p + 0.5) * size / out) is computed as (2p + 1) * size / (2 * out).
Source frames are small, so the extra texel reads cost little.

## Colour encoding

Frame values are already display-encoded, as sRGB image data is.
`Renderer::new` refuses sRGB target formats, which would encode them a second time and brighten the picture.
A host whose surface prefers an sRGB format draws through a view of its plain twin (`TextureFormat::remove_srgb_suffix`).
WebGPU canvases only offer plain formats, so browser and native output match.
Filters blend the encoded values, not linear light.

## Backends

The crate enables no wgpu backend.
Each host build enables the ones it ships: `webgpu` in a browser, Vulkan, Metal, DX12 or GLES natively.
Native builds translate the WGSL with wgpu's shader compiler, naga.
A WebGPU-only browser build leaves naga out and hands the WGSL to the browser's own compiler, which can accept or reject different code.

## In the browser

[`web/present`](../../web/present/README.md) is the browser host, the `@evenfall/present` package.
Its Rust half, `evenfall-present-web`, wraps `Renderer` around a canvas's WebGPU surface.
wgpu reaches WebGPU through wasm-bindgen, so this is a second WebAssembly module with JavaScript imports, next to the core's module, which has none.
It weighs about 112 KB, 39 KB compressed, plus 10 KB of compressed JavaScript glue.

- **Loading.** `Presenter.create` loads the module on first use, and only when `navigator.gpu` exists, so browsers without WebGPU never download it.
  It resolves to null when WebGPU has no adapter or device.
- **The canvas's context.** A canvas holds one kind of context for good.
  `create` asks for the canvas's WebGPU context only once a device exists, so after null the canvas can still draw in 2D.
- **Size.** The host keeps the canvas's drawing buffer equal to its box in device pixels and passes that size to `resize`, which reconfigures the surface.
  `draw` then draws the last frame again, so a resize or a settings change shows at once, even while paused.
- **Failures.** An outdated or lost surface is reconfigured, and that frame is skipped.
  A lost device, or a WebGPU error nothing caught, makes every later `draw` throw.
- **Capture.** Pages can't read a WebGPU canvas back reliably, and Chromium with its software adapter shows no picture in `drawImage` or screenshots.
  `capture` draws the last frame with the current settings into an offscreen texture of the canvas's size and format, and resolves to its RGBA bytes.

The player's [`GameScreen`](../../web/player/src/screen.ts) tries the presenter first and falls back to Canvas 2D at 240x160, which the browser scales as plain pixels.
Frames that arrive while WebGPU is starting wait, newest only.
When the device fails during play, the player mounts a fresh canvas, which draws in 2D for the rest of the visit.

[`scripts/build-wasm.sh`](../../web/present/scripts/build-wasm.sh) builds the module with the `wasm-release` profile and runs `wasm-bindgen`.
The tool must be exactly the version of the `wasm-bindgen` crate in `Cargo.lock`, so the script reads it there.
It uses a matching `wasm-bindgen` on `PATH`, and otherwise installs that version once into `target/tools/`.

## How it is checked

[`reference.rs`](../../crates/present/src/reference.rs) defines each preset in f64, one output pixel at a time.
Hand-derived fixtures check both the reference and the GPU:

- Red comes from bits 0-4 and blue from bits 10-14, and bit 15 changes nothing.
- Two pixels stretched to three columns give the second pixel to the middle column, which sampling pixel edges instead of centres would not.
- Two pixels stretched 2.5x by `VanillaPlus` blend exactly one column, halfway, where plain bilinear would blend three.
- An integer-scaled picture sits centred on black, with the odd border pixel on the right.

Whole GBA frames with one-pixel detail and a checkerboard dither are then drawn at sizes from 1x to 2560x1600, fitted and integer-scaled.
`Pixel` must match the reference exactly, and `VanillaPlus` within one 8-bit step.
At whole multiples, `VanillaPlus` must draw the same bytes as `Pixel`.
The viewport's own tests compare placements with hand-computed rectangles.

In the browser, [`screen.spec.ts`](../../web/player/tests/browser/screen.spec.ts) runs the built player in Chromium with its software WebGPU adapter, SwiftShader.
A ROM shows red, green, blue and white quadrants, and the test reads the screen back with `capture`.
It checks that the drawing buffer follows the canvas's device size, that the quadrants' centres keep their exact colours, and that the menu's choices apply.
Pixel leaves no blended pixel in a row across the red-green edge, Vanilla+ leaves one or two, and integer scaling leaves a black border around the largest whole multiple.
A smaller window resizes the buffer while paused, and the choices survive a reload.
SwiftShader needs `--enable-features=Vulkan`, without which it loses the device right after creating it.
A second test hides WebGPU and checks the 2D fallback.

The GPU tests need an adapter, and a software rasterizer such as Mesa's lavapipe (Vulkan) or llvmpipe (GL) is enough.
Without an adapter they pass with a note.
`EVENFALL_REQUIRE_GPU=1` turns a missing adapter into a failure, and CI sets it on a job with lavapipe installed.
The tests share one device, because the Vulkan loader crashes when instances on parallel threads label their pipelines, as wgpu's debug builds do.
wgpu's `WGPU_BACKEND` variable picks the backend:

```sh
EVENFALL_REQUIRE_GPU=1 cargo test -p evenfall-present
WGPU_BACKEND=gl EVENFALL_REQUIRE_GPU=1 cargo test -p evenfall-present
```

## Known limits

- The web player is the only host.
  There is no native graphical host, so mobile suspend and resume are unhandled.
- Browsers without WebGPU get Pixel only, through Canvas 2D.
- `draw` writes its settings to one uniform buffer, so a host submits each draw before the next.
- Placement assumes square source pixels.
- The tests run on software rasterizers: lavapipe and llvmpipe through wgpu's Vulkan and GL backends, and SwiftShader in Chromium.
  Real GPUs, Metal, DX12, Safari and Firefox are untested.
- Smooth presets, colour correction and LCD effects don't exist yet ([#1](https://github.com/evenfall-emu/evenfall/issues/1)).
