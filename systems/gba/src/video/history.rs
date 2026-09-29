//! Mid-scanline register, palette and BG VRAM histories. The compact mixed
//! palette/VRAM representation is part of ZiGBA's durable bus state.

use crate::bus::{
    AccessWidth, AffineSegment, BackdropSegment, BlendSegment, Bus, MosaicSegment, ScrollSegment,
    WindowSegment,
};

const MAX_SEGMENTS: usize = 64;
const MMIO_BASE: u32 = 0x0400_0000;
const VRAM_SEGMENT_MARKER: u16 = 0xFFFF;

#[derive(Default, Clone, Copy)]
pub(crate) struct VideoHistoryView {
    pub palette_records: u64,
    pub vram_records: u64,
    pub backdrop_touched: bool,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct VramWriteSnapshot {
    indices: [usize; 4],
    old_values: [u8; 4],
    count: usize,
}

#[derive(Clone, Copy)]
pub(crate) enum AffineTransition {
    PreserveAccumulator,
    ReloadReferenceOrigin,
}

#[derive(Clone, Copy)]
pub(crate) struct BlendState {
    pub cnt: u16,
    pub alpha: u16,
    pub y: u16,
}

#[derive(Clone, Copy)]
pub(crate) struct AffineState {
    pub pa: i32,
    pub pc: i32,
    pub ref_x: i32,
    pub ref_y: i32,
}

impl Bus {
    fn visible_dot(&self) -> Option<u16> {
        (self.ppu_timing.line < 160 && self.ppu_timing.cycle_in_line < 960)
            .then_some(self.ppu_timing.cycle_in_line / 4)
    }

    pub(crate) fn begin_scroll_segment(&mut self, _background: usize) {
        if self.visible_dot().is_none() || self.scroll_segment_line == self.ppu_timing.line {
            return;
        }
        self.scroll_segment_line = self.ppu_timing.line;
        for index in 0..4 {
            self.scroll_segment_counts[index] = 1;
            self.scroll_segments[index][0] = ScrollSegment {
                start_x: Self::scroll_segment_start(0, self.bg_controls[index]),
                hofs: self.bg_horizontal_offsets[index],
                vofs: self.bg_vertical_offsets[index],
            };
        }
    }

    pub(crate) fn end_scroll_segment(&mut self, background: usize) {
        let Some(x) = self.visible_dot() else {
            return;
        };
        if self.scroll_segment_line != self.ppu_timing.line {
            return;
        }
        let count = usize::from(self.scroll_segment_counts[background]);
        if count >= MAX_SEGMENTS {
            return;
        }
        self.scroll_segments[background][count] = ScrollSegment {
            start_x: Self::scroll_segment_start(x, self.bg_controls[background]),
            hofs: self.bg_horizontal_offsets[background],
            vofs: self.bg_vertical_offsets[background],
        };
        self.scroll_segment_counts[background] += 1;
    }

    pub(super) fn scroll_segment_start(x: u16, control: u16) -> u16 {
        0x8000 | ((control & 3) << 8) | x
    }
    pub(super) fn scroll_segment_x(segment: ScrollSegment) -> u16 {
        segment.start_x & 0xFF
    }

    fn scroll_segment_priority(&self, background: usize, segment: ScrollSegment) -> u8 {
        if segment.start_x & 0x8000 == 0 {
            (self.bg_controls[background] & 3) as u8
        } else {
            ((segment.start_x >> 8) & 3) as u8
        }
    }

    pub(super) fn background_priority_for(&self, background: usize, line: usize, x: usize) -> u8 {
        if self.scroll_segment_line == line as u16 && self.scroll_segment_counts[background] > 0 {
            let segments = &self.scroll_segments[background]
                [..usize::from(self.scroll_segment_counts[background])];
            let mut index = segments.len() - 1;
            while index > 0 && usize::from(Self::scroll_segment_x(segments[index])) > x {
                index -= 1;
            }
            return self.scroll_segment_priority(background, segments[index]);
        }
        (self.bg_controls[background] & 3) as u8
    }

