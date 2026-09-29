//! Text, affine, and bitmap backgrounds. Lines without raster history render
//! each background into a color row (`text_row`, `affine_row`, `bitmap_row`)
//! for layer-major composition. The per-dot renderers are the reference and
//! draw lines with raster history.

use super::history::{AffineState, VideoHistoryView};
use super::scanline_memo::{ALL_VRAM_PAGES, PAGE_SIZE, VRAM_SIZE};
use super::tile_row_cache::{Cache, TRANSPARENT, reverse_nibbles};
use crate::bus::{Bus, SCREEN_WIDTH};

/// Length of a background row buffer. Text rows hold 32 whole tiles, tile
/// `k` at `8k`; the line starts at the fine scroll offset (`hofs & 7`), so a
/// partly scrolled-off first tile needs no special case. Affine and bitmap
/// rows start at zero.
pub(super) const ROW: usize = 256;
const BG_VRAM: usize = 0x10000;

/// Render one text background line into `out` and return the 1 KiB BG VRAM
/// pages it read; the screen line starts at `out[hofs & 7]`. `line` is the
/// source line after vertical mosaic. Out of line: inlined into the line
/// pipeline, its loop spills to the stack.
#[inline(never)]
pub(super) fn text_row<const COLOR_256: bool>(
    vram: &[u8; VRAM_SIZE],
    cache: &mut Cache,
    line: usize,
    control: u16,
    hofs: u16,
    vofs: u16,
    out: &mut [u16; ROW],
) -> u64 {
    let char_base = usize::from((control >> 2) & 3) * 0x4000;
    let screen_base = usize::from((control >> 8) & 31) * 0x800;
    let wide = control & (1 << 14) != 0;
    let height = if control & (1 << 15) != 0 { 512 } else { 256 };
    let world_y = (line + usize::from(vofs)) & (height - 1);
    let blocks = if wide { 2 } else { 1 };
    let map_row = screen_base + (world_y / 256) * blocks * 0x800 + (world_y / 8 % 32) * 64;
    // The line's map entries; blocks past BG VRAM read as zero.
    let mut entries = [0u16; 64];
    let mut pages = 0u64;
    for block in 0..blocks {
        let block_row = map_row + block * 0x800;
        if block_row < BG_VRAM {
            let bytes: &[u8; 64] = vram[block_row..block_row + 64].try_into().unwrap();
            for (entry, pair) in entries[block * 32..].iter_mut().zip(bytes.as_chunks::<2>().0) {
                *entry = u16::from_le_bytes(*pair);
            }
            pages |= 1 << (block_row / PAGE_SIZE);
        }
    }
    let column_mask = if wide { 63 } else { 31 };
    let first_column = usize::from(hofs) / 8;
    let tile_y = world_y % 8;
    let row_bytes = if COLOR_256 { 8 } else { 4 };
    let pairs = cache.all_pairs();
    for (tile, pixels) in out.as_chunks_mut::<8>().0.iter_mut().enumerate().take(31) {
        let entry = entries[(first_column + tile) & column_mask];
        let source_y = tile_y ^ if entry & (1 << 11) != 0 { 7 } else { 0 };
        let row_base =
            char_base + usize::from(entry & 0x3FF) * row_bytes * 8 + source_y * row_bytes;
        if row_base >= BG_VRAM {
            *pixels = [TRANSPARENT; 8];
            continue;
        }
        pages |= 1 << (row_base / PAGE_SIZE);
        let flipped = entry & (1 << 10) != 0;
        if COLOR_256 {
            let raw = u64::from_le_bytes(vram[row_base..row_base + 8].try_into().unwrap());
            let source = if flipped { raw.swap_bytes() } else { raw };
            for (pixel, color) in pixels.iter_mut().enumerate() {
                *color = pairs.bg256[usize::from((source >> (pixel * 8)) as u8)];
            }
        } else {
            let raw = u32::from_le_bytes(vram[row_base..row_base + 4].try_into().unwrap());
            let source = if flipped { reverse_nibbles(raw) } else { raw };
            let bank = &pairs.banks[usize::from(entry >> 12)];
            for (pair, byte) in pixels.as_chunks_mut::<2>().0.iter_mut().zip(source.to_le_bytes()) {
                let colors = bank[usize::from(byte)];
                *pair = [colors as u16, (colors >> 16) as u16];
            }
        }
    }
    pages
}

