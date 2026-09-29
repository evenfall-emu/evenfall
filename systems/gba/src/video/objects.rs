//! Compacted OAM, OBJ fetch budget, row renderer and per-pixel reference.

use super::history::VideoHistoryView;
use super::scanline_memo::{self, ObjectRows};
use crate::bus::{Bus, CompactObject, ObjectLineData, SCREEN_HEIGHT, SCREEN_WIDTH};

#[derive(Clone, Copy)]
struct ObjectShape {
    affine: bool,
    object_width: usize,
    object_height: usize,
    display_width: usize,
    display_height: usize,
    object_y: usize,
    first_processed_x: usize,
}

impl Bus {
    fn object_shape(attr0: u16, attr1: u16) -> Option<ObjectShape> {
        let affine = attr0 & (1 << 8) != 0;
        if !affine && attr0 & (1 << 9) != 0 || attr0 >> 10 & 3 == 3 {
            return None;
        }
        let shape = usize::from(attr0 >> 14);
        if shape == 3 {
            return None;
        }
        let size = usize::from(attr1 >> 14);
        let widths = [[8, 16, 32, 64], [16, 32, 32, 64], [8, 8, 16, 32]];
        let heights = [[8, 16, 32, 64], [8, 8, 16, 32], [16, 32, 32, 64]];
        let object_width = widths[shape][size];
        let object_height = heights[shape][size];
        let doubled = affine && attr0 & (1 << 9) != 0;
        let display_width = if doubled { object_width * 2 } else { object_width };
        let display_height = if doubled { object_height * 2 } else { object_height };
        let object_y = usize::from(attr0 & 0xFF);
        let object_x = usize::from(attr1 & 0x1FF);
        let first_processed_x = if object_x >= SCREEN_WIDTH && object_x + display_width >= 512 {
            512 - object_x
        } else {
            0
        };
        (first_processed_x < display_width).then_some(ObjectShape {
            affine,
            object_width,
            object_height,
            display_width,
            display_height,
            object_y,
            first_processed_x,
        })
    }

    pub(crate) fn object_rows(attr0: u16, attr1: u16) -> Option<ObjectRows> {
        Self::object_shape(attr0, attr1).map(|shape| ObjectRows {
            top: shape.object_y as u8,
            height: shape.display_height as u16,
        })
    }

    fn build_oam_compact(&mut self) {
        let mut count = 0;
        for object in 0..128 {
            let offset = object * 8;
            let attr0 = u16::from_le_bytes([self.oam[offset], self.oam[offset + 1]]);
            let attr1 = u16::from_le_bytes([self.oam[offset + 2], self.oam[offset + 3]]);
            let Some(shape) = Self::object_shape(attr0, attr1) else {
                continue;
            };
            if shape.object_y >= SCREEN_HEIGHT && shape.object_y + shape.display_height <= 256 {
                continue;
            }
            let attr2 = u16::from_le_bytes([self.oam[offset + 4], self.oam[offset + 5]]);
            self.oam_compact[count] = CompactObject {
                attr0,
                attr1,
                attr2,
                affine: shape.affine,
                object_width: shape.object_width as u8,
                object_height: shape.object_height as u8,
                display_width: shape.display_width as u8,
                display_height: shape.display_height as u8,
                object_y: shape.object_y as u8,
                first_processed_x: shape.first_processed_x as u8,
            };
            count += 1;
        }
        self.oam_compact_count = count as u8;
        self.oam_compact_valid = true;
    }

