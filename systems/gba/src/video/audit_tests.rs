//! ZiGBA `src/bus.zig` video cache oracles and the full `test/bus_test.zig` text-map oracle.

use crate::bus::{AccessWidth, Bus, SCREEN_HEIGHT, SCREEN_WIDTH, WindowSegment};
use evenfall_state::{Sink, Source};

fn next(seed: &mut u32) -> u32 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 17;
    *seed ^= *seed << 5;
    *seed
}

fn fresh_video(source: &Bus) -> Bus {
    let mut fresh = Bus::init(&[]);
    fresh.palette.copy_from_slice(&source.palette);
    fresh.vram.copy_from_slice(&source.vram[..]);
    fresh.oam.copy_from_slice(&source.oam);
    fresh.dispcnt = source.dispcnt;
    fresh.green_swap_enabled = source.green_swap_enabled;
    fresh.bg_controls = source.bg_controls;
    fresh.bg_horizontal_offsets = source.bg_horizontal_offsets;
    fresh.bg_vertical_offsets = source.bg_vertical_offsets;
    fresh.bg_affine_parameters = source.bg_affine_parameters;
    fresh.bg_affine_current = source.bg_affine_current;
    fresh.bg_reference_points = source.bg_reference_points;
    fresh.win0h = source.win0h;
    fresh.win1h = source.win1h;
    fresh.win0v = source.win0v;
    fresh.win1v = source.win1v;
    fresh.winin = source.winin;
    fresh.winout = source.winout;
    fresh.mosaic = source.mosaic;
    fresh.bldcnt = source.bldcnt;
    fresh.bldalpha = source.bldalpha;
    fresh.bldy = source.bldy;
    fresh
}

fn same_surfaces(expected: &Bus, actual: &Bus) {
    assert_eq!(expected.frame_pixels[..], actual.frame_pixels[..], "pixels");
    assert_eq!(expected.frame_priorities[..], actual.frame_priorities[..], "priorities");
    assert_eq!(expected.frame_layers[..], actual.frame_layers[..], "layers");
    assert_eq!(expected.frame_semitransparent[..], actual.frame_semitransparent[..], "semi OBJ");
    assert_eq!(expected.frame_obj_window[..], actual.frame_obj_window[..], "OBJ window");
}

fn randomize_text_video(bus: &mut Bus, seed: &mut u32) {
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
    bus.dispcnt = (next(seed) as u16 & 0xEF00) | if next(seed) & 1 == 0 { 1 << 6 } else { 0 };
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
}

/// ZiGBA: "reused rectangle window masks follow every other mask writer"
#[test]
fn reused_rectangle_window_masks_follow_every_other_mask_writer() {
    fn check(bus: &mut Bus, line: usize) {
        bus.resolve_window_masks(line);
        let mut unmasked = 0x3F;
        for (x, &mask) in bus.line_window_masks.iter().enumerate() {
            assert_eq!(bus.window_mask(line, x), mask, "line={line}, x={x}");
            unmasked &= mask;
        }
        assert_eq!(unmasked, bus.line_unmasked_layers);
    }

    let mut bus = Bus::init(&[]);
    bus.win0h = 0x1060;
    bus.win1h = 0x3090;
    bus.win0v = 0x0080;
    bus.win1v = 0x00A0;
    bus.winin = 0x2A15;
    bus.winout = 0x3B39;
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
        winin: 0x0001,
        winout: 0x0002,
        ..Default::default()
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
                spans.palette[..2].copy_from_slice(&0x7C00u16.to_le_bytes());
                spans.palette[2..4].copy_from_slice(&0x001Fu16.to_le_bytes());
                spans.vram.fill(0x11);
                let mut pixels = fresh_video(&spans);
                pixels.dispcnt |= 1 << 15;
                pixels.render_scanline(42);
                spans.render_scanline(42);
                pixels.dispcnt = spans.dispcnt;
                pixels.line_window_key = spans.line_window_key;
                pixels.scanline_memo.registers = spans.scanline_memo.registers;
                same_surfaces(&spans, &pixels);
                assert_eq!(spans.frame_second_pixels[..], pixels.frame_second_pixels[..]);
                assert_eq!(spans.frame_second_priorities[..], pixels.frame_second_priorities[..]);
                assert_eq!(spans.frame_second_layers[..], pixels.frame_second_layers[..]);
                assert_eq!(spans.line_window_masks, pixels.line_window_masks);
                assert_eq!(spans.line_unmasked_layers, pixels.line_unmasked_layers);
                assert_eq!(spans.line_window_key, pixels.line_window_key);
                assert_eq!(spans.scanline_memo.registers, pixels.scanline_memo.registers);
                assert_eq!(spans.line_needs_second_surface, pixels.line_needs_second_surface);
                assert_eq!(spans.line_has_color_effects, pixels.line_has_color_effects);
                assert_eq!(
                    std::mem::discriminant(&spans.line_semitransparent_obj),
                    std::mem::discriminant(&pixels.line_semitransparent_obj)
                );
                let mut expected = Vec::new();
                let mut actual = Vec::new();
                spans.write_state(&mut Sink::new(Some(&mut expected), None)).unwrap();
                pixels.write_state(&mut Sink::new(Some(&mut actual), None)).unwrap();
                assert_eq!(expected, actual);
            }
        }
    }
}

