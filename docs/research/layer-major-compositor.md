# Layer-major compositor

A scanline without relevant raster history is composed layer by layer rather
than pixel by pixel. Each enabled background renders a whole color row, the
objects render two rows, and one kernel then selects, merges and blends eight
pixels at a time. [`video/compose.rs`](../../systems/gba/src/video/compose.rs)
owns the line pipeline and the kernel, [`video/lanes.rs`](../../systems/gba/src/video/lanes.rs)
the vector lanes, [`video/backgrounds.rs`](../../systems/gba/src/video/backgrounds.rs)
the row renderers and [`video/objects.rs`](../../systems/gba/src/video/objects.rs)
the object rows. ZiGBA's front-to-back passes in
[`video/render.rs`](../../systems/gba/src/video/render.rs) remain the
reference and draw every line with raster history.

## Pipeline

1. **Background rows.** Each enabled background of the display mode writes
   240 colors; bit 15 marks a transparent pixel (palette colors are 15-bit).
   - Text rows write whole 8-pixel tiles into a 256-entry buffer at
     `8 - (hofs & 7) + 8k`, so the partly scrolled first tile needs no
     special case. A 4bpp tile row decodes one byte into two colors through a
     per-bank table built on first use after a palette change
     ([`tile-row-cache.md`](tile-row-cache.md)); a horizontal flip reverses
     the row's nibbles before decoding. Vertical mosaic selects the source
     line and horizontal mosaic repeats each block's first pixel.
   - Affine rows step the reference point by PA and PC per pixel. A map that
     lies wholly in BG VRAM is read through power-of-two masks with the map
     size and wrap mode as const parameters; maps that run past BG VRAM, and
     mosaic, use a checked per-sample form that reads tile zero there, like
     the per-dot renderer.
   - Bitmap rows copy one (possibly translated) source row when PA is 1.0,
     PC is 0 and mosaic is off; otherwise they sample like affine rows.
2. **Object rows.** Objects are visited in descending OAM order, as in
   ZiGBA. Every opaque pixel overwrites the _front_ row, so it ends holding
   the lowest-indexed object, and replaces the _minimum_ row when its
   priority is at or below the stored one, so that row ends holding the
   smallest priority, ties to the lower index. Regular, affine, mosaic and
   8bpp objects all use the reference traversal; only the pixel sink differs.
3. **Kernel.** For each group of eight pixels, `compose_line` keeps the top
   and second surfaces (color and a meta word: layer, blend flags, priority)
   in registers. It starts from the backdrop and paints the enabled
   backgrounds back to front: a visible pixel (opaque and admitted by the
   window) becomes the top surface and pushes the previous top down. Then
   objects merge, then color effects apply, and only the final color is
   stored in `frame_pixels`.

The object merge reproduces ZiGBA's per-pixel compositor exactly. Replaying
its sequential rule over the objects at one pixel gives: if any admitted
object's priority is at or above the top background's, the front object
becomes the top surface whatever its own priority and the background becomes
the second surface; otherwise the minimum object replaces the second surface
when its priority is at or above that surface's. The rows carry exactly the
two objects this needs.

The kernel is monomorphized over three line kinds (no effects, brightness
only, alpha blending with brightness) and whether the line has objects; the
second surface is tracked only when the line alpha-blends. Blend targets are
folded into each layer's meta word per line, so the per-pixel tests are bit
masks.

## Why explicit lanes

The same selection written as loops over surface arrays did not vectorize
reliably. LLVM rewrote `second[x] = if visible { top[x] } else { second[x] }`
as a conditional store, which SSE2 emulates lane by lane with branches, and
rewrote selects between two loads as a load from a selected address. Small
source changes moved a pass between these forms.

