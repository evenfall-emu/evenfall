# Scanline memo

The scanline memo lets `Bus::render_scanline` leave a line's frame surfaces as
they are when nothing the line depends on has changed since it was last
composed. [`video/scanline_memo.rs`](../../systems/gba/src/video/scanline_memo.rs)
owns the memo; [`video/render.rs`](../../systems/gba/src/video/render.rs)
owns the register key and recorded reads, and
[`bus/mod.rs`](../../systems/gba/src/bus/mod.rs) marks writes. The memo is
transient: `#[codec(skip)]` excludes it from save states, and a state load,
reset or accepted framebuffer invalidates it. Rust stores it in a box; its
size is not Evenfall's in-memory size.

The memo infers no pixel. A kept line holds bytes the renderer wrote from
inputs that are still equal; the whole mechanism exists to prove that equality
cheaply.

## Context

In static phases (menus, dialogue, pause and idle scenes) nearly every frame
repeats the previous one, even though games keep writing video memory. In a
Evenfall idle scene of a commercial game, a frame writes 1,024 palette bytes (none of
them changed), 380 VRAM bytes (57 changed), 256 OAM words and 6 LCD
registers, yet 95.7% of line renders produce exactly the pixels the frame
surfaces already hold.

A scanline without raster history is a pure function of two inputs: the live
LCD registers it reads and the video memory its composition reads. If both are
unchanged, recomposing the line would write the same bytes again.

## Mechanism

### Key

`ScanlineRegisters` holds every live register that a line without
raster history reads: DISPCNT, green swap, the four BGxCNT, the BG offsets,
the six window registers, BLDCNT, BLDALPHA and BLDY, MOSAIC, the BG affine
parameters and the current affine reference points.

Any change to one of them calls `Memo::mark_registers`. The next rendered line
rebuilds the key once (`Memo::set_registers`), and a key that differs from the
previous one gets a new version number. A line composed under the current
version skips the key comparison entirely. After a version change, each line
compares its stored key once and adopts the new version if the keys are equal,
so register values that a game writes between lines are still kept wherever
they repeat frame after frame.

### Recorded reads

Each composition records the 1 KiB VRAM pages it read (`Reads.vram_pages`):
text map rows, text tile rows, bitmap rows and regular-object tile rows.

- The layer-major text rows record the page of each tile row as they read it
  (a tile row never crosses a page) and the pages of the map rows.
- The front-to-back reference's tile-row path relies on a page holding 32
  whole 4bpp tiles or 16 whole 8bpp tiles, so a map entry's page follows from
  its tile number alone; consecutive lines usually share their map row, so
  the pages of a map block are cached per background and block (`TilePages`).
- An untransformed bitmap row records the pages of its source row.
- Lines that use the per-dot text path, affine backgrounds, transformed
  bitmaps, and affine or mosaic objects record every page.
- A line also records whether it read an OAM affine matrix.

Every line depends on the whole palette and on the attributes of each object
whose rows include it; these need no recording.

### Change marks

Changes are marked where they happen, not found by comparison:

| Event                                                                         | Mark                                                   |
| ----------------------------------------------------------------------------- | ------------------------------------------------------ |
| A palette write that changes at least one byte                                | `mark_palette`                                         |
| A VRAM write that changes a byte (`write_vram_byte`)                          | `mark_vram` for that byte's page                       |
| Any halfword or word OAM write, from the CPU or DMA (OAM ignores byte writes) | `mark_oam`                                             |
| RegisterRamReset                                                              | a mark for each region it clears, and `mark_registers` |
| Any write to the LCD register block, of any width, from the CPU or DMA        | `mark_registers`                                       |
| The per-line affine reference advance and its reload at the frame wrap        | `mark_registers`                                       |

Marking changed bytes rather than stores is what makes the memo work on real
games. The Evenfall idle scene above rewrites its whole palette every frame, so
marking every palette store would keep no line at all. Marks are recorded
only when bytes actually change.

### Application

Before each rendered line, `Memo::observe` applies pending marks:

- a changed palette invalidates every line;
- a changed VRAM page invalidates the lines that read it;
- each object whose 8 bytes differ from the memo's own 1 KiB copy of OAM
  invalidates its old and new rows when its attributes (bytes 0–5) changed,
  and every line that read a matrix when its affine parameter (bytes 6–7)
  changed.

