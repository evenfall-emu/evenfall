use crate::bus::{AccessWidth, Bus, SCREEN_WIDTH, ScrollSegment};

use super::history::AffineTransition;

fn next(seed: &mut u32) -> u32 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 17;
    *seed ^= *seed << 5;
    *seed
}

fn twin(source: &Bus) -> Bus {
    let mut bus = Bus::init(&[]);
    bus.palette.copy_from_slice(&source.palette);
    bus.vram.copy_from_slice(source.vram.as_ref());
    bus.oam.copy_from_slice(&source.oam);
    bus.dispcnt = source.dispcnt;
    bus.bg_controls = source.bg_controls;
    bus.bg_horizontal_offsets = source.bg_horizontal_offsets;
    bus.bg_vertical_offsets = source.bg_vertical_offsets;
    bus.bg_affine_current = source.bg_affine_current;
    bus.bg_affine_parameters = source.bg_affine_parameters;
    bus.win0h = source.win0h;
    bus.win1h = source.win1h;
    bus.win0v = source.win0v;
    bus.win1v = source.win1v;
    bus.winin = source.winin;
    bus.winout = source.winout;
    bus.mosaic = source.mosaic;
    bus.bldcnt = source.bldcnt;
    bus.bldalpha = source.bldalpha;
    bus.bldy = source.bldy;
    bus.green_swap_enabled = source.green_swap_enabled;
    bus
}

fn assert_line(left: &Bus, right: &Bus, line: usize) {
    let range = line * SCREEN_WIDTH..(line + 1) * SCREEN_WIDTH;
    assert_eq!(left.frame_pixels[range.clone()], right.frame_pixels[range.clone()]);
    assert_eq!(left.frame_priorities[range.clone()], right.frame_priorities[range.clone()]);
    assert_eq!(left.frame_layers[range.clone()], right.frame_layers[range.clone()]);
    assert_eq!(
        left.frame_semitransparent[range.clone()],
        right.frame_semitransparent[range.clone()]
    );
    assert_eq!(left.frame_obj_window[range.clone()], right.frame_obj_window[range.clone()]);
    if left.line_needs_second_surface {
        assert_eq!(
            left.frame_second_pixels[range.clone()],
            right.frame_second_pixels[range.clone()]
        );
        assert_eq!(
            left.frame_second_priorities[range.clone()],
            right.frame_second_priorities[range.clone()]
        );
        assert_eq!(left.frame_second_layers[range.clone()], right.frame_second_layers[range]);
    }
}

/// ZiGBA: "window spans match the point oracle for every horizontal bound"
#[test]
fn window_spans_match_the_point_oracle_for_every_horizontal_bound() {
    let mut bus = Bus::init(&[]);
    bus.dispcnt = (1 << 13) | (1 << 14);
    bus.win0v = 160;
    bus.win1v = 160;
    bus.winin = 0x2A15;
    bus.winout = 0x39;
    for bounds in 0..=u16::MAX {
        bus.win0h = bounds;
        bus.win1h = bounds.wrapping_mul(40503).wrapping_add(12345);
        bus.resolve_window_masks(42);
        let mut unmasked = 0x3F;
        for x in 0..SCREEN_WIDTH {
            let expected = bus.window_mask(42, x);
            assert_eq!(bus.line_window_masks[x], expected, "bounds={bounds:#06x}, x={x}");
            unmasked &= expected;
        }
        assert_eq!(bus.line_unmasked_layers, unmasked);
    }
}