    pub(super) fn background_uses_priority(
        &self,
        background: usize,
        line: usize,
        priority: u8,
    ) -> bool {
        if self.scroll_segment_line == line as u16 && self.scroll_segment_counts[background] > 0 {
            return self.scroll_segments[background]
                [..usize::from(self.scroll_segment_counts[background])]
                .iter()
                .any(|&segment| self.scroll_segment_priority(background, segment) == priority);
        }
        self.bg_controls[background] & 3 == u16::from(priority)
    }

    fn palette_write_entries(address: u32, width: AccessWidth) -> ([u16; 4], usize) {
        let mut entries = [0; 4];
        let mut count = 0;
        let width = match width {
            AccessWidth::Byte => 1,
            AccessWidth::Half => 2,
            AccessWidth::Word => 4,
        };
        for offset in 0..width {
            let entry = (((address.wrapping_add(offset) & 0x3FF) / 2) & 0x1FF) as u16;
            if !entries[..count].contains(&entry) {
                entries[count] = entry;
                count += 1;
            }
        }
        (entries, count)
    }

    pub(super) fn palette_segment(entry: u16, x: u16, color: u16) -> BackdropSegment {
        BackdropSegment {
            start_x: ((entry & 0xFF) << 8) | x,
            color: (color & 0x7FFF) | ((entry & 0x100) << 7),
        }
    }

    fn palette_segment_entry(segment: BackdropSegment) -> u16 {
        (segment.start_x >> 8) | ((segment.color >> 15) << 8)
    }

    pub(super) fn palette_entry_color(&self, entry: usize) -> u16 {
        let offset = entry * 2;
        u16::from_le_bytes([self.palette[offset], self.palette[offset + 1]]) & 0x7FFF
    }

    /// Snapshot one physical write before changing VRAM. The caller records
    /// each bus beat after the write, at that beat's completion dot.
    pub(crate) fn snapshot_vram_write(
        &self,
        address: u32,
        width: AccessWidth,
    ) -> VramWriteSnapshot {
        let mut snapshot = VramWriteSnapshot::default();
        let boundary = self.vram_obj_boundary();
        if width == AccessWidth::Byte {
            let Some(index) = self.vram_index(address & !1) else {
                return snapshot;
            };
            if index < boundary {
                snapshot.indices[..2].copy_from_slice(&[index, index + 1]);
                snapshot.old_values[..2].copy_from_slice(&self.vram[index..index + 2]);
                snapshot.count = 2;
            }
            return snapshot;
        }
        let width = if width == AccessWidth::Half { 2 } else { 4 };
        for offset in 0..width {
            let Some(index) = self.vram_index(address.wrapping_add(offset)) else {
                continue;
            };
            if !snapshot.indices[..snapshot.count].contains(&index) {
                snapshot.indices[snapshot.count] = index;
                snapshot.old_values[snapshot.count] = self.vram[index];
                snapshot.count += 1;
            }
        }
        snapshot
    }

    fn vram_index(&self, address: u32) -> Option<usize> {
        let index = (address & 0x1_FFFF) as usize;
        if index >= 0x18000 {
            let mirrored = index - 0x8000;
            if self.dispcnt & 7 >= 3 && mirrored < 0x14000 { None } else { Some(mirrored) }
        } else {
            Some(index)
        }
    }

    pub(super) fn vram_obj_boundary(&self) -> usize {
        if self.dispcnt & 7 >= 3 { 0x14000 } else { 0x10000 }
    }

    /// Whether this line's history can hold no further VRAM write, so that
    /// [`Self::record_vram_write`] would record nothing: a write records two
    /// segments per byte it snapshots, and at least one byte or none.
    #[inline(always)]
    pub(crate) fn vram_history_full(&self) -> bool {
        self.backdrop_segment_line == self.ppu_timing.line
            && usize::from(self.backdrop_segment_count) + 2 > MAX_SEGMENTS
    }

