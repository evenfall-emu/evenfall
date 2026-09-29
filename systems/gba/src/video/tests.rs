use crate::bus::{
    Access, AccessKind, AccessSource, AccessWidth, Bus, SCREEN_HEIGHT, SCREEN_WIDTH, ScrollSegment,
    WindowSegment,
};

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
    bus.current_cycle = source.current_cycle;
    bus.ppu_timing = source.ppu_timing;
    bus.dispcnt = source.dispcnt;
    bus.bg_controls = source.bg_controls;
    bus.bg_horizontal_offsets = source.bg_horizontal_offsets;
    bus.bg_vertical_offsets = source.bg_vertical_offsets;
    bus.bg_affine_current = source.bg_affine_current;
    bus.bg_affine_parameters = source.bg_affine_parameters;
    bus.bg_reference_points = source.bg_reference_points;
    bus.scroll_segments = source.scroll_segments;
    bus.scroll_segment_counts = source.scroll_segment_counts;
    bus.scroll_segment_line = source.scroll_segment_line;
    bus.backdrop_segments = source.backdrop_segments;
    bus.backdrop_segment_count = source.backdrop_segment_count;
    bus.backdrop_segment_line = source.backdrop_segment_line;
    bus.win_segments = source.win_segments;
    bus.win_segment_count = source.win_segment_count;
    bus.win_segment_line = source.win_segment_line;
    bus.blend_segments = source.blend_segments;
    bus.blend_segment_count = source.blend_segment_count;
    bus.blend_segment_line = source.blend_segment_line;
    bus.affine_segments = source.affine_segments;
    bus.affine_segment_counts = source.affine_segment_counts;
    bus.affine_segment_line = source.affine_segment_line;
    bus.mosaic_segments = source.mosaic_segments;
    bus.mosaic_segment_count = source.mosaic_segment_count;
    bus.mosaic_segment_line = source.mosaic_segment_line;
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

fn write_half(bus: &mut Bus, address: u32, value: u16) {
    bus.write(
        Access {
            address,
            width: AccessWidth::Half,
            kind: AccessKind::Nonsequential,
            source: AccessSource::CpuData,
        },
        u32::from(value),
    );
}

fn assert_surfaces(expected: &Bus, actual: &Bus) {
    assert_eq!(expected.frame_pixels[..], actual.frame_pixels[..]);
    assert_eq!(expected.frame_priorities[..], actual.frame_priorities[..]);
    assert_eq!(expected.frame_layers[..], actual.frame_layers[..]);
    assert_eq!(expected.frame_semitransparent[..], actual.frame_semitransparent[..]);
    assert_eq!(expected.frame_obj_window[..], actual.frame_obj_window[..]);
}

fn randomize_video(bus: &mut Bus, seed: &mut u32) {
    *bus = Bus::init(&[]);
    for byte in &mut bus.palette {
        *byte = next(seed) as u8;
    }
    for chunk in bus.vram.as_chunks_mut::<4>().0 {
        let word = match next(seed) & 3 {
            0 => 0,
            1 => next(seed) | 0x1111_1111,
            2 => next(seed) & 0x0F0F_00F0,
            _ => next(seed),
        };
        chunk.copy_from_slice(&word.to_le_bytes());
    }
    for background in 0..4 {
        let mut control = next(seed) as u16 & !(1 << 6);
        if control & (1 << 7) != 0 && (control >> 2) & 3 == 3 {
            control &= !(1 << 2);
        }
        bus.bg_controls[background] = control;
        bus.bg_horizontal_offsets[background] = next(seed) as u16 & 0x1FF;
        bus.bg_vertical_offsets[background] = next(seed) as u16 & 0x1FF;
    }
    bus.win0h = next(seed) as u16;
    bus.win1h = next(seed) as u16;
    bus.win0v = 0x00A0;
    bus.win1v = next(seed) as u16;
    bus.winin = next(seed) as u16;
    bus.winout = next(seed) as u16;
    bus.bldcnt = next(seed) as u16;
    bus.bldalpha = next(seed) as u16;
    bus.bldy = next(seed) as u16;
    bus.dispcnt = next(seed) as u16 & 0xEF00;
}