/// ZiGBA: "text tile rows match the per-dot text renderer"
#[test]
fn text_tile_rows_match_the_per_dot_text_renderer() {
    let mut seed = 0x7E47_0B6A;
    for _ in 0..90 {
        let mut rows = Bus::init(&[]);
        for color in &mut rows.palette {
            *color = next(&mut seed) as u8;
        }
        for chunk in rows.vram.chunks_exact_mut(4) {
            let value = match next(&mut seed) & 3 {
                0 => 0,
                1 => next(&mut seed) | 0x1111_1111,
                2 => next(&mut seed) & 0x0F0F_00F0,
                _ => next(&mut seed),
            };
            chunk.copy_from_slice(&value.to_le_bytes());
        }
        rows.dispcnt =
            ((next(&mut seed) as u16) & 0x0F00) | (1 << 8) | ((next(&mut seed) as u16 & 3) << 13);
        for background in 0..4 {
            let mut control = (next(&mut seed) as u16) & !(1 << 6);
            if control & (1 << 7) != 0 && (control >> 2) & 3 == 3 {
                control &= !(1 << 2);
            }
            rows.bg_controls[background] = control;
            rows.bg_horizontal_offsets[background] = next(&mut seed) as u16 & 0x1FF;
            rows.bg_vertical_offsets[background] = next(&mut seed) as u16 & 0x1FF;
        }
        rows.win0h = next(&mut seed) as u16;
        rows.win1h = next(&mut seed) as u16;
        rows.win0v = 0x00A0;
        rows.win1v = next(&mut seed) as u16;
        rows.winin = next(&mut seed) as u16;
        rows.winout = next(&mut seed) as u16;
        rows.bldcnt = next(&mut seed) as u16;
        rows.bldalpha = next(&mut seed) as u16;
        rows.bldy = next(&mut seed) as u16;
        let line = (next(&mut seed) as usize) % 160;
        let mut dots = twin(&rows);
        dots.scroll_segment_line = line as u16;
        for background in 0..4 {
            dots.scroll_segment_counts[background] = 1;
            dots.scroll_segments[background][0] = ScrollSegment {
                start_x: Bus::scroll_segment_start(0, dots.bg_controls[background]),
                hofs: dots.bg_horizontal_offsets[background],
                vofs: dots.bg_vertical_offsets[background],
            };
        }
        rows.render_scanline(line);
        dots.render_scanline(line);
        assert_line(&rows, &dots, line);
    }
}

/// ZiGBA: "regular object rows match the per-pixel object renderer"
#[test]
fn regular_object_rows_match_the_per_pixel_object_renderer() {
    let mut seed = 0x0B1E_C7A5;
    for _ in 0..100 {
        let mut rows = Bus::init(&[]);
        let line = (next(&mut seed) as usize) % 160;
        rows.dispcnt = 0x1000 | (next(&mut seed) as u16 & 0x40) | if seed & 3 == 0 { 3 } else { 0 };
        rows.mosaic = next(&mut seed) as u16;
        rows.bldcnt = 0x1150;
        rows.bldalpha = 0x0A06;
        for byte in &mut rows.palette {
            *byte = next(&mut seed) as u8;
        }
        for byte in rows.vram.iter_mut() {
            *byte = next(&mut seed) as u8;
        }
        for object in 0..128 {
            let offset = object * 8;
            let mut attr0 = next(&mut seed) as u16 & !((1 << 8) | (1 << 12));
            if object < 12 {
                attr0 &= !(1 << 9);
                attr0 = (attr0 & !0xFF) | ((seed as usize + object) % 160) as u16;
            }
            let mut attr1 = next(&mut seed) as u16;
            if object < 12 {
                attr1 = (attr1 & !0x1FF) | ((object * 27 + 220) % 512) as u16;
            }
            rows.oam[offset..offset + 2].copy_from_slice(&attr0.to_le_bytes());
            rows.oam[offset + 2..offset + 4].copy_from_slice(&attr1.to_le_bytes());
            rows.oam[offset + 4..offset + 6]
                .copy_from_slice(&(next(&mut seed) as u16).to_le_bytes());
        }
        // A known-active regular OBJ exercises the optimized row path even
        // when the random OAM entries happen to miss the scanline.
        rows.oam[..2].copy_from_slice(&(line as u16).to_le_bytes());
        rows.oam[2..4].copy_from_slice(&34u16.to_le_bytes());
        rows.oam[4..6].copy_from_slice(&512u16.to_le_bytes());
        let mut dots = twin(&rows);
        // Entry one is unchanged, but its history forces the per-pixel path.
        dots.backdrop_segment_line = line as u16;
        dots.backdrop_segment_count = 1;
        dots.backdrop_segments[0] = Bus::palette_segment(1, 0, dots.palette_entry_color(1));
        rows.render_scanline(line);
        dots.render_scanline(line);
        assert_line(&rows, &dots, line);
    }
}