/// Horizontal mosaic: each block of `size` pixels repeats its first pixel.
pub(super) fn apply_horizontal_mosaic(row: &mut [u16; SCREEN_WIDTH], size: usize) {
    if size == 2 {
        for pair in row.as_chunks_mut::<2>().0 {
            pair[1] = pair[0];
        }
        return;
    }
    let (mut color, mut left) = (0, 0);
    for pixel in row.iter_mut() {
        if left == 0 {
            (color, left) = (*pixel, size);
        }
        *pixel = color;
        left -= 1;
    }
}

/// Render one affine background line into `out` and return where its
/// screen line starts. `mosaic` is the horizontal mosaic block size (1 when
/// mosaic is off).
pub(super) fn affine_row(
    vram: &[u8; VRAM_SIZE],
    colors: &[u16; 256],
    control: u16,
    affine: AffineState,
    mosaic: usize,
    out: &mut [u16; ROW],
) -> usize {
    let char_base = usize::from((control >> 2) & 3) * 0x4000;
    let screen_base = usize::from((control >> 8) & 31) * 0x800;
    let size = usize::from(control >> 14);
    let wrap = control & (1 << 13) != 0;
    let map_bytes = 256 << (2 * size);
    if mosaic == 1 && screen_base + map_bytes <= BG_VRAM {
        let map = AffineMap {
            tiles: &vram[screen_base..screen_base + map_bytes],
            characters: vram[char_base..char_base + 0x4000].try_into().unwrap(),
            colors,
        };
        if affine.pa == 0x100 && affine.pc == 0 {
            return match (wrap, size) {
                (true, 0) => map.translated_row::<true, 4>(affine, out),
                (true, 1) => map.translated_row::<true, 5>(affine, out),
                (true, 2) => map.translated_row::<true, 6>(affine, out),
                (true, _) => map.translated_row::<true, 7>(affine, out),
                (false, 0) => map.translated_row::<false, 4>(affine, out),
                (false, 1) => map.translated_row::<false, 5>(affine, out),
                (false, 2) => map.translated_row::<false, 6>(affine, out),
                (false, _) => map.translated_row::<false, 7>(affine, out),
            };
        }
        let out = out.first_chunk_mut().unwrap();
        match (wrap, size) {
            (true, 0) => map.row::<true, 4>(affine, out),
            (true, 1) => map.row::<true, 5>(affine, out),
            (true, 2) => map.row::<true, 6>(affine, out),
            (true, _) => map.row::<true, 7>(affine, out),
            (false, 0) => map.row::<false, 4>(affine, out),
            (false, 1) => map.row::<false, 5>(affine, out),
            (false, 2) => map.row::<false, 6>(affine, out),
            (false, _) => map.row::<false, 7>(affine, out),
        }
        return 0;
    }
    // Maps that run past BG VRAM read tile zero there, like the per-dot
    // renderer; mosaic samples block starts.
    let dimension = 128i32 << size;
    let tiles = (dimension / 8) as usize;
    let texel = |x: i32, y: i32| {
        let source_x = x >> 8;
        let source_y = y >> 8;
        let inside = (source_x as u32) < dimension as u32 && (source_y as u32) < dimension as u32;
        // Affine dimensions are powers of two: masking is Euclidean modulo
        // even for negative source coordinates.
        let sx = (source_x & (dimension - 1)) as usize;
        let sy = (source_y & (dimension - 1)) as usize;
        let map = screen_base + sy / 8 * tiles + sx / 8;
        let tile = if map < BG_VRAM { vram[map] } else { 0 };
        let index = vram[char_base + usize::from(tile) * 64 + sy % 8 * 8 + sx % 8];
        if inside || wrap { colors[usize::from(index)] } else { TRANSPARENT }
    };
    for start in (0..SCREEN_WIDTH).step_by(mosaic) {
        let x = start as i32;
        let color = texel(affine.ref_x + affine.pa * x, affine.ref_y + affine.pc * x);
        out[start..(start + mosaic).min(SCREEN_WIDTH)].fill(color);
    }
    0
}

