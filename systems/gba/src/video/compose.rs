//! Layer-major scanline composition for lines without raster history.
//!
//! Every enabled background renders a color row (bit 15 marks transparency)
//! and objects render two rows: the front object (lowest OAM index) and the
//! object with the smallest priority. Branch-free passes over fixed arrays
//! then pick the top two surfaces per pixel, merge objects and apply color
//! effects. Only `frame_pixels` is written; the front-to-back reference path
//! in `render.rs` also fills the auxiliary surfaces.
//!
//! A surface is a color (RGB555) and a meta word holding the layer (0–3 BG,
//! 4 OBJ, 5 backdrop), blend flags and the priority (4 for the backdrop, 7
//! for no object). One kernel, [`compose_line`], keeps both surfaces of a
//! group of pixels in registers while it paints every background back to
//! front, merges objects and blends; see [`super::lanes`].

use super::backgrounds::{ROW, affine_row, apply_horizontal_mosaic, bitmap_row, text_row};
use super::history::{AffineState, VideoHistoryView};
use super::lanes::Lanes;
use super::scanline_memo::ALL_VRAM_PAGES;
use crate::bus::{Bus, LineScan, SCREEN_WIDTH};

#[cfg(target_arch = "x86_64")]
type Native = super::lanes::Sse2;
#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
type Native = super::lanes::Simd128;
#[cfg(not(any(target_arch = "x86_64", all(target_arch = "wasm32", target_feature = "simd128"))))]
type Native = super::lanes::Scalar;

const LAYER: u16 = 7;
/// The front object is semi-transparent.
const SEMI: u16 = 1 << 3;
/// First target of alpha blending (BLDCNT effect 1).
const ALPHA_FIRST: u16 = 1 << 4;
/// First target of a brightness effect (BLDCNT effect 2 or 3).
const BRIGHT_FIRST: u16 = 1 << 5;
const SECOND_TARGET: u16 = 1 << 6;
const PRIORITY_SHIFT: u16 = 8;
const NO_OBJECT: u16 = 7 << PRIORITY_SHIFT;

/// Line kinds of [`compose_line`]: no color effect, brightness only, alpha
/// blending (BLDCNT effect 0 or 1 with semi-transparent objects, or effect 1),
/// alpha blending with brightness (effect 2 or 3 with semi-transparent
/// objects). The second surface is tracked from `ALPHA` on.
const DIRECT: u8 = 0;
const BRIGHT: u8 = 1;
const ALPHA: u8 = 2;
const ALPHA_BRIGHT: u8 = 3;

/// A composed surface: color before effects, priority, layer.
pub type Surface = (u16, u8, u8);

/// Top and second surfaces of the last composed line.
#[derive(Clone, Copy)]
struct Surfaces {
    top: [u16; SCREEN_WIDTH],
    top_meta: [u16; SCREEN_WIDTH],
    second: [u16; SCREEN_WIDTH],
    second_meta: [u16; SCREEN_WIDTH],
    /// The backdrop's color and meta when the line showed nothing else;
    /// the rows then hold an earlier line.
    backdrop_only: Option<(u16, u16)>,
}

/// The front and minimum-priority object rows.
struct ObjectRows {
    front: [u16; SCREEN_WIDTH],
    front_meta: [u16; SCREEN_WIDTH],
    min: [u16; SCREEN_WIDTH],
    min_meta: [u16; SCREEN_WIDTH],
}

/// Transient per-line buffers of the layer-major compositor.
pub(crate) struct LineLayers {
    pub(super) backgrounds: [[u16; ROW]; 4],
    /// Where each background row's screen line starts.
    offsets: [usize; 4],
    surfaces: Surfaces,
    objects: ObjectRows,
    /// Layer and blend-target bits of the OBJ layer on this line.
    object_meta: u16,
    /// Per line (0–255), the compacted objects that cover it, one bit per
    /// `Bus::oam_compact` index; rebuilt with the compacted OAM.
    pub(super) object_lines: [u128; 256],
}

impl Default for LineLayers {
    fn default() -> Self {
        let plane = [0; SCREEN_WIDTH];
        let surfaces = Surfaces {
            top: plane,
            top_meta: plane,
            second: plane,
            second_meta: plane,
            backdrop_only: None,
        };
        Self {
            backgrounds: [[0; ROW]; 4],
            offsets: [0; 4],
            surfaces,
            objects: ObjectRows {
                front: plane,
                front_meta: plane,
                min: plane,
                min_meta: [NO_OBJECT; SCREEN_WIDTH],
            },
            object_meta: 0,
            object_lines: [0; 256],
        }
    }
}

