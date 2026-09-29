//! Layer-major scanline composition for lines without raster history.
//!
//! Every enabled background renders a color row (bit 15 marks transparency)
//! and objects render two rows: the front object (lowest OAM index) and the
//! object with the smallest priority. Branch-free passes over fixed arrays
//! then pick the top two surfaces per pixel, merge objects and apply color
//! effects. Only `frame_pixels` is written; the front-to-back reference path
//! in `render.rs` also fills the auxiliary surfaces.
//!
//! A surface is a `u32` entry: RGB555 color in bits 0–14, the layer (0–3 BG,
//! 4 OBJ, 5 backdrop) in bits 16–18, blend flags, and the priority in bits
//! 29–31 (4 for the backdrop, 7 for no object).

use super::backgrounds::{
    ROW, ROW_ORIGIN, affine_row, apply_horizontal_mosaic, bitmap_row, text_row,
};
use super::history::{AffineState, VideoHistoryView};
use super::scanline_memo::ALL_VRAM_PAGES;
use crate::bus::{Bus, LineScan, SCREEN_WIDTH};

pub(super) const LAYER_SHIFT: u32 = 16;
/// The front object is semi-transparent.
pub(super) const SEMI: u32 = 1 << 19;
/// First target of alpha blending (BLDCNT effect 1).
const ALPHA_FIRST: u32 = 1 << 20;
/// First target of a brightness effect (BLDCNT effect 2 or 3).
const BRIGHT_FIRST: u32 = 1 << 21;
const SECOND_TARGET: u32 = 1 << 22;
pub(super) const PRIORITY_SHIFT: u32 = 29;
pub(super) const NO_OBJECT: u32 = 7 << PRIORITY_SHIFT;
const COLOR: u32 = 0x7FFF;

/// A composed surface: color before effects, priority, layer.
pub type Surface = (u16, u8, u8);

const DIRECT: u8 = 0;
const BRIGHT: u8 = 1;
const BLEND: u8 = 2;

/// Transient per-line buffers of the layer-major compositor.
pub(crate) struct LineLayers {
    pub(super) backgrounds: [[u16; ROW]; 4],
    pub(super) top: [u32; SCREEN_WIDTH],
    pub(super) second: [u32; SCREEN_WIDTH],
    pub(super) object_front: [u32; SCREEN_WIDTH],
    pub(super) object_min: [u32; SCREEN_WIDTH],
    /// Layer and blend-target bits of the OBJ layer on this line.
    pub(super) object_meta: u32,
}

impl Default for LineLayers {
    fn default() -> Self {
        Self {
            backgrounds: [[0; ROW]; 4],
            top: [0; SCREEN_WIDTH],
            second: [0; SCREEN_WIDTH],
            object_front: [0; SCREEN_WIDTH],
            object_min: [NO_OBJECT; SCREEN_WIDTH],
            object_meta: 0,
        }
    }
}

#[derive(Clone, Copy)]
struct Effects {
    eva: u32,
    evb: u32,
    factor: u32,
    brighten: bool,
}