/// An affine map that lies wholly in BG VRAM.
struct AffineMap<'a> {
    /// Tile numbers, one map row after another.
    tiles: &'a [u8],
    /// The 256 8bpp tiles of the character base.
    characters: &'a [u8; 0x4000],
    colors: &'a [u16; 256],
}

impl AffineMap<'_> {
    /// A row with PA 1.0 and PC 0 reads one map row: render it a tile at a
    /// time like a text row and return the offset of its screen line.
    /// Screen `x` samples source column `(ref_x >> 8) + x` exactly, and the
    /// clip boundaries are tile boundaries.
    #[inline(never)]
    fn translated_row<const WRAP: bool, const TILE_SHIFT: u32>(
        &self,
        affine: AffineState,
        out: &mut [u16; ROW],
    ) -> usize {
        let mask = (8i32 << TILE_SHIFT) - 1;
        let (source_x, source_y) = (affine.ref_x >> 8, affine.ref_y >> 8);
        let chunks = out.as_chunks_mut::<8>().0;
        if !WRAP && source_y & !mask != 0 {
            chunks.fill([TRANSPARENT; 8]);
            return 0;
        }
        let sy = (source_y & mask) as usize;
        let columns = 1 << TILE_SHIFT;
        let map_row = &self.tiles[(sy >> 3) << TILE_SHIFT..][..columns];
        let first = source_x >> 3;
        for (tile, pixels) in chunks.iter_mut().enumerate().take(31) {
            let column = first + tile as i32;
            if !WRAP && column as u32 >= columns as u32 {
                *pixels = [TRANSPARENT; 8];
                continue;
            }
            let number = usize::from(map_row[column as usize & (columns - 1)]);
            let row: &[u8; 8] =
                self.characters[number * 64 + (sy & 7) * 8..][..8].try_into().unwrap();
            for (pixel, &index) in pixels.iter_mut().zip(row) {
                *pixel = self.colors[usize::from(index)];
            }
        }
        (source_x & 7) as usize
    }

    /// `TILE_SHIFT` is log2 of the map width in tiles.
    #[inline(never)]
    fn row<const WRAP: bool, const TILE_SHIFT: u32>(
        &self,
        affine: AffineState,
        out: &mut [u16; SCREEN_WIDTH],
    ) {
        let mask = (8u32 << TILE_SHIFT) - 1;
        let tiles = &self.tiles[..1 << (2 * TILE_SHIFT)];
        let (mut x, mut y) = (affine.ref_x, affine.ref_y);
        for color in out.iter_mut() {
            let (source_x, source_y) = ((x >> 8) as u32, (y >> 8) as u32);
            x += affine.pa;
            y += affine.pc;
            let (sx, sy) = (source_x & mask, source_y & mask);
            let at = ((sy >> 3) << TILE_SHIFT | sx >> 3) as usize;
            let tile = tiles[at & ((1 << (2 * TILE_SHIFT)) - 1)];
            let index = self.characters
                [(usize::from(tile) << 6 | (sy as usize & 7) << 3) | (sx as usize & 7)];
            let texel = self.colors[usize::from(index)];
            *color = if WRAP || (source_x | source_y) & !mask == 0 { texel } else { TRANSPARENT };
        }
    }
}