/// One enabled background of the line.
#[derive(Clone, Copy)]
struct Slot<'a> {
    row: &'a [u16; SCREEN_WIDTH],
    /// Its window-mask bit.
    bit: u16,
    meta: u16,
}

#[derive(Clone, Copy)]
struct Effects {
    eva: u16,
    evb: u16,
    factor: u16,
    brighten: bool,
}

impl Bus {
    /// Compose a line from the live registers and memory. Returns the BG
    /// VRAM pages it read, which decide whether VRAM raster history matters
    /// and which the scanline memo records for the line.
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
        // Layers and color effects read the decoded palette; a line of
        // backdrop alone needs only its first color, read below.
        if self.dispcnt & 0x1F00 != 0 || effects {
            self.tile_row_cache.prepare_palette(&self.palette);
        }

        let bldcnt = self.bldcnt;
        let effect = (bldcnt >> 6) & 3;
        let first_flag = match effect {
            1 => ALPHA_FIRST,
            2 | 3 => BRIGHT_FIRST,
            _ => 0,
        };
        let meta = |layer: u16, priority: u16| {
            let mut meta = layer | priority << PRIORITY_SHIFT;
            if bldcnt >> layer & 1 != 0 {
                meta |= first_flag;
            }
            if bldcnt >> (8 + layer) & 1 != 0 {
                meta |= SECOND_TARGET;
            }
            meta
        };

        let (pages, keys, count) = self.render_background_rows(line);

        let mut objects = false;
        if self.dispcnt & 0x1000 != 0 {
            if self.object_line_index != line as u16 {
                self.build_object_line(line);
            }
            if self.object_line_count > 0 {
                objects = true;
                self.line_layers.objects.min_meta.fill(NO_OBJECT);
                self.line_layers.object_meta = meta(4, 0);
                self.render_objects::<true>(line, false, &history);
            }
        }