    pub(super) fn build_object_line(&mut self, line: usize) {
        if !self.oam_compact_valid {
            self.build_oam_compact();
        }
        let mut remaining = if self.dispcnt & (1 << 5) != 0 { 954 } else { 1210 };
        let mut count = 0;
        for entry in &self.oam_compact[..usize::from(self.oam_compact_count)] {
            let display_y = (line + 256 - usize::from(entry.object_y)) & 0xFF;
            if display_y >= usize::from(entry.display_height) {
                continue;
            }
            let pixels = usize::from(entry.display_width - entry.first_processed_x);
            let setup = if entry.affine { 10 } else { 0 };
            let cycles_per_pixel = if entry.affine { 2 } else { 1 };
            if remaining <= setup {
                break;
            }
            remaining -= setup;
            let processed = pixels.min(remaining / cycles_per_pixel);
            if processed == 0 {
                break;
            }
            remaining -= processed * cycles_per_pixel;
            self.object_line[count] = ObjectLineData {
                attr0: entry.attr0,
                attr1: entry.attr1,
                attr2: entry.attr2,
                affine: entry.affine,
                object_width: entry.object_width,
                object_height: entry.object_height,
                display_width: entry.display_width,
                display_height: entry.display_height,
                display_y: display_y as u8,
                first_processed_x: entry.first_processed_x,
                processed_end: usize::from(entry.first_processed_x).saturating_add(processed) as u8,
            };
            count += 1;
            if processed != pixels || remaining == 0 {
                break;
            }
        }
        self.object_line_count = count as u8;
        self.object_line_index = line as u16;
    }

    fn object_tile_number(
        base: usize,
        tile_x: usize,
        tile_y: usize,
        width: usize,
        color_256: bool,
        mapping_1d: bool,
    ) -> usize {
        let step = if color_256 { 2 } else { 1 };
        if mapping_1d {
            (base + tile_y * (width / 8) * step + tile_x * step) & 0x3FF
        } else {
            ((base + tile_y * 32) & 0x3E0) | ((base + tile_x * step) & 0x1F)
        }
    }