/// ZiGBA: "memoized scanlines match fresh renders after every kind of input change"
#[test]
fn memoized_scanlines_match_fresh_renders_after_every_kind_of_input_change() {
    let mut memoized = Bus::init(&[]);
    let mut seed = 0x5CA1_AB1E;
    for _ in 0..12 {
        randomize_text_video(&mut memoized, &mut seed);
        memoized.dispcnt = (memoized.dispcnt & !7) | (next(&mut seed) % 6) as u16 | (1 << 12);
        for control in &mut memoized.bg_controls {
            if next(&mut seed).is_multiple_of(4) {
                *control |= 1 << 6;
            }
        }
        memoized.mosaic = next(&mut seed) as u16;
        for object in 0..128 {
            let offset = object * 8;
            let mut attr0 = next(&mut seed) as u16 & !(1 << 12);
            if !next(&mut seed).is_multiple_of(8) {
                attr0 &= !(1 << 8);
            }
            if attr0 & (1 << 8) == 0 {
                attr0 &= !(1 << 9);
            }
            memoized.oam[offset..offset + 2].copy_from_slice(&attr0.to_le_bytes());
            for byte in &mut memoized.oam[offset + 2..offset + 6] {
                *byte = next(&mut seed) as u8;
            }
            let parameter = ((next(&mut seed) % 0x401) as i16 - 0x200).to_le_bytes();
            memoized.oam[offset + 6..offset + 8].copy_from_slice(&parameter);
        }
        memoized.oam_compact_valid = false;
        for _ in 0..40 {
            let bulk = next(&mut seed) & 1 != 0;
            let change_line = if bulk { 0 } else { next(&mut seed) as usize % SCREEN_HEIGHT };
            let change = if next(&mut seed).is_multiple_of(40) { 6 } else { next(&mut seed) % 6 };
            let reset_flags = (((next(&mut seed) % 7) + 1) << 2) as u8;
            let offset = next(&mut seed);
            let mut values = [0u16; 512];
            for value in &mut values {
                *value = next(&mut seed) as u16;
            }
            let matrix_parameter = ((next(&mut seed) % 0x401) as i16 - 0x200) as u16;
            let mut fresh = fresh_video(&memoized);
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
                                let background = (offset % 4) as usize;
                                if bulk {
                                    bus.bg_horizontal_offsets[background] = values[0] & 0x1FF;
                                } else {
                                    bus.bg_vertical_offsets[background] = values[0] & 0x1FF;
                                }
                                bus.scanline_memo.mark_registers();
                            }
                            1 if bulk => {
                                for (half, &value) in values[..16].iter().enumerate() {
                                    bus.write_palette(
                                        0x0500_0000 + offset % 32 * 32 + half as u32 * 2,
                                        AccessWidth::Half,
                                        u32::from(value),
                                    );
                                }
                            }
                            1 => bus.write_palette(
                                0x0500_0000 + offset % 1024,
                                AccessWidth::Half,
                                u32::from(values[0]),
                            ),
                            2 | 3 if bulk => {
                                for (half, &value) in values.iter().enumerate() {
                                    bus.write_vram(
                                        0x0600_0000
                                            + offset % (96 * 1024 / 1024) * 1024
                                            + half as u32 * 2,
                                        AccessWidth::Half,
                                        u32::from(value),
                                    );
                                }
                            }
                            2 | 3 => bus.write_vram(
                                0x0600_0000 + offset % (96 * 1024),
                                AccessWidth::Half,
                                u32::from(values[0]),
                            ),
                            4 if bulk => {
                                for (half, &value) in values[..3].iter().enumerate() {
                                    bus.write_oam(
                                        0x0700_0000 + offset % 128 * 8 + half as u32 * 2,
                                        AccessWidth::Half,
                                        u32::from(value),
                                    );
                                }
                            }
                            4 => bus.write_oam(
                                0x0700_0000 + offset % 1024,
                                AccessWidth::Half,
                                u32::from(values[0]),
                            ),
                            5 => bus.write_oam(
                                0x0700_0000 + offset % 128 * 8 + 6,
                                AccessWidth::Half,
                                u32::from(matrix_parameter),
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
            same_surfaces(&fresh, &memoized);
        }
    }
}

