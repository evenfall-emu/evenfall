# Video

The GBA's video chip draws a 240×160 picture about 60 times a second.
Outside its bitmap modes it keeps no picture in memory.
It builds each line on the fly from a few layers: up to four backgrounds, up to 128 sprites, and a backdrop color behind everything.
Windows can hide layers in parts of the screen, and color effects can blend or fade them.

Games lean on the fact that the picture is built line by line.
By changing a register between two lines, or even in the middle of one, they make water ripple, fake 3D floors and curved roads, and split the screen into a status bar and a scrolling field.
These tricks are called raster effects, and an emulator has to reproduce them exactly.

## A few terms

- **Dot.** One pixel position on a line.
  The chip spends four master-clock cycles per dot.
- **Scanline** or **line.** One row of the picture.
  Lines 0–159 are visible, lines 160–227 are the invisible vertical blank.
- **HBlank and VBlank.** The pause after each visible line (horizontal blank), and the 68 invisible lines after the last one (vertical blank).
  Games do most of their video updates during these pauses, when nothing is being drawn.
- **Tile.** An 8×8 block of pixels.
  Backgrounds and sprites are made of tiles stored in video memory (VRAM).
- **Map.** A grid of tile numbers that lays tiles out into a background.
- **Palette.** A table of 15-bit colors in palette RAM.
  Tiles store color indices into it, and index 0 means transparent.
- **Object** or **sprite.** A movable image made of tiles.
  Their positions and settings live in object attribute memory (OAM).
- **Priority.** A number from 0 (front) to 3 (back) that decides which layer covers which.

## How Evenfall draws a frame

Evenfall draws one whole line at a time, at the moment that line's HBlank begins.
[`video/timing.rs`](../../../systems/gba/src/video/timing.rs) (`PpuTiming`) tracks which line and dot the display is on, and [`video/render.rs`](../../../systems/gba/src/video/render.rs) draws the line from the registers in force at that moment.