impl Bus {
    /// Compose a line from the live registers and memory. Returns the BG
    /// VRAM pages it read, which decide whether VRAM raster history matters.
    pub(super) fn compose_scanline_fast(&mut self, line: usize) -> u128 {
        let first = line * SCREEN_WIDTH;
        if self.dispcnt & 0x80 != 0 {
            self.frame_pixels[first..first + SCREEN_WIDTH].fill(0x7FFF);
            return 0;
        }
        let history = VideoHistoryView::default();
        self.line_semitransparent_obj = LineScan::Unknown;
        let second = self.needs_second_surface(line);
        let mut effects = second || (self.bldcnt & 0x80 != 0 && self.bldcnt & 0x3F != 0);
        if effects && self.color_effects_inert(line) {
            effects = false;
        }
        self.line_needs_second_surface = second;
        self.line_has_color_effects = effects;
        if self.dispcnt & 0x8000 != 0 {
            self.frame_obj_window[first..first + SCREEN_WIDTH].fill(false);
            if self.dispcnt & 0x1000 != 0 {
                self.render_objects::<false>(line, true, &history);
            }
        }
        self.resolve_window_masks(line);
        self.tile_row_cache.prepare_palette(&self.palette);

        let bldcnt = u32::from(self.bldcnt);
        let effect = (bldcnt >> 6) & 3;
        let first_flag = match effect {
            1 => ALPHA_FIRST,
            2 | 3 => BRIGHT_FIRST,
            _ => 0,
        };
        let meta = |layer: u32, priority: u32| {
            let mut meta = layer << LAYER_SHIFT | priority << PRIORITY_SHIFT;
            if bldcnt >> layer & 1 != 0 {
                meta |= first_flag;
            }
            if bldcnt >> (8 + layer) & 1 != 0 {
                meta |= SECOND_TARGET;
            }
            meta
        };

        let (pages, keys, count) = self.render_background_rows(line);
        self.scanline_memo.reads.vram_pages |= pages;

        let mut objects = false;
        if self.dispcnt & 0x1000 != 0 {
            if self.object_line_index != line as u16 {
                self.build_object_line(line);
            }
            if self.object_line_count > 0 {
                objects = true;
                self.line_layers.object_min.fill(NO_OBJECT);
                self.line_layers.object_meta = meta(4, 0);
                self.render_objects::<true>(line, false, &history);
            }
        }

        let blend = effects && second;
        let backdrop = u32::from(self.tile_row_cache.words[0]) | meta(5, 4);
        let LineLayers { backgrounds, top, second: below, .. } = &mut *self.line_layers;
        let window = &self.line_window_masks;
        top.fill(backdrop);
        if count == 0 && !objects && !effects {
            self.frame_pixels[first..first + SCREEN_WIDTH].fill(backdrop as u16 & 0x7FFF);
            if self.green_swap_enabled {
                self.apply_green_swap(line);
            }
            return pages;
        }
        if blend {
            below.fill(backdrop);
        }
        for &key in keys[..count].iter().rev() {
            let background = usize::from(key % 8);
            let row: &[u16; SCREEN_WIDTH] =
                backgrounds[background][ROW_ORIGIN..][..SCREEN_WIDTH].try_into().unwrap();
            let meta = meta(u32::from(key % 8), u32::from(key / 8));
            let bit = 1 << background;
            if blend {
                background_pass::<true>(row, window, bit, meta, top, below);
            } else {
                background_pass::<false>(row, window, bit, meta, top, below);
            }
        }

        let parameters = Effects {
            eva: u32::from(self.bldalpha & 31).min(16),
            evb: u32::from((self.bldalpha >> 8) & 31).min(16),
            factor: u32::from(self.bldy & 31).min(16),
            brighten: effect == 2,
        };
        let mode = if !effects {
            DIRECT
        } else if blend {
            BLEND
        } else {
            BRIGHT
        };
        let out: &mut [u16; SCREEN_WIDTH] =
            (&mut self.frame_pixels[first..first + SCREEN_WIDTH]).try_into().unwrap();
        let layers = &mut *self.line_layers;
        match (objects, mode) {
            (false, DIRECT) => finish::<false, DIRECT>(layers, window, parameters, out),
            (false, BRIGHT) => finish::<false, BRIGHT>(layers, window, parameters, out),
            (false, _) => finish::<false, BLEND>(layers, window, parameters, out),
            (true, DIRECT) => finish::<true, DIRECT>(layers, window, parameters, out),
            (true, BRIGHT) => finish::<true, BRIGHT>(layers, window, parameters, out),
            (true, _) => finish::<true, BLEND>(layers, window, parameters, out),
        }
        if self.green_swap_enabled {
            self.apply_green_swap(line);
        }
        pages
    }

