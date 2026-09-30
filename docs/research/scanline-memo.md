# Scanline memo

In menus, dialogue boxes, pause screens and idle scenes, nearly every frame looks exactly like the one before.
Games still keep writing to video memory, often rewriting the same values, so the display chip still has to be emulated.
But redrawing an unchanged line only writes the same bytes again.

The scanline memo skips that work.
It remembers what each line's drawing depended on, and when none of it has changed since the line was last drawn, it leaves the line as it is.
It never guesses a pixel: a kept line holds exactly the bytes the renderer wrote from inputs that are still equal, and the whole mechanism exists to prove that equality cheaply.

In one measured idle scene of a commercial game, a frame writes 1,024 palette bytes (none of them different), 380 VRAM bytes (57 of them different), 256 OAM words and 6 display registers, yet 95.7% of lines come out exactly as they already were.

The code is split in three places:

- [`video/scanline_memo.rs`](../../systems/gba/src/video/scanline_memo.rs) holds the memo,
- [`video/render.rs`](../../systems/gba/src/video/render.rs) builds the register key and records what each line reads,
- [`bus/mod.rs`](../../systems/gba/src/bus/mod.rs) marks writes.

The memo isn't part of the machine's state.
`#[codec(skip)]` leaves it out of save states, and loading a state, resetting or accepting a rebuilt frame clears it.

## What a line depends on

A line without raster history (no register or memory change during the line) depends on only two things: the display registers it reads, and the video memory it reads.
If both are unchanged, drawing it again would give the same result.
The memo tracks each of them in its own way.

### The register key

`ScanlineRegisters` holds every register a line without raster history reads: `DISPCNT`, green swap, the four `BGxCNT`, the background offsets, the six window registers, `BLDCNT`, `BLDALPHA` and `BLDY`, `MOSAIC`, the affine parameters and the current affine reference points.

Any change to one of them calls `Memo::mark_registers`.
The next line drawn rebuilds the key once (`Memo::set_registers`), and a key that differs from the previous one gets a new version number.
A line drawn under the current version skips comparing keys entirely.
After a version change, each line compares its stored key once, and adopts the new version if the keys are equal.
So registers a game writes between lines are still matched wherever they repeat frame after frame.

### Recorded reads

Each line records the 1 KiB VRAM pages it read (`Reads.vram_pages`): map rows, tile rows, bitmap rows and sprite tile rows.

- The layer-major text rows record the page of each tile row as they read it (a tile row never crosses a page), and the pages of the map rows.
- The reference renderer's tile-row path relies on a page holding exactly 32 whole 16-color tiles or 16 whole 256-color tiles, so a map entry's page follows from its tile number alone.
  Consecutive lines usually share their map row, so the pages of a map block are cached per background and block (`TilePages`).
- An unscaled bitmap row records the pages of its source row.
- Lines that use the per-dot text path, affine backgrounds, transformed bitmaps, or affine or mosaic sprites record every page.
- A line also records whether it read a sprite rotation matrix from OAM.

Every line depends on the whole palette, and on the attributes of each sprite that covers it, so those need no recording.

## Marking changes

Changes are marked where they happen, not found by comparison afterwards:

| Event                                                                      | Mark                                                   |
| -------------------------------------------------------------------------- | ------------------------------------------------------ |
| a palette write that changes at least one byte                             | `mark_palette`                                         |
| a VRAM write that changes a byte (`write_vram_byte`)                       | `mark_vram` for that byte's page                       |
| any 16- or 32-bit OAM write, from the CPU or DMA (OAM ignores byte writes) | `mark_oam`                                             |
| `RegisterRamReset`                                                         | a mark for each region it clears, and `mark_registers` |
| any write to the display registers, of any width, from the CPU or DMA      | `mark_registers`                                       |
| the per-line affine reference step, and its reload when the frame wraps    | `mark_registers`                                       |

Marking changed bytes rather than every store is what makes the memo work on real games.
The idle scene above rewrites its whole palette every frame with the same values, so marking every palette store would keep no line at all.

Before each line is drawn, `Memo::observe` applies the pending marks:

- A changed palette invalidates every line.
- A changed VRAM page invalidates the lines that read it.
- Each sprite whose 8 bytes differ from the memo's own 1 KiB copy of OAM invalidates its old and new lines when its attributes (bytes 0–5) changed, and every line that read a matrix when its matrix entry (bytes 6–7) changed.