    /// Render the line's objects in descending OAM order. `ROWS` records
    /// color objects in the layer-major object rows (no window or palette
    /// history); otherwise each pixel is composed into the reference surfaces.
    pub(super) fn render_objects<const ROWS: bool>(
        &mut self,
        line: usize,
        window_pass: bool,
        history: &VideoHistoryView,
    ) {
        if self.object_line_index != line as u16 {
            self.build_object_line(line);
        }
        let mapping_1d = self.dispcnt & (1 << 6) != 0;
        for record_index in (0..usize::from(self.object_line_count)).rev() {
            let record = self.object_line[record_index];
            let attr0 = record.attr0;
            let attr1 = record.attr1;
            let attr2 = record.attr2;
            let object_mode = (attr0 >> 10) & 3;
            if window_pass {
                if object_mode != 2 {
                    continue;
                }
            } else if object_mode >= 2 {
                continue;
            }
            if !record.affine && attr0 & (1 << 12) == 0 && history.palette_records == 0 {
                self.render_regular_object::<ROWS>(line, &record, window_pass, mapping_1d);
                continue;
            }
            self.scanline_memo.reads.vram_pages = scanline_memo::ALL_VRAM_PAGES;
            self.scanline_memo.reads.oam_matrices = true;
            let color_256 = attr0 & (1 << 13) != 0;
            let width = usize::from(record.object_width);
            let height = usize::from(record.object_height);
            let display_width = usize::from(record.display_width);
            let display_height = usize::from(record.display_height);
            let vertical_mosaic = attr0 & (1 << 12) != 0;
            let horizontal_mosaic = vertical_mosaic && !window_pass;
            let mosaic_horizontal = usize::from(self.mosaic >> 8 & 15) + 1;
            let mosaic_vertical = usize::from(self.mosaic >> 12 & 15) + 1;
            let display_y = usize::from(record.display_y);
            let sampled_y = if vertical_mosaic {
                display_y.saturating_sub(line % mosaic_vertical)
            } else {
                display_y
            };
            let object_x = usize::from(attr1 & 0x1FF);
            let extension = if horizontal_mosaic {
                (mosaic_horizontal - (object_x + display_width) % mosaic_horizontal)
                    % mosaic_horizontal
            } else {
                0
            };
            let base_tile = usize::from(attr2 & if color_256 { 0x3FE } else { 0x3FF });
            let bank = usize::from(attr2 >> 12);
            let priority = ((attr2 >> 10) & 3) as u8;
            let matrix_base = usize::from((attr1 >> 9) & 31) * 32;
            let read_matrix = |offset: usize| {
                i16::from_le_bytes([
                    self.oam[matrix_base + offset],
                    self.oam[matrix_base + offset + 1],
                ])
            };
            let (pa, pb, pc, pd) =
                (read_matrix(6), read_matrix(14), read_matrix(22), read_matrix(30));
            let processed_end = usize::from(record.processed_end);
            let drawn = if processed_end == display_width {
                display_width + extension
            } else {
                processed_end
            };
            let first = if object_x < SCREEN_WIDTH { object_x } else { 0 };
            let end = SCREEN_WIDTH.min(if object_x < SCREEN_WIDTH {
                object_x + drawn
            } else {
                (object_x + drawn).saturating_sub(512)
            });
            for x in first..end.max(first) {
                let display_x = (x + 512 - object_x) & 0x1FF;
                let sampled_x = if horizontal_mosaic {
                    let horizontal = usize::from(self.mosaic_for(line, x) >> 8 & 15) + 1;
                    display_x.saturating_sub(x % horizontal).min(display_width - 1)
                } else {
                    display_x
                };
                let (source_x, source_y) = if record.affine {
                    let cx = sampled_x as i32 - (display_width / 2) as i32;
                    let cy = sampled_y as i32 - (display_height / 2) as i32;
                    let sx = ((i32::from(pa) * cx + i32::from(pb) * cy) >> 8) + (width / 2) as i32;
                    let sy = ((i32::from(pc) * cx + i32::from(pd) * cy) >> 8) + (height / 2) as i32;
                    if sx < 0 || sy < 0 || sx >= width as i32 || sy >= height as i32 {
                        continue;
                    }
                    (sx as usize, sy as usize)
                } else {
                    (
                        if attr1 & (1 << 12) != 0 { width - 1 - sampled_x } else { sampled_x },
                        if attr1 & (1 << 13) != 0 { height - 1 - sampled_y } else { sampled_y },
                    )
                };
                let tile = Self::object_tile_number(
                    base_tile,
                    source_x / 8,
                    source_y / 8,
                    width,
                    color_256,
                    mapping_1d,
                );
                if (3..=5).contains(&(self.dispcnt & 7)) && tile < 512 {
                    continue;
                }
                let index = if color_256 {
                    self.vram[0x10000 + tile * 32 + source_y % 8 * 8 + source_x % 8]
                } else {
                    let packed =
                        self.vram[0x10000 + tile * 32 + source_y % 8 * 4 + source_x % 8 / 2];
                    if source_x & 1 == 0 { packed & 15 } else { packed >> 4 }
                };
                if index == 0 {
                    continue;
                }
                let i = line * SCREEN_WIDTH + x;
                if window_pass {
                    self.frame_obj_window[i] = true;
                    continue;
                }
                let palette = if color_256 {
                    256 + usize::from(index)
                } else {
                    256 + bank * 16 + usize::from(index)
                };
                if ROWS {
                    let color = self.tile_row_cache.words[palette];
                    self.put_object_pixel(x, color, priority, object_mode == 1);
                    continue;
                }
                if self.line_window_masks[x] & (1 << 4) == 0 {
                    continue;
                }
                let color = self.palette_color_for(history, x, palette);
                self.compose_object_pixel(i, color, priority, object_mode == 1);
            }
        }
    }

    fn compose_object_pixel(&mut self, i: usize, color: u16, priority: u8, semi: bool) {
        if self.frame_layers[i] != 4 && priority > self.frame_priorities[i] {
            if self.line_needs_second_surface && priority <= self.frame_second_priorities[i] {
                self.frame_second_pixels[i] = color & 0x7FFF;
                self.frame_second_priorities[i] = priority;
                self.frame_second_layers[i] = 4;
            }
            return;
        }
        if self.line_needs_second_surface && self.frame_layers[i] != 4 {
            self.frame_second_pixels[i] = self.frame_pixels[i];
            self.frame_second_priorities[i] = self.frame_priorities[i];
            self.frame_second_layers[i] = self.frame_layers[i];
        }
        self.frame_pixels[i] = color & 0x7FFF;
        self.frame_priorities[i] = priority;
        self.frame_layers[i] = 4;
        self.frame_semitransparent[i] = semi;
    }

    /// Eight independent lanes, in display order. The scalar pixel compositor
    /// remains the reference; LLVM may vectorize the fixed-size lane loops.
    fn compose_object_lanes(
        &mut self,
        first: usize,
        x: usize,
        indices: &[u8; 8],
        palette_start: usize,
        priority: u8,
        semi: bool,
    ) {
        for (lane, &index) in indices.iter().enumerate() {
            if index == 0 || self.line_window_masks[x + lane] & (1 << 4) == 0 {
                continue;
            }
            let color = self.palette_entry_color(palette_start + usize::from(index));
            self.compose_object_pixel(first + lane, color, priority, semi);
        }
    }