        let blend = effects && second;
        let backdrop = u16::from_le_bytes([self.palette[0], self.palette[1]]) & 0x7FFF;
        let backdrop_meta = meta(5, 4);
        let layers = &mut *self.line_layers;
        layers.surfaces.backdrop_only = None;
        if count == 0 && !objects && !effects {
            layers.surfaces.backdrop_only = Some((backdrop, backdrop_meta));
            self.frame_pixels[first..first + SCREEN_WIDTH].fill(backdrop);
            if self.green_swap_enabled {
                self.apply_green_swap(line);
            }
            return pages;
        }
        let empty = Slot { row: &layers.objects.front, bit: 0, meta: 0 };
        let mut slots = [empty; 4];
        for (slot, &key) in slots.iter_mut().zip(keys[..count].iter().rev()) {
            let background = usize::from(key % 8);
            *slot = Slot {
                row: layers.backgrounds[background][layers.offsets[background] & 7..]
                    [..SCREEN_WIDTH]
                    .try_into()
                    .unwrap(),
                bit: 1 << background,
                meta: meta(u16::from(key % 8), u16::from(key / 8)),
            };
        }
        let slots = &slots[..count];
        let backdrop = Surface16 { color: backdrop, meta: backdrop_meta };
        let parameters = Effects {
            eva: (self.bldalpha & 31).min(16),
            evb: ((self.bldalpha >> 8) & 31).min(16),
            factor: (self.bldy & 31).min(16),
            brighten: effect == 2,
        };
        let mode = match (effects, blend, effect >= 2) {
            (false, ..) => DIRECT,
            (true, false, _) => BRIGHT,
            (true, true, false) => ALPHA,
            (true, true, true) => ALPHA_BRIGHT,
        };
        let window = &self.line_window_masks;
        let rows = &layers.objects;
        let surfaces = &mut layers.surfaces;
        let out: &mut [u16; SCREEN_WIDTH] =
            (&mut self.frame_pixels[first..first + SCREEN_WIDTH]).try_into().unwrap();
        let line_inputs =
            LineInputs { slots, window, backdrop, objects: rows, effects: parameters };
        let compose = match (objects, mode) {
            (false, DIRECT) => compose_line::<Native, false, DIRECT>,
            (false, BRIGHT) => compose_line::<Native, false, BRIGHT>,
            (false, ALPHA) => compose_line::<Native, false, ALPHA>,
            (false, _) => compose_line::<Native, false, ALPHA_BRIGHT>,
            (true, DIRECT) => compose_line::<Native, true, DIRECT>,
            (true, BRIGHT) => compose_line::<Native, true, BRIGHT>,
            (true, ALPHA) => compose_line::<Native, true, ALPHA>,
            (true, _) => compose_line::<Native, true, ALPHA_BRIGHT>,
        };
        compose(&line_inputs, surfaces, out);
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
            let mut offset = 0;
            let screen: &mut [u16; SCREEN_WIDTH] = row.first_chunk_mut().unwrap();
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
                    offset = usize::from(hofs) & 7;
                    if horizontal > 1 {
                        let screen: &mut [u16; SCREEN_WIDTH] =
                            (&mut row[offset..offset + SCREEN_WIDTH]).try_into().unwrap();
                        apply_horizontal_mosaic(screen, horizontal);
                    }
                }
                (1, 2) | (2, 2 | 3) => {
                    offset = affine_row(
                        &self.vram,
                        &self.tile_row_cache.bg256,
                        control,
                        affine,
                        horizontal,
                        row,
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
            self.line_layers.offsets[background] = offset;
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
        let surfaces = &self.line_layers.surfaces;
        let decode =
            |color: u16, meta: u16| (color, (meta >> PRIORITY_SHIFT) as u8, (meta & LAYER) as u8);
        if let Some((color, meta)) = surfaces.backdrop_only {
            return (
                decode(color, meta),
                false,
                decode(surfaces.second[x], surfaces.second_meta[x]),
            );
        }
        (
            decode(surfaces.top[x], surfaces.top_meta[x]),
            surfaces.top_meta[x] & SEMI != 0,
            decode(surfaces.second[x], surfaces.second_meta[x]),
        )
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
        let meta = self.line_layers.object_meta
            | u16::from(priority) << PRIORITY_SHIFT
            | if semi { SEMI } else { 0 };
        let rows = &mut self.line_layers.objects;
        rows.front[x] = color;
        rows.front_meta[x] = meta;
        if u16::from(priority) <= rows.min_meta[x] >> PRIORITY_SHIFT {
            rows.min[x] = color;
            rows.min_meta[x] = meta;
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
        let meta = self.line_layers.object_meta
            | u16::from(priority) << PRIORITY_SHIFT
            | if semi { SEMI } else { 0 };
        let rows = &mut self.line_layers.objects;
        for (lane, &index) in indices.iter().enumerate() {
            if index == 0 {
                continue;
            }
            let at = x + lane;
            let color = words[(palette_start + usize::from(index)) & 511];
            rows.front[at] = color;
            rows.front_meta[at] = meta;
            if u16::from(priority) <= rows.min_meta[at] >> PRIORITY_SHIFT {
                rows.min[at] = color;
                rows.min_meta[at] = meta;
            }
        }
    }
}

#[derive(Clone, Copy)]
struct Surface16 {
    color: u16,
    meta: u16,
}

/// Everything [`compose_line`] reads.
struct LineInputs<'a> {
    /// Enabled backgrounds, back to front.
    slots: &'a [Slot<'a>],
    window: &'a [u8; SCREEN_WIDTH],
    backdrop: Surface16,
    objects: &'a ObjectRows,
    effects: Effects,
}