    /// Render the enabled backgrounds of the display mode into their rows.
    /// Returns the VRAM pages read and the enabled layers' priority keys
    /// (`priority * 8 + background`) sorted front to back.
    fn render_background_rows(&mut self, line: usize) -> (u128, [u8; 4], usize) {
        let mode = self.dispcnt & 7;
        let mut pages = 0;
        let mut keys = [0u8; 4];
        let mut count = 0;
        for background in 0..4 {
            if self.dispcnt & (1 << (background + 8)) == 0 {
                continue;
            }
            let control = self.bg_controls[background];
            let mosaic = control & (1 << 6) != 0;
            let horizontal = if mosaic { usize::from(self.mosaic & 15) + 1 } else { 1 };
            let affine = self.live_affine(background.saturating_sub(2) & 1);
            let row = &mut self.line_layers.backgrounds[background];
            let screen: &mut [u16; SCREEN_WIDTH] =
                (&mut row[ROW_ORIGIN..ROW_ORIGIN + SCREEN_WIDTH]).try_into().unwrap();
            match (mode, background) {
                (0, _) | (1, 0 | 1) => {
                    let vertical = usize::from((self.mosaic >> 4) & 15) + 1;
                    let source_line = if mosaic { line - line % vertical } else { line };
                    let (hofs, vofs) = (
                        self.bg_horizontal_offsets[background],
                        self.bg_vertical_offsets[background],
                    );
                    let cache = &mut self.tile_row_cache;
                    let read = if control & (1 << 7) != 0 {
                        text_row::<true>(&self.vram, cache, source_line, control, hofs, vofs, row)
                    } else {
                        text_row::<false>(&self.vram, cache, source_line, control, hofs, vofs, row)
                    };
                    pages |= u128::from(read);
                    if horizontal > 1 {
                        let screen: &mut [u16; SCREEN_WIDTH] =
                            (&mut row[ROW_ORIGIN..ROW_ORIGIN + SCREEN_WIDTH]).try_into().unwrap();
                        apply_horizontal_mosaic(screen, horizontal);
                    }
                }
                (1, 2) | (2, 2 | 3) => {
                    affine_row(
                        &self.vram,
                        &self.tile_row_cache.bg256,
                        control,
                        affine,
                        horizontal,
                        screen,
                    );
                    pages = ALL_VRAM_PAGES;
                }
                (3..=5, 2) => {
                    let page = if mode >= 4 && self.dispcnt & (1 << 4) != 0 { 0xA000 } else { 0 };
                    pages |= bitmap_row(
                        &self.vram,
                        &self.tile_row_cache.bg256,
                        mode,
                        page,
                        affine,
                        horizontal,
                        screen,
                    );
                }
                _ => continue,
            }
            let key = ((control & 3) as u8) * 8 + background as u8;
            let mut slot = count;
            while slot > 0 && keys[slot - 1] > key {
                keys[slot] = keys[slot - 1];
                slot -= 1;
            }
            keys[slot] = key;
            count += 1;
        }
        (pages, keys, count)
    }

    /// Surfaces at `x` of the last line drawn by the layer-major compositor:
    /// the top surface, whether it is a semi-transparent object, and the
    /// second surface, which is tracked only on lines that alpha-blend.
    pub fn composed_surfaces(&self, x: usize) -> (Surface, bool, Surface) {
        let decode = |entry: u32| {
            (
                (entry & COLOR) as u16,
                (entry >> PRIORITY_SHIFT) as u8,
                (entry >> LAYER_SHIFT & 7) as u8,
            )
        };
        let top = self.line_layers.top[x];
        (decode(top), top & SEMI != 0, decode(self.line_layers.second[x]))
    }

    fn live_affine(&self, index: usize) -> AffineState {
        AffineState {
            pa: i32::from(self.bg_affine_parameters[index][0]),
            pc: i32::from(self.bg_affine_parameters[index][2]),
            ref_x: self.bg_affine_current[index][0],
            ref_y: self.bg_affine_current[index][1],
        }
    }

    /// Record one opaque object pixel in the object rows. Objects arrive in
    /// descending OAM order, so the front row keeps the last one and the
    /// minimum row keeps the smallest priority, ties to the lower index.
    pub(super) fn put_object_pixel(&mut self, x: usize, color: u16, priority: u8, semi: bool) {
        let layers = &mut *self.line_layers;
        let entry = u32::from(color)
            | layers.object_meta
            | u32::from(priority) << PRIORITY_SHIFT
            | if semi { SEMI } else { 0 };
        layers.object_front[x] = entry;
        if u32::from(priority) <= layers.object_min[x] >> PRIORITY_SHIFT {
            layers.object_min[x] = entry;
        }
    }

