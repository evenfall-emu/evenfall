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
   - Text rows write 31 whole 8-pixel tiles into a 256-entry buffer, tile
     `k` at `8k`, and the kernel reads the line from offset `hofs & 7`, so
     the partly scrolled first tile needs no special case. The line's map
     entries are copied first. A 4bpp tile row decodes one byte into two
     colors through per-bank tables built once after a palette change
     ([`tile-row-cache.md`](tile-row-cache.md)); a horizontal flip reverses
     the row's nibbles before decoding. Vertical mosaic selects the source
     line and horizontal mosaic repeats each block's first pixel.
   - Affine rows step the reference point by PA and PC per pixel. A map that
     lies wholly in BG VRAM is read through power-of-two masks with the map
     size and wrap mode as const parameters; with PA 1.0 and PC 0 the row
     reads one map row and is rendered a tile at a time like a text row. Maps
     that run past BG VRAM, and mosaic, use a checked per-sample form that
     reads tile zero there, like the per-dot renderer.
   - Bitmap rows copy one (possibly translated) source row when PA is 1.0,
     PC is 0 and mosaic is off; otherwise they sample like affine rows.
2. **Object rows.** Objects are visited in descending OAM order, as in
   ZiGBA. Every opaque pixel overwrites the _front_ row, so it ends holding
   the lowest-indexed object, and replaces the _minimum_ row when its
   priority is at or below the stored one, so that row ends holding the
   smallest priority, ties to the lower index. The line's object list comes
   from the compacted OAM, which records per line the objects that cover it.
   Regular objects decode whole tile rows, non-mosaic affine objects step
   their matrix per pixel, and mosaic objects use the reference per-pixel
   loop with a row sink.
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
instantiated for eight 16-bit lanes of the [`wide`](https://crates.io/crates/wide)
crate's `i16x8` where it has a vector backend (SSE2 on x86-64, NEON on
AArch64, `simd128` on WebAssembly built with that target feature), and for a
scalar lane on every other target. `wide` keeps the lanes free of `unsafe`
code. Its eight-element array fallback is not used: it made the scalar
WebAssembly module's tiles 1.19× slower than the scalar lane. Logical shifts
go through the unsigned view, and the widening byte load uses
`u16x8::from_u8x16_low`; widening each lane compiled to eight scalar extracts
on SSE2. Blending uses 16-bit multiplies, which all three backends provide
directly. The scalar lane is the reference: a randomized unit test requires
the vector kernel to produce the scalar kernel's colors and surfaces for every
line kind, on x86-64 and on AArch64 (run under qemu).

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
- The NEON lanes are checked under qemu only; their speed on AArch64 hardware
  is unmeasured.
- Rotated affine rows still cost about 23 host instructions per pixel.
- Lines with other raster histories could be split into segments at their
  recorded dots instead of using the reference passes, if a workload shows
  them to matter.