    pub(crate) fn record_vram_write(&mut self, snapshot: VramWriteSnapshot) {
        let Some(x) = self.visible_dot() else {
            return;
        };
        if self.backdrop_segment_line != self.ppu_timing.line {
            self.backdrop_segment_line = self.ppu_timing.line;
            self.backdrop_segment_count = 0;
        }
        if usize::from(self.backdrop_segment_count) + snapshot.count * 2 > MAX_SEGMENTS {
            return;
        }
        for (&index, &old_value) in
            snapshot.indices[..snapshot.count].iter().zip(&snapshot.old_values[..snapshot.count])
        {
            let first = usize::from(self.backdrop_segment_count);
            self.backdrop_segments[first] = BackdropSegment {
                start_x: VRAM_SEGMENT_MARKER,
                color: x | (u16::from(old_value) << 8),
            };
            self.backdrop_segments[first + 1] = BackdropSegment {
                start_x: index as u16,
                color: ((index >> 16) as u16) | (u16::from(self.vram[index]) << 1),
            };
            self.backdrop_segment_count += 2;
        }
    }

    pub(super) fn vram_byte_for(&self, history: &VideoHistoryView, x: usize, index: usize) -> u8 {
        if index >= self.vram_obj_boundary() {
            return 0;
        }
        let mut records = history.vram_records;
        let mut historical = None;
        while records != 0 {
            let offset = records.trailing_zeros() as usize;
            records &= records - 1;
            let header = self.backdrop_segments[offset];
            let payload = self.backdrop_segments[offset + 1];
            let wanted = usize::from(payload.start_x) | (usize::from(payload.color & 1) << 16);
            if wanted == index {
                if historical.is_none() {
                    historical = Some((header.color >> 8) as u8);
                }
                if usize::from(header.color & 0xFF) <= x {
                    historical = Some((payload.color >> 1) as u8);
                }
            }
        }
        historical.unwrap_or(self.vram[index])
    }

    /// VRAM indices written during the line's visible dots.
    pub(super) fn vram_history_indices(
        &self,
        history: &VideoHistoryView,
    ) -> impl Iterator<Item = usize> + '_ {
        let mut records = history.vram_records;
        std::iter::from_fn(move || {
            if records == 0 {
                return None;
            }
            let offset = records.trailing_zeros() as usize;
            records &= records - 1;
            let payload = self.backdrop_segments[offset + 1];
            Some(usize::from(payload.start_x) | (usize::from(payload.color & 1) << 16))
        })
    }

    pub(super) fn video_history_view(&self, line: usize) -> VideoHistoryView {
        let mut view = VideoHistoryView::default();
        if self.backdrop_segment_line != line as u16 {
            return view;
        }
        let mut index = 0;
        while index < usize::from(self.backdrop_segment_count) {
            let segment = self.backdrop_segments[index];
            if segment.start_x == VRAM_SEGMENT_MARKER {
                if index + 1 >= usize::from(self.backdrop_segment_count) {
                    break;
                }
                view.vram_records |= 1 << index;
                index += 2;
            } else {
                view.palette_records |= 1 << index;
                view.backdrop_touched |= Self::palette_segment_entry(segment) == 0;
                index += 1;
            }
        }
        view
    }

    fn has_palette_segment(&self, entry: u16) -> bool {
        let mut index = 0;
        while index < usize::from(self.backdrop_segment_count) {
            let segment = self.backdrop_segments[index];
            if segment.start_x == VRAM_SEGMENT_MARKER {
                if index + 1 >= usize::from(self.backdrop_segment_count) {
                    return false;
                }
                index += 2;
            } else {
                if Self::palette_segment_entry(segment) == entry {
                    return true;
                }
                index += 1;
            }
        }
        false
    }