`compose_line` is therefore written once against the small `Lanes` trait and
instantiated for SSE2 (eight 16-bit lanes, x86-64), WebAssembly `simd128`
(eight lanes, only when the module is built with that target feature) and a
scalar lane on every other target. Blending uses 16-bit multiplies, which
SSE2 and `simd128` provide directly. The scalar lane is the reference: a
randomized unit test requires the SSE2 kernel to produce the scalar kernel's
colors and surfaces for every line kind.

## Raster history

A line with raster history uses the reference passes, which resolve every
recorded write at its dot. One exception keeps the compositor: when a line's
only history is VRAM writes, the compositor draws the line and returns the
1 KiB VRAM pages its backgrounds read; if no recorded write falls in those
pages, the history cannot change the line and the result stands. Otherwise the
reference passes draw it again. A page-flipped bitmap drawn during HDraw is
the common case: its writes land in the hidden page.

Splitting a line into segments at recorded writes was not built. Across the
benchmark ROMs, lines with raster history other than VRAM writes are rare:

| Workload   | Lines with history | VRAM-only |
| ---------- | -----------------: | --------: |
| tiles      |              0.48% |     0.47% |
| affine     |              0.10% |     0.09% |
| bitmap-dma |             45.17% |    45.17% |
| others     |  under 1 in 10,000 |         — |

## Surfaces

The compositor writes no per-frame auxiliary surface. The top and second
surfaces of the last composed line stay in its buffers and are exposed through
`Bus::composed_surfaces`, which tests use in place of ZiGBA's frame-sized
priority, layer and second-surface arrays; those arrays belong to the
reference passes. The scanline memo's check mode recomposes kept lines and
compares their colors.

## Measurements

Native x86-64, pinned CPU 3, HLE boot, release-LTO harnesses, 1,200 frames
(`perf stat`); host instructions and cycles, ZiGBA against Evenfall before
and after this design:

| Workload   | ZiGBA            | Before           | After            |
| ---------- | ---------------- | ---------------- | ---------------- |
| tiles      | 11.91 G / 3.00 G | 19.98 G / 4.66 G | 9.28 G / 1.50 G  |
| affine     | 13.04 G / 2.58 G | 18.59 G / 3.07 G | 15.33 G / 2.49 G |
| bitmap-dma | 21.64 G / 4.03 G | 27.20 G / 4.52 G | 18.13 G / 2.72 G |
| idle       | 0.19 G / 0.08 G  | 0.38 G / 0.08 G  | 0.24 G / 0.04 G  |

Loop-time ratios from the benchmark runner are in
[`performance.md`](../performance.md).

What did not pay:

- Passes over surface arrays, one per background, relying on
  auto-vectorization: 13.56 G instructions on tiles against 10.76 G with the
  explicit lane kernel at the time, and a later variant with double-buffered
  16-bit planes was worse still (15.53 G).
- The content-keyed 4bpp row cache inside the new rows: the tiles ROM uses
  every palette bank with flips, so its 1,024 entries missed constantly; the
  pair tables cut tiles from 10.76 G to 9.28 G instructions.

## Tests

- [`video/tests.rs`](../../systems/gba/src/video/tests.rs): "layer major lines
  match the front to back reference" randomizes every display mode, BG and
  OBJ mosaic, affine and bitmap transforms, every object kind, the OBJ window,
  windows, effects, green swap and forced blank, and compares colors and the
  top and second surfaces with the reference; "vram history selects the
  reference only for pages the line reads" covers the history exception.
- [`video/compose.rs`](../../systems/gba/src/video/compose.rs): "vector lanes
  match the scalar kernel".
- The ported ZiGBA renderer tests compare the compositor with the per-dot and
  per-pixel reference renderers.

## Open questions

- The `simd128` lanes are checked only through whole-frame agreement with the
  scalar build; no unit test runs inside WebAssembly.
- Affine rows still cost about 23 host instructions per pixel; a rotation
  that keeps within one tile column could reuse tile lookups.
- Lines with other raster histories could be split into segments at their
  recorded dots instead of using the reference passes, if a workload shows
  them to matter.
