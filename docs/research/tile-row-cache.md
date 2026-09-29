# Tile-row decoding

A 4bpp background tile row stores eight palette indices in four bytes, low
nibble first: bytes `21 43 65 87` decode to indices `1 2 3 4 5 6 7 8`, and
index zero is transparent. [`video/tile_row_cache.rs`](../../systems/gba/src/video/tile_row_cache.rs)
turns such rows into colors for both text renderers: `text_row` in
[`video/backgrounds.rs`](../../systems/gba/src/video/backgrounds.rs), which
feeds the [layer-major compositor](layer-major-compositor.md), and
`Bus::decode_text_row`, the tile-row path of the front-to-back reference.
ZiGBA's [`performance.md`](../../refs/zigba/docs/performance.md) describes
the content-keyed row cache this replaces.

## Design

The cache is transient host state, excluded from save states. It holds a copy
of the 1 KiB palette and three decoded forms of it:

| Field   | Content                                                           |
| ------- | ----------------------------------------------------------------- |
| `words` | the 512 RGB555 palette words, BG then OBJ                         |
| `bg256` | the 256 BG words with index zero replaced by the transparent mark |
| `pairs` | per 16-color bank, the two colors of each of the 256 byte values  |

A decoded color with bit 15 set is transparent; palette colors use 15 bits.

**Pairs decode two pixels per lookup.** Entry `b` of a bank holds the color
of byte `b`'s low nibble in bits 0–15 and of its high nibble in bits 16–31, so
a row takes four table reads. A bank's table is built on its first use after
a background palette change (256 entries); a bitmask records which banks are
built.

**Flips transform the source.** A vertical flip selects which source row is
read. A horizontal flip reverses the row's nibbles (`reverse_nibbles`, a byte
swap and a nibble swap) before decoding, so no decoded row is reversed.

**The palette is validated by content.** Every line that decodes live colors
first calls `Cache::prepare_palette`, which compares the live palette with the
copy as 64-bit words, without an early exit so the loop is not replaced by a
`memcmp` call (a byte loop on WebAssembly). A difference replaces the copy,
decodes the words and, when the background half changed, clears the built
banks. Comparing content also observes direct edits of the backing memory and
restored save states, including restoring older bytes, so no write hook or
invalidation counter exists.

**8bpp rows need no table.** Each byte is already one index; `bg256` maps it
to a color.

Lines with palette or VRAM raster history use the per-dot reference, which
resolves the value visible at each dot; historical values are never replaced
by decoded live ones.

## Measurements

The earlier design kept 1,024 content-keyed entries, each a whole row's
colors for one source word and bank. The tiles benchmark ROM spreads its 64
tiles over all 16 banks with flips, so its rows missed that cache constantly.
Replacing the cache lookup with pair tables in the layer-major text rows took
the tiles workload from 10.76 G to 9.28 G host instructions per 1,200 frames
and from 2.16 G to 1.49 G cycles (release-LTO, pinned CPU 3).

The word-wise palette comparison made the WebAssembly build's idle workload
twice as fast in Node (loop time 0.54 of the build before it), because every
line compared the palette with a byte loop. It adds about 0.02 G native
instructions per 1,200 idle frames and no cycles.

## Tests

- [`video/tile_row_cache.rs`](../../systems/gba/src/video/tile_row_cache.rs): "expanded rows
  validate palette bytes banks and source replacement" checks fixed expected
  rows, banks, palette changes including restored bytes, OBJ-only changes and
  4,096 distinct sources; "reversed nibbles mirror the row".
- [`video/tests.rs`](../../systems/gba/src/video/tests.rs): "text tile rows
  match the per-dot text renderer" and "layer major lines match the front to
  back reference" compare whole lines with the per-dot reference;
  "ordinary and effects-aware text spans agree across map wraps flips and
  transparency" uses an independent pixel oracle.

## Open questions

- Any background palette change rebuilds the banks it touches on their next
  use, so a fade rebuilds up to 16 tables per frame.
- Objects decode their tile rows through the palette words without a pair
  table.