Lines composed earlier in the frame are invalidated like later ones, so a
change in the middle of a frame costs only the lines that read what changed.
In the idle scene the memo applies marks about once per frame.

Lines with raster history (scroll, palette or VRAM, window, blend, affine or
mosaic records within the line) are always composed and never kept
(`Bus::has_raster_history`, `Memo::forget`). For every other line,
`Memo::holds` decides whether the surfaces are kept; otherwise the line is
composed and `Memo::store` records its key version and reads.

### An alternative: snapshot comparison

A simpler design compares all video memory (96 KiB of VRAM, the palette and
OAM) with a snapshot at every frame start, and the whole key on every line.
It can keep the same static lines, but a write within a frame invalidates
every later line of that frame and, through the fresh snapshot, the whole
next frame. Change marks also preserve hits when frames change mid-frame.

## Invariants

The memo is correct only while these rules hold. The key, each write path and
each invalidation point carries a short comment naming its part in them.

1. **Every renderer input is in the key, in the raster history or in the
   recorded reads.** A new register, memory region or host option that changes
   pixels must join one of them. A renderer read must also stay inside the
   memory it records: text tile fetches past the background side of VRAM read
   as zero rather than touching object tile data.
2. **Every path that changes the palette, VRAM or OAM marks it, and every
   change to a key register calls `mark_registers`.** A new write path (a
   debugger poke, a DMA or HLE fast path, a direct copy) must store through
   the existing functions or mark.
3. **Anything that replaces the surfaces, the registers or video memory
   wholesale calls `Memo::invalidate`.** State load, reset and framebuffer
   replacement (`Bus::accept_canonical_framebuffer`) do. Tests and tools that
   edit `Bus` fields directly must do the same.
4. **Recovery replays from a canonical copy.** Rust's
   [`VideoRecovery`](../../systems/gba/src/video_recovery.rs) anchors on a
   copy of the live `Machine` taken at a canonical frame, and the copy
   invalidates its memo, as state load does. Frames selective video skips
   compose nothing; their marks apply before the next rendered line.

## Check mode

Check mode is the safety net against an input or write path someone forgets.
It is on in debug and test builds; optimized builds can opt in with the
`scanline-memo-check` Cargo feature. It panics on the first violation:

- every line the memo would keep is composed again (`Bus::check_kept_scanline`),
  and its colors must come out exactly as kept (kept lines are always drawn by
  the layer-major compositor, which writes no other frame surface);
- the key is rebuilt for every line and must equal the live key whenever no
  register change was marked (`Memo::verify_registers`);
- each frame start compares the palette, VRAM and OAM with a snapshot taken at
  the previous one, and every difference must be covered by a mark made after
  that snapshot (`Memo::unmarked_change`). Marks still pending when a snapshot is
  taken describe changes the snapshot already holds, so they cannot cover
  later ones.

Check mode catches unmarked video-memory changes and recomposes held lines,
but verifies only paths a workload reaches. The original Evenfall mutation
tests are evidence for the checker; the Rust memo tests include unmarked
memory and stale-mark cases.

## When it helps

The memo helps in static phases (menus, dialogue, idle and pause scenes),
where lines repeat both their key and the pages they read. It does not help
when the key changes every frame (scrolling, per-frame affine motion), when
the palette changes (a fade invalidates every line), or when a game changes
the tiles or maps its lines read every frame. There every line is composed and
the memo costs only its bookkeeping, which measures as zero within noise.

## Tests and reproduction

- [`video/scanline_memo.rs`](../../systems/gba/src/video/scanline_memo.rs): "marked memory
  changes invalidate the lines that read them", "check mode finds video memory
  changed without a mark" and "check mode does not let marks made before its
  snapshot cover later changes".
- [`video/tests.rs`](../../systems/gba/src/video/tests.rs) compares kept and
  fresh lines after changed bitmap pages and checks raster history.
- Broader bus-write and reset tests are in
  [`tests/bus.rs`](../../systems/gba/tests/bus.rs).

A release check-mode build enables `scanline-memo-check`.

## Open questions

- Background fetches at or past the OBJ side of VRAM read as zero, but lines
  that address it still record the OBJ pages their tile numbers name. This
  costs only memo hits.
- Applying VRAM marks scans every line's pages: once per frame in the measured
  workloads, but up to once per line for a game that changes VRAM between
  lines.
- Any palette change invalidates every line, so fades keep nothing.
- Check mode verifies only paths a workload reaches; each new mark needs a
  unit test that reaches it.