    /// Eight [`Self::put_object_pixel`] lanes of palette indices (0 is
    /// transparent) from `palette_start` in the decoded palette words.
    pub(super) fn put_object_lanes(
        &mut self,
        x: usize,
        indices: &[u8; 8],
        palette_start: usize,
        priority: u8,
        semi: bool,
    ) {
        let words = &self.tile_row_cache.words;
        let layers = &mut *self.line_layers;
        let base = layers.object_meta
            | u32::from(priority) << PRIORITY_SHIFT
            | if semi { SEMI } else { 0 };
        let front: &mut [u32; 8] = (&mut layers.object_front[x..x + 8]).try_into().unwrap();
        let min: &mut [u32; 8] = (&mut layers.object_min[x..x + 8]).try_into().unwrap();
        for lane in 0..8 {
            let index = usize::from(indices[lane]);
            let entry = u32::from(words[(palette_start + index) & 511]) | base;
            let opaque = index != 0;
            front[lane] = if opaque { entry } else { front[lane] };
            let behind = u32::from(priority) <= min[lane] >> PRIORITY_SHIFT;
            min[lane] = if opaque && behind { entry } else { min[lane] };
        }
    }
}

/// Paint one background over the surfaces, back to front: a visible pixel
/// becomes the top surface and pushes the previous top to the second.
#[inline(never)]
fn background_pass<const SECOND: bool>(
    row: &[u16; SCREEN_WIDTH],
    window: &[u8; SCREEN_WIDTH],
    bit: u8,
    meta: u32,
    top: &mut [u32; SCREEN_WIDTH],
    second: &mut [u32; SCREEN_WIDTH],
) {
    for x in 0..SCREEN_WIDTH {
        let color = row[x];
        let visible = (color & 0x8000 == 0) & (window[x] & bit != 0);
        if SECOND {
            second[x] = if visible { top[x] } else { second[x] };
        }
        top[x] = if visible { u32::from(color) | meta } else { top[x] };
    }
}

/// Merge the object rows, apply color effects and write the line's colors.
/// Objects follow ZiGBA's compositor: once any admitted object is at or
/// above the top background, the front object (lowest OAM index) becomes the
/// top surface whatever its priority, and the background moves to second.
/// Otherwise the minimum-priority object can only take the second surface.
#[inline(never)]
fn finish<const OBJECTS: bool, const MODE: u8>(
    layers: &mut LineLayers,
    window: &[u8; SCREEN_WIDTH],
    effects: Effects,
    out: &mut [u16; SCREEN_WIDTH],
) {
    for x in 0..SCREEN_WIDTH {
        let w = window[x];
        let mut top = layers.top[x];
        let mut second = layers.second[x];
        if OBJECTS {
            let front = layers.object_front[x];
            let min = layers.object_min[x];
            let admitted = w & 0x10 != 0;
            let min_priority = min >> PRIORITY_SHIFT;
            let wins = admitted & (min_priority <= top >> PRIORITY_SHIFT);
            if MODE == BLEND {
                let behind = admitted & !wins & (min_priority <= second >> PRIORITY_SHIFT);
                second = if wins {
                    top
                } else if behind {
                    min
                } else {
                    second
                };
                layers.second[x] = second;
            }
            top = if wins { front } else { top };
            layers.top[x] = top;
        }
        let color = top & COLOR;
        let effective = w & 0x20 != 0;
        let result = match MODE {
            DIRECT => color,
            BRIGHT => {
                let bright = effective & (top & (SEMI | BRIGHT_FIRST) == BRIGHT_FIRST);
                if bright { brightness(color, effects) } else { color }
            }
            _ => {
                let alpha =
                    effective & (top & (SEMI | ALPHA_FIRST) != 0) & (second & SECOND_TARGET != 0);
                let bright = effective & (top & (SEMI | BRIGHT_FIRST) == BRIGHT_FIRST);
                if alpha {
                    alpha_blend(color, second & COLOR, effects)
                } else if bright {
                    brightness(color, effects)
                } else {
                    color
                }
            }
        };
        out[x] = result as u16;
    }
}

#[inline(always)]
fn alpha_blend(top: u32, bottom: u32, effects: Effects) -> u32 {
    let mut result = 0;
    for shift in [0, 5, 10] {
        let a = (top >> shift) & 31;
        let b = (bottom >> shift) & 31;
        result |= ((a * effects.eva + b * effects.evb) >> 4).min(31) << shift;
    }
    result
}

#[inline(always)]
fn brightness(color: u32, effects: Effects) -> u32 {
    let mut result = 0;
    for shift in [0, 5, 10] {
        let channel = (color >> shift) & 31;
        let adjusted = if effects.brighten {
            channel + (((31 - channel) * effects.factor) >> 4)
        } else {
            channel - ((channel * effects.factor) >> 4)
        };
        result |= adjusted << shift;
    }
    result
}