/// Render one bitmap line (modes 3–5, BG2) and return the VRAM pages read.
/// Out of line: its row loop gets registers of its own.
#[inline(never)]
pub(super) fn bitmap_row(
    vram: &[u8; VRAM_SIZE],
    colors: &[u16; 256],
    mode: u16,
    page: usize,
    affine: AffineState,
    mosaic: usize,
    out: &mut [u16; SCREEN_WIDTH],
) -> u128 {
    let (width, height) = if mode == 5 { (160i32, 128i32) } else { (240, 160) };
    let texel = |sx: i32, sy: i32| {
        if sx < 0 || sy < 0 || sx >= width || sy >= height {
            return TRANSPARENT;
        }
        let index = sy as usize * width as usize + sx as usize;
        if mode == 4 {
            colors[usize::from(vram[page + index])]
        } else {
            let offset = page + index * 2;
            u16::from_le_bytes([vram[offset], vram[offset + 1]]) & 0x7FFF
        }
    };
    if mosaic == 1 && affine.pa == 0x100 && affine.pc == 0 {
        // One source row, possibly translated: (ref_x + 256x) >> 8 is
        // (ref_x >> 8) + x exactly.
        let sy = affine.ref_y >> 8;
        let sx = affine.ref_x >> 8;
        if sy < 0 || sy >= height {
            out.fill(TRANSPARENT);
            return 0;
        }
        // The screen columns whose source column is in the bitmap; the
        // texels at the others are transparent.
        let first = (-sx).clamp(0, SCREEN_WIDTH as i32) as usize;
        let end = (width - sx).clamp(0, SCREEN_WIDTH as i32).max(first as i32) as usize;
        out[..first].fill(TRANSPARENT);
        out[end..].fill(TRANSPARENT);
        if first < end {
            // The first shown pixel: its column, sx + first, is in the row.
            let pixel = sy as usize * width as usize + (sx + first as i32) as usize;
            if mode == 4 {
                let indices = &vram[page + pixel..][..end - first];
                for (color, &index) in out[first..end].iter_mut().zip(indices) {
                    *color = colors[usize::from(index)];
                }
            } else {
                let bytes = &vram[page + 2 * pixel..][..2 * (end - first)];
                for (color, pair) in out[first..end].iter_mut().zip(bytes.as_chunks::<2>().0) {
                    *color = u16::from_le_bytes(*pair) & 0x7FFF;
                }
            }
        }
        let bytes = if mode == 4 { 1 } else { 2 };
        let start = page + sy as usize * width as usize * bytes;
        let end = start + width as usize * bytes - 1;
        let pages = (start / PAGE_SIZE)..=(end / PAGE_SIZE);
        return pages.fold(0, |pages, page| pages | 1 << page);
    }
    if mosaic > 1 {
        for start in (0..SCREEN_WIDTH).step_by(mosaic) {
            let x = start as i32;
            let color =
                texel((affine.ref_x + affine.pa * x) >> 8, (affine.ref_y + affine.pc * x) >> 8);
            out[start..(start + mosaic).min(SCREEN_WIDTH)].fill(color);
        }
    } else {
        let (mut x, mut y) = (affine.ref_x, affine.ref_y);
        for color in out.iter_mut() {
            *color = texel(x >> 8, y >> 8);
            x += affine.pa;
            y += affine.pc;
        }
    }
    ALL_VRAM_PAGES
}

struct TextLine {
    priority: u8,
    background: usize,
    first: usize,
}

struct TextRow {
    colors: [u16; 8],
    indices: [u8; 8],
    fully_opaque: bool,
}

struct TextLanes {
    top: [bool; 8],
    second: [bool; 8],
}