    pub(crate) fn begin_palette_segments(&mut self, address: u32, width: AccessWidth) {
        if self.visible_dot().is_none() {
            return;
        }
        if self.backdrop_segment_line != self.ppu_timing.line {
            self.backdrop_segment_line = self.ppu_timing.line;
            self.backdrop_segment_count = 0;
        }
        let (entries, count) = Self::palette_write_entries(address, width);
        let mut reserved =
            entries[..count].iter().filter(|&&entry| self.has_palette_segment(entry)).count();
        for &entry in &entries[..count] {
            if self.has_palette_segment(entry) {
                continue;
            }
            if usize::from(self.backdrop_segment_count) + reserved + 2 > MAX_SEGMENTS {
                continue;
            }
            self.backdrop_segments[usize::from(self.backdrop_segment_count)] =
                Self::palette_segment(entry, 0, self.palette_entry_color(entry as usize));
            self.backdrop_segment_count += 1;
            reserved += 1;
        }
    }

    pub(crate) fn end_palette_segments(&mut self, address: u32, width: AccessWidth) {
        let Some(x) = self.visible_dot() else {
            return;
        };
        if self.backdrop_segment_line != self.ppu_timing.line {
            return;
        }
        let (entries, count) = Self::palette_write_entries(address, width);
        for &entry in &entries[..count] {
            if !self.has_palette_segment(entry) {
                continue;
            }
            if usize::from(self.backdrop_segment_count) >= MAX_SEGMENTS {
                return;
            }
            self.backdrop_segments[usize::from(self.backdrop_segment_count)] =
                Self::palette_segment(entry, x, self.palette_entry_color(entry as usize));
            self.backdrop_segment_count += 1;
        }
    }

    pub(super) fn palette_color_for(
        &self,
        history: &VideoHistoryView,
        x: usize,
        entry: usize,
    ) -> u16 {
        let mut records = history.palette_records;
        let mut historical = None;
        while records != 0 {
            let index = records.trailing_zeros() as usize;
            records &= records - 1;
            let segment = self.backdrop_segments[index];
            if Self::palette_segment_entry(segment) as usize == entry
                && usize::from(segment.start_x & 0xFF) <= x
            {
                historical = Some(segment.color & 0x7FFF);
            }
        }
        historical.unwrap_or_else(|| self.palette_entry_color(entry))
    }

    pub(super) fn backdrop_color(&self) -> u16 {
        self.palette_entry_color(0)
    }

    fn window_segment_start(x: u16, dispcnt: u16) -> u16 {
        0x8000 | (dispcnt & 0x6000) | x
    }
    fn window_segment_winout(winout: u16, dispcnt: u16) -> u16 {
        winout | 0x8000 | if dispcnt & 0x80 != 0 { 0x4000 } else { 0 }
    }

    fn window_segment(&self, x: u16) -> WindowSegment {
        WindowSegment {
            start_x: Self::window_segment_start(x, self.dispcnt),
            win0h: self.win0h,
            win0v: self.win0v,
            win1h: self.win1h,
            win1v: self.win1v,
            winin: self.winin,
            winout: Self::window_segment_winout(self.winout, self.dispcnt),
        }
    }

    pub(crate) fn begin_window_segment(&mut self) {
        if self.visible_dot().is_none() || self.win_segment_line == self.ppu_timing.line {
            return;
        }
        self.win_segment_line = self.ppu_timing.line;
        self.win_segment_count = 1;
        self.win_segments[0] = self.window_segment(0);
    }

    pub(crate) fn end_window_segment(&mut self) {
        let Some(x) = self.visible_dot() else {
            return;
        };
        if self.win_segment_line != self.ppu_timing.line
            || usize::from(self.win_segment_count) >= MAX_SEGMENTS
        {
            return;
        }
        self.win_segments[usize::from(self.win_segment_count)] = self.window_segment(x);
        self.win_segment_count += 1;
    }

