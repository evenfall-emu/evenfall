# Layer-major compositor

The obvious way to draw a GBA line is pixel by pixel: for each of the 240 dots, look at every background and sprite, find the frontmost visible one and the one under it, and apply color effects.
That is how the reference renderer works, and it is simple to get right.
It is also slow, because every dot repeats the same decisions with branches the CPU can't predict.

The layer-major compositor turns the loop inside out.
Each enabled background first draws its whole row of 240 colors, the sprites draw two rows, and then one kernel picks, merges and blends eight pixels at a time with SIMD instructions.
Drawing a whole row of one layer is regular work that runs fast, and the final merge has no per-pixel branches at all.

The code is split across a few files:

- [`video/compose.rs`](../../systems/gba/src/video/compose.rs) holds the line pipeline and the kernel,
- [`video/lanes.rs`](../../systems/gba/src/video/lanes.rs) the vector lanes,
- [`video/backgrounds.rs`](../../systems/gba/src/video/backgrounds.rs) the background rows,
- [`video/objects.rs`](../../systems/gba/src/video/objects.rs) the sprite rows.

The front-to-back passes in [`video/render.rs`](../../systems/gba/src/video/render.rs) remain the reference, and still draw every line with raster history.

## The pipeline

### 1. Background rows

Each enabled background writes 240 colors.
Palette colors are 15-bit, so bit 15 marks a transparent pixel.

- **Text rows** write 31 whole 8-pixel tiles into a 256-entry buffer, tile `k` at position `8k`.
  The kernel reads the line from offset `hofs & 7`, so the partly scrolled first tile needs no special case.
  The line's map entries are copied first.
  A 16-color tile row decodes one byte into two colors through per-bank tables rebuilt after a palette change ([`tile-row-cache.md`](tile-row-cache.md)), and a horizontal flip reverses the row's nibbles before decoding.
  Vertical mosaic picks the source line, and horizontal mosaic repeats each block's first pixel.
- **Affine rows** step the reference point by PA and PC per pixel.
  A map that lies entirely in background VRAM is read through power-of-two masks, with the map size and wrap mode as const parameters.
  With PA = 1.0 and PC = 0, the row reads one map row and is drawn a tile at a time, like a text row.
  Maps that run past background VRAM, and mosaic, use a checked per-sample form that reads tile zero there, like the per-dot renderer.
- **Bitmap rows** copy one source row, possibly shifted, when PA = 1.0, PC = 0 and mosaic is off.
  Otherwise they sample like affine rows.

### 2. Sprite rows

Sprites are visited from the highest OAM number down, as in the reference, and write two rows:

- The **front** row takes every opaque pixel, so it ends holding the lowest-numbered sprite.
- The **minimum** row takes a pixel when its priority is at or in front of the stored one, so it ends holding the frontmost priority, with ties going to the lower number.

The line's sprite list comes from a compacted OAM that records which sprites cover each line.
Regular sprites decode whole tile rows, non-mosaic affine sprites step their matrix per pixel, and mosaic sprites use the reference per-pixel loop writing into a row.

### 3. The kernel

For each group of eight pixels, `compose_line` keeps the top and second surfaces in registers: a color and a meta word (layer, blend flags, priority).
It starts from the backdrop and paints the enabled backgrounds from back to front.
A visible pixel (opaque and allowed by the window) becomes the top surface and pushes the previous top down.
Then sprites merge in, color effects apply, and only the final color is stored in `frame_pixels`.

The sprite merge reproduces the reference compositor exactly.
Playing its sequential rule through for all sprites at one pixel gives a simple result:

- If any sprite allowed there has priority at or in front of the top background's, the front-row sprite becomes the top surface, whatever its own priority, and the background becomes the second surface.
- Otherwise the minimum-row sprite replaces the second surface when its priority is at or in front of that surface's.

The two sprite rows carry exactly the two sprites this needs.