That alone would miss raster effects inside a line.
So while a line is being scanned, [`video/history.rs`](../../../systems/gba/src/video/history.rs) records every change to the registers and memory that matter, with the dot where it lands, and the renderer replays those changes as it draws ([Raster effects](#raster-effects)).

The result is a 240×160 frame of raw 15-bit colors.
Any color correction happens afterwards in the host ([Presentation](#presentation)).

The fast renderers draw several pixels at once.
Simpler per-pixel reference renderers stay in the tree, and tests compare the two.
The [tile-row cache](../../../systems/gba/src/video/tile_row_cache.rs) and the [scanline memo](../../../systems/gba/src/video/scanline_memo.rs) speed things up without changing a single pixel ([research notes](../../research/README.md)).

## Display timing

GBATEK gives the frame geometry ([LCD Dimensions and Timings][gbatek-timings]):

| Quantity     | Value                                                                      |
| ------------ | -------------------------------------------------------------------------- |
| Visible line | 240 dots, 960 cycles                                                       |
| HBlank       | 272 cycles                                                                 |
| Line         | 1,232 cycles                                                               |
| Frame        | 160 visible + 68 VBlank lines = 228 lines, 280,896 cycles (about 59.73 Hz) |

Evenfall produces these events:

| When                           | What happens                                                                                                             |
| ------------------------------ | ------------------------------------------------------------------------------------------------------------------------ |
| Cycle 960 of lines 0–159       | the HBlank flag is set, the line is drawn, affine backgrounds step to the next line, HBlank DMA starts, HBlank interrupt |
| Cycle 960 of lines 160–227     | the HBlank flag is set, HBlank interrupt                                                                                 |
| Start of line 160              | the VBlank flag is set, VBlank interrupt, VBlank DMA starts                                                              |
| Start of the line equal to LYC | VCOUNT-match interrupt                                                                                                   |
| Wrap from line 227 to line 0   | affine backgrounds reload their starting point, and the frame is complete                                                |

Each interrupt fires only if the game enabled it.
The HBlank interrupt fires on all 228 lines, as GBATEK says, while HBlank DMA runs only on visible lines ([`timers-dma-interrupts.md`](timers-dma-interrupts.md)).

`DISPSTAT` (`04000004h`) and `VCOUNT` (`04000006h`) let games see where the display is ([LCD I/O Interrupts and Status][gbatek-dispstat]):

| Bits | Field                                      | In Evenfall                                 |
| ---- | ------------------------------------------ | ------------------------------------------- |
| 0    | VBlank flag, read-only                     | set on lines 160–227                        |
| 1    | HBlank flag, read-only                     | set from cycle 960 to the end of every line |
| 2    | VCOUNT match, read-only                    | set while `VCOUNT` equals LYC               |
| 3–5  | VBlank, HBlank and VCOUNT-match interrupts | stored                                      |
| 6–7  | unused                                     | not writable (write mask `FF38h`)           |
| 8–15 | LYC, the line to match                     | stored                                      |

`VCOUNT` reads the current line, 0–227, and ignores writes.

GBATEK describes two edges Evenfall doesn't model.
The HBlank flag stays clear for 1,006 cycles of each line, although drawing takes 960.
The VBlank flag clears again on line 227.
mGBA 0.10.5 agrees in outline.
Evenfall raises the HBlank flag, interrupt and DMA together at cycle 960 and keeps the VBlank flag set through line 227.
No available hardware test pins either edge.

`PpuTiming::cycles_until_next_event` tells the bus when the next event is due.
The bus never skips past one in a single step, so however far the host advances time, a line is always drawn before the timers, DMA or input that come after it.

## Display control and background modes

`DISPCNT` (`04000000h`) chooses what the display shows ([LCD I/O Display Control][gbatek-dispcnt]):

| Bits  | Field                                                                               |
| ----- | ----------------------------------------------------------------------------------- |
| 0–2   | background mode                                                                     |
| 3     | Game Boy Color mode, reserved on the GBA and ignored                                |
| 4     | which of two bitmap frames to show (modes 4 and 5)                                  |
| 5     | frees HBlank for OAM access, which shortens the sprite budget ([Objects](#objects)) |
| 6     | sprite tile layout: 0 = 2D, 1 = 1D                                                  |
| 7     | forced blank                                                                        |
| 8–12  | enable BG0, BG1, BG2, BG3 and sprites                                               |
| 13–15 | enable window 0, window 1 and the sprite window                                     |

At boot the BIOS leaves `DISPCNT` at `0080h`, forced blank, and Evenfall's HLE boot does the same.
A forced-blank line is white (`7FFFh`).

The mode decides what kind of backgrounds are available:

| Mode | BG0  | BG1  | BG2                                       | BG3    |
| ---- | ---- | ---- | ----------------------------------------- | ------ |
| 0    | text | text | text                                      | text   |
| 1    | text | text | affine                                    | –      |
| 2    | –    | –    | affine                                    | affine |
| 3    | –    | –    | 240×160 direct color, one frame           | –      |
| 4    | –    | –    | 240×160 8-bit palette indices, two frames | –      |
| 5    | –    | –    | 160×128 direct color, two frames          | –      |

Text backgrounds scroll, affine backgrounds can also rotate and scale, and bitmap modes 3–5 let a game draw pixels directly, at the cost of the other layers.
GBATEK calls modes 6 and 7 prohibited, and Evenfall draws only the backdrop in them.
Bitmap modes still need the BG2 enable bit, and their second frame starts at VRAM offset `A000h`.

`BGxCNT` (`04000008h`–`0400000Eh`) configures each background ([LCD I/O BG Control][gbatek-bgcnt]):

| Bits  | Field                                                              |
| ----- | ------------------------------------------------------------------ |
| 0–1   | priority, 0 frontmost                                              |
| 2–3   | where its tiles start, in 16 KiB steps                             |
| 6     | mosaic                                                             |
| 7     | colors: 0 = 16 colors from one of 16 palette banks, 1 = 256 colors |
| 8–12  | where its map starts, in 2 KiB steps                               |
| 13    | affine only: outside the map, 0 = transparent, 1 = wrap around     |
| 14–15 | map size                                                           |

In mode 4 each pixel is a palette index, and index 0 is transparent.
A sprite behind the bitmap shows through, and the backdrop stays available for blending.
Modes 3 and 5 store colors directly, so a zero pixel is opaque black ([GBATEK Mode 4][gbatek-bitmap], mGBA's [`software-bg.c`][mgba-software-bg]).
Pixels outside mode 5's smaller bitmap are transparent and show the backdrop.

The undocumented register `GREENSWP` (`04000002h`) swaps the green parts of each horizontal pixel pair when bit 0 is set, after color effects.

## Text backgrounds

A text background is a map of 256 or 512 dots in each direction, built from 32×32-tile screen blocks of 2 KiB.
The write-only 9-bit `BGxHOFS` and `BGxVOFS` registers scroll it, and it wraps around at the edges.

Each 16-bit map entry holds:

- a 10-bit tile number,
- a horizontal flip (bit 10) and a vertical flip (bit 11),
- in 16-color mode, a palette bank (bits 12–15), ignored in 256-color mode.

A tile takes 32 bytes in 16-color mode and 64 bytes in 256-color mode.
Color index 0 is transparent.

### Tiles past the background part of VRAM

A tile number can point past VRAM offset `10000h`, where the sprite tiles live in the tiled modes.
For example, 16-color tiles from character base 3, and 256-color tiles from bases 1–3, reach up to `1BFFFh`.
What the hardware draws there is messy:

- NanoBoyAdvance's [`vram-mirror` hardware test][nba-hw-test], which passes on GBA and GBA SP, finds that accesses above `06010000h` always go to the sprite engine.
  GBATEK agrees that backgrounds never show sprite data there ([GBA Unpredictable Things][gbatek-unpredictable]).
- On GBA and GBA SP with one background enabled, such tiles look filled with their own map entry (vertical stripes).
  With several layers the data "is somehow derived from the other layers".
- On a Nintendo DS running GBA games, they are transparent.

This matters for at least one commercial game: Monster Gate (J) shows spurious dashes when an emulator reads sprite VRAM here, and mGBA and NanoBoyAdvance both changed their models because of it.
mGBA 0.10.5 draws whole invalid tiles transparent but still reads sprite VRAM for some cases.
NanoBoyAdvance returns the last halfword the background engine fetched.

Evenfall reads every background fetch at or past the mode's sprite boundary (`10000h`, or `14000h` in bitmap modes) as zero, which gives transparent tiles and map entry 0 (`Bus::vram_byte_for`, `Bus::decode_text_row`).
That is the DS behavior.
Evenfall picks it over the GBA's stripes because the multi-layer case has never been measured.

## Affine backgrounds

An affine background can be rotated, scaled and sheared.
BG2 in modes 1–2 and BG3 in mode 2 are affine.
Their map is a square of 128 to 1,024 dots (`128 << size`) of one-byte tile numbers, always in 256 colors.

A 2×2 matrix (`BGxPA`–`BGxPD`, signed 8.8 fixed point) and a reference point (`BGxX`, `BGxY`, signed 28-bit with 8 fraction bits) map screen dots to map coordinates ([LCD I/O BG Rotation/Scaling][gbatek-affine]).
For dot x on a line, the source is the line's origin plus `x * PA` horizontally and `x * PC` vertically.
A source outside the map is transparent, unless `BGxCNT` bit 13 asks for wrapping.

The line's origin comes from hidden internal registers, not from a formula over the line number.
This is what makes per-line effects like fake 3D floors possible:

- Writing either half of a reference point copies the whole value into the internal register at once, so it becomes the origin of the current line.
- After each visible line is drawn, each enabled affine layer adds PB and PD to its internal registers.
  A disabled layer doesn't advance.
- With vertical mosaic, the internal registers hold for a whole mosaic group and then advance by the group's height.
- When the frame wraps to line 0, the programmed reference points are loaded again.

Bitmap modes use the same BG2 path, so bitmaps can be rotated and scaled too.
This follows GBATEK, with mGBA and NanoBoyAdvance as cross-checks.

## Objects

OAM holds 128 sprite entries of three 16-bit attributes each, with 32 groups of affine parameters interleaved between them ([LCD OBJ - OAM Attributes][gbatek-oam]):

| Attribute | Bits  | Field                                                                          |
| --------- | ----- | ------------------------------------------------------------------------------ |
| 0         | 0–7   | Y, wrapping at 256                                                             |
| 0         | 8     | affine                                                                         |
| 0         | 9     | double size (affine) or hidden (regular)                                       |
| 0         | 10–11 | mode: normal, semi-transparent, sprite window; 3 is prohibited and never drawn |
| 0         | 12    | mosaic                                                                         |
| 0         | 13    | 256 colors                                                                     |
| 0         | 14–15 | shape: square, wide, tall; 3 is prohibited and never drawn                     |
| 1         | 0–8   | X, wrapping at 512                                                             |
| 1         | 9–13  | affine parameter group (affine), or bits 12 and 13 for flips (regular)         |
| 1         | 14–15 | size                                                                           |
| 2         | 0–9   | first tile number                                                              |
| 2         | 10–11 | priority relative to backgrounds                                               |
| 2         | 12–15 | palette bank (16-color sprites)                                                |

Sprites take their tiles from VRAM at `10000h` and their colors from the sprite half of palette RAM (entries 256–511).
A sprite whose X lies past the right edge wraps around to the left edge once X plus its width reaches 512, and its off-screen part is clipped.

### Sprites per line

The sprite engine can only do so much per line.
It walks OAM in order with a fixed budget of cycles: 1,210, or 954 when `DISPCNT` bit 5 frees HBlank for OAM access.

- A regular sprite costs one cycle per pixel of its width.
- An affine sprite costs ten setup cycles plus two per pixel of its displayed width, which is doubled for double-size sprites.
- Every sprite that covers the line uses budget, even when all its pixels are off the right edge.

When the budget runs out inside a sprite, only the left part it got through is drawn, and every later OAM entry is missing from that line.
For a sprite wrapped in from the left, the count starts at the screen edge.
The constants and formulas are GBATEK's and match mGBA.
NanoBoyAdvance's cycle-level renderer confirms the partial drawing.
Evenfall works out the admitted sprites once per line in `Bus::build_object_line`.

### Tile layout

Sprite tiles are addressed by a 10-bit tile number ([LCD OBJ - VRAM Character Mapping][gbatek-obj-mapping]).
256-color sprites ignore the lowest bit of their first tile number.

- In 1D layout, a sprite's tiles follow one another in memory, row after row, and the number wraps at 1,024.
- In 2D layout, sprite tiles form a grid 32 tiles wide.
  The column wraps on its own, so going past column 31 doesn't move to the next row.
  256-color sprites advance two tile numbers per tile.

In bitmap modes the frame buffer takes up the lower half of sprite VRAM, so GBATEK says tile numbers 0–511 are unavailable ([OAM attributes][gbatek-oam]).
Evenfall applies this per tile (`Bus::object_tile_number`): a tile below 512 is transparent, and the rest of the sprite still draws.
NanoBoyAdvance ([`ppu.hh`][nba-ppu-hh]) and SkyEmu ([`gba.h`][skyemu-obj]) do the same.
mGBA instead hides the whole sprite when its first tile is below 512 ([`software-obj.c`][mgba-obj-bitmap]), which also hides the valid part of a sprite that starts at tile 511.

### Affine and double-size sprites

An affine sprite samples its image through the 8.8 matrix of its parameter group, around the center of its display area.
For a dot at offset (dx, dy) from that center, the source pixel is `(PA * dx + PB * dy, PC * dx + PD * dy)` shifted right by 8, plus half the sprite's size.
Sources outside the sprite are transparent.
A double-size sprite gets twice the display width and height, so a rotated image isn't clipped by its own box.

## Priority and blending targets

Every dot ends up showing one surface, chosen in this order:

1. Backgrounds, front to back: priority 0 first, and at equal priority the lower background number first.
   The first non-transparent pixel is the top surface.
2. Sprites, from the highest OAM number down.
   A sprite pixel whose priority is numerically greater than the background on top goes behind it.
   At equal priority the sprite is in front.
   A sprite pixel that lands on another sprite pixel always replaces it, so the lower OAM number wins every overlap between sprites, whatever their priorities.
   This follows GBATEK: sprite 0 always has priority over sprites 1–127 ([OAM attributes][gbatek-oam]).
3. The backdrop (palette entry 0), when nothing else is there.

For blending, the renderer also keeps the next visible surface under the top one on each dot.
Only the frontmost sprite pixel takes part in effects, so sprites never blend with each other.
When one sprite covers another, the non-sprite surface under both is the second surface.
A sprite hidden behind the top background can be the second surface, so a background can blend over a sprite.

mGBA and NanoBoyAdvance resolve sprites differently.
On each dot they keep the sprite with the lowest priority number, with OAM order breaking ties, and a transparent pixel of a sprite still counts.
The two models disagree whenever overlapping sprites have different priorities ([Open questions](#open-questions)).

## Windows

A window doesn't draw anything.
It decides, dot by dot, which layers may appear and whether color effects apply ([LCD I/O Window Feature][gbatek-window]).
Games use windows for spotlights, for hiding part of the screen during a transition, or to keep a status bar free of effects.

There are four regions: window 0, window 1, the sprite window and outside.
Where they overlap, the first in that order decides.
The pixels that survive are then resolved by ordinary priority.
mGBA builds the regions the same way ([`video-software.c`][mgba-window-split]).

| Register         | Address                  | Layout                                                                 |
| ---------------- | ------------------------ | ---------------------------------------------------------------------- |
| `WIN0H`, `WIN1H` | `04000040h`, `04000042h` | bits 0–7 right edge X2 (exclusive), bits 8–15 left edge X1 (inclusive) |
| `WIN0V`, `WIN1V` | `04000044h`, `04000046h` | bits 0–7 bottom edge Y2 (exclusive), bits 8–15 top edge Y1 (inclusive) |
| `WININ`          | `04000048h`              | bits 0–5 window 0 settings, bits 8–13 window 1 settings                |
| `WINOUT`         | `0400004Ah`              | bits 0–5 outside settings, bits 8–13 sprite window settings            |

Each 6-bit setting enables BG0–BG3 (bits 0–3), sprites (bit 4) and color effects (bit 5).
Bits 6–7 of each byte are unused, and Evenfall masks `WININ` and `WINOUT` with `3F3Fh`, as mGBA does.
A layer shows only when both its `DISPCNT` enable and the region's bit for it are set.
The effects bit only allows the effect `BLDCNT` chooses.
It doesn't choose targets or change priority.

When none of `DISPCNT` bits 13–15 is set, windows hide nothing.

A sprite in sprite-window mode draws nothing itself.
It marks the window wherever its pixel isn't transparent, and its palette and priority are ignored.
It needs both `DISPCNT` bit 12 (sprites) and bit 15 (sprite window).
A dot already inside window 0 or 1 keeps that window's settings.
This matches mGBA ([`software-obj.c`][mgba-objwin]).

For ordinary edges the window is the rectangle `[X1, X2) × [Y1, Y2)`.
The sources disagree about the rest.
GBATEK says X2 > 240 or X1 > X2 counts as X2 = 240, and the same vertically with 160.
mGBA clips off-screen edges, but when X2 < X1 it wraps the window around the screen edges, `[X1, 240) ∪ [0, X2)`, and the same vertically.
Evenfall follows mGBA (`Bus::coordinate_in_window`): equal edges give an empty window, an edge past the screen is clipped, and a right edge left of the left edge wraps.
The mGBA suite's ["Window offscreen reset"][suite-window-reset] case doesn't cover every inverted combination, so this is Evenfall's choice, not a settled fact.

## Color effects

`BLDCNT` (`04000050h`) chooses the first targets (bits 0–5: BG0–BG3, sprites, backdrop), the effect (bits 6–7: none, blend, brighten, darken) and the second targets (bits 8–13).
`BLDALPHA` (`04000052h`) holds the blend weights EVA and EVB, and `BLDY` (`04000054h`) holds the brightness weight EVY.
Each weight counts in sixteenths and saturates at 16 ([LCD I/O Color Special Effects][gbatek-effects]).

For each 5-bit color component, rounding down:

| Effect   | Result                                     |
| -------- | ------------------------------------------ |
| Blend    | `min(31, (top * EVA + second * EVB) / 16)` |
| Brighten | `top + (31 − top) * EVY / 16`              |
| Darken   | `top − top * EVY / 16`                     |

An effect applies only where the window allows effects and the top surface's layer is a first target.
Blending also needs the second surface's layer to be a second target.
Otherwise the top pixel shows unchanged.

A semi-transparent sprite (mode 1) always blends as a first target, whatever `BLDCNT`'s sprite bit and effect say.
Only the frontmost sprite pixel takes part, so sprites never blend with each other.
When the surface underneath isn't a second target, Evenfall shows the sprite pixel unchanged and doesn't brighten or darken it.

## Mosaic

Mosaic makes a layer look blocky, by repeating one pixel over a group of dots.
Games use it for transitions and pixelated effects.

`MOSAIC` (`0400004Ch`) holds four 4-bit sizes, each the group size minus one: background horizontal (bits 0–3), background vertical (4–7), sprite horizontal (8–11) and sprite vertical (12–15).
Groups are 1 to 16 dots.
Backgrounds opt in with `BGxCNT` bit 6 and sprites with attribute 0 bit 12 ([LCD I/O Mosaic Function][gbatek-mosaic]).

Mosaic isn't a filter applied afterwards.
Each layer rounds screen coordinates down to its group before anything else (scrolling, the affine matrix, tile lookup, transparency, windows, priority, effects), so a whole group takes the look of its top-left dot:

- Groups line up with the screen, not with scrolling, tiles or a sprite's position.
  Text layers round the screen position and then add the scroll, so a group can cross a tile edge without changing color.
- Affine and bitmap layers round the screen position before applying the reference point and matrix.
- Sprites use the same screen grid, converted to sprite coordinates before flips or the matrix.
  A group that starts left of the sprite uses its first column.
  At the right edge the last sample fills the rest of the group, as in mGBA.
- The sprite window rounds vertically but not horizontally, following mGBA.

Sources: GBATEK's `MOSAIC` register and top-left sampling rule, and mGBA's [text][mgba-mosaic-text], [affine and bitmap][mgba-mosaic-affine] and [sprite][mgba-mosaic-obj] mosaic code.

## Raster effects

Games change video registers and memory while a line is being drawn: scroll splits, palette changes, window moves, and tables of values that HBlank DMA writes line after line.
Evenfall draws a whole line at its HBlank, so it records every mid-line change it models with the dot where the change takes effect.

**Which dot a write lands on.** A write that completes at cycle c of a visible line (c < 960) takes effect from dot `c / 4`.
That dot and every later one see the new value, and earlier dots the old one.
A write during HBlank or VBlank applies to the whole next line.
Exactly when the display reads its data relative to a CPU write hasn't been measured, so this rule is Evenfall's choice.

Each kind of change has its own log per line (the `Bus` fields `scroll_segments`, `backdrop_segments`, `win_segments`, `blend_segments`, `affine_segments` and `mosaic_segments`).
Each log holds 64 entries per line, or per background for scroll and affine.
Palette and background VRAM changes share one log, where a palette change takes one entry and a VRAM byte two.
Changes past that limit are dropped, and that line is drawn approximately.

| Takes effect at the dot of the write          | Takes effect for the whole line                                  |
| --------------------------------------------- | ---------------------------------------------------------------- |
| palette RAM, for every layer and the backdrop | OAM and sprite VRAM                                              |
| background VRAM below the sprite boundary     | `DISPCNT` mode, frame select, sprite layout and HBlank-free bits |
| `BGxHOFS`, `BGxVOFS`                          | `DISPCNT` layer enables (bits 8–12) and sprite window (bit 15)   |
| `BGxCNT` priority bits                        | the other `BGxCNT` fields                                        |
| `WIN0H/V`, `WIN1H/V`, `WININ`, `WINOUT`       | leaving forced blank (bit 7 cleared)                             |
| `DISPCNT` window 0 and 1 enables (bits 13–14) | vertical mosaic sizes                                            |
| entering forced blank (`DISPCNT` bit 7 set)   | `GREENSWP`                                                       |
| `BLDCNT`, `BLDALPHA`, `BLDY`                  |                                                                  |
| horizontal mosaic sizes                       |                                                                  |
| BG2 and BG3 `PA`, `PC` and reference points   |                                                                  |

"The whole line" means the value when the line is drawn at cycle 960 applies to all of it.
PB and PD only affect the step to the next line, so a mid-line write to them has nothing to split.

Some cases need more detail:

- **16-bit video buses.** Palette RAM and VRAM take 16 bits at a time, so a 32-bit CPU write lands as two halves on consecutive cycles.
  Each half becomes visible at its own dot, and the pair still costs two cycles.
  A DMA triggered by either half waits until both are done.
- **Palette and VRAM together.** They share one log but are replayed independently.
- **Forced blank.** Setting bit 7 mid-line turns the rest of the line white.
  Clearing it isn't split, and the line follows the value at its HBlank.
  GBATEK says the display comes back only after two lines and marks that as uncertain.
  Modeling it would need display-pipeline state Evenfall doesn't keep.
- **Background priority.** A priority change reorders the layers from its dot on.
  NanoBoyAdvance also does this ([`bus.cc`][nba-bus], [`merge.cc`][nba-merge]).
  GBATEK doesn't say at which dot a write lands.
- **Affine writes.** A PA or PC write keeps the position reached with the old value and continues from it with the new slope.
  A reference-point write replaces the current line's origin, following GBATEK.
  The test for this accepts both Evenfall's split at source x = 100 and NanoBoyAdvance's at x = 108, because no one has measured where the split falls.

The most common raster effect is simpler than all this: a scroll value that HBlank DMA writes at line y's HBlank applies to all of line y + 1.

## Presentation

The emulated frame is raw 15-bit color, and it is the only thing headless hashes and tests look at.
A host may transform it for display, for example to correct colors for a modern screen, without touching the emulated frame.
A shared presentation layer with scaling and shader presets for Evenfall's graphical frontends is planned ([#1](https://github.com/evenfall-emu/evenfall/issues/1)).

## How it is checked

- [`tests/bus/video.rs`](../../../systems/gba/tests/bus/video.rs) has a test for each rule on this page, named after it: transparent index 0 in mode 4, tiles past the background part of VRAM, the hidden affine origin, the sprite budget, sprite tile layout and the bitmap-mode tile limit, sprite overlap and blending targets, the window effects bit and inverted windows, mosaic ordering, and every mid-line split.
- Probe ROMs check whole frames from real GBA code: display interrupts (`probe-display-irq` counts 456 HBlank, 2 VBlank and 2 VCOUNT interrupts over two frames), forced blank, the three bitmap modes, green swap, scroll wrap, 256-color text, affine rotation and wrapping, mode 1, sprites with flips, double size, rotation, semi-transparency and mosaic, windows and the sprite window, blending, brightening and darkening, background mosaic, and the mid-line window, brightness, affine, mosaic and combined splits ([`probe/README.md`](../../../probe/README.md)).
  Mid-line probes land their split where CPU timing puts it, so they check that a genuine split happens on the target line and that every other pixel is exact.
- `probe-objrot` agrees with mGBA pixel for pixel, and `probe-hdma` covers the per-line scroll table.

## Open questions

- **HBlank and VBlank flag edges.** GBATEK and mGBA set the HBlank flag about 46–48 cycles after drawing ends and clear the VBlank flag on line 227.
  Evenfall uses cycle 960 and lines 160–227.
  A capture of `DISPSTAT` around both edges, and of the HBlank interrupt and DMA timing, would settle it.
- **Sprite-over-sprite priority.** Evenfall lets the lower OAM number win every overlap (GBATEK), while mGBA and NanoBoyAdvance look at the priority field first.
  Evenfall also compares each sprite only with the surface it lands on.
  So a lower-numbered sprite whose priority loses to a background still covers that background when a higher-numbered sprite covered it first.
  A hardware test with overlapping sprites of different priorities over a background would settle both.
- **Semi-transparent sprite without a second target.** GBATEK says `BLDCNT` may still brighten or darken such a pixel.
  Evenfall leaves it unchanged.
- **Tiles past the background part of VRAM.** Evenfall reads them as zero (DS behavior).
  The GBA's stripes and multi-layer behavior aren't modeled, and map fetches there are undocumented.
- **Sprite tile limit in bitmap modes.** Per-tile (Evenfall, NanoBoyAdvance, SkyEmu) and whole-sprite (mGBA) rules differ for a sprite starting at tile 511.
- **Inverted window edges.** GBATEK's clamp and mGBA's wrap, which Evenfall follows, disagree for X1 > X2, Y1 > Y2 and edges past the screen.
- **Mid-line write timing.** The `c / 4` rule and the affine split point are unmeasured.
  NanoBoyAdvance's model lands affine splits 8 dots later.
- **Changes applied to the whole line.** OAM and sprite VRAM, layer enables, other `BGxCNT` fields, vertical mosaic and leaving forced blank aren't split mid-line.
  GBATEK's two-line delay after forced blank isn't modeled.
- **Video memory contention.** A CPU access to video memory that collides with the display's own reads costs no extra cycle in Evenfall ([`memory.md`](memory.md)).
- **Mosaic corner cases.** Rare combinations of sprite-window and wrapped-coordinate mosaic have no hardware test.

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