    fn render_regular_object<const ROWS: bool>(
        &mut self,
        line: usize,
        record: &ObjectLineData,
        window_pass: bool,
        mapping_1d: bool,
    ) {
        let attr0 = record.attr0;
        let attr1 = record.attr1;
        let attr2 = record.attr2;
        let color_256 = attr0 & (1 << 13) != 0;
        let width = usize::from(record.object_width);
        let height = usize::from(record.object_height);
        let object_x = usize::from(attr1 & 0x1FF);
        let drawn = usize::from(record.processed_end);
        let first = if object_x < SCREEN_WIDTH { object_x } else { 0 };
        let end = SCREEN_WIDTH.min(if object_x < SCREEN_WIDTH {
            object_x + drawn
        } else {
            (object_x + drawn).saturating_sub(512)
        });
        if end <= first {
            return;
        }
        let display_y = usize::from(record.display_y);
        let source_y = if attr1 & (1 << 13) != 0 { height - 1 - display_y } else { display_y };
        let base_tile = usize::from(attr2 & if color_256 { 0x3FE } else { 0x3FF });
        let flip = attr1 & (1 << 12) != 0;
        let first_display = (first + 512 - object_x) & 0x1FF;
        let end_display = first_display + end - first;
        let mut indices = [0u8; 64];
        for tile_x in 0..width / 8 {
            let display_start = if flip { width - 8 - tile_x * 8 } else { tile_x * 8 };
            if display_start >= end_display || display_start + 8 <= first_display {
                continue;
            }
            let tile = Self::object_tile_number(
                base_tile,
                tile_x,
                source_y / 8,
                width,
                color_256,
                mapping_1d,
            );
            if (3..=5).contains(&(self.dispcnt & 7)) && tile < 512 {
                continue;
            }
            let row_base = 0x10000 + tile * 32 + source_y % 8 * if color_256 { 8 } else { 4 };
            self.scanline_memo.reads.add_vram(row_base);
            for lane in 0..8 {
                let index = if color_256 {
                    self.vram[row_base + lane]
                } else {
                    let packed = self.vram[row_base + lane / 2];
                    if lane & 1 == 0 { packed & 15 } else { packed >> 4 }
                };
                indices[display_start + if flip { 7 - lane } else { lane }] = index;
            }
        }
        let palette_start = if color_256 { 256 } else { 256 + usize::from(attr2 >> 12) * 16 };
        let priority = ((attr2 >> 10) & 3) as u8;
        let semi = attr0 & 0xC00 == 0x400;
        let mut x = first;
        let mut display_x = first_display;
        while x + 8 <= end {
            let lanes: &[u8; 8] = indices[display_x..display_x + 8].try_into().unwrap();
            if window_pass {
                for (lane, &index) in lanes.iter().enumerate() {
                    if index != 0 {
                        self.frame_obj_window[line * SCREEN_WIDTH + x + lane] = true;
                    }
                }
            } else if ROWS {
                self.put_object_lanes(x, lanes, palette_start, priority, semi);
            } else {
                self.compose_object_lanes(
                    line * SCREEN_WIDTH + x,
                    x,
                    lanes,
                    palette_start,
                    priority,
                    semi,
                );
            }
            x += 8;
            display_x += 8;
        }
        while x < end {
            let index = indices[display_x];
            if index != 0 {
                let i = line * SCREEN_WIDTH + x;
                if window_pass {
                    self.frame_obj_window[i] = true;
                } else if ROWS {
                    let color = self.tile_row_cache.words[palette_start + usize::from(index)];
                    self.put_object_pixel(x, color, priority, semi);
                } else if self.line_window_masks[x] & (1 << 4) != 0 {
                    self.compose_object_pixel(
                        i,
                        self.palette_entry_color(palette_start + usize::from(index)),
                        priority,
                        semi,
                    );
                }
            }
            x += 1;
            display_x += 1;
        }
    }
}