impl Bus {
    fn decode_text_row<const COLOR_256: bool>(
        &mut self,
        entry: u16,
        row_base: usize,
    ) -> Option<TextRow> {
        if row_base + (if COLOR_256 { 8 } else { 4 }) > 0x10000 {
            return None;
        }
        let flipped = entry & (1 << 10) != 0;
        let mut indices = [0; 8];
        let mut colors = [0; 8];
        if COLOR_256 {
            indices.copy_from_slice(&self.vram[row_base..row_base + 8]);
            if indices == [0; 8] {
                return None;
            }
            if flipped {
                indices.reverse();
            }
            for x in 0..8 {
                colors[x] = self.palette_entry_color(indices[x] as usize);
            }
        } else {
            let source = u32::from_le_bytes(self.vram[row_base..row_base + 4].try_into().unwrap());
            if source == 0 {
                return None;
            }
            let source = if flipped { reverse_nibbles(source) } else { source };
            self.tile_row_cache.row(source, (entry >> 12) as u8, &mut colors);
            indices = colors.map(|color| u8::from(color & TRANSPARENT == 0));
        }
        Some(TextRow { colors, fully_opaque: indices.iter().all(|&index| index != 0), indices })
    }

    fn text_open_lanes<const SIMPLE: bool>(
        &self,
        layout: &TextLine,
        x: usize,
        length: usize,
    ) -> TextLanes {
        let mut lanes = TextLanes { top: [false; 8], second: [false; 8] };
        for lane in 0..length {
            let i = layout.first + x + lane;
            let admitted =
                SIMPLE || self.line_window_masks[x + lane] & (1 << layout.background) != 0;
            lanes.top[lane] = admitted && self.frame_layers[i] == 5;
            lanes.second[lane] = admitted
                && !lanes.top[lane]
                && self.line_needs_second_surface
                && self.frame_second_layers[i] == 5;
        }
        lanes
    }

    fn write_text_row(
        &mut self,
        layout: &TextLine,
        x: usize,
        row: &TextRow,
        source_start: usize,
        length: usize,
        lanes: &TextLanes,
    ) {
        if row.fully_opaque && source_start == 0 && length == 8 && lanes.top == [true; 8] {
            let i = layout.first + x;
            self.frame_pixels[i..i + 8].copy_from_slice(&row.colors);
            self.frame_priorities[i..i + 8].fill(layout.priority);
            self.frame_layers[i..i + 8].fill(layout.background as u8);
            return;
        }
        for lane in 0..length {
            let source = source_start + lane;
            if row.indices[source] == 0 {
                continue;
            }
            let i = layout.first + x + lane;
            if lanes.top[lane] {
                self.frame_pixels[i] = row.colors[source];
                self.frame_priorities[i] = layout.priority;
                self.frame_layers[i] = layout.background as u8;
            } else if lanes.second[lane] {
                self.frame_second_pixels[i] = row.colors[source];
                self.frame_second_priorities[i] = layout.priority;
                self.frame_second_layers[i] = layout.background as u8;
            }
        }
    }

    fn text_tile_sources<const COLOR_256: bool>(
        &mut self,
        line: usize,
        background: usize,
    ) -> ([u16; 64], [usize; 64], usize, usize) {
        let control = self.bg_controls[background];
        let char_base = usize::from((control >> 2) & 3) * 0x4000;
        let screen_base = usize::from((control >> 8) & 31) * 0x800;
        let width = if control & (1 << 14) != 0 { 512 } else { 256 };
        let height = if control & (1 << 15) != 0 { 512 } else { 256 };
        let world_y = (line + usize::from(self.bg_vertical_offsets[background])) & (height - 1);
        let row_bytes = if COLOR_256 { 8 } else { 4 };
        let map_row =
            screen_base + (world_y / 256) * (width / 256) * 0x800 + (world_y / 8 % 32) * 64;
        let mut entries = [0u16; 64];
        let mut row_bases = [0usize; 64];
        for block in 0..width / 256 {
            let block_row = map_row + block * 0x800;
            let mut block_entries = [0u16; 32];
            if block_row < 0x10000 {
                for (column, entry) in block_entries.iter_mut().enumerate() {
                    let i = block_row + column * 2;
                    *entry = u16::from_le_bytes([self.vram[i], self.vram[i + 1]]);
                }
                self.scanline_memo.reads.add_vram(block_row);
            }
            self.scanline_memo.reads.add_tile_rows(
                &mut self.scanline_memo.tile_pages[background][block],
                char_base,
                COLOR_256,
                block_entries,
            );
            for (column, entry) in block_entries.into_iter().enumerate() {
                let index = block * 32 + column;
                entries[index] = entry;
                let source_y = if entry & (1 << 11) != 0 { 7 - world_y % 8 } else { world_y % 8 };
                row_bases[index] = char_base
                    + usize::from(entry & 0x3FF) * if COLOR_256 { 64 } else { 32 }
                    + source_y * row_bytes;
            }
        }
        (entries, row_bases, width, usize::from(self.bg_horizontal_offsets[background]))
    }