    pub(crate) fn begin_mosaic_segment(&mut self) {
        if self.visible_dot().is_none() || self.mosaic_segment_line == self.ppu_timing.line {
            return;
        }
        self.mosaic_segment_line = self.ppu_timing.line;
        self.mosaic_segment_count = 1;
        self.mosaic_segments[0] = MosaicSegment { start_x: 0, mosaic: self.mosaic };
    }

    pub(crate) fn end_mosaic_segment(&mut self) {
        let Some(x) = self.visible_dot() else {
            return;
        };
        if self.mosaic_segment_line != self.ppu_timing.line
            || usize::from(self.mosaic_segment_count) >= MAX_SEGMENTS
        {
            return;
        }
        self.mosaic_segments[usize::from(self.mosaic_segment_count)] =
            MosaicSegment { start_x: x, mosaic: self.mosaic };
        self.mosaic_segment_count += 1;
    }

    pub(super) fn mosaic_for(&self, line: usize, x: usize) -> u16 {
        if self.mosaic_segment_line == line as u16 && self.mosaic_segment_count > 0 {
            let segments = &self.mosaic_segments[..usize::from(self.mosaic_segment_count)];
            let mut index = segments.len() - 1;
            while index > 0 && usize::from(segments[index].start_x) > x {
                index -= 1;
            }
            return segments[index].mosaic;
        }
        self.mosaic
    }

    pub(crate) fn begin_blend_segment(&mut self) {
        if self.visible_dot().is_none() || self.blend_segment_line == self.ppu_timing.line {
            return;
        }
        self.blend_segment_line = self.ppu_timing.line;
        self.blend_segment_count = 1;
        self.blend_segments[0] = BlendSegment {
            start_x: 0,
            bldcnt: self.bldcnt,
            bldalpha: self.bldalpha,
            bldy: self.bldy,
        };
    }

    pub(crate) fn end_blend_segment(&mut self) {
        let Some(x) = self.visible_dot() else {
            return;
        };
        if self.blend_segment_line != self.ppu_timing.line
            || usize::from(self.blend_segment_count) >= MAX_SEGMENTS
        {
            return;
        }
        self.blend_segments[usize::from(self.blend_segment_count)] = BlendSegment {
            start_x: x,
            bldcnt: self.bldcnt,
            bldalpha: self.bldalpha,
            bldy: self.bldy,
        };
        self.blend_segment_count += 1;
    }

    pub(super) fn blend_for(&self, line: usize, x: usize) -> BlendState {
        if self.blend_segment_line == line as u16 && self.blend_segment_count > 0 {
            let segments = &self.blend_segments[..usize::from(self.blend_segment_count)];
            let mut index = segments.len() - 1;
            while index > 0 && usize::from(segments[index].start_x) > x {
                index -= 1;
            }
            let s = segments[index];
            return BlendState { cnt: s.bldcnt, alpha: s.bldalpha, y: s.bldy };
        }
        BlendState { cnt: self.bldcnt, alpha: self.bldalpha, y: self.bldy }
    }

    pub(crate) fn begin_affine_segment(&mut self) {
        if self.visible_dot().is_none() || self.affine_segment_line == self.ppu_timing.line {
            return;
        }
        self.affine_segment_line = self.ppu_timing.line;
        for index in 0..2 {
            self.affine_segment_counts[index] = 1;
            self.affine_segments[index][0] = AffineSegment {
                start_x: 0,
                pa: self.bg_affine_parameters[index][0],
                pc: self.bg_affine_parameters[index][2],
                ref_x: self.bg_affine_current[index][0],
                ref_y: self.bg_affine_current[index][1],
            };
        }
    }

