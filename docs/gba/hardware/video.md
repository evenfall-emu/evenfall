# Video

The GBA's LCD controller (PPU) draws a 240×160 image from up to four
background layers, 128 objects, two rectangular windows and an object window,
with alpha blending, brightness effects and mosaic. This page describes that
hardware as far as the sources establish it, the model ZiGBA implements, and
where the two may differ.

Evenfall renders by scanline. [`video/timing.rs`](../../../systems/gba/src/video/timing.rs)
(`PpuTiming`) tracks the LCD position and derives `DISPSTAT` and `VCOUNT`. The
renderer in [`video/render.rs`](../../../systems/gba/src/video/render.rs)
composes each visible line
when that line's HBlank begins, from the registers in force at that moment
plus a bounded per-dot history of the mid-line writes described under
[Raster effects](#raster-effects) in
[`video/history.rs`](../../../systems/gba/src/video/history.rs). The result is a native BGR555 framebuffer;
color correction is a separate host step ([Presentation](#presentation)).
Rust's fixed-array lane loops replace Zig's `@Vector` operations; scalar
reference renderers and differential tests remain in the tree. The
[`tile-row cache`](../../../systems/gba/src/video/tile_row_cache.rs) and
[`scanline memo`](../../../systems/gba/src/video/scanline_memo.rs) do not change
pixel output; see the [research notes](../../research/README.md).

## LCD timing

One dot takes four master cycles. GBATEK gives the frame geometry
([LCD Dimensions and Timings][gbatek-timings]):

| Quantity     | Value                                                                      |
| ------------ | -------------------------------------------------------------------------- |
| Visible line | 240 dots, 960 cycles                                                       |
| HBlank       | 272 cycles                                                                 |
| Line         | 1,232 cycles                                                               |
| Frame        | 160 visible + 68 VBlank lines = 228 lines, 280,896 cycles (about 59.73 Hz) |

ZiGBA delivers LCD events at these positions:

| Position                       | Effect                                                                                                                         |
| ------------------------------ | ------------------------------------------------------------------------------------------------------------------------------ |
| Cycle 960 of lines 0–159       | the line is rendered; enabled affine reference points advance; HBlank DMA is requested; HBlank IRQ if enabled                  |
| Cycle 960 of lines 160–227     | HBlank IRQ if enabled                                                                                                          |
| Cycle 1006 of every line       | HBlank flag set                                                                                                                |
| Start of line 160              | VBlank flag set; VBlank IRQ if enabled; VBlank DMA is requested                                                                |
| Start of the line equal to LYC | VCOUNT-match IRQ if enabled                                                                                                    |
| Wrap from line 227 to line 0   | affine reference points reload; the frame is complete                                                                          |

The HBlank IRQ fires on all 228 lines, as GBATEK states; the
`probe-display-irq` probe counts 456 HBlank, 2 VBlank and 2 VCOUNT IRQs over
two frames. HBlank DMA fires only on visible lines
([`timers-dma-interrupts.md`](timers-dma-interrupts.md)).

`DISPSTAT` (`04000004h`) and `VCOUNT` (`04000006h`,
[LCD I/O Interrupts and Status][gbatek-dispstat]):

| Bits | Field                                       | ZiGBA                                       |
| ---- | ------------------------------------------- | ------------------------------------------- |
| 0    | VBlank flag, read-only                      | set on lines 160–227                        |
| 1    | HBlank flag, read-only                      | set from cycle 1006 to the end of every line |
| 2    | VCOUNT match, read-only                     | set while `VCOUNT` equals LYC               |
| 3–5  | VBlank, HBlank and VCOUNT-match IRQ enables | stored                                      |
| 6–7  | unused                                      | not writable (write mask `FF38h`)           |
| 8–15 | LYC, the VCOUNT compare value               | stored                                      |

`VCOUNT` reads the current line, 0–227; writes are ignored.

The HBlank flag stays clear for 1,006 cycles of each line although drawing
takes 960, as GBATEK states; the IRQ and HBlank DMA still come at cycle 960.
The mGBA suite's "H-blank bit start" test, recorded on hardware, times the
flag's edges with polling loops: its fourth to sixth flip intervals match
for a rise at cycle 1006 or later and not for any earlier cycle. Its other
checks time the flag from a `Halt` wake, so they include the BIOS IRQ path's
cost (see [compatibility](../../compatibility.md#hblank-flag)). ZiGBA raises
the flag with the IRQ at cycle 960; this is a deliberate difference
([`roadmap.md`](../../roadmap.md#mgba-suite-rom-findings)), pinned by
"hblank flag rises at cycle 1006 after the hblank irq" in
[`tests/bus.rs`](../../../systems/gba/tests/bus.rs). GBATEK also clears the
VBlank flag again on line 227, which the core does not model; pinned mGBA
0.10.5 agrees in outline, using a 1,008-cycle drawing period and clearing
the VBlank flag at line 227.

`PpuTiming::cycles_until_next_event` gives the bus the next boundary that
delivers an event: every visible HBlank, VBlank entry, the frame wrap, and,
while their IRQs are enabled, each VBlank-line HBlank and the VCOUNT-match
line. The bus never advances past such a boundary in one step, so a large
time advance cannot reorder a scanline render, an affine reference update or
a frame reload relative to timers, DMA or input. Other boundaries only move
the position from which `DISPSTAT` and `VCOUNT` reads are derived.

## Display control and background modes

`DISPCNT` (`04000000h`, [LCD I/O Display Control][gbatek-dispcnt]):

| Bits  | Field                                                               |
| ----- | ------------------------------------------------------------------- |
| 0–2   | BG mode                                                             |
| 3     | CGB mode; reserved on the GBA and ignored by ZiGBA                  |
| 4     | Bitmap frame select (Modes 4 and 5)                                 |
| 5     | HBlank interval free; shortens the OBJ budget ([Objects](#objects)) |
| 6     | OBJ character mapping: 0 = 2D, 1 = 1D                               |
| 7     | Forced blank                                                        |
| 8–12  | BG0, BG1, BG2, BG3 and OBJ display enables                          |
| 13–15 | WIN0, WIN1 and OBJ-window enables                                   |

The BIOS boot calls `RegisterRamReset`, which leaves `DISPCNT` at `0080h`
(forced blank); ZiGBA's HLE boot starts in the same state. A forced-blank
line is white (`7FFFh`), as pinned by `probe-forceblank`.

| Mode | BG0  | BG1  | BG2                                       | BG3    |
| ---- | ---- | ---- | ----------------------------------------- | ------ |
| 0    | text | text | text                                      | text   |
| 1    | text | text | affine                                    | –      |
| 2    | –    | –    | affine                                    | affine |
| 3    | –    | –    | 240×160 direct color, one frame           | –      |
| 4    | –    | –    | 240×160 8-bit palette indices, two frames | –      |
| 5    | –    | –    | 160×128 direct color, two frames          | –      |

GBATEK lists Modes 6 and 7 as prohibited; ZiGBA draws only the backdrop in
them. The bitmap modes need the BG2 enable bit like any other layer. Their
second frame starts at VRAM offset `A000h`.

`BGxCNT` (`04000008h`–`0400000Eh`, [LCD I/O BG Control][gbatek-bgcnt]):

| Bits  | Field                                                                         |
| ----- | ----------------------------------------------------------------------------- |
| 0–1   | Priority, 0 highest                                                           |
| 2–3   | Character base, in 16 KiB units                                               |
| 6     | Mosaic enable                                                                 |
| 7     | Color depth: 0 = 4 bits per pixel with 16 palette banks, 1 = 8 bits per pixel |
| 8–12  | Screen (map) base, in 2 KiB units                                             |
| 13    | Affine display-area overflow: 0 = transparent, 1 = wrap                       |
| 14–15 | Screen size                                                                   |

In Mode 4 the framebuffer byte is a palette index and index zero is
transparent: it does not take part in priority resolution, so a
lower-priority OBJ shows through and the backdrop stays available as a
color-effects second target. Modes 3 and 5 hold direct BGR555 pixels, so a
zero pixel is opaque black
([GBATEK Mode 4][gbatek-bitmap];
mGBA tests the index before composition in
[`software-bg.c`][mgba-software-bg]). Pixels outside the Mode 5 bitmap are
transparent and show the backdrop. The tests "Mode 4 index zero lets a
lower-priority OBJ show through but Mode 3 color zero is opaque" and
"Mode 5 renders its 160-pixel bitmap and uses backdrop color outside it" in
[ZiGBA's `bus_test.zig`](../../../refs/zigba/test/bus_test.zig) pin these rules; the
`probe-bitmap3`, `probe-bitmap4` and `probe-bitmap5` probes check whole
frames.

The undocumented green-swap register `GREENSWP` (`04000002h`) bit 0
exchanges the green components of each horizontal pixel pair after color
effects (`probe-greenswap`).

## Text backgrounds

A text background is a map of 256 or 512 dots in each direction, built from
32×32-entry screen blocks of 2 KiB and scrolled by the write-only 9-bit
`BGxHOFS`/`BGxVOFS` registers; the map wraps in both directions
(`probe-scrollwrap`). Each 16-bit map entry holds a 10-bit tile number, a
horizontal flip (bit 10), a vertical flip (bit 11) and, in 4-bit mode, a
palette bank (bits 12–15; ignored in 8-bit mode, `probe-text8`). Tiles are 32
bytes (4-bit) or 64 bytes (8-bit) from the character base, and color index
zero is transparent.

### Fetches past the BG side of VRAM

Tile numbers can address character data past `10000h`: 4-bit tiles from
character base 3, and 8-bit tiles from bases 1–3, reach up to `1BFFFh`. Text
and affine maps in the last screen blocks reach `117FFh` and `137FFh`. In the
tiled modes that range belongs to the OBJ engine. NanoBoyAdvance's
[hardware test][nba-hw-test] `vram-mirror`, which passes on GBA and GBA SP,
finds that accesses above `06010000h` always go through the OBJ engine, and
GBATEK's "Using Invalid Tile Numbers"
([GBA Unpredictable Things][gbatek-unpredictable]) says the BG engine never
shows OBJ data there. GBATEK describes what it shows instead:

- On GBA and GBA SP, with one BG enabled, such tiles look as if filled with
  their own 16-bit map entry (vertical stripes). With several layers the data
  "is somehow derived from the other layers", depending on priority order and
  scrolling.
- On the NDS in GBA mode, they are zero-filled (transparent).

The commercial game Monster Gate (J) shows spurious dashes when an emulator
reads OBJ VRAM for these fetches, and both mGBA and NanoBoyAdvance changed
their models because of it. Pinned mGBA 0.10.5 draws whole invalid tiles
transparent but still reads OBJ VRAM for partial 4-bit tiles and for map
entries. NanoBoyAdvance returns a shared latch holding the last halfword the
BG engine fetched.

ZiGBA reads every BG fetch at or past the mode's OBJ boundary (`10000h`, or
`14000h` in the bitmap modes) as zero: transparent character data and map
entry zero (`Bus::vram_byte_for`, `Bus::decode_text_row`). That is the NDS
behavior. It is chosen over the GBA stripes because the multi-layer case is
unmeasured and reproducing it would need the hardware's BG fetch order on
affected lines. Map fetches past `10000h` have no documented behavior at
all. The tests "text tiles past the BG side of VRAM are transparent" and
"text map entries past the BG side of VRAM read as zero" pin the rule.

## Affine backgrounds

BG2 in Modes 1–2 and BG3 in Mode 2 are affine: a square map of 128 to 1,024
dots (`128 << size`) of one-byte tile numbers, always 8 bits per pixel. The
parameters `BGxPA`–`BGxPD` are signed 8.8 fixed-point values and the reference
point `BGxX`/`BGxY` is a signed 28-bit value with 8 fraction bits
([LCD I/O BG Rotation/Scaling][gbatek-affine]). For screen dot x the source
coordinate is the line origin plus `x·PA` horizontally and `x·PC` vertically.
A source outside the map is transparent unless `BGxCNT` bit 13 selects
wrapping, which reduces it modulo the map size, negative coordinates
included.

The line origin comes from hidden internal reference registers, not from
`BGxX/Y + PB/PD · VCOUNT`:

- A write to either half of a reference point immediately copies the complete
  sign-extended 28-bit value into the internal register, so it becomes the
  origin of the current line (GBATEK: "the new value specifies the origin of
  the current scanline").
- After each visible line is rendered, each enabled affine layer adds PB and
  PD to its internal registers. A disabled layer does not advance.
- With vertical mosaic, the internal registers hold across each mosaic group
  and then advance by the whole group height.
- At the frame wrap the programmed reference points reload. GBATEK copies
  them during VBlank; since a write during VBlank also copies immediately,
  the two are indistinguishable on visible lines.

The bitmap modes use the same BG2 path, so bitmaps can be rotated and scaled
too. This follows GBATEK's internal-reference description and is
cross-checked against pinned mGBA and NanoBoyAdvance. The probes
`probe-affinerot` (Mode 2, 90-degree rotation), `probe-affinewrap` (the same
with wrapping) and `probe-mode1` check whole frames; the tests "mid-frame
affine reference write becomes the next scanline origin" and "disabled affine
background does not advance its hidden reference" pin the register rules.

## Objects

OAM holds 128 entries of three attributes plus 32 interleaved affine
parameter groups ([LCD OBJ - OAM Attributes][gbatek-oam]):

| Attribute | Bits  | Field                                                                             |
| --------- | ----- | --------------------------------------------------------------------------------- |
| 0         | 0–7   | Y, wrapping at 256                                                                |
| 0         | 8     | Affine                                                                            |
| 0         | 9     | Double size (affine) or hidden (regular)                                          |
| 0         | 10–11 | Mode: normal, semi-transparent, OBJ window; 3 is prohibited and never drawn       |
| 0         | 12    | Mosaic                                                                            |
| 0         | 13    | 8 bits per pixel                                                                  |
| 0         | 14–15 | Shape: square, wide, tall; 3 is prohibited and never drawn                        |
| 1         | 0–8   | X, wrapping at 512                                                                |
| 1         | 9–13  | Affine parameter group (affine), or bits 12/13 horizontal/vertical flip (regular) |
| 1         | 14–15 | Size                                                                              |
| 2         | 0–9   | Base tile number                                                                  |
| 2         | 10–11 | Priority relative to BGs                                                          |
| 2         | 12–15 | Palette bank (4-bit objects)                                                      |

Objects draw from OBJ VRAM at `10000h` with the OBJ palette (entries
256–511). An object whose X lies past the right edge wraps to the left edge
when X plus its width reaches 512; its off-screen left part is clipped.

### Evaluation budget

The OBJ engine walks OAM in order with a fixed per-line budget: 1,210 cycles
normally, or 954 when `DISPCNT` bit 5 frees HBlank for OAM access. A regular
object costs one cycle per processed pixel of its width; an affine object
costs ten setup cycles plus two per processed pixel of its display width,
which is doubled for double-size objects. Every object that is vertically
active on the line consumes budget, even when all its pixels lie past the
right edge. When the budget ends inside an object, only its processed
left-to-right prefix is drawn and every later OAM entry is absent. For an
object wrapped in from the left, processing starts at the screen edge.

The constants and cost formulas are GBATEK's and match pinned mGBA's budget
model; NanoBoyAdvance's cycle-level renderer corroborates the partial
left-to-right processing and the left-edge clipping. ZiGBA computes the
admitted objects once per line in `Bus::build_object_line`. The test
"HBlank-free OBJ budget partially draws the final evaluated object" pins both
budgets.

### Tile addressing

OBJ character addressing is a 10-bit tile-number operation
([LCD OBJ - VRAM Character Mapping][gbatek-obj-mapping]); 8-bit objects
ignore the low bit of the base tile.

- In 1D mapping the tile row stride is the object's width in tiles, doubled
  for 8-bit objects, and the resulting name wraps at 1,024.
- In 2D mapping the character space is a 32-column matrix. The five column
  bits wrap on their own, so crossing column 31 does not carry into the row;
  8-bit objects advance the column by two names per tile.

In the bitmap modes the framebuffer occupies the lower 16 KiB of OBJ VRAM, so
GBATEK says tile numbers 0–511 are unavailable ([OAM attributes][gbatek-oam]).
ZiGBA applies that to each resolved tile fetch (`Bus::object_tile_number` and
the check after it): a tile below 512 is transparent, while the rest of the
object still draws. NanoBoyAdvance's mode-dependent OBJ VRAM boundary
([`ppu.hh`][nba-ppu-hh]) and SkyEmu's per-tile guard ([`gba.h`][skyemu-obj])
behave the same way. Pinned mGBA instead rejects the whole object when its
base tile is below 512 ([`software-obj.c`][mgba-obj-bitmap]), which would
also hide the valid tile-512 part of an object starting at tile 511. Tests
"bitmap modes suppress each OBJ tile below character name 512" and "OBJ
character names wrap at 1024 in 1D mapping and within the row in 2D mapping"
pin the addressing.

### Affine and double-size objects

An affine object samples its texture through the 8.8 matrix of its parameter
group, about the center of its display area: for display offset (dx, dy) from
that center, the source is `(PA·dx + PB·dy, PC·dx + PD·dy)` shifted right by
8, plus half the object size. Sources outside the object are transparent. A
double-size object has twice the display width and height, so an unscaled
texture sits centered in it (`probe-objdouble`). `probe-objrot` checks a
90-degree rotation, including a transparent edge column, and agrees with
pinned mGBA pixel for pixel. `probe-obj` and `probe-objflip` cover overlap,
palette-zero transparency and flips.

## Priority and second targets

Pixels are resolved in this order:

1. BGs are composed front to back: priority 0 first, and at equal priority
   the lower BG number first. The first opaque pixel on a dot is the top
   surface.
2. Objects are composed from the highest OAM number down. An OBJ pixel with a
   BG-relative priority numerically greater than the BG on top of that dot
   goes behind it; at equal priority the OBJ is in front. An OBJ pixel that
   lands on another OBJ pixel always replaces it, so the lower OAM number
   wins every OBJ-to-OBJ overlap regardless of priority, following GBATEK's
   statement that OBJ0 always has priority above OBJ1–127
   ([OAM attributes][gbatek-oam]).
3. The backdrop (palette entry 0) is the surface of last resort.

For color effects the renderer also keeps, per dot, the next visible surface
under the top one. Only the top-most OBJ pixel takes part in effects, so
there is no OBJ-to-OBJ blending: when one OBJ replaces another, the non-OBJ
surface beneath both is kept as the second surface. An OBJ hidden behind the
top BG is kept as the second surface when it is the next visible one, so
BG-over-OBJ alpha blending works. The tests "lower OAM index wins overlapping
OBJ regardless of BG-relative priority", "lower overlapping OBJ is excluded
from the color-effects second target" and "BLDALPHA uses an OBJ behind the
top BG as the second visible target" pin these rules.

Pinned mGBA and NanoBoyAdvance resolve the OBJ layer differently: per dot they
keep the OBJ with the numerically lowest priority field, OAM order breaking
ties, and a transparent pixel of a better-priority object still updates the
stored priority. The two models differ whenever overlapping objects have
different priorities (see [Open questions](#open-questions)).

## Windows

A window selects a per-dot control mask; it is not a color layer
([LCD I/O Window Feature][gbatek-window]). The regions are WIN0, WIN1, the
OBJ window and outside, with precedence WIN0 > WIN1 > OBJ window > outside.
The precedence only chooses which mask applies; the pixels that survive it
are resolved by ordinary priority, transparency and OAM order. mGBA builds
WIN1 splits before overlaying WIN0 in the same way
([`video-software.c`][mgba-window-split]).

| Register         | Address                  | Layout                                                                 |
| ---------------- | ------------------------ | ---------------------------------------------------------------------- |
| `WIN0H`, `WIN1H` | `04000040h`, `04000042h` | bits 0–7 right edge X2 (exclusive), bits 8–15 left edge X1 (inclusive) |
| `WIN0V`, `WIN1V` | `04000044h`, `04000046h` | bits 0–7 bottom Y2 (exclusive), bits 8–15 top Y1 (inclusive)           |
| `WININ`          | `04000048h`              | bits 0–5 WIN0 control, bits 8–13 WIN1 control                          |
| `WINOUT`         | `0400004Ah`              | bits 0–5 outside control, bits 8–13 OBJ-window control                 |

Each 6-bit control enables BG0–BG3 (bits 0–3), OBJ (bit 4) and color
effects (bit 5). Bits 6–7 of each byte are unused; ZiGBA, like mGBA, masks
`WININ` and `WINOUT` with `3F3Fh`. A layer is visible only when both its
`DISPCNT` enable and the winning region's layer bit are set. The effects bit
gates the effects `BLDCNT` selects; it does not choose targets or change
priority (test "window color-effect bit gates brightness independently of
its layer mask").

With none of `DISPCNT` bits 13–15 set, windows restrict nothing. An OBJ in OBJ
window mode contributes to the mask wherever its texel is non-zero and draws
no color of its own; its palette and priority are ignored, and both
`DISPCNT` bit 12 (OBJ enable) and bit 15 are required. A dot already inside
WIN0 or WIN1 keeps that control. This matches mGBA's OBJ-window flag
([`software-obj.c`][mgba-objwin]); `probe-objwin` and `probe-window` check
whole frames.

For ordinary ranges the rectangle is `[X1, X2) × [Y1, Y2)`. The sources
disagree on the rest. GBATEK says X2 > 240 or X1 > X2 is treated as X2 = 240,
and likewise Y2 > 160 or Y1 > Y2 as Y2 = 160. Pinned mGBA clamps off-screen
edges but splits an in-range `X2 < X1` interval across both screen edges,
`[X1, 240) ∪ [0, X2)`, and the same vertically. ZiGBA follows mGBA
(`Bus::coordinate_in_window`): equal edges give an empty window, an edge past
the screen clips, and a right edge left of the left edge wraps. The mGBA
suite's visual case ["Window offscreen reset"][suite-window-reset] does not
cover every inverted combination, so the test "clipped, equal and wrapped
window bounds keep BG2 visible on every pixel" records ZiGBA's choice rather
than settling it.

## Color special effects

`BLDCNT` (`04000050h`) selects first targets in bits 0–5 (BG0–BG3, OBJ,
backdrop), the effect in bits 6–7 (none, alpha blend, brightness increase,
brightness decrease) and second targets in bits 8–13. `BLDALPHA`
(`04000052h`) holds EVA and EVB and `BLDY` (`04000054h`) holds EVY; each
coefficient saturates at 16 ([LCD I/O Color Special Effects][gbatek-effects]).
Per 5-bit color component, with integer truncation:

| Effect              | Result                                 |
| ------------------- | -------------------------------------- |
| Alpha blend         | `min(31, (top·EVA + second·EVB) / 16)` |
| Brightness increase | `top + (31 − top)·EVY / 16`            |
| Brightness decrease | `top − top·EVY / 16`                   |

An effect applies only where the window control's effects bit is set and the
top surface's layer is a first target. Alpha blending also requires the
second surface's layer to be a second target; otherwise the top pixel shows
unchanged. The `probe-blend`, `probe-brighten` and `probe-darken` probes
check the equations' results over full frames.

A semi-transparent OBJ (mode 1) forces itself to be an alpha first target
regardless of `BLDCNT`'s OBJ first-target bit and effect mode; only the
top-most OBJ pixel takes part, so objects never blend with each other
(`probe-objsemi`). When the surface beneath is not a second target, ZiGBA
shows the OBJ pixel unchanged and applies no brightness effect to it.

## Mosaic

`MOSAIC` (`0400004Ch`) holds four 4-bit sizes, each the group size minus one:
BG horizontal (bits 0–3), BG vertical (4–7), OBJ horizontal (8–11) and OBJ
vertical (12–15), for groups of 1 to 16 dots. Backgrounds opt in with
`BGxCNT` bit 6 (the bitmap modes with `BG2CNT`) and objects with attribute 0
bit 12. A size of zero leaves every group one dot wide
([LCD I/O Mosaic Function][gbatek-mosaic]).

Mosaic lives inside the scanline renderer, not in a post-processing filter.
Each participating layer quantizes screen coordinates before scrolling, the
affine transform, tile lookup, transparency, windowing, priority and color
effects, so a group keeps the layer and transparency of its upper-left source
dot:

- Groups align to screen coordinates, not to scroll, tiles or an object's
  origin. Text layers quantize screen X/Y and then add HOFS/VOFS, so a group
  can cross tile and screen-block edges without changing its texel.
- Affine and bitmap layers quantize screen X/Y before applying the reference
  point and matrix.
- Objects convert the same global grid to local coordinates before flips or
  the matrix. A group that begins left of the object clamps to its first
  source column. At the right edge the last sample extends through the rest
  of the global group, matching mGBA's rounded object end. Vertical
  visibility stays the object's ordinary line range, and the sampled row
  clamps at the top edge.
- The OBJ-window pass quantizes the source row vertically but samples its
  mask horizontally without mosaic, following mGBA.

Sources: GBATEK's `MOSAIC` register and upper-left sampling rule; pinned mGBA
[text grouping][mgba-mosaic-text], [affine/bitmap setup][mgba-mosaic-affine]
and [object grouping][mgba-mosaic-obj]. `probe-mosaic` and `probe-objmosaic`
check full frames; tests such as "affine background mosaic quantizes screen
coordinates before matrix sampling" and "OBJ-window mosaic quantizes source
rows but not horizontal mask samples" cover the ordering rules.

## Raster effects

Games change video registers and memory while a line is being drawn: scroll
splits, palette changes, window moves and HBlank DMA tables. ZiGBA renders a
whole line at its HBlank, so every mid-line change it models is recorded
with the dot at which it takes effect.

**Dot policy.** A write that completes at cycle c of a visible line (c < 960)
takes effect from dot `c / 4`: that dot and every later one on the line see
the new value, earlier dots the old. A write during HBlank or VBlank applies
to the whole of the next drawn line. The exact phase of the LCD's fetches
relative to a completing CPU write is unmeasured, so this mapping is a
policy, not a hardware claim.

Each kind of change keeps its own per-line log of split points (`Bus`
fields `scroll_segments`, `backdrop_segments`, `win_segments`,
`blend_segments`, `affine_segments` and `mosaic_segments`). Each log holds 64
entries per line (per background for scroll and affine); palette and BG-VRAM
changes share one log, where a palette point takes one entry and a VRAM byte
point two. Writes beyond that bound are dropped from the log, and that line
renders them approximately.

| Changes at the dot of the write                                                 | Stays end-of-line state                                                 |
| ------------------------------------------------------------------------------- | ----------------------------------------------------------------------- |
| Palette RAM, all 512 entries, for backdrop, text, affine, bitmap and OBJ colors | OAM and OBJ VRAM                                                        |
| BG VRAM bytes below the OBJ boundary, for text, affine and bitmap fetches       | `DISPCNT` mode, frame select, OBJ mapping and HBlank-free bits          |
| `BGxHOFS`/`BGxVOFS`                                                             | `DISPCNT` BG and OBJ enables (bits 8–12) and OBJ-window enable (bit 15) |
| `BGxCNT` priority bits                                                          | other `BGxCNT` fields                                                   |
| `WIN0H/V`, `WIN1H/V`, `WININ`, `WINOUT`                                         | forced-blank release (bit 7 cleared)                                    |
| `DISPCNT` WIN0/WIN1 enables (bits 13–14)                                        | vertical mosaic sizes                                                   |
| `DISPCNT` forced-blank assertion (bit 7 set)                                    | `GREENSWP`                                                              |
| `BLDCNT`, `BLDALPHA`, `BLDY`                                                    |                                                                         |
| Horizontal `MOSAIC` sizes                                                       |                                                                         |
| BG2/BG3 `PA`, `PC` and reference points `BGxX/Y`                                |                                                                         |

"End-of-line" means the value when the line is rendered at cycle 960 applies
to the whole line. PB and PD only affect the per-line reference advance, so
a mid-line write to them has no dot to split at.

Details of the modeled cases:

- **16-bit video buses.** Palette RAM and VRAM have 16-bit buses, so a 32-bit
  CPU write commits its low and high halfwords on consecutive one-cycle
  beats. Each half becomes visible at its own completion dot, and the pair
  still costs two cycles; a DMA raised by either beat waits until the whole
  access completes. Tests "mid-scanline Mode 4 palette word write exposes
  each halfword at its completion dot" and "mid-scanline Mode 3 VRAM word
  write exposes each halfword at its completion dot" pin this.
- **Palette and VRAM together.** The two histories share one log but resolve
  independently (test "mixed mid-scanline palette and BG VRAM writes retain
  their independent histories").
- **Window enables.** A `DISPCNT` change to bits 13–14 takes effect at its dot
  (test "mid-scanline DISPCNT WIN0 enable affects only pixels at and after
  its completion").
- **Forced blank.** Setting bit 7 mid-line whitens the dots from the write
  onward (test "mid-scanline forced-blank assertion whites only later
  pixels"). Clearing it is not split: the line renders according to the
  value at its HBlank. GBATEK says the display resumes only after two
  vertical lines and marks that wording uncertain, and modeling the delay
  would need display-pipeline state ZiGBA does not keep.
- **BG priority.** A `BGxCNT` priority change reorders layers from its dot on
  (test "mid-scanline BGCNT priority write changes only later background
  pixels"). NanoBoyAdvance also synchronizes the PPU before an MMIO write and
  rebuilds its BG order from current priorities ([`bus.cc`][nba-bus],
  [`merge.cc`][nba-merge]); GBATEK defines priorities and the same-priority
  BG-number order but not the dot at which a write lands.
- **Affine writes.** A PA or PC write keeps the source coordinate accumulated
  under the old parameter and continues from it with the new gradient. A
  `BGxX/Y` write replaces the current line's origin, following GBATEK's
  internal-reference rule. The test "mid-scanline Mode 3 affine write
  preserves pixels rendered before completion" checks witnesses far from the
  split, which distinguish continuity from restarting the new matrix at
  x = 0. It accepts both ZiGBA's split at source x = 100 and
  NanoBoyAdvance's fetch-phase result at x = 108, because no adjacent-pixel
  affine boundary has been measured.

The `probe-midwin`, `probe-midbldy`, `probe-midaffine`, `probe-midmosaic` and
`probe-midcombo` probes (scroll, backdrop palette and window together) write
registers mid-line from real ROM code. Their split dot moves with CPU timing,
so they require a genuine split on the target line and exact pixels
elsewhere ([`probe/README.md`](../../../probe/README.md)). `probe-hdma` covers
the common HBlank DMA pattern: a scroll value written at line y's HBlank
applies to line y + 1.

## Presentation

The emulated framebuffer is raw BGR555 and is the only input to headless
framebuffer hashes and tests. Hosts may re-express it for display, for
example with color correction, without touching it.

## Open questions

- **VBlank flag edge and HBlank IRQ timing.** GBATEK and mGBA clear the
  VBlank flag on line 227; the core keeps it through line 227, and the mGBA
  suite does not time that edge. The HBlank IRQ and DMA request stay at cycle
  960, which no hardware test here pins against the flag's 1006.
- **OBJ-to-OBJ priority.** ZiGBA lets the lower OAM number win every OBJ
  overlap (GBATEK); mGBA and NanoBoyAdvance resolve by priority field first.
  ZiGBA also compares each OBJ only with the surface it lands on, so a
  lower-numbered OBJ whose priority loses to a BG still covers that BG when a
  higher-numbered OBJ covered it first. A hardware test with overlapping
  objects of different priorities over a BG would decide both.
- **Semi-transparent OBJ without a second target.** GBATEK says `BLDCNT` may
  still apply a brightness effect to a semi-transparent OBJ pixel that does
  not overlap a second target; ZiGBA leaves such a pixel unchanged.
- **Invalid BG tiles on GBA.** ZiGBA reads fetches past the BG side of VRAM
  as zero (NDS behavior). The GBA's striped single-layer output and its
  multi-layer latch behavior are unmodeled, and map fetches there are
  undocumented.
- **OBJ tile restriction in bitmap modes.** Per-fetch (ZiGBA, NanoBoyAdvance,
  SkyEmu) and whole-object (mGBA) rejection differ for an object starting at
  tile 511; a hardware boundary test would decide.
- **Inverted window coordinates.** GBATEK's clamp and mGBA's split (which
  ZiGBA follows) disagree for `X1 > X2`, `Y1 > Y2` and edges past the screen.
- **Mid-line write phase.** The `c / 4` dot policy and the affine split point
  are unmeasured; NanoBoyAdvance's fetch-phase model lands affine splits
  8 dots later.
- **Mid-line changes kept at end of line.** OAM and OBJ VRAM writes, BG and
  OBJ enables, other `BGxCNT` fields, vertical mosaic and forced-blank
  release are not split mid-line; GBATEK's two-line delay after forced blank
  is not modeled.
- **Video memory contention.** CPU accesses to video memory that collide
  with LCD fetches cost no extra cycle in ZiGBA
  ([`memory.md`](memory.md) lists this with the other bus questions).
- **Mosaic corners.** Hardware tests for rare OBJ-window and wrapped-coordinate
  mosaic combinations are missing.

[gbatek-timings]: https://rust-console.github.io/gbatek-gbaonly/#lcddimensionsandtimings
[gbatek-dispstat]: https://rust-console.github.io/gbatek-gbaonly/#lcdiointerruptsandstatus
[gbatek-dispcnt]: https://rust-console.github.io/gbatek-gbaonly/#lcdiodisplaycontrol
[gbatek-bgcnt]: https://rust-console.github.io/gbatek-gbaonly/#lcdiobgcontrol
[gbatek-bitmap]: https://rust-console.github.io/gbatek-gbaonly/#lcdvrambitmapbgmodes
[gbatek-unpredictable]: https://rust-console.github.io/gbatek-gbaonly/#gbaunpredictablethings
[gbatek-affine]: https://rust-console.github.io/gbatek-gbaonly/#lcdiobgrotationscaling
[gbatek-oam]: https://rust-console.github.io/gbatek-gbaonly/#lcdobjoamattributes
[gbatek-obj-mapping]: https://rust-console.github.io/gbatek-gbaonly/#lcdobjvramcharactertilemapping
[gbatek-window]: https://rust-console.github.io/gbatek-gbaonly/#lcdiowindowfeature
[gbatek-effects]: https://rust-console.github.io/gbatek-gbaonly/#lcdiocolorspecialeffects
[gbatek-mosaic]: https://rust-console.github.io/gbatek-gbaonly/#lcdiomosaicfunction
[mgba-software-bg]: https://github.com/mgba-emu/mgba/blob/0.10.5/src/gba/renderers/software-bg.c#L146-L180
[mgba-obj-bitmap]: https://github.com/mgba-emu/mgba/blob/0.10.5/src/gba/renderers/software-obj.c#L157-L164
[mgba-objwin]: https://github.com/mgba-emu/mgba/blob/0.10.5/src/gba/renderers/software-obj.c#L1251-L1317
[mgba-window-split]: https://github.com/mgba-emu/mgba/blob/0.10.5/src/gba/renderers/video-software.c#L2963-L2998
[mgba-mosaic-text]: https://github.com/mgba-emu/mgba/blob/26b7884bc25a5933960f3cdcd98bac1ae14d42e2/src/gba/renderers/software-mode0.c#L425-L435
[mgba-mosaic-affine]: https://github.com/mgba-emu/mgba/blob/26b7884bc25a5933960f3cdcd98bac1ae14d42e2/src/gba/renderers/software-private.h#L171-L190
[mgba-mosaic-obj]: https://github.com/mgba-emu/mgba/blob/26b7884bc25a5933960f3cdcd98bac1ae14d42e2/src/gba/renderers/software-obj.c#L8-L70
[nba-hw-test]: https://github.com/nba-emu/hw-test
[nba-ppu-hh]: https://github.com/nba-emu/NanoBoyAdvance/blob/55b5cf0ae3d929582ac5bfd486558173502b8354/src/nba/src/hw/ppu/ppu.hh#L59-L60
[nba-bus]: https://github.com/nba-emu/NanoBoyAdvance/blob/55b5cf0ae3d929582ac5bfd486558173502b8354/src/nba/src/bus/bus.cc
[nba-merge]: https://github.com/nba-emu/NanoBoyAdvance/blob/55b5cf0ae3d929582ac5bfd486558173502b8354/src/nba/src/hw/ppu/merge.cc
[skyemu-obj]: https://github.com/skylersaleh/SkyEmu/blob/01516d6798e3652b583e6a366085bb51c43b528d/src/gba.h#L2047-L2056
[suite-window-reset]: https://github.com/mgba-emu/suite/blob/e6942030d25ffe3ba76c72b73a86da073ec857cc/src/video.c#L376-L403