Lines drawn earlier in the frame are invalidated like later ones, so a change in the middle of a frame only costs the lines that read what changed.
In the idle scene the memo applies marks about once per frame.

Lines with raster history (scroll, palette, VRAM, window, blend, affine or mosaic changes during the line) are always drawn and never kept (`Bus::has_raster_history`, `Memo::forget`).
For every other line, `Memo::holds` decides whether to keep it.
Otherwise the line is drawn, and `Memo::store` records its key version and reads.

### Why not compare snapshots

A simpler design would compare all video memory (96 KiB of VRAM, the palette and OAM) with a snapshot at every frame start, and the whole key on every line.
It keeps the same static lines, but a write in the middle of a frame invalidates every later line of that frame and, through the new snapshot, the whole next frame.
Change marks keep lines even when something changes mid-frame.

## Rules the memo depends on

The memo is only correct while these rules hold.
The key, each write path and each invalidation point carries a short comment naming its part in them.

1. **Every renderer input is in the key, the raster history or the recorded reads.**
   A new register, memory region or host option that changes pixels must join one of them.
   A renderer read must also stay inside the memory it records: text tile fetches past the background part of VRAM read as zero instead of touching sprite tiles.
2. **Every path that changes the palette, VRAM or OAM marks it, and every change to a key register calls `mark_registers`.**
   A new write path (a debugger write, a DMA or BIOS fast path, a direct copy) must store through the existing functions, or mark.
3. **Anything that replaces the frame, the registers or video memory wholesale calls `Memo::invalidate`.**
   Loading a state, resetting and accepting a rebuilt frame (`Bus::accept_canonical_framebuffer`) do.
   Tests and tools that edit `Bus` fields directly must do the same.
4. **Rebuilding skipped frames starts from a clean copy.**
   [`VideoRecovery`](../../systems/gba/src/video_recovery.rs) replays from a copy of the machine taken at a drawn frame, and the copy clears its memo, as loading a state does.
   Frames skipped during fast-forward draw nothing, and their marks apply before the next line drawn.

## Check mode

Check mode is the safety net against an input or write path someone forgets.
It is on in debug and test builds, and optimized builds can turn it on with the `scanline-memo-check` Cargo feature.
It panics on the first violation:

- Every line the memo would keep is drawn again (`Bus::check_kept_scanline`), and its colors must come out exactly as kept.
  Kept lines are always drawn by the layer-major compositor, which writes nothing else per frame.
- The key is rebuilt for every line, and must equal the live key whenever no register change was marked (`Memo::verify_registers`).
- Each frame start compares the palette, VRAM and OAM with a snapshot taken at the previous one, and every difference must be covered by a mark made after that snapshot (`Memo::unmarked_change`).
  Marks still pending when a snapshot is taken describe changes the snapshot already holds, so they can't cover later ones.

Check mode only verifies the paths a workload reaches.

## When it helps

The memo helps in static scenes (menus, dialogue, idle and pause screens), where lines repeat both their key and the pages they read.
It doesn't help when the key changes every frame (scrolling, rotating), when the palette changes (a fade invalidates every line), or when a game rewrites the tiles or maps its lines read every frame.
There every line is drawn, and the memo's bookkeeping costs nothing measurable.

## How it is checked

- [`video/scanline_memo.rs`](../../systems/gba/src/video/scanline_memo.rs) checks that marked changes invalidate the lines that read them, that check mode finds video memory changed without a mark, and that marks made before a snapshot can't cover later changes.
- [`video/tests.rs`](../../systems/gba/src/video/tests.rs) compares kept and freshly drawn lines after every kind of input change, including changed bitmap pages and raster history.
- [`tests/bus.rs`](../../systems/gba/tests/bus.rs) covers the wider bus write and reset paths.
- Every debug and test run has check mode on, so the whole test suite exercises it.

## Open questions

- Background fetches past the background part of VRAM read as zero, but lines that address them still record the sprite pages their tile numbers name.
  This only costs kept lines.
- Applying VRAM marks scans every line's pages.
  That happens about once per frame in the measured workloads, but could be once per line for a game that changes VRAM between lines.
- Any palette change invalidates every line, so fades keep nothing.
- Check mode only verifies paths a workload reaches, so each new mark needs a unit test that reaches it.