/// Compose a line, `V::WIDTH` pixels at a time. Each visible background,
/// back to front, becomes the top surface and pushes the previous top to the
/// second. Objects follow ZiGBA's compositor: once any admitted object is at
/// or above the top background, the front object (lowest OAM index) becomes
/// the top surface whatever its priority, and the background moves to
/// second; otherwise the minimum-priority object can only take the second
/// surface. Then color effects apply. `surfaces` receives the final top and
/// second surfaces (the second only when `MODE` alpha-blends).
#[inline(never)]
fn compose_line<V: Lanes, const OBJECTS: bool, const MODE: u8>(
    inputs: &LineInputs<'_>,
    surfaces: &mut Surfaces,
    out: &mut [u16; SCREEN_WIDTH],
) {
    let second_surface = MODE >= ALPHA;
    let effects = inputs.effects;
    let eva = V::splat(effects.eva);
    let evb = V::splat(effects.evb);
    let factor = V::splat(effects.factor);
    let brighten = V::splat(if effects.brighten { 0xFFFF } else { 0 });
    let zero = V::splat(0);
    // Per-slot vectors, splatted once per line.
    let count = inputs.slots.len().min(4);
    let mut rows = [&inputs.objects.front; 4];
    let (mut bits, mut metas) = ([zero; 4], [zero; 4]);
    for (index, slot) in inputs.slots.iter().take(4).enumerate() {
        rows[index] = slot.row;
        bits[index] = V::splat(slot.bit);
        metas[index] = V::splat(slot.meta);
    }
    let mut x = 0;
    while x < SCREEN_WIDTH {
        let window = V::load_bytes(inputs.window, x);
        let mut top = V::splat(inputs.backdrop.color);
        let mut top_meta = V::splat(inputs.backdrop.meta);
        let mut second = top;
        let mut second_meta = top_meta;
        for index in 0..count {
            let color = V::load(rows[index], x);
            // Transparent pixels have bit 15 set: negative as i16.
            let hidden = window.and(bits[index]).eq(zero).or(zero.gt(color));
            if second_surface {
                second = V::select(hidden, second, top);
                second_meta = V::select(hidden, second_meta, top_meta);
            }
            top = V::select(hidden, top, color);
            top_meta = V::select(hidden, top_meta, metas[index]);
        }
        if OBJECTS {
            let objects = inputs.objects;
            let admitted = window.any(0x10);
            let min_meta = V::load(&objects.min_meta, x);
            let min_priority = min_meta.shr::<8>();
            let wins = min_priority.gt(top_meta.shr::<8>()).and_not(admitted);
            if second_surface {
                let behind = wins.or(min_priority.gt(second_meta.shr::<8>())).and_not(admitted);
                second = V::select(wins, top, V::select(behind, V::load(&objects.min, x), second));
                second_meta = V::select(wins, top_meta, V::select(behind, min_meta, second_meta));
            }
            top = V::select(wins, V::load(&objects.front, x), top);
            top_meta = V::select(wins, V::load(&objects.front_meta, x), top_meta);
        }
        let result = match MODE {
            DIRECT => top,
            BRIGHT => {
                let bright = top_meta
                    .and(V::splat(SEMI | BRIGHT_FIRST))
                    .eq(V::splat(BRIGHT_FIRST))
                    .and(window.any(0x20));
                V::select(bright, brightness(top, factor, brighten), top)
            }
            _ => {
                let effective = window.any(0x20);
                let alpha = top_meta
                    .any(SEMI | ALPHA_FIRST)
                    .and(second_meta.any(SECOND_TARGET))
                    .and(effective);
                let adjusted = if MODE == ALPHA_BRIGHT {
                    let bright = top_meta
                        .and(V::splat(SEMI | BRIGHT_FIRST))
                        .eq(V::splat(BRIGHT_FIRST))
                        .and(effective);
                    V::select(bright, brightness(top, factor, brighten), top)
                } else {
                    top
                };
                V::select(alpha, alpha_blend(top, second, eva, evb), adjusted)
            }
        };
        result.store(out, x);
        top.store(&mut surfaces.top, x);
        top_meta.store(&mut surfaces.top_meta, x);
        if second_surface {
            second.store(&mut surfaces.second, x);
            second_meta.store(&mut surfaces.second_meta, x);
        }
        x += V::WIDTH;
    }
}

#[inline(always)]
fn alpha_blend<V: Lanes>(top: V, bottom: V, eva: V, evb: V) -> V {
    let channel = |a: V, b: V| {
        let (a, b) = (a.and(V::splat(31)), b.and(V::splat(31)));
        a.mul(eva).add(b.mul(evb)).shr::<4>().min(V::splat(31))
    };
    let red = channel(top, bottom);
    let green = channel(top.shr::<5>(), bottom.shr::<5>());
    let blue = channel(top.shr::<10>(), bottom.shr::<10>());
    red.or(green.shl::<5>()).or(blue.shl::<10>())
}

