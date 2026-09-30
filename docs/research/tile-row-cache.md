# Tile-row decoding

Most GBA backgrounds are made of 16-color tiles.
A row of such a tile is eight palette indices packed into four bytes, low nibble first: bytes `21 43 65 87` mean indices `1 2 3 4 5 6 7 8`, and index 0 is transparent.
Turning each row into colors one nibble at a time, with a palette lookup per pixel, is a large share of drawing time on tile-heavy scenes.

[`video/tile_row_cache.rs`](../../systems/gba/src/video/tile_row_cache.rs) decodes two pixels per table lookup instead, for both text renderers:

- `text_row` in [`video/backgrounds.rs`](../../systems/gba/src/video/backgrounds.rs), which feeds the [layer-major compositor](layer-major-compositor.md),
- `Bus::decode_text_row`, the tile-row path of the reference renderer.

## How it works

The cache isn't part of the machine's state and is left out of save states.
It holds a copy of the 1 KiB palette and three decoded forms of it:

| Field   | Contents                                                               |
| ------- | ---------------------------------------------------------------------- |
| `words` | the 512 palette colors, backgrounds then sprites                       |
| `bg256` | the 256 background colors, with index 0 replaced by a transparent mark |
| `pairs` | per 16-color bank, the two colors of each of the 256 byte values       |

A decoded color with bit 15 set is transparent, since palette colors only use 15 bits.

**Two pixels per lookup.** Entry `b` of a bank's `pairs` table holds the color of byte `b`'s low nibble in bits 0–15, and of its high nibble in bits 16–31, so a whole row takes four table reads.
A bank's table is built the first time it is used after a background palette change (256 entries), and a bitmask records which banks are built.

**Flips change the source, not the result.** A vertical flip picks which source row is read.
A horizontal flip reverses the row's nibbles before decoding (`reverse_nibbles`, a byte swap then a nibble swap), so no decoded row ever needs reversing.

**The palette is checked by content.** Before decoding live colors, every line calls `Cache::prepare_palette`, which compares the live palette with the copy as 64-bit words.
The comparison has no early exit, so it isn't turned into a `memcmp` call, which is a byte loop in WebAssembly.
A difference replaces the copy, decodes the colors, and, if the background half changed, clears the built banks.
Comparing content also catches direct edits of the memory and restored save states, even ones that restore older bytes, so the cache needs no write hook or invalidation counter.

**256-color rows need no table.** Each byte is already one index, and `bg256` maps it to a color.

Lines with palette or VRAM changes during the line use the per-dot reference renderer, which uses the value visible at each dot, so decoded live colors never replace historical ones.

## How it is checked

- [`video/tile_row_cache.rs`](../../systems/gba/src/video/tile_row_cache.rs) checks fixed expected rows, banks, palette changes (including restored bytes), sprite-only palette changes and 4,096 different source rows, and that reversed nibbles mirror a row.
- [`video/tests.rs`](../../systems/gba/src/video/tests.rs) compares whole lines with the per-dot reference (`text_tile_rows_match_the_per_dot_text_renderer`, `layer_major_lines_match_the_front_to_back_reference`), and checks text spans against an independent pixel oracle across map wraps, flips and transparency.

## Open questions

- Any background palette change rebuilds the banks it touches on their next use, so a fade can rebuild up to 16 tables per frame.
- Sprites decode their tile rows through the palette colors, without a pair table.
