//! Scanline dispatch: lines without raster history use the layer-major
//! compositor in `compose.rs`; lines with history use ZiGBA's front-to-back
//! passes, which are also the reference for the layer-major compositor.

use super::history::{BlendState, VideoHistoryView};
use super::scanline_memo::{self, Memory, PAGE_SIZE};
use crate::bus::{Bus, LineScan, SCREEN_HEIGHT, SCREEN_WIDTH, ScanlineRegisters, WindowSegment};

const WINDOW_KEY_INVALID: u64 = 1 << 63;

impl Bus {
    pub(crate) fn render_scanline(&mut self, line: usize) {
        self.object_line_index = 0xFFFF;
        if self.scanline_memo.changes_registers {
            self.scanline_memo.set_registers(self.scanline_registers());
        } else if scanline_memo::CHECK {
            self.scanline_memo.verify_registers(self.scanline_registers());
        }
        self.scanline_memo.observe(
            line,
            Memory { palette: &self.palette, vram: &self.vram, oam: &self.oam },
            Self::object_rows,
        );
        if self.has_raster_history(line) {
            self.scanline_memo.forget(line);
            self.compose_history_scanline(line);
        } else if self.scanline_memo.holds(line) {
            self.scanline_memo.kept += 1;
            if scanline_memo::CHECK {
                self.check_kept_scanline(line);
            }
        } else {
            let pages = self.compose_scanline_fast(line);
            self.scanline_memo.store(line, pages);
        }
    }

    /// Mid-line VRAM writes that touch no page the line reads cannot change
    /// it (a page-flipped bitmap drawn during HDraw, for example), so those
    /// lines keep the layer-major compositor. Other histories use the
    /// reference passes.
    fn compose_history_scanline(&mut self, line: usize) {
        let history = self.video_history_view(line);
        let at = line as u16;
        let vram_only = history.palette_records == 0
            && self.scroll_segment_line != at
            && self.win_segment_line != at
            && self.blend_segment_line != at
            && self.affine_segment_line != at
            && self.mosaic_segment_line != at;
        if vram_only {
            let pages = self.compose_scanline_fast(line);
            let boundary = self.vram_obj_boundary();
            let touched = self
                .vram_history_indices(&history)
                .any(|index| index < boundary && pages >> (index / PAGE_SIZE).min(127) & 1 != 0);
            if !touched {
                return;
            }
        }
        self.compose_scanline_reference(line);
    }

    fn check_kept_scanline(&mut self, line: usize) {
        let first = line * SCREEN_WIDTH;
        let pixels: [u16; SCREEN_WIDTH] =
            self.frame_pixels[first..first + SCREEN_WIDTH].try_into().unwrap();
        self.compose_scanline_fast(line);
        let composed = &self.frame_pixels[first..first + SCREEN_WIDTH];
        if let Some(x) = pixels.iter().zip(composed).position(|(kept, fresh)| kept != fresh) {
            panic!("scanline memo: kept line {line} differs in color at x={x}");
        }
    }

    fn has_raster_history(&self, line: usize) -> bool {
        let line = line as u16;
        self.scroll_segment_line == line
            || self.backdrop_segment_line == line
            || self.win_segment_line == line
            || self.blend_segment_line == line
            || self.affine_segment_line == line
            || self.mosaic_segment_line == line
    }

    fn scanline_registers(&self) -> ScanlineRegisters {
        ScanlineRegisters {
            dispcnt: self.dispcnt,
            green_swap: u16::from(self.green_swap_enabled),
            bg_controls: self.bg_controls,
            bg_horizontal_offsets: self.bg_horizontal_offsets,
            bg_vertical_offsets: self.bg_vertical_offsets,
            windows: [self.win0h, self.win1h, self.win0v, self.win1v, self.winin, self.winout],
            blend: [self.bldcnt, self.bldalpha, self.bldy],
            mosaic: self.mosaic,
            bg_affine_parameters: self.bg_affine_parameters,
            bg_affine_current: self.bg_affine_current,
        }
    }