    // Isolate the text loop from unrelated bitmap and Bus inlining decisions.
    #[inline(never)]
    fn render_text_tile_rows<const SIMPLE: bool, const COLOR_256: bool>(
        &mut self,
        line: usize,
        background: usize,
        priority: u8,
    ) {
        let (entries, row_bases, width, hofs) =
            self.text_tile_sources::<COLOR_256>(line, background);
        let layout = TextLine { priority, background, first: line * SCREEN_WIDTH };
        let mut x = 0;
        while x < SCREEN_WIDTH {
            let world_x = (x + hofs) & (width - 1);
            let column = (world_x / 8) & (width / 8 - 1);
            let start = world_x % 8;
            let length = (8 - start).min(SCREEN_WIDTH - x);
            let lanes = self.text_open_lanes::<SIMPLE>(&layout, x, length);
            if (lanes.top.contains(&true) || lanes.second.contains(&true))
                && let Some(row) =
                    self.decode_text_row::<COLOR_256>(entries[column], row_bases[column])
            {
                self.write_text_row(&layout, x, &row, start, length, &lanes);
            }
            x += length;
        }
    }

    // Keep mosaic/raster text code out of compose_scanline's hot-path codegen.
    #[inline(never)]
    pub(super) fn render_text_background(
        &mut self,
        line: usize,
        background: usize,
        priority: u8,
        history: &VideoHistoryView,
    ) {
        let control = self.bg_controls[background];
        let identity_mosaic = self.mosaic & 0xFF == 0
            && !(self.mosaic_segment_line == line as u16 && self.mosaic_segment_count > 0);
        if (control & (1 << 6) == 0 || identity_mosaic)
            && history.vram_records == 0
            && history.palette_records == 0
            && !(self.scroll_segment_line == line as u16
                && self.scroll_segment_counts[background] > 0)
        {
            let simple = self.line_unmasked_layers & (1 << background) != 0
                && !self.line_needs_second_surface;
            match (simple, control & (1 << 7) != 0) {
                (true, true) => {
                    self.render_text_tile_rows::<true, true>(line, background, priority)
                }
                (true, false) => {
                    self.render_text_tile_rows::<true, false>(line, background, priority)
                }
                (false, true) => {
                    self.render_text_tile_rows::<false, true>(line, background, priority)
                }
                (false, false) => {
                    self.render_text_tile_rows::<false, false>(line, background, priority)
                }
            }
            return;
        }
        self.scanline_memo.reads.vram_pages = ALL_VRAM_PAGES;
        let char_base = usize::from((control >> 2) & 3) * 0x4000;
        let screen_base = usize::from((control >> 8) & 31) * 0x800;
        let color_256 = control & (1 << 7) != 0;
        let map_width = if control & (1 << 14) != 0 { 512 } else { 256 };
        let map_height = if control & (1 << 15) != 0 { 512 } else { 256 };
        let mosaic_enabled = control & (1 << 6) != 0;
        let vertical = usize::from((self.mosaic >> 4) & 15) + 1;
        let sampled_line = if mosaic_enabled { line - line % vertical } else { line };
        let mut hofs = [self.bg_horizontal_offsets[background]; SCREEN_WIDTH];
        let mut vofs = [self.bg_vertical_offsets[background]; SCREEN_WIDTH];
        if self.scroll_segment_line == line as u16 && self.scroll_segment_counts[background] > 0 {
            let segments = &self.scroll_segments[background]
                [..usize::from(self.scroll_segment_counts[background])];
            let mut cursor = 0;
            for (index, segment) in segments.iter().enumerate() {
                let end = if index + 1 < segments.len() {
                    usize::from(Self::scroll_segment_x(segments[index + 1]))
                } else {
                    SCREEN_WIDTH
                };
                while cursor < end.min(SCREEN_WIDTH) {
                    hofs[cursor] = segment.hofs;
                    vofs[cursor] = segment.vofs;
                    cursor += 1;
                }
            }
        }
        for x in 0..SCREEN_WIDTH {
            if self.background_priority_for(background, line, x) != priority
                || self.line_window_masks[x] & (1 << background) == 0
            {
                continue;
            }
            let i = line * SCREEN_WIDTH + x;
            if !self.background_lane_open(i) {
                continue;
            }
            let horizontal = usize::from(self.mosaic_for(line, x) & 15) + 1;
            let sampled_x = if mosaic_enabled { x - x % horizontal } else { x };
            let world_y = (sampled_line + usize::from(vofs[x])) % map_height;
            let world_x = (sampled_x + usize::from(hofs[x])) % map_width;
            let tile_x = world_x / 8;
            let tile_y = world_y / 8;
            let block = tile_x / 32 + tile_y / 32 * (map_width / 256);
            let entry_index = screen_base + block * 0x800 + ((tile_y % 32) * 32 + tile_x % 32) * 2;
            let entry = u16::from(self.vram_byte_for(history, x, entry_index))
                | (u16::from(self.vram_byte_for(history, x, entry_index + 1)) << 8);
            let source_x = if entry & (1 << 10) != 0 { 7 - world_x % 8 } else { world_x % 8 };
            let source_y = if entry & (1 << 11) != 0 { 7 - world_y % 8 } else { world_y % 8 };
            let tile = usize::from(entry & 0x3FF);
            let index = if color_256 {
                self.vram_byte_for(history, x, char_base + tile * 64 + source_y * 8 + source_x)
            } else {
                let byte = self.vram_byte_for(
                    history,
                    x,
                    char_base + tile * 32 + source_y * 4 + source_x / 2,
                );
                if source_x & 1 == 0 { byte & 15 } else { byte >> 4 }
            };
            if index == 0 {
                continue;
            }
            let palette = if color_256 {
                usize::from(index)
            } else {
                usize::from(entry >> 12) * 16 + usize::from(index)
            };
            self.compose_background_pixel(
                i,
                self.palette_color_for(history, x, palette),
                priority,
                background as u8,
            );
        }
    }