/// ZiGBA: "vector color effects match the per-dot effects unit"
#[test]
fn vector_color_effects_match_the_per_dot_effects_unit() {
    let mut vector = Bus::init(&[]);
    let mut dots = Bus::init(&[]);
    let mut seed = 0xB1E4_D0C5;
    let first = 77 * SCREEN_WIDTH;
    for _ in 0..500 {
        vector.bldcnt = next(&mut seed) as u16;
        vector.bldalpha = next(&mut seed) as u16;
        vector.bldy = next(&mut seed) as u16;
        vector.line_needs_second_surface = next(&mut seed) & 1 != 0;
        for x in 0..SCREEN_WIDTH {
            vector.line_window_masks[x] = next(&mut seed) as u8 & 0x3F;
            vector.frame_pixels[first + x] = next(&mut seed) as u16 & 0x7FFF;
            vector.frame_second_pixels[first + x] = next(&mut seed) as u16 & 0x7FFF;
            vector.frame_layers[first + x] = next(&mut seed) as u8 % 6;
            vector.frame_second_layers[first + x] = next(&mut seed) as u8 % 6;
            vector.frame_semitransparent[first + x] = next(&mut seed) & 3 == 0;
        }
        dots.bldcnt = vector.bldcnt;
        dots.bldalpha = vector.bldalpha;
        dots.bldy = vector.bldy;
        dots.line_needs_second_surface = vector.line_needs_second_surface;
        dots.line_window_masks = vector.line_window_masks;
        dots.frame_pixels[first..first + SCREEN_WIDTH]
            .copy_from_slice(&vector.frame_pixels[first..first + SCREEN_WIDTH]);
        dots.frame_second_pixels[first..first + SCREEN_WIDTH]
            .copy_from_slice(&vector.frame_second_pixels[first..first + SCREEN_WIDTH]);
        dots.frame_layers[first..first + SCREEN_WIDTH]
            .copy_from_slice(&vector.frame_layers[first..first + SCREEN_WIDTH]);
        dots.frame_second_layers[first..first + SCREEN_WIDTH]
            .copy_from_slice(&vector.frame_second_layers[first..first + SCREEN_WIDTH]);
        dots.frame_semitransparent[first..first + SCREEN_WIDTH]
            .copy_from_slice(&vector.frame_semitransparent[first..first + SCREEN_WIDTH]);
        vector.apply_color_effects_vector(77);
        dots.apply_color_effects_resolved(77, false);
        assert_eq!(
            dots.frame_pixels[first..first + SCREEN_WIDTH],
            vector.frame_pixels[first..first + SCREEN_WIDTH]
        );
    }
    // Independent RGB555 witness: red 24 mixed with blue 16 using 6/10.
    vector.bldcnt = (1 << 5) | (1 << 6) | (1 << 8);
    vector.bldalpha = (10 << 8) | 6;
    vector.line_needs_second_surface = true;
    vector.line_window_masks[0] = 0x3F;
    vector.frame_layers[first] = 5;
    vector.frame_second_layers[first] = 0;
    vector.frame_semitransparent[first] = false;
    vector.frame_pixels[first] = 24;
    vector.frame_second_pixels[first] = 16 << 10;
    vector.apply_color_effects_vector(77);
    assert_eq!(vector.frame_pixels[first], 9 | (10 << 10));
}

/// ZiGBA: "mixed mid-scanline palette and BG VRAM writes retain their independent histories"
#[test]
fn mixed_palette_and_bg_vram_history_resolves_each_at_its_own_dot() {
    let mut bus = Bus::init(&[]);
    bus.dispcnt = 3;
    bus.ppu_timing.line = 42;
    bus.ppu_timing.cycle_in_line = 4 * 100;
    bus.palette[14..16].copy_from_slice(&0x001Fu16.to_le_bytes());
    bus.vram[0x120] = 0x12;
    let snapshot = bus.snapshot_vram_write(0x0600_0120, AccessWidth::Half);
    bus.vram[0x120] = 0x34;
    bus.record_vram_write(snapshot);

    bus.ppu_timing.cycle_in_line = 4 * 110;
    bus.begin_palette_segments(0x0500_000E, AccessWidth::Half);
    bus.palette[14..16].copy_from_slice(&0x7C00u16.to_le_bytes());
    bus.end_palette_segments(0x0500_000E, AccessWidth::Half);
    let view = bus.video_history_view(42);
    assert_eq!(bus.vram_byte_for(&view, 99, 0x120), 0x12);
    assert_eq!(bus.vram_byte_for(&view, 100, 0x120), 0x34);
    assert_eq!(bus.palette_color_for(&view, 99, 7), 0x001F);
    assert_eq!(bus.palette_color_for(&view, 109, 7), 0x001F);
    assert_eq!(bus.palette_color_for(&view, 110, 7), 0x7C00);
    assert_eq!(bus.vram_byte_for(&view, 110, 0x120), 0x34);
}