    /// ZiGBA's front-to-back compositor, which also fills the auxiliary
    /// surfaces (priorities, layers, second surface, semi-transparency).
    pub(super) fn compose_scanline_reference(&mut self, line: usize) {
        let first = line * SCREEN_WIDTH;
        if self.dispcnt & 0x80 != 0 && !self.has_forced_blank_history(line) {
            self.frame_pixels[first..first + SCREEN_WIDTH].fill(0x7FFF);
            self.frame_priorities[first..first + SCREEN_WIDTH].fill(0);
            self.frame_layers[first..first + SCREEN_WIDTH].fill(5);
            return;
        }
        let mode = self.dispcnt & 7;
        let history = self.video_history_view(line);
        self.line_semitransparent_obj = LineScan::Unknown;
        self.line_needs_second_surface = self.needs_second_surface(line);
        self.line_has_color_effects =
            self.line_needs_second_surface || (self.bldcnt & 0x80 != 0 && self.bldcnt & 0x3F != 0);
        if self.line_has_color_effects && self.color_effects_inert(line) {
            self.line_has_color_effects = false;
        }
        if history.backdrop_touched {
            for x in 0..SCREEN_WIDTH {
                let color = self.palette_color_for(&history, x, 0);
                self.frame_pixels[first + x] = color;
                if self.line_needs_second_surface {
                    self.frame_second_pixels[first + x] = color;
                }
            }
        } else {
            let color = self.backdrop_color();
            self.frame_pixels[first..first + SCREEN_WIDTH].fill(color);
            if self.line_needs_second_surface {
                self.frame_second_pixels[first..first + SCREEN_WIDTH].fill(color);
            }
        }
        self.frame_priorities[first..first + SCREEN_WIDTH].fill(4);
        self.frame_layers[first..first + SCREEN_WIDTH].fill(5);
        if self.line_needs_second_surface {
            self.frame_second_priorities[first..first + SCREEN_WIDTH].fill(4);
            self.frame_second_layers[first..first + SCREEN_WIDTH].fill(5);
        }
        self.frame_semitransparent[first..first + SCREEN_WIDTH].fill(false);
        self.frame_obj_window[first..first + SCREEN_WIDTH].fill(false);
        if self.dispcnt & 0x9000 == 0x9000 {
            self.render_objects::<false>(line, true, &history);
        }
        self.resolve_window_masks(line);
        if mode <= 1 && history.palette_records == 0 {
            self.tile_row_cache.prepare_palette(&self.palette);
        }
        if (3..=5).contains(&mode) && self.dispcnt & (1 << 10) == 0 {
            self.finish_scanline(line, &history);
            return;
        }
        if mode <= 2 && self.dispcnt & 0x0F00 != 0 {
            'passes: for priority in 0..4 {
                let range = match mode {
                    0 => 0..4,
                    1 => 0..3,
                    _ => 2..4,
                };
                for background in range {
                    if self.dispcnt & (1 << (background + 8)) == 0
                        || !self.background_uses_priority(background, line, priority)
                    {
                        continue;
                    }
                    if !self.background_line_open(first) {
                        break 'passes;
                    }
                    if background >= 2 && mode != 0 {
                        self.render_affine_background(line, background, priority, &history);
                    } else {
                        self.render_text_background(line, background, priority, &history);
                    }
                }
            }
        } else if (3..=5).contains(&mode) {
            self.render_bitmap_background(line, mode as u8, &history);
        }
        self.finish_scanline(line, &history);
    }

    #[inline(never)]
    pub(super) fn needs_second_surface(&mut self, line: usize) -> bool {
        if self.blend_segment_line == line as u16 && self.blend_segment_count > 0 {
            return true;
        }
        if self.bldcnt & 0x3F00 == 0 {
            return false;
        }
        if self.bldcnt & 0xC0 == 0x40 && self.bldcnt & 0x3F != 0 {
            return true;
        }
        self.semitransparent_object_on_line(line)
    }

    fn semitransparent_object_on_line(&mut self, line: usize) -> bool {
        match self.line_semitransparent_obj {
            LineScan::Present => return true,
            LineScan::Absent => return false,
            LineScan::Unknown => {}
        }
        let mut present = false;
        if self.dispcnt & (1 << 12) != 0 {
            if self.object_line_index != line as u16 {
                self.build_object_line(line);
            }
            present = self.object_line[..usize::from(self.object_line_count)]
                .iter()
                .any(|record| record.attr0 & 0xC00 == 0x400);
        }
        self.line_semitransparent_obj = if present { LineScan::Present } else { LineScan::Absent };
        present
    }

    #[inline(never)]
    pub(super) fn color_effects_inert(&mut self, line: usize) -> bool {
        if self.blend_segment_line == line as u16 && self.blend_segment_count > 0 {
            return false;
        }
        let possible = (self.dispcnt >> 8 & 15)
            | (if self.dispcnt & 0x1000 != 0 { 1 << 4 } else { 0 })
            | (1 << 5);
        if self.bldcnt & 0x3F & possible != 0 {
            return false;
        }
        self.bldcnt & 0x3F00 == 0 || !self.semitransparent_object_on_line(line)
    }

    pub(crate) fn advance_affine_references(&mut self, line: usize) {
        let mode = self.dispcnt & 7;
        for index in 0..2 {
            let active = if index == 0 { (1..=5).contains(&mode) } else { mode == 2 };
            if !active || self.dispcnt & (1 << (index + 10)) == 0 {
                continue;
            }
            let mosaic = self.bg_controls[index + 2] & (1 << 6) != 0;
            let vertical = usize::from(self.mosaic >> 4 & 15) + 1;
            if mosaic && !(line + 1).is_multiple_of(vertical) && line != SCREEN_HEIGHT - 1 {
                continue;
            }
            let count = if mosaic { vertical as i32 } else { 1 };
            for (coordinate, parameter) in [(0, 1), (1, 3)] {
                self.bg_affine_current[index][coordinate] = self.bg_affine_current[index]
                    [coordinate]
                    .wrapping_add(i32::from(self.bg_affine_parameters[index][parameter]) * count);
            }
            self.scanline_memo.mark_registers();
        }
    }

    fn finish_scanline(&mut self, line: usize, history: &VideoHistoryView) {
        if self.dispcnt & (1 << 12) != 0 {
            self.render_objects::<false>(line, false, history);
        }
        if self.line_has_color_effects {
            self.apply_color_effects(line);
        }
        if self.green_swap_enabled {
            self.apply_green_swap(line);
        }
        self.apply_forced_blank(line);
    }

    fn apply_forced_blank(&mut self, line: usize) {
        if !self.has_forced_blank_history(line) {
            return;
        }
        for x in 0..SCREEN_WIDTH {
            if self.display_control_for(line, x) & 0x80 != 0 {
                let i = line * SCREEN_WIDTH + x;
                self.frame_pixels[i] = 0x7FFF;
                self.frame_priorities[i] = 0;
                self.frame_layers[i] = 5;
            }
        }
    }

    pub(super) fn apply_green_swap(&mut self, line: usize) {
        let first = line * SCREEN_WIDTH;
        for x in (0..SCREEN_WIDTH).step_by(2) {
            let left = self.frame_pixels[first + x];
            let right = self.frame_pixels[first + x + 1];
            self.frame_pixels[first + x] = (left & !0x03E0) | (right & 0x03E0);
            self.frame_pixels[first + x + 1] = (right & !0x03E0) | (left & 0x03E0);
        }
    }

    fn apply_color_effects(&mut self, line: usize) {
        if self.blend_segment_line == line as u16 && self.blend_segment_count > 0 {
            self.apply_color_effects_resolved(line, true);
        } else {
            self.apply_color_effects_vector(line);
        }
    }

    /// Fixed 16-lane chunks instead of Zig's `@Vector(16, u16)`; per-lane
    /// window and target decisions keep this loop scalar on current LLVM.
    pub(super) fn apply_color_effects_vector(&mut self, line: usize) {
        let effect = (self.bldcnt >> 6) & 3;
        let first_targets = self.bldcnt & 0x3F;
        let second_targets = (self.bldcnt >> 8) & 0x3F;
        let eva = (self.bldalpha & 31).min(16);
        let evb = ((self.bldalpha >> 8) & 31).min(16);
        let factor = (self.bldy & 31).min(16);
        let first = line * SCREEN_WIDTH;
        for base in (0..SCREEN_WIDTH).step_by(16) {
            let i = first + base;
            let mut colors: [u16; 16] = self.frame_pixels[i..i + 16].try_into().unwrap();
            for (lane, color) in colors.iter_mut().enumerate() {
                let x = base + lane;
                if self.line_window_masks[x] & 0x20 == 0 {
                    continue;
                }
                let index = i + lane;
                let semi = self.frame_semitransparent[index];
                let first_target = first_targets & (1 << self.frame_layers[index]) != 0;
                let alpha = (semi || effect == 1 && first_target)
                    && self.line_needs_second_surface
                    && second_targets & (1 << self.frame_second_layers[index]) != 0;
                let brightness = !semi && first_target && effect >= 2;
                if !alpha && !brightness {
                    continue;
                }
                let original = *color;
                let mut adjusted = 0;
                for shift in [0, 5, 10] {
                    let top = (original >> shift) & 31;
                    let component = if alpha {
                        let bottom = (self.frame_second_pixels[index] >> shift) & 31;
                        ((top * eva + bottom * evb) >> 4).min(31)
                    } else if effect == 2 {
                        top + (((31 - top) * factor) >> 4)
                    } else {
                        top - ((top * factor) >> 4)
                    };
                    adjusted |= component << shift;
                }
                *color = adjusted;
            }
            self.frame_pixels[i..i + 16].copy_from_slice(&colors);
        }
    }

    pub(super) fn apply_color_effects_resolved(&mut self, line: usize, segmented: bool) {
        for x in 0..SCREEN_WIDTH {
            let blend = if segmented {
                self.blend_for(line, x)
            } else {
                BlendState { cnt: self.bldcnt, alpha: self.bldalpha, y: self.bldy }
            };
            self.apply_color_effect_at(line, x, blend);
        }
    }

    fn apply_color_effect_at(&mut self, line: usize, x: usize, blend: BlendState) {
        if self.line_window_masks[x] & 0x20 == 0 {
            return;
        }
        let i = line * SCREEN_WIDTH + x;
        let effect = (blend.cnt >> 6) & 3;
        let semi = self.frame_semitransparent[i];
        if !semi && (effect == 0 || blend.cnt & (1 << self.frame_layers[i]) == 0) {
            return;
        }
        let color = self.frame_pixels[i];
        let mut result = 0;
        if semi || effect == 1 {
            if !self.line_needs_second_surface
                || blend.cnt & (1 << (self.frame_second_layers[i] + 8)) == 0
            {
                return;
            }
            let second = self.frame_second_pixels[i];
            let eva = (blend.alpha & 31).min(16);
            let evb = ((blend.alpha >> 8) & 31).min(16);
            for shift in [0, 5, 10] {
                let a = (color >> shift) & 31;
                let b = (second >> shift) & 31;
                result |= ((a * eva + b * evb) / 16).min(31) << shift;
            }
        } else {
            let factor = (blend.y & 31).min(16);
            for shift in [0, 5, 10] {
                let channel = (color >> shift) & 31;
                let adjusted = if effect == 2 {
                    channel + (31 - channel) * factor / 16
                } else {
                    channel - channel * factor / 16
                };
                result |= adjusted << shift;
            }
        }
        self.frame_pixels[i] = result;
    }

    pub(super) fn resolve_window_masks(&mut self, line: usize) {
        if self.win_segment_line == line as u16 && self.win_segment_count > 0 {
            self.line_window_key = WINDOW_KEY_INVALID;
            self.line_unmasked_layers = 0x3F;
            for x in 0..SCREEN_WIDTH {
                self.line_window_masks[x] = self.window_mask(line, x);
                self.line_unmasked_layers &= self.line_window_masks[x];
            }
            return;
        }
        let enabled = self.dispcnt & (7 << 13);
        if enabled == 0 {
            self.line_window_key = WINDOW_KEY_INVALID;
            self.line_unmasked_layers = 0x3F;
            self.line_window_masks.fill(0x3F);
            return;
        }
        let win0 = enabled & (1 << 13) != 0 && Self::coordinate_in_window(line, self.win0v);
        let win1 = enabled & (1 << 14) != 0 && Self::coordinate_in_window(line, self.win1v);
        if enabled & (1 << 15) == 0 {
            let key = u64::from(self.win0h)
                | (u64::from(self.win1h) << 16)
                | (u64::from(self.winin & 0x3F3F) << 32)
                | (u64::from(self.winout & 0x3F) << 48)
                | (u64::from(win0) << 54)
                | (u64::from(win1) << 55);
            if key == self.line_window_key {
                return;
            }
            self.line_window_key = key;
            self.line_window_masks.fill(self.winout as u8 & 0x3F);
            if win1 {
                self.fill_window_mask(self.win1h, (self.winin >> 8) as u8 & 0x3F);
            }
            if win0 {
                self.fill_window_mask(self.win0h, self.winin as u8 & 0x3F);
            }
            self.line_unmasked_layers = self.line_window_masks[0];
            if win1 {
                self.include_window_boundary_masks(self.win1h);
            }
            if win0 {
                self.include_window_boundary_masks(self.win0h);
            }
            return;
        }
        self.line_window_key = WINDOW_KEY_INVALID;
        self.line_unmasked_layers = 0x3F;
        for x in 0..SCREEN_WIDTH {
            self.line_window_masks[x] = if win0 && Self::coordinate_in_window(x, self.win0h) {
                self.winin as u8 & 0x3F
            } else if win1 && Self::coordinate_in_window(x, self.win1h) {
                (self.winin >> 8) as u8 & 0x3F
            } else if self.frame_obj_window[line * SCREEN_WIDTH + x] {
                (self.winout >> 8) as u8 & 0x3F
            } else {
                self.winout as u8 & 0x3F
            };
            self.line_unmasked_layers &= self.line_window_masks[x];
        }
    }

    fn fill_window_mask(&mut self, bounds: u16, mask: u8) {
        let start = usize::from(bounds >> 8).min(SCREEN_WIDTH);
        let end = usize::from(bounds & 0xFF).min(SCREEN_WIDTH);
        if bounds >> 8 <= bounds & 0xFF {
            self.line_window_masks[start..end].fill(mask);
        } else {
            self.line_window_masks[..end].fill(mask);
            self.line_window_masks[start..].fill(mask);
        }
    }

    fn include_window_boundary_masks(&mut self, bounds: u16) {
        let start = usize::from(bounds >> 8);
        let end = usize::from(bounds & 0xFF);
        if start < SCREEN_WIDTH {
            self.line_unmasked_layers &= self.line_window_masks[start];
        }
        if end < SCREEN_WIDTH {
            self.line_unmasked_layers &= self.line_window_masks[end];
        }
    }

    pub(super) fn window_mask(&self, line: usize, x: usize) -> u8 {
        if self.win_segment_line == line as u16 && self.win_segment_count > 0 {
            let segments = &self.win_segments[..usize::from(self.win_segment_count)];
            let mut index = segments.len() - 1;
            while index > 0 && usize::from(segments[index].start_x & 0xFF) > x {
                index -= 1;
            }
            let s = segments[index];
            return self.window_mask_resolved(
                line,
                x,
                self.window_segment_dispcnt(s),
                s.win0h,
                s.win0v,
                s.win1h,
                s.win1v,
                s.winin,
                s.winout,
            );
        }
        self.window_mask_resolved(
            line,
            x,
            self.dispcnt,
            self.win0h,
            self.win0v,
            self.win1h,
            self.win1v,
            self.winin,
            self.winout,
        )
    }

    fn display_control_for(&self, line: usize, x: usize) -> u16 {
        if self.win_segment_line == line as u16 && self.win_segment_count > 0 {
            let segments = &self.win_segments[..usize::from(self.win_segment_count)];
            let mut index = segments.len() - 1;
            while index > 0 && usize::from(segments[index].start_x & 0xFF) > x {
                index -= 1;
            }
            return self.window_segment_dispcnt(segments[index]);
        }
        self.dispcnt
    }

    fn has_forced_blank_history(&self, line: usize) -> bool {
        self.win_segment_line == line as u16
            && self.win_segments[..usize::from(self.win_segment_count)]
                .iter()
                .any(|s| s.winout & 0x8000 != 0)
    }

    fn window_mask_resolved(
        &self,
        line: usize,
        x: usize,
        dispcnt: u16,
        win0h: u16,
        win0v: u16,
        win1h: u16,
        win1v: u16,
        winin: u16,
        winout: u16,
    ) -> u8 {
        if dispcnt & 0xE000 == 0 {
            return 0x3F;
        }
        if dispcnt & 0x2000 != 0
            && Self::coordinate_in_window(x, win0h)
            && Self::coordinate_in_window(line, win0v)
        {
            return winin as u8 & 0x3F;
        }
        if dispcnt & 0x4000 != 0
            && Self::coordinate_in_window(x, win1h)
            && Self::coordinate_in_window(line, win1v)
        {
            return (winin >> 8) as u8 & 0x3F;
        }
        if dispcnt & 0x8000 != 0 && self.frame_obj_window[line * SCREEN_WIDTH + x] {
            return (winout >> 8) as u8 & 0x3F;
        }
        winout as u8 & 0x3F
    }

    fn window_segment_dispcnt(&self, segment: WindowSegment) -> u16 {
        let mut dispcnt = self.dispcnt;
        if segment.start_x & 0x8000 != 0 {
            dispcnt = (dispcnt & !0x6000) | (segment.start_x & 0x6000);
        }
        if segment.winout & 0x8000 != 0 {
            dispcnt &= !0x80;
            if segment.winout & 0x4000 != 0 {
                dispcnt |= 0x80;
            }
        }
        dispcnt
    }

    pub(super) fn coordinate_in_window(coordinate: usize, bounds: u16) -> bool {
        let start = usize::from(bounds >> 8);
        let end = usize::from(bounds & 0xFF);
        if start <= end {
            coordinate >= start && coordinate < end
        } else {
            coordinate >= start || coordinate < end
        }
    }

    pub(super) fn background_lane_open(&self, i: usize) -> bool {
        self.frame_layers[i] == 5
            || (self.line_needs_second_surface && self.frame_second_layers[i] == 5)
    }

    fn background_line_open(&self, first: usize) -> bool {
        if self.line_needs_second_surface {
            self.frame_second_layers[first..first + SCREEN_WIDTH].contains(&5)
        } else {
            self.frame_layers[first..first + SCREEN_WIDTH].contains(&5)
        }
    }

    pub(super) fn compose_background_pixel(
        &mut self,
        i: usize,
        color: u16,
        priority: u8,
        layer: u8,
    ) {
        if self.frame_layers[i] == 5 {
            self.frame_pixels[i] = color & 0x7FFF;
            self.frame_priorities[i] = priority;
            self.frame_layers[i] = layer;
        } else if self.line_needs_second_surface && self.frame_second_layers[i] == 5 {
            self.frame_second_pixels[i] = color & 0x7FFF;
            self.frame_second_priorities[i] = priority;
            self.frame_second_layers[i] = layer;
        }
    }
}