    pub(super) fn render_affine_background(
        &mut self,
        line: usize,
        background: usize,
        priority: u8,
        history: &VideoHistoryView,
    ) {
        self.scanline_memo.reads.vram_pages = ALL_VRAM_PAGES;
        let affine_index = background - 2;
        let control = self.bg_controls[background];
        let char_base = usize::from((control >> 2) & 3) * 0x4000;
        let screen_base = usize::from((control >> 8) & 31) * 0x800;
        let dimension = 128i32 << (control >> 14);
        let tiles = (dimension / 8) as usize;
        for x in 0..SCREEN_WIDTH {
            if self.background_priority_for(background, line, x) != priority
                || self.line_window_masks[x] & (1 << background) == 0
            {
                continue;
            }
            let i = line * SCREEN_WIDTH + x;
            if !self.background_lane_open(i) {
                continue;
            }
            let horizontal = usize::from(self.mosaic_for(line, x) & 15) + 1;
            let sample = if control & (1 << 6) != 0 { x - x % horizontal } else { x };
            let affine = self.affine_for(affine_index, line, x);
            let mut source_x = (affine.ref_x + affine.pa * sample as i32) >> 8;
            let mut source_y = (affine.ref_y + affine.pc * sample as i32) >> 8;
            if source_x < 0 || source_y < 0 || source_x >= dimension || source_y >= dimension {
                if control & (1 << 13) == 0 {
                    continue;
                }
                // Affine dimensions are powers of two: masking is Euclidean
                // modulo even for negative source coordinates.
                source_x &= dimension - 1;
                source_y &= dimension - 1;
            }
            let sx = source_x as usize;
            let sy = source_y as usize;
            let tile =
                usize::from(self.vram_byte_for(history, x, screen_base + sy / 8 * tiles + sx / 8));
            let index =
                self.vram_byte_for(history, x, char_base + tile * 64 + (sy % 8) * 8 + sx % 8);
            if index != 0 {
                self.compose_background_pixel(
                    i,
                    self.palette_color_for(history, x, index as usize),
                    priority,
                    background as u8,
                );
            }
        }
    }