The kernel is compiled separately for three kinds of line (no effects, brightness only, alpha blending with brightness), and for lines with and without sprites.
The second surface is only tracked when the line blends.
Each layer's blend targets are folded into its meta word once per line, so the per-pixel tests are bit masks.

## Why explicit SIMD lanes

The same selection written as loops over arrays didn't vectorize reliably.
LLVM rewrote `second[x] = if visible { top[x] } else { second[x] }` as a conditional store, which SSE2 can only emulate lane by lane with branches.
It also rewrote a choice between two loads as a load from a chosen address.
Small source changes moved a loop from one form to another.

So `compose_line` is written once against a small `Lanes` trait.
It is compiled for eight 16-bit lanes of the [`wide`](https://crates.io/crates/wide) crate's `i16x8` where that has a vector backend (SSE2 on x86-64, NEON on AArch64, `simd128` in the WebAssembly build that enables it), and for a scalar lane everywhere else.
`wide` keeps the lanes free of `unsafe` code.

A few details matter for speed:

- `wide`'s own eight-element fallback isn't used, because it made the scalar WebAssembly module 1.19x slower on the tiles workload than the scalar lane.
- Logical shifts go through the unsigned view.
- The widening byte load uses `u16x8::from_u8x16_low`, since widening each lane separately compiled to eight scalar extracts on SSE2.
- Blending uses 16-bit multiplies, which all three backends have.

The scalar lane is the reference.
A randomized test requires the vector kernel to produce the scalar kernel's colors and surfaces for every kind of line, on x86-64 and on AArch64 (run under qemu).

## Lines with raster history

A line whose registers or video memory change while it is drawn uses the reference passes, which apply every recorded change at its dot.

One exception keeps the fast path.
When a line's only changes are VRAM writes, the compositor draws the line and reports the 1 KiB VRAM pages its backgrounds read.
If no recorded write falls in those pages, the writes can't have changed the line, and the result stands.
Otherwise the reference passes draw it again.
The common case is a double-buffered bitmap game drawing its next frame during the visible part of the line: its writes all land in the hidden page.

Splitting a line at its recorded changes was never built, because lines with other changes are rare on the benchmark ROMs:

| Workload   | Lines with history | Only VRAM writes |
| ---------- | -----------------: | ---------------: |
| tiles      |              0.48% |            0.47% |
| affine     |              0.10% |            0.09% |
| bitmap-dma |             45.17% |           45.17% |
| others     |  under 1 in 10,000 |                — |

## Surfaces

The compositor writes nothing per frame besides the pixels.
The top and second surfaces of the last line drawn stay in its buffers, and `Bus::composed_surfaces` exposes them for tests.
The frame-sized priority, layer and second-surface arrays belong to the reference passes only.
The scanline memo's check mode redraws kept lines and compares their colors.

## How it is checked

- `layer_major_lines_match_the_front_to_back_reference` in [`video/tests.rs`](../../systems/gba/src/video/tests.rs) randomizes every display mode, background and sprite mosaic, affine and bitmap transforms, every kind of sprite, the sprite window, windows, effects, green swap and forced blank, and compares colors and both surfaces with the reference.
- `vram_history_selects_the_reference_only_for_pages_the_line_reads` covers the exception for VRAM writes.
- `vector_lanes_match_the_scalar_kernel` in [`video/compose.rs`](../../systems/gba/src/video/compose.rs) compares the SIMD and scalar kernels.
- The other renderer tests compare the compositor with the per-dot and per-pixel reference renderers.

## Open questions

- The `simd128` lanes are only checked through whole frames matching the scalar build.
  No unit test runs inside WebAssembly.
- The NEON lanes are only checked under qemu, and their speed on real AArch64 hardware is unmeasured.
- Rotated affine rows still cost about 23 host instructions per pixel.
- Lines with other raster changes could be split at their recorded dots instead of using the reference passes, if a workload shows they matter.