#[test]
fn video_register_decoders_guard_out_of_range_addresses_before_subtracting() {
    for address in [0, 0x0400_0000, 0x0400_0006, 0x0400_000f, 0x0400_001f, 0x0400_0040] {
        assert_eq!(Bus::background_control_index(address), None);
        assert_eq!(Bus::background_offset_register(address), None);
        assert_eq!(Bus::affine_parameter_register(address), None);
    }
    assert_eq!(Bus::background_control_index(0x0400_0008), Some(0));
    assert_eq!(Bus::background_control_index(0x0400_000e), Some(3));
    assert_eq!(Bus::background_offset_register(0x0400_0010), Some((0, false)));
    assert_eq!(Bus::background_offset_register(0x0400_001e), Some((3, true)));
    assert_eq!(Bus::affine_parameter_register(0x0400_0020), Some((0, 0)));
    assert_eq!(Bus::affine_parameter_register(0x0400_0036), Some((1, 3)));
    assert_eq!(Bus::affine_parameter_register(0x0400_0031), None);
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

/// ZiGBA: "reused rectangle window masks follow every other mask writer"
#[test]
fn reused_rectangle_window_masks_follow_every_other_mask_writer() {
    let mut bus = Bus::init(&[]);
    bus.win0h = 0x1060;
    bus.win1h = 0x3090;
    bus.win0v = 0x0080;
    bus.win1v = 0x00A0;
    bus.winin = 0x2A15;
    bus.winout = 0x3B39;
    let check = |bus: &mut Bus, line| {
        bus.resolve_window_masks(line);
        let mut unmasked = 0x3F;
        for x in 0..SCREEN_WIDTH {
            assert_eq!(bus.line_window_masks[x], bus.window_mask(line, x), "x={x}, line={line}");
            unmasked &= bus.line_window_masks[x];
        }
        assert_eq!(bus.line_unmasked_layers, unmasked);
    };
    let rectangles = (1 << 13) | (1 << 14);
    for line in [10, 10, 130, 150, 10] {
        bus.dispcnt = rectangles;
        check(&mut bus, line);
    }
    for other in [0, rectangles | (1 << 15)] {
        bus.dispcnt = rectangles;
        check(&mut bus, 10);
        bus.dispcnt = other;
        bus.frame_obj_window[10 * SCREEN_WIDTH + 20] = true;
        check(&mut bus, 10);
        bus.frame_obj_window[10 * SCREEN_WIDTH + 20] = false;
        bus.dispcnt = rectangles;
        check(&mut bus, 10);
    }
    bus.dispcnt = rectangles;
    check(&mut bus, 10);
    bus.win_segment_line = 10;
    bus.win_segment_count = 1;
    bus.win_segments[0] = WindowSegment {
        start_x: 0x8000 | (1 << 13),
        win0h: 0x00F0,
        win0v: 0x00A0,
        winin: 1,
        winout: 2,
        ..WindowSegment::default()
    };
    check(&mut bus, 10);
    bus.win_segment_line = 0xFFFF;
    check(&mut bus, 10);
    bus.winin = 0x2A16;
    check(&mut bus, 10);
    bus.win0h = 0x1061;
    check(&mut bus, 10);
}

/// ZiGBA: "window spans preserve complete scanline rendering state against the pixel path"
#[test]
fn window_spans_preserve_complete_scanline_rendering_state_against_the_pixel_path() {
    let cases = [
        (0x00F0, 0x00F0, 160, 160),
        (0, 0x00F0, 160, 160),
        (0xEF01, 0x00F0, 160, 160),
        (0xE9FF, 0xF500, 160, 160),
        (0xFAF5, 0xF0FF, 160, 160),
        (0xF0F0, 0, 160, 160),
        (0x1ED2, 0x50E6, 160, 160),
        (0x00F0, 0x00F0, 0, 0),
        (0x1ED2, 0x50E6, 0xA032, 0x0032),
    ];
    for (h0, h1, v0, v1) in cases {
        for enabled in [0, 1 << 13, 1 << 14, (1 << 13) | (1 << 14)] {
            for mode in [0, 3] {
                let mut spans = Bus::init(&[]);
                spans.dispcnt = mode | (1 << 8) | (1 << 10) | enabled;
                spans.win0h = h0;
                spans.win1h = h1;
                spans.win0v = v0;
                spans.win1v = v1;
                spans.winin = 0x1425;
                spans.winout = if enabled == 0 { 0x3F } else { 0x16 };
                spans.bg_controls[0] = 31 << 8;
                spans.bg_affine_current[0][1] = 42 * 256;
                spans.bldcnt = (1 << 0) | (1 << 2) | (2 << 6);
                spans.bldy = 7;
                spans.palette[0..2].copy_from_slice(&0x7C00u16.to_le_bytes());
                spans.palette[2..4].copy_from_slice(&0x001Fu16.to_le_bytes());
                spans.vram.fill(0x11);
                let mut pixels = twin(&spans);
                pixels.dispcnt |= 1 << 15;
                pixels.render_scanline(42);
                spans.render_scanline(42);
                assert_line(&spans, &pixels, 42);
                assert_eq!(spans.line_window_masks, pixels.line_window_masks);
                assert_eq!(spans.line_unmasked_layers, pixels.line_unmasked_layers);
                assert_eq!(spans.line_needs_second_surface, pixels.line_needs_second_surface);
                assert_eq!(spans.line_has_color_effects, pixels.line_has_color_effects);
            }
        }
    }
}

/// ZiGBA: "text tile rows match the per-dot text renderer"
#[test]
fn text_tile_rows_match_the_per_dot_text_renderer() {
    let mut seed = 0x7E47_0B6A;
    for _ in 0..400 {
        let mut rows = Bus::init(&[]);
        randomize_video(&mut rows, &mut seed);
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

/// ZiGBA bus_test: "ordinary and effects-aware text spans agree across map wraps flips and transparency"
#[test]
fn ordinary_and_effects_aware_text_spans_agree_across_map_wraps_flips_and_transparency() {
    for color_256 in [false, true] {
        for variant in 0..4 {
            let mut bus = Bus::init(&[]);
            bus.dispcnt = 1 << 9;
            bus.bg_controls[1] = 2 | (24 << 8) | (3 << 14) | u16::from(color_256) << 7;
            bus.bg_horizontal_offsets[1] = 507;
            bus.bg_vertical_offsets[1] = 503;
            match variant {
                1 => {
                    bus.dispcnt |= 1 << 13;
                    bus.win0h = 0x00FF;
                    bus.win0v = 0x00FF;
                    bus.winin = 0x3F;
                }
                2 => {
                    bus.bldcnt = (1 << 1) | (1 << 6) | (1 << 13);
                    bus.bldalpha = 16;
                }
                3 => {
                    bus.dispcnt |= 1 << 13;
                    bus.win0h = (100 << 8) | 160;
                    bus.win0v = 0x00FF;
                    bus.winout = 0x3F;
                }
                _ => {}
            }
            for index in 0..256 {
                bus.palette[index * 2..index * 2 + 2].copy_from_slice(
                    &(0x8000 | ((index * 73 + 0x1234) & 0x7FFF) as u16).to_le_bytes(),
                );
            }
            let depth = if color_256 { 256 } else { 16 };
            for tile in 0..16 {
                for row in 0..8 {
                    for column in 0..8 {
                        let index = ((tile * 17 + row * 3 + column * 5) % depth) as u8;
                        if color_256 {
                            bus.vram[tile * 64 + row * 8 + column] = index;
                        } else {
                            bus.vram[tile * 32 + row * 4 + column / 2] |=
                                index << ((column & 1) * 4);
                        }
                    }
                }
            }
            for tile_y in 0..64 {
                for tile_x in 0..64 {
                    let entry = (((tile_x + tile_y * 3) % 16)
                        | ((tile_x & 1) << 10)
                        | ((tile_y & 1) << 11)
                        | ((tile_y % 16) << 12)) as u16;
                    let offset = 0xC000
                        + (tile_y / 32 * 2 + tile_x / 32) * 0x800
                        + (tile_y % 32 * 32 + tile_x % 32) * 2;
                    bus.vram[offset..offset + 2].copy_from_slice(&entry.to_le_bytes());
                }
            }
            for y in 0..SCREEN_HEIGHT {
                bus.render_scanline(y);
                for x in 0..SCREEN_WIDTH {
                    let world_x = (x + 507) % 512;
                    let world_y = (y + 503) % 512;
                    let tile_x = world_x / 8;
                    let tile_y = world_y / 8;
                    let column = if tile_x & 1 != 0 { 7 - world_x % 8 } else { world_x % 8 };
                    let row = if tile_y & 1 != 0 { 7 - world_y % 8 } else { world_y % 8 };
                    let tile = (tile_x + tile_y * 3) % 16;
                    let index = (tile * 17 + row * 3 + column * 5) % depth;
                    let hidden = variant == 3 && (100..160).contains(&x);
                    let palette_index = if index == 0 || hidden {
                        0
                    } else if color_256 {
                        index
                    } else {
                        tile_y % 16 * 16 + index
                    };
                    let expected = ((palette_index * 73 + 0x1234) & 0x7FFF) as u16;
                    let i = y * SCREEN_WIDTH + x;
                    assert_eq!(
                        bus.frame_pixels[i], expected,
                        "depth={depth}, variant={variant}, x={x}, y={y}"
                    );
                    let visible = index != 0 && !hidden;
                    assert_eq!(bus.frame_priorities[i], if visible { 2 } else { 4 });
                    assert_eq!(bus.frame_layers[i], if visible { 1 } else { 5 });
                }
            }
        }
    }
}

/// ZiGBA: "regular object rows match the per-pixel object renderer"
#[test]
fn regular_object_rows_match_the_per_pixel_object_renderer() {
    let mut seed = 0x0B1E_C7A5;
    for _ in 0..600 {
        let mut rows = Bus::init(&[]);
        randomize_video(&mut rows, &mut seed);
        let mode =
            if next(&mut seed).is_multiple_of(4) { 3 + next(&mut seed) as u16 % 3 } else { 0 };
        rows.dispcnt = (rows.dispcnt & !7) | mode | (1 << 12);
        rows.mosaic = next(&mut seed) as u16;
        for object in 0..128 {
            let offset = object * 8;
            let mut attr0 = next(&mut seed) as u16;
            if !next(&mut seed).is_multiple_of(8) {
                attr0 &= !(1 << 8);
            }
            if !next(&mut seed).is_multiple_of(8) {
                attr0 &= !(1 << 12);
            }
            if attr0 & (1 << 8) == 0 && next(&mut seed) & 1 != 0 {
                attr0 &= !(1 << 9);
            }
            rows.oam[offset..offset + 2].copy_from_slice(&attr0.to_le_bytes());
            let mut attr1 = next(&mut seed) as u16;
            match next(&mut seed) % 4 {
                0 => attr1 = (attr1 & !0x1FF) | (512 - 64 + (attr1 & 63)),
                1 => attr1 = (attr1 & !0x1FF) | (SCREEN_WIDTH as u16 - 64 + (attr1 & 63)),
                _ => {}
            }
            rows.oam[offset + 2..offset + 4].copy_from_slice(&attr1.to_le_bytes());
            rows.oam[offset + 4..offset + 6]
                .copy_from_slice(&(next(&mut seed) as u16).to_le_bytes());
        }
        let line = (next(&mut seed) as usize) % 160;
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
    for _ in 0..2000 {
        vector.bldcnt = next(&mut seed) as u16;
        vector.bldalpha = next(&mut seed) as u16;
        vector.bldy = next(&mut seed) as u16;
        vector.line_needs_second_surface = next(&mut seed) & 1 != 0;
        let window_density = next(&mut seed) % 3;
        for x in 0..SCREEN_WIDTH {
            vector.line_window_masks[x] =
                if window_density == 0 || !next(&mut seed).is_multiple_of(2 * window_density) {
                    next(&mut seed) as u8 & 0x3F | 0x20
                } else {
                    next(&mut seed) as u8 & 0x1F
                };
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

/// ZiGBA: "memoized scanlines notice every change to the memory they read"
#[test]
fn memoized_scanlines_notice_every_change_to_the_memory_they_read() {
    const PALETTE_BASE: u32 = 0x0500_0000;
    const VRAM_BASE: u32 = 0x0600_0000;
    const OAM_BASE: u32 = 0x0700_0000;
    let mut memoized = Bus::init(&[]);
    let mut seed = 0x3E30_11E5;
    for byte in &mut memoized.palette {
        *byte = next(&mut seed) as u8;
    }
    // BG tile 1, affine OBJ tiles 0-3, and regular OBJ tile 64.
    for (start, len) in [(0x20, 64), (0x40, 32), (0x10000, 128), (0x10800, 32)] {
        for byte in &mut memoized.vram[start..start + len] {
            *byte = 0x11 | next(&mut seed) as u8;
        }
    }
    memoized.dispcnt = (1 << 6) | (1 << 8) | (1 << 12);
    memoized.bg_controls[0] = 1 | (31 << 8);
    for entry in 0..1024 {
        memoized.vram[0xF800 + entry * 2..0xF800 + entry * 2 + 2]
            .copy_from_slice(&1u16.to_le_bytes());
    }
    for object in 0..128 {
        memoized.oam[object * 8..object * 8 + 2].copy_from_slice(&(1u16 << 9).to_le_bytes());
    }
    let objects =
        [40 | (1 << 8), 40 | (1 << 14), 0, 0x100, 100, 100, 64, 0, 0, 0, 0, 0, 0, 0, 0, 0x100];
    for (index, half) in objects.into_iter().enumerate() {
        memoized.oam[index * 2..index * 2 + 2].copy_from_slice(&(half as u16).to_le_bytes());
    }
    let toggles: [(u32, usize, u16); 9] = [
        (VRAM_BASE, 40 * 1024, 0xFFFF),    // Unread page.
        (VRAM_BASE, 0xF800 + 64 + 4, 3),   // BG map entry on line 8.
        (VRAM_BASE, 0x20, 0x3333),         // BG tile 1, row 0.
        (VRAM_BASE, 0x10000 + 16, 0x3333), // Affine OBJ tile.
        (VRAM_BASE, 0x10800 + 12, 0x3333), // Regular OBJ tile.
        (OAM_BASE, 6, 0x180),              // Affine matrix PA.
        (OAM_BASE, 8, 100 ^ 120),          // Regular OBJ Y.
        (PALETTE_BASE, 2, 0x1F),
        (VRAM_BASE, 40 * 1024, 0xFFFF), // Unread page after memo priming.
    ];
    for mid_frame in [false, true] {
        for (index, &(base, offset, mask)) in toggles.iter().enumerate() {
            let unread_page = index == toggles.len() - 1;
            if unread_page {
                for line in 0..SCREEN_HEIGHT {
                    memoized.render_scanline(line);
                }
            }
            let mut fresh = twin(&memoized);
            fresh.scanline_memo.invalidate();
            let kept = memoized.scanline_memo.kept;
            for line in 0..SCREEN_HEIGHT {
                if line == usize::from(mid_frame) {
                    for bus in [&mut memoized, &mut fresh] {
                        let memory = if base == VRAM_BASE {
                            bus.vram.as_ref()
                        } else if base == OAM_BASE {
                            &bus.oam[..]
                        } else {
                            &bus.palette[..]
                        };
                        let value = u16::from_le_bytes([memory[offset], memory[offset + 1]]) ^ mask;
                        // Zig calls the memory write method directly, without
                        // recording a raster segment. Write in VBlank here so
                        // the public bus path preserves that same condition.
                        let timing = bus.ppu_timing;
                        bus.ppu_timing.line = SCREEN_HEIGHT as u16;
                        write_half(bus, base + offset as u32, value);
                        bus.ppu_timing = timing;
                    }
                }
                memoized.render_scanline(line);
                fresh.render_scanline(line);
            }
            if unread_page {
                assert_eq!(
                    memoized.scanline_memo.kept,
                    kept + SCREEN_HEIGHT as u64 - 16,
                    "mid_frame={mid_frame}, toggle={index}"
                );
            }
            assert_surfaces(&fresh, &memoized);
        }
    }
}

/// ZiGBA: "the frame reload of affine reference points reaches the scanline memo key"
#[test]
fn the_frame_reload_of_affine_reference_points_reaches_the_scanline_memo_key() {
    let mut bus = Bus::init(&[]);
    // BG2 advances by PD = 1.0 until mode 0 hides it at line 80. Only the
    // frame reload can restore its reference to zero in the next frame.
    write_half(&mut bus, 0x0400_0026, 0x100);
    write_half(&mut bus, 0x0400_0000, 1 | (1 << 10));
    bus.elapse(80 * 1232);
    write_half(&mut bus, 0x0400_0000, 0);
    bus.elapse(2 * 228 * 1232 - bus.now());
    assert_eq!(bus.scanline_memo.registers.bg_affine_current[0][1], 0);
}

/// ZiGBA: "the scanline memo never keeps lines whose surfaces were replaced"
#[test]
fn the_scanline_memo_never_keeps_lines_whose_surfaces_were_replaced() {
    let mut bus = Bus::init(&[]);
    bus.dispcnt = 0x80;
    let mut saved = Vec::new();
    bus.write_state(&mut evenfall_state::Sink::new(Some(&mut saved), None)).unwrap();
    let black = [0u16; SCREEN_WIDTH * SCREEN_HEIGHT];
    for replacement in 0..4 {
        match replacement {
            0 => {}
            1 => bus.read_state(&mut evenfall_state::Source::new(&saved)).unwrap(),
            2 => bus.accept_canonical_framebuffer(&black),
            _ => bus.hard_reset(),
        }
        for line in 0..SCREEN_HEIGHT {
            bus.render_scanline(line);
        }
        assert!(bus.frame_pixels.iter().all(|&pixel| pixel == 0x7FFF), "replacement {replacement}");
    }
}

/// ZiGBA: "scanlines with raster history are rendered again in the next frame"
#[test]
fn scanlines_with_raster_history_are_rendered_again_in_the_next_frame() {
    const MMIO_BASE: u32 = 0x0400_0000;
    let addresses = [0x10, 0x20, 0x40, 0x4C, 0x54]; // Scroll, affine, window, mosaic, blend.
    let mut seed = 0x4A57_E41E;
    for _ in 0..4 {
        for offset in addresses {
            let mut memoized = Bus::init(&[]);
            randomize_video(&mut memoized, &mut seed);
            memoized.dispcnt = 1 | (0b111 << 8) | (1 << 13);
            memoized.bg_controls[0] |= 1 << 6;
            memoized.win0v = SCREEN_HEIGHT as u16;
            memoized.bldcnt = 0x3F | (3 << 6);
            for line in 0..SCREEN_HEIGHT {
                memoized.render_scanline(line);
            }
            let mut fresh = twin(&memoized);
            let history_line = next(&mut seed) as usize % SCREEN_HEIGHT;
            let value = next(&mut seed) as u16;
            for line in 0..SCREEN_HEIGHT {
                if line == history_line {
                    for bus in [&mut memoized, &mut fresh] {
                        bus.ppu_timing.line = line as u16;
                        bus.ppu_timing.cycle_in_line = 4 * 120;
                        write_half(bus, MMIO_BASE + offset, value);
                    }
                }
                memoized.render_scanline(line);
                fresh.render_scanline(line);
            }
            for bus in [&mut memoized, &mut fresh] {
                bus.scroll_segment_line = 0xFFFF;
                bus.win_segment_line = 0xFFFF;
                bus.blend_segment_line = 0xFFFF;
                bus.affine_segment_line = 0xFFFF;
                bus.mosaic_segment_line = 0xFFFF;
            }
            fresh.scanline_memo.invalidate();
            for line in 0..SCREEN_HEIGHT {
                memoized.render_scanline(line);
                fresh.render_scanline(line);
            }
            assert_surfaces(&fresh, &memoized);
        }
    }
}

/// ZiGBA: "memoized scanlines match fresh renders after every kind of input change"
#[test]
fn memoized_scanlines_match_fresh_renders_after_every_kind_of_input_change() {
    const PALETTE_BASE: u32 = 0x0500_0000;
    const VRAM_BASE: u32 = 0x0600_0000;
    const OAM_BASE: u32 = 0x0700_0000;
    let mut seed = 0x5CA1_AB1E;
    for _ in 0..12 {
        let mut memoized = Bus::init(&[]);
        randomize_video(&mut memoized, &mut seed);
        memoized.dispcnt = (next(&mut seed) as u16 & !7) | (next(&mut seed) % 6) as u16 | (1 << 12);
        for control in &mut memoized.bg_controls {
            if next(&mut seed) & 3 == 0 {
                *control |= 1 << 6;
            }
        }
        memoized.mosaic = next(&mut seed) as u16;
        for object in 0..128 {
            let mut attr0 = next(&mut seed) as u16 & !(1 << 12);
            if next(&mut seed) & 7 != 0 {
                attr0 &= !(1 << 8);
            }
            if attr0 & (1 << 8) == 0 {
                attr0 &= !(1 << 9);
            }
            let pos = object * 8;
            memoized.oam[pos..pos + 2].copy_from_slice(&attr0.to_le_bytes());
            for byte in &mut memoized.oam[pos + 2..pos + 6] {
                *byte = next(&mut seed) as u8;
            }
            let parameter = (next(&mut seed) % 1025) as i16 - 512;
            memoized.oam[pos + 6..pos + 8].copy_from_slice(&parameter.to_le_bytes());
        }
        for line in 0..SCREEN_HEIGHT {
            memoized.render_scanline(line);
        }
        for line in 0..SCREEN_HEIGHT {
            memoized.render_scanline(line);
        }
        assert_eq!(memoized.scanline_memo.kept, SCREEN_HEIGHT as u64);
        // Changes before the frame and inside it distinguish a whole-frame
        // invalidation from preserving earlier lines and refreshing later ones.
        for bulk in [true, false] {
            for change in 0..7 {
                let change_line = if bulk { 0 } else { next(&mut seed) as usize % SCREEN_HEIGHT };
                let offset = next(&mut seed);
                let mut values = [0u16; 512];
                for value in &mut values {
                    *value = next(&mut seed) as u16;
                }
                let reset_flags = (((next(&mut seed) % 7) + 1) << 2) as u8;
                let matrix_parameter = ((next(&mut seed) % 1025) as i16 - 512) as u16;
                let mut fresh = twin(&memoized);
                fresh.scanline_memo.invalidate();
                let history_line = next(&mut seed) as usize % SCREEN_HEIGHT;
                let history_color = next(&mut seed) as u16 & 0x7FFF;
                for bus in [&mut memoized, &mut fresh] {
                    bus.backdrop_segment_line = history_line as u16;
                    bus.backdrop_segment_count = 1;
                    bus.backdrop_segments[0] = Bus::palette_segment(0, 0, history_color);
                }
                for line in 0..SCREEN_HEIGHT {
                    if line == change_line {
                        for bus in [&mut memoized, &mut fresh] {
                            match change {
                                0 => {
                                    let bg = offset as usize % 4;
                                    if bulk {
                                        bus.bg_horizontal_offsets[bg] = values[0] & 0x1FF;
                                    } else {
                                        bus.bg_vertical_offsets[bg] = values[0] & 0x1FF;
                                    }
                                    bus.scanline_memo.mark_registers();
                                }
                                1 if bulk => {
                                    for (index, &value) in values[..16].iter().enumerate() {
                                        write_half(
                                            bus,
                                            PALETTE_BASE + offset % 32 * 32 + index as u32 * 2,
                                            value,
                                        );
                                    }
                                }
                                1 => write_half(bus, PALETTE_BASE + offset % 1024, values[0]),
                                2 | 3 if bulk => {
                                    for (index, &value) in values.iter().enumerate() {
                                        write_half(
                                            bus,
                                            VRAM_BASE + offset % 96 * 1024 + index as u32 * 2,
                                            value,
                                        );
                                    }
                                }
                                2 | 3 => {
                                    write_half(bus, VRAM_BASE + offset % (96 * 1024), values[0])
                                }
                                4 if bulk => {
                                    for (index, &value) in values[..3].iter().enumerate() {
                                        write_half(
                                            bus,
                                            OAM_BASE + offset % 128 * 8 + index as u32 * 2,
                                            value,
                                        );
                                    }
                                }
                                4 => write_half(bus, OAM_BASE + offset % 1024, values[0]),
                                5 => write_half(
                                    bus,
                                    OAM_BASE + offset % 128 * 8 + 6,
                                    matrix_parameter,
                                ),
                                _ => {
                                    let dispcnt = bus.dispcnt;
                                    bus.register_ram_reset(reset_flags);
                                    bus.dispcnt = dispcnt;
                                }
                            }
                        }
                    }
                    memoized.render_scanline(line);
                    fresh.render_scanline(line);
                }
                assert_surfaces(&fresh, &memoized);
            }
        }
    }
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

/// ZiGBA: "front-to-back BG passes keep the two front-most BG pixels"
#[test]
fn front_to_back_bg_passes_match_independently_rendered_layers() {
    let mut seed = 0xF20B_ACC5;
    for _ in 0..300 {
        let mut full = Bus::init(&[]);
        randomize_video(&mut full, &mut seed);
        full.dispcnt = (full.dispcnt & 0xEF40) | (next(&mut seed) as u16 % 3) | (1 << 13);
        full.winin &= !0x2020;
        full.winout &= !0x2020;
        if next(&mut seed) & 1 != 0 {
            full.bldcnt = (full.bldcnt & 0x3F00) | (1 << 6) | 1;
        }
        for background in 0..2 {
            for reference in &mut full.bg_affine_current[background] {
                *reference = (next(&mut seed) % 0x18001) as i32 - 0x8000;
            }
            for parameter in &mut full.bg_affine_parameters[background] {
                *parameter = (next(&mut seed) % 0x401) as i16 - 0x200;
            }
        }
        let line = next(&mut seed) as usize % SCREEN_HEIGHT;
        let first = line * SCREEN_WIDTH;
        let mut drawn = [[false; SCREEN_WIDTH]; 4];
        let mut colors = [[0u16; SCREEN_WIDTH]; 4];
        for background in 0..4 {
            let mut single = twin(&full);
            single.dispcnt &= !0xF00 | (1 << (background + 8));
            single.render_scanline(line);
            for x in 0..SCREEN_WIDTH {
                drawn[background][x] = single.frame_layers[first + x] == background as u8;
                colors[background][x] = single.frame_pixels[first + x];
            }
        }
        full.render_scanline(line);
        for x in 0..SCREEN_WIDTH {
            let mut surfaces = [None; 2];
            for priority in 0..4 {
                for (background, admitted) in drawn.iter().enumerate() {
                    if !admitted[x] || full.bg_controls[background] & 3 != priority {
                        continue;
                    }
                    if surfaces[0].is_none() {
                        surfaces[0] = Some(background);
                    } else if surfaces[1].is_none() {
                        surfaces[1] = Some(background);
                    }
                }
            }
            let index = first + x;
            for (depth, surface) in surfaces
                .into_iter()
                .take(if full.line_needs_second_surface { 2 } else { 1 })
                .enumerate()
            {
                let (pixel, priority, layer) = if depth == 0 {
                    (
                        full.frame_pixels[index],
                        full.frame_priorities[index],
                        full.frame_layers[index],
                    )
                } else {
                    (
                        full.frame_second_pixels[index],
                        full.frame_second_priorities[index],
                        full.frame_second_layers[index],
                    )
                };
                if let Some(background) = surface {
                    assert_eq!(layer, background as u8, "x={x}, line={line}, depth={depth}");
                    assert_eq!(priority, (full.bg_controls[background] & 3) as u8);
                    assert_eq!(pixel, colors[background][x]);
                } else {
                    assert_eq!(layer, 5);
                    assert_eq!(priority, 4);
                    assert_eq!(pixel, full.backdrop_color());
                }
            }
        }
    }
}