#[inline(always)]
fn brightness<V: Lanes>(color: V, factor: V, brighten: V) -> V {
    let channel = |value: V| {
        let value = value.and(V::splat(31));
        let up = value.add(V::splat(31).sub(value).mul(factor).shr::<4>());
        let down = value.sub(value.mul(factor).shr::<4>());
        V::select(brighten, up, down)
    };
    let red = channel(color);
    let green = channel(color.shr::<5>());
    let blue = channel(color.shr::<10>());
    red.or(green.shl::<5>()).or(blue.shl::<10>())
}

#[cfg(all(test, target_arch = "x86_64"))]
mod tests {
    use super::*;
    use crate::video::lanes::{Scalar, Sse2};

    fn next(seed: &mut u32) -> u32 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 17;
        *seed ^= *seed << 5;
        *seed
    }

    fn check<const OBJECTS: bool, const MODE: u8>(inputs: &LineInputs<'_>) {
        let empty = Surfaces {
            top: [0; SCREEN_WIDTH],
            top_meta: [0; SCREEN_WIDTH],
            second: [0; SCREEN_WIDTH],
            second_meta: [0; SCREEN_WIDTH],
            backdrop_only: None,
        };
        let (mut vector, mut scalar) = (empty, empty);
        let (mut vector_out, mut scalar_out) = ([0; SCREEN_WIDTH], [0; SCREEN_WIDTH]);
        compose_line::<Sse2, OBJECTS, MODE>(inputs, &mut vector, &mut vector_out);
        compose_line::<Scalar, OBJECTS, MODE>(inputs, &mut scalar, &mut scalar_out);
        assert_eq!(vector_out, scalar_out);
        assert_eq!(vector.top, scalar.top);
        assert_eq!(vector.top_meta, scalar.top_meta);
        assert_eq!(vector.second, scalar.second);
        assert_eq!(vector.second_meta, scalar.second_meta);
    }

    /// The SSE2 lanes compute exactly what the scalar reference lane does.
    #[test]
    fn vector_lanes_match_the_scalar_kernel() {
        let mut seed = 0x1A4E_5EED;
        for _ in 0..500 {
            let mut rows = [[0u16; SCREEN_WIDTH]; 4];
            for row in &mut rows {
                for color in row.iter_mut() {
                    *color = next(&mut seed) as u16;
                }
            }
            let mut window = [0u8; SCREEN_WIDTH];
            let mut objects = ObjectRows {
                front: [0; SCREEN_WIDTH],
                front_meta: [0; SCREEN_WIDTH],
                min: [0; SCREEN_WIDTH],
                min_meta: [0; SCREEN_WIDTH],
            };
            for (x, mask) in window.iter_mut().enumerate() {
                *mask = next(&mut seed) as u8 & 0x3F;
                objects.front[x] = next(&mut seed) as u16 & 0x7FFF;
                objects.min[x] = next(&mut seed) as u16 & 0x7FFF;
                let meta = next(&mut seed) as u16 & 0x7F;
                objects.front_meta[x] = meta | (next(&mut seed) as u16 % 4) << PRIORITY_SHIFT;
                objects.min_meta[x] = meta | (next(&mut seed) as u16 % 8) << PRIORITY_SHIFT;
            }
            let count = next(&mut seed) as usize % 5;
            let slots: Vec<Slot<'_>> = (0..count)
                .map(|index| Slot {
                    row: &rows[index],
                    bit: 1 << index,
                    meta: (next(&mut seed) as u16 & 0x77) | (next(&mut seed) as u16 % 4) << 8,
                })
                .collect();
            let inputs = LineInputs {
                slots: &slots,
                window: &window,
                backdrop: Surface16 {
                    color: next(&mut seed) as u16 & 0x7FFF,
                    meta: (next(&mut seed) as u16 & 0x70) | 5 | 4 << PRIORITY_SHIFT,
                },
                objects: &objects,
                effects: Effects {
                    eva: next(&mut seed) as u16 % 17,
                    evb: next(&mut seed) as u16 % 17,
                    factor: next(&mut seed) as u16 % 17,
                    brighten: next(&mut seed) & 1 != 0,
                },
            };
            check::<false, DIRECT>(&inputs);
            check::<false, BRIGHT>(&inputs);
            check::<false, ALPHA>(&inputs);
            check::<false, ALPHA_BRIGHT>(&inputs);
            check::<true, DIRECT>(&inputs);
            check::<true, BRIGHT>(&inputs);
            check::<true, ALPHA>(&inputs);
            check::<true, ALPHA_BRIGHT>(&inputs);
        }
    }
}
