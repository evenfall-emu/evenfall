# Tile-row cache

A 4bpp background tile row stores eight palette indices in four bytes, low
nibble first: bytes `21 43 65 87` decode to indices `1 2 3 4 5 6 7 8`. The same
row appears many times on one screen and again in every following frame. The
tile-row cache in [`video/tile_row_cache.rs`](../../systems/gba/src/video/tile_row_cache.rs)
remembers each row's decoded indices and palette-expanded colors instead of
unpacking and looking up the same nibbles again. The tile-row text renderer
(`Bus::decode_text_row`, called from `Bus::render_text_tile_rows` in
[`video/backgrounds.rs`](../../systems/gba/src/video/backgrounds.rs)) is its
only user. ZiGBA's [`performance.md`](../../refs/zigba/docs/performance.md)
describes the original optimization.

## Design

The cache holds 1,024 entries, a 512-byte copy of the background palette, and
256 decoded RGB555 palette words. Rust's in-memory layout is not part of the
save-state format. An entry holds:

| Field              | Content                                                  |
| ------------------ | -------------------------------------------------------- |
| `source`           | the four raw bytes of the row, as one 32-bit word        |
| `indices`          | the eight decoded palette indices                        |
| `colors`           | the eight RGB555 colors for one palette bank             |
| `palette_bank_tag` | the bank those colors belong to, plus one; zero for none |
| `fully_opaque`     | whether every index is nonzero                           |

**Lookup is keyed on content.** `Cache::row` hashes both the source word and
palette bank to select an entry, then compares the full source word and bank
tag. A source mismatch decodes the row into that entry; a bank mismatch
re-expands its colors. Collisions therefore cost only performance. A
zero-filled entry already holds the correct decode of source zero, so no valid
bit is needed, and the renderer skips all-zero rows before looking them up.

Because the key is the row's bytes, not its address, the cache needs no write
hooks and no invalidation counter. CPU writes, DMA, BIOS VRAM clearing,
direct edits of the backing memory and restored save states all supply their
current bytes at the next lookup. Identical rows at different VRAM addresses
share one entry.

**Flips stay outside the key.** A vertical flip selects which source row is
looked up. A horizontal flip is applied to the canonical entry when the row is
drawn: fixed-array lanes select colors and indices in display order. Stable
Rust does not use ZiGBA's `@Vector` shuffle here.

**Colors are validated against the palette.** `Cache::row(source, bank)`
expands the eight indices through decoded palette words for one bank and tags
the entry with that bank; a different bank rebuilds the row's colors. Once
per eligible scanline (modes 0 and 1 without palette raster history),
`Bus::compose_scanline` calls `Cache::prepare_palette`, which compares the live
512-byte background palette with the copy; any difference replaces the copy,
decodes all 256 palette words once, and clears every entry's tag. Replacing an
entry's source clears its tag too.
Comparing bytes also observes direct edits and restored states, including
restoring older bytes. Every caller that uses expanded colors must establish
the matching palette copy first, including after direct edits or a state
restore.

**Opaque rows are stored whole.** `fully_opaque` is computed when a row is
decoded. When it is set and all eight lanes are open, the renderer stores the
eight colors, priorities and layers directly instead of selecting lane by lane.

**What stays outside the cache.** 8bpp rows already store one index per byte,
so the renderer reads them with one fixed-size load and looks up their colors
directly. Lines with palette or VRAM raster history, non-identity mosaic or a
scroll write within the line use the per-dot renderer, which resolves the
value visible at each dot; historical values are never replaced by cached live
ones. The cache is transient host state and is excluded from save states.

## Measurements

On ZiGBA's Node/WASM comparison, decoded indices together with 8bpp and flip
loop changes reduced work by 4.2% on one scene; the cache's individual
contribution was not isolated. Browser comparisons disagreed on the direction
of the gain. Expanded colors and opaque-row handling were measured only with
other changes. These figures justify retaining the exact cache design, not a
performance claim for the Rust port; that still needs its own benchmark.

## Tests

- [`video/tile_row_cache.rs`](../../systems/gba/src/video/tile_row_cache.rs): "expanded rows
  validate palette bytes banks and source replacement" checks fixed expected
  rows, repeated hits, bank tags, palette changes including restored bytes,
  and replacement across more distinct rows than the cache holds.
- [`video/tests.rs`](../../systems/gba/src/video/tests.rs): "text tile rows
  match the per-dot text renderer" compares whole lines with the per-dot
  reference; "ordinary and effects-aware text spans agree across map wraps
  flips and transparency" uses an independent pixel oracle.
- ZiGBA's additional DMA/history tests remain in
  [`test/bus_test.zig`](../../refs/zigba/test/bus_test.zig) until ported.

## Open questions

- The cache covers only 4bpp text backgrounds. Affine backgrounds, bitmap
  modes and objects decode without it.
- Any palette byte change clears every entry's colors, so during a fade every
  row's colors are rebuilt in each frame.