/// ZiGBA: "scanlines with raster history are rendered again in the next frame"
#[test]
fn scanlines_with_raster_history_are_rendered_again_in_the_next_frame() {
    let mut memoized = Bus::init(&[]);
    let mut seed = 0x4A57_E41E;
    let addresses = [0x0400_0010, 0x0400_0020, 0x0400_0040, 0x0400_004C, 0x0400_0054];
    for _ in 0..16 {
        for address in addresses {
            randomize_text_video(&mut memoized, &mut seed);
            memoized.dispcnt = 1 | (0b111 << 8) | (1 << 13);
            memoized.bg_controls[0] |= 1 << 6;
            memoized.win0v = SCREEN_HEIGHT as u16;
            memoized.bldcnt = 0x3F | (3 << 6);
            for line in 0..SCREEN_HEIGHT {
                memoized.render_scanline(line);
            }
            let mut fresh = fresh_video(&memoized);
            let history_line = next(&mut seed) as usize % SCREEN_HEIGHT;
            let value = next(&mut seed) as u16;
            for line in 0..SCREEN_HEIGHT {
                if line == history_line {
                    for bus in [&mut memoized, &mut fresh] {
                        bus.ppu_timing.line = line as u16;
                        bus.ppu_timing.cycle_in_line = 4 * 120;
                        bus.write_mmio_half(address, value);
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
            same_surfaces(&fresh, &memoized);
        }
    }
}

/// ZiGBA: "the frame reload of affine reference points reaches the scanline memo key"
#[test]
fn the_frame_reload_of_affine_reference_points_reaches_the_scanline_memo_key() {
    let mut bus = Bus::init(&[]);
    bus.write_mmio_half(0x0400_0026, 0x100);
    bus.write_mmio_half(0x0400_0000, 1 | (1 << 10));
    bus.elapse(80 * 1232);
    bus.write_mmio_half(0x0400_0000, 0);
    bus.elapse(2 * 228 * 1232 - bus.current_cycle);
    assert_eq!(bus.scanline_memo.registers.bg_affine_current[0][1], 0);
}

/// ZiGBA: "the scanline memo never keeps lines whose surfaces were replaced"
#[test]
fn the_scanline_memo_never_keeps_lines_whose_surfaces_were_replaced() {
    let mut bus = Bus::init(&[]);
    bus.dispcnt = 0x0080;
    let mut state = Vec::new();
    bus.write_state(&mut Sink::new(Some(&mut state), None)).unwrap();
    let black = vec![0u16; SCREEN_WIDTH * SCREEN_HEIGHT];
    for replacement in 0..4 {
        match replacement {
            0 => {}
            1 => bus.read_state(&mut Source::new(&state)).unwrap(),
            2 => bus.accept_canonical_framebuffer(&black),
            _ => bus.hard_reset(),
        }
        for line in 0..SCREEN_HEIGHT {
            bus.render_scanline(line);
        }
        assert!(bus.frame_pixels.iter().all(|&pixel| pixel == 0x7FFF));
    }
}

/// ZiGBA `test/bus_test.zig`: "ordinary and effects-aware text spans agree across map wraps flips and transparency"
#[test]
fn ordinary_and_effects_aware_text_spans_agree_across_map_wraps_flips_and_transparency() {
    for color_256 in [false, true] {
        for variant in 0..4 {
            let mut bus = Bus::init(&[]);
            bus.dispcnt = 1 << 9;
            bus.bg_controls[1] = 2 | (24 << 8) | (3 << 14) | (u16::from(color_256) << 7);
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
            bus.elapse(228 * 1232);
            for y in 0..SCREEN_HEIGHT {
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
                    let at = y * SCREEN_WIDTH + x;
                    assert_eq!(
                        bus.frame_pixels[at], expected,
                        "depth={depth}, variant={variant}, x={x}, y={y}"
                    );
                    let visible = index != 0 && !hidden;
                    assert_eq!(bus.frame_priorities[at], if visible { 2 } else { 4 });
                    assert_eq!(bus.frame_layers[at], if visible { 1 } else { 5 });
                }
            }
        }
    }
}