/// ZiGBA: "mid-scanline Mode 3 affine write preserves pixels rendered before completion"
#[test]
fn affine_matrix_split_preserves_accumulated_source_coordinate() {
    let mut bus = Bus::init(&[]);
    bus.ppu_timing.line = 42;
    bus.ppu_timing.cycle_in_line = 4 * 100;
    bus.begin_affine_segment();
    bus.bg_affine_parameters[0][0] = 0x200;
    bus.end_affine_segment(0, AffineTransition::PreserveAccumulator);
    let before = bus.affine_for(0, 42, 99);
    let after = bus.affine_for(0, 42, 100);
    assert_eq!(before.ref_x + before.pa * 99, 99 * 256);
    assert_eq!(after.ref_x + after.pa * 100, 100 * 256);
    assert_eq!(after.ref_x + after.pa * 101, 102 * 256);
}

/// ZiGBA: "memoized scanlines notice every change to the memory they read"
#[test]
fn memoized_scanline_invalidates_on_a_changed_bitmap_page() {
    let mut bus = Bus::init(&[]);
    bus.dispcnt = 3 | (1 << 10);
    bus.vram[0] = 0x1F;
    bus.render_scanline(0);
    assert_eq!(bus.frame_pixels[0], 0x001F);
    bus.render_scanline(0);
    assert_eq!(bus.scanline_memo.kept, 1);

    bus.vram[0] = 3;
    bus.scanline_memo.mark_vram(0);
    bus.render_scanline(0);
    assert_eq!(bus.scanline_memo.kept, 1);
    assert_eq!(bus.frame_pixels[0], 3);

    let mut fresh = twin(&bus);
    fresh.render_scanline(0);
    assert_line(&bus, &fresh, 0);
}

/// ZiGBA: "front-to-back BG passes keep the two front-most BG pixels"
#[test]
fn front_to_back_passes_preserve_priority_and_the_next_visible_bg() {
    let mut bus = Bus::init(&[]);
    bus.dispcnt = (1 << 8) | (1 << 9);
    bus.bg_controls[0] = 1 | (28 << 8);
    bus.bg_controls[1] = (1 << 2) | (29 << 8);
    bus.vram[..4].fill(0x11);
    bus.vram[0x4000..0x4004].fill(0x22);
    bus.palette[2..4].copy_from_slice(&0x001Fu16.to_le_bytes());
    bus.palette[4..6].copy_from_slice(&0x03E0u16.to_le_bytes());
    bus.bldcnt = (1 << 1) | (1 << 6) | (1 << 8);
    bus.bldalpha = (8 << 8) | 8;
    bus.render_scanline(0);
    assert_eq!(bus.frame_layers[0], 1);
    assert_eq!(bus.frame_priorities[0], 0);
    assert_eq!(bus.frame_second_layers[0], 0);
    assert_eq!(bus.frame_second_priorities[0], 1);
    assert_eq!(bus.frame_second_pixels[0], 0x001F);
    assert_eq!(bus.frame_pixels[0], 0x01EF);

    // At equal BG priority, the lower BG number is the first surface.
    bus.bg_controls[0] &= !3;
    bus.bldcnt = (1 << 0) | (1 << 6) | (1 << 9);
    bus.scanline_memo.mark_registers();
    bus.render_scanline(0);
    assert_eq!(bus.frame_layers[0], 0);
    assert_eq!(bus.frame_second_layers[0], 1);
    assert_eq!(bus.frame_second_pixels[0], 0x03E0);
    assert_eq!(bus.frame_pixels[0], 0x01EF);
}