    pub(super) fn render_bitmap_background(
        &mut self,
        line: usize,
        mode: u8,
        history: &VideoHistoryView,
    ) {
        self.scanline_memo.reads.vram_pages = ALL_VRAM_PAGES;
        let control = self.bg_controls[2];
        let width = if mode == 5 { 160 } else { 240 };
        let height = if mode == 5 { 128 } else { 160 };
        let page = if mode >= 4 && self.dispcnt & (1 << 4) != 0 { 0xA000 } else { 0 };
        if mode == 4
            && history.vram_records == 0
            && history.palette_records == 0
            && (self.affine_segment_line != line as u16 || self.affine_segment_counts[0] == 0)
            && (self.scroll_segment_line != line as u16 || self.scroll_segment_counts[2] == 0)
            && (self.mosaic_segment_line != line as u16 || self.mosaic_segment_count == 0)
            && (control & (1 << 6) == 0 || self.mosaic & 15 == 0)
            && self.bg_affine_parameters[0][0] == 0x100
            && self.bg_affine_parameters[0][2] == 0
            && self.bg_affine_current[0] == [0, (line as i32) << 8]
            && self.line_unmasked_layers & (1 << 2) != 0
        {
            let first = line * SCREEN_WIDTH;
            let row = &self.vram[page + first..page + first + SCREEN_WIDTH];
            let palette = &self.palette;
            let pixels = &mut self.frame_pixels[first..first + SCREEN_WIDTH];
            let priorities = &mut self.frame_priorities[first..first + SCREEN_WIDTH];
            let layers = &mut self.frame_layers[first..first + SCREEN_WIDTH];
            for (x, &index) in row.iter().enumerate() {
                if index != 0 {
                    let color = usize::from(index) * 2;
                    pixels[x] = u16::from_le_bytes([palette[color], palette[color + 1]]) & 0x7FFF;
                    priorities[x] = (control & 3) as u8;
                    layers[x] = 2;
                }
            }
            return;
        }
        for x in 0..SCREEN_WIDTH {
            if self.line_window_masks[x] & (1 << 2) == 0 {
                continue;
            }
            let horizontal = usize::from(self.mosaic_for(line, x) & 15) + 1;
            let sample = if control & (1 << 6) != 0 { x - x % horizontal } else { x };
            let affine = self.affine_for(0, line, x);
            let sx = (affine.ref_x + affine.pa * sample as i32) >> 8;
            let sy = (affine.ref_y + affine.pc * sample as i32) >> 8;
            if sx < 0 || sy < 0 || sx >= width || sy >= height {
                continue;
            }
            let index = sy as usize * width as usize + sx as usize;
            let color = if mode == 4 {
                let palette = self.vram_byte_for(history, x, page + index);
                if palette == 0 {
                    continue;
                }
                self.palette_color_for(history, x, palette as usize)
            } else {
                let offset = page + index * 2;
                u16::from(self.vram_byte_for(history, x, offset))
                    | (u16::from(self.vram_byte_for(history, x, offset + 1)) << 8)
            };
            self.compose_background_pixel(
                line * SCREEN_WIDTH + x,
                color,
                self.background_priority_for(2, line, x),
                2,
            );
        }
    }
}