    pub(crate) fn end_affine_segment(&mut self, background: usize, transition: AffineTransition) {
        let Some(x) = self.visible_dot() else {
            return;
        };
        if self.affine_segment_line != self.ppu_timing.line {
            return;
        }
        let count = usize::from(self.affine_segment_counts[background]);
        if count >= MAX_SEGMENTS {
            return;
        }
        let pa = self.bg_affine_parameters[background][0];
        let pc = self.bg_affine_parameters[background][2];
        let mut ref_x = self.bg_affine_current[background][0];
        let mut ref_y = self.bg_affine_current[background][1];
        if matches!(transition, AffineTransition::PreserveAccumulator) && count > 0 {
            let previous = self.affine_segments[background][count - 1];
            let split_x = i32::from(x);
            ref_x = previous
                .ref_x
                .wrapping_add(i32::from(previous.pa).wrapping_mul(split_x))
                .wrapping_sub(i32::from(pa).wrapping_mul(split_x));
            ref_y = previous
                .ref_y
                .wrapping_add(i32::from(previous.pc).wrapping_mul(split_x))
                .wrapping_sub(i32::from(pc).wrapping_mul(split_x));
        }
        self.affine_segments[background][count] =
            AffineSegment { start_x: x, pa, pc, ref_x, ref_y };
        self.affine_segment_counts[background] += 1;
    }

    pub(super) fn affine_for(&self, background: usize, line: usize, x: usize) -> AffineState {
        if self.affine_segment_line == line as u16 && self.affine_segment_counts[background] > 0 {
            let segments = &self.affine_segments[background]
                [..usize::from(self.affine_segment_counts[background])];
            let mut index = segments.len() - 1;
            while index > 0 && usize::from(segments[index].start_x) > x {
                index -= 1;
            }
            let s = segments[index];
            return AffineState {
                pa: i32::from(s.pa),
                pc: i32::from(s.pc),
                ref_x: s.ref_x,
                ref_y: s.ref_y,
            };
        }
        AffineState {
            pa: i32::from(self.bg_affine_parameters[background][0]),
            pc: i32::from(self.bg_affine_parameters[background][2]),
            ref_x: self.bg_affine_current[background][0],
            ref_y: self.bg_affine_current[background][1],
        }
    }

    pub(crate) fn background_control_index(address: u32) -> Option<usize> {
        if !(MMIO_BASE + 8..=MMIO_BASE + 14).contains(&address) || address & 1 != 0 {
            return None;
        }
        Some(((address - MMIO_BASE - 8) / 2) as usize)
    }

    pub(crate) fn background_offset_register(address: u32) -> Option<(usize, bool)> {
        if !(MMIO_BASE + 0x10..=MMIO_BASE + 0x1E).contains(&address) || address & 1 != 0 {
            return None;
        }
        Some((((address - MMIO_BASE - 0x10) / 4) as usize, address & 2 != 0))
    }

    pub(crate) fn affine_parameter_register(address: u32) -> Option<(usize, usize)> {
        let background = if (MMIO_BASE + 0x20..=MMIO_BASE + 0x26).contains(&address) {
            0
        } else if (MMIO_BASE + 0x30..=MMIO_BASE + 0x36).contains(&address) {
            1
        } else {
            return None;
        };
        if address & 1 != 0 {
            return None;
        }
        Some((background, ((address - MMIO_BASE - 0x20 - background as u32 * 0x10) / 2) as usize))
    }

    pub(crate) fn affine_reference_register(address: u32) -> Option<(usize, usize, bool)> {
        let background = if (MMIO_BASE + 0x28..=MMIO_BASE + 0x2E).contains(&address) {
            0
        } else if (MMIO_BASE + 0x38..=MMIO_BASE + 0x3E).contains(&address) {
            1
        } else {
            return None;
        };
        if address & 1 != 0 {
            return None;
        }
        let offset = address - MMIO_BASE - 0x28 - background as u32 * 0x10;
        Some((background, (offset / 4) as usize, offset & 2 != 0))
    }
}
