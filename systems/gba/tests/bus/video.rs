//! Video integration oracles, exercised through Bus writes and time.

use super::*;

#[test]
fn mid_scanline_mode_4_palette_word_write_exposes_each_halfword_at_its_completion_dot() {
    let mut bus = Bus::init(&[]);
    write_half(&mut bus, 0x0400_0000, 4 | (1 << 10));
    write_half(&mut bus, 0x0400_0020, 0x0100);
    write_word(&mut bus, 0x0500_0004, 0x001f_001f);
    write_half(&mut bus, 0x0600_0062, 0x0302);
    bus.elapse(280896 - bus.now());
    bus.elapse(394);
    let cycles = write_word(&mut bus, 0x0500_0004, 0x7c00_03e0);
    bus.elapse(960 - 396);
    assert_eq!(2, cycles);
    assert_eq!(0x03e0, bus.framebuffer()[98]);
    assert_eq!(0x7c00, bus.framebuffer()[99]);
}

#[test]
fn full_mid_scanline_video_log_never_leaves_an_orphaned_palette_start_value() {
    let mut bus = Bus::init(&[]);
    write_half(&mut bus, 0x0400_0000, 4 | (1 << 10));
    bus.vram[..240].fill(2);
    for index in 0..15 {
        write_half(&mut bus, 0x0601_2000 + index * 2, (index + 1) as u16);
    }
    write_half(&mut bus, 0x0500_0002, 0x001f);
    write_half(&mut bus, 0x0500_0002, 0x03e0);
    write_half(&mut bus, 0x0500_0004, 0x7c00);
    bus.elapse(960 - bus.now());
    assert_eq!(0x7c00, bus.framebuffer()[100]);
}

#[test]
fn mid_scanline_mode_3_affine_write_preserves_pixels_rendered_before_completion() {
    let mut bus = Bus::init(&[]);
    write_half(&mut bus, 0x0400_0000, 3 | (1 << 10));
    write_half(&mut bus, 0x0400_0020, 0);
    for (address, value) in [
        (0x0600_0000, 0x001f),
        (0x0600_0064, 0x03e0),
        (0x0600_00c8, 0x03e0),
        (0x0600_00d8, 0x03e0),
        (0x0600_0190, 0x7c00),
    ] {
        write_half(&mut bus, address, value);
    }
    bus.elapse(280896 - bus.now());
    bus.elapse(399);
    write_half(&mut bus, 0x0400_0020, 0x0100);
    bus.elapse(960 - 400);
    assert_eq!(0x001f, bus.framebuffer()[50]);
    assert_eq!(0x03e0, bus.framebuffer()[200]);
}

#[test]
fn mid_scanline_mode_3_vram_word_write_exposes_each_halfword_at_its_completion_dot() {
    let mut bus = Bus::init(&[]);
    write_half(&mut bus, 0x0400_0000, 3 | (1 << 10));
    write_half(&mut bus, 0x0400_0020, 0x0100);
    write_word(&mut bus, 0x0600_00c4, 0x001f_001f);
    bus.elapse(280896 - bus.now());
    bus.elapse(394);
    let cycles = write_word(&mut bus, 0x0600_00c4, 0x7c00_03e0);
    bus.elapse(960 - 396);
    assert_eq!(2, cycles);
    assert_eq!(0x03e0, bus.framebuffer()[98]);
    assert_eq!(0x7c00, bus.framebuffer()[99]);
}

#[test]
fn mixed_mid_scanline_palette_and_bg_vram_writes_retain_their_independent_histories() {
    let mut bus = Bus::init(&[]);
    write_half(&mut bus, 0x0400_0000, 4 | (1 << 10));
    write_half(&mut bus, 0x0400_0020, 0);
    write_half(&mut bus, 0x0500_0002, 0x001f);
    write_half(&mut bus, 0x0500_0004, 0x7c00);
    write_half(&mut bus, 0x0600_0000, 0x0101);
    bus.elapse(280896 - bus.now());
    bus.elapse(319);
    write_half(&mut bus, 0x0500_0002, 0x03e0);
    bus.elapse(319);
    write_half(&mut bus, 0x0600_0000, 0x0202);
    bus.elapse(159);
    write_half(&mut bus, 0x0500_0004, 0x7fff);
    bus.elapse(960 - 800);
    for (x, color) in
        [(79, 0x001f), (80, 0x03e0), (159, 0x03e0), (160, 0x7c00), (199, 0x7c00), (200, 0x7fff)]
    {
        assert_eq!(color, bus.framebuffer()[x], "x={x}");
    }
}

#[test]
fn mode_5_renders_its_160_pixel_bitmap_and_uses_backdrop_color_outside_it() {
    let mut bus = Bus::init(&[]);
    write_half(&mut bus, 0x0400_0000, 5 | (1 << 10));
    write_half(&mut bus, 0x0500_0000, 0x1234);
    write_half(&mut bus, 0x0600_0000, 0x001f);
    write_half(&mut bus, 0x0600_013e, 0x03e0);
    bus.elapse(956);
    assert_eq!(0x001f, bus.framebuffer()[0]);
    assert_eq!(0x03e0, bus.framebuffer()[159]);
    assert_eq!(0x1234, bus.framebuffer()[160]);
}

#[test]
fn mode_4_index_zero_lets_a_lower_priority_obj_show_through_but_mode_3_color_zero_is_opaque() {
    for mode in [4, 3] {
        let mut bus = Bus::init(&[]);
        write_half(&mut bus, 0x0400_0000, mode | (1 << 10) | (1 << 12));
        write_half(&mut bus, 0x0500_0000, 0x7c00);
        write_half(&mut bus, 0x0500_0002, 0x03e0);
        write_half(&mut bus, 0x0500_0202, 0x001f);
        write_half(&mut bus, 0x0600_0000, if mode == 4 { 0x0100 } else { 0 });
        write_half(&mut bus, 0x0601_4000, 0x0011);
        write_half(&mut bus, 0x0700_0000, 0);
        write_half(&mut bus, 0x0700_0002, 0);
        write_half(&mut bus, 0x0700_0004, 512 | (1 << 10));
        bus.elapse(280896 + 960 - bus.now());
        if mode == 4 {
            assert_eq!(0x001f, bus.framebuffer()[0]);
            assert_eq!(0x03e0, bus.framebuffer()[1]);
        } else {
            assert_eq!(0, bus.framebuffer()[0]);
        }
    }
}

#[test]
fn transparent_mode_4_pixel_leaves_backdrop_eligible_behind_semi_transparent_obj() {
    let mut bus = Bus::init(&[]);
    for (addr, value) in [
        (0x0400_0000, 4 | (1 << 10) | (1 << 12)),
        (0x0500_0000, 0x7c00),
        (0x0500_0202, 0x001f),
        (0x0601_4000, 0x0001),
        (0x0700_0000, 1 << 10),
        (0x0700_0002, 0),
        (0x0700_0004, 512),
        (0x0400_0050, 1 << 13),
        (0x0400_0052, 8 | (8 << 8)),
    ] {
        write_half(&mut bus, addr, value);
    }
    bus.elapse(960 - bus.now() + 280896);
    assert_eq!(0x3c0f, bus.framebuffer()[0]);
}

#[test]
fn bitmap_bg2_applies_its_affine_transform_before_horizontal_and_vertical_mosaic() {
    let mut bus = Bus::init(&[]);
    for (addr, value) in [
        (0x0400_0000, 3 | (1 << 10)),
        (0x0400_000c, 1 << 6),
        (0x0400_0028, 0x0100),
        (0x0400_002a, 0),
        (0x0400_004c, 3 | (1 << 4)),
        (0x0600_0002, 0x001f),
        (0x0600_01e2, 0x03e0),
    ] {
        write_half(&mut bus, addr, value);
    }
    bus.elapse(280896 + 1232 + 960 - bus.now());
    for x in 0..4 {
        assert_eq!(0x001f, bus.framebuffer()[x]);
        assert_eq!(0x001f, bus.framebuffer()[240 + x]);
    }
}

#[test]
fn mid_frame_affine_reference_write_becomes_the_next_scanline_origin() {
    let mut bus = Bus::init(&[]);
    for (addr, value) in [
        (0x0400_0000, 3 | (1 << 10)),
        (0x0400_0022, 0x0100),
        (0x0400_0026, 0),
        (0x0600_0000, 0x001f),
        (0x0600_0014, 0x03e0),
        (0x0600_0016, 0x7c00),
    ] {
        write_half(&mut bus, addr, value);
    }
    bus.elapse(280896 + 960 - bus.now());
    assert_eq!(0x001f, bus.framebuffer()[0]);
    write_half(&mut bus, 0x0400_0028, 10 << 8);
    write_half(&mut bus, 0x0400_002a, 0);
    bus.elapse(280896 + 1232 + 960 - bus.now());
    assert_eq!(0x03e0, bus.framebuffer()[240]);
}

#[test]
fn disabled_affine_background_does_not_advance_its_hidden_reference() {
    let mut bus = Bus::init(&[]);
    for (addr, value) in [
        (0x0400_0000, 3),
        (0x0400_0022, 0x0100),
        (0x0400_0026, 0),
        (0x0600_0000, 0x001f),
        (0x0600_0002, 0x03e0),
    ] {
        write_half(&mut bus, addr, value);
    }
    bus.elapse(960 - bus.now());
    write_half(&mut bus, 0x0400_0000, 3 | (1 << 10));
    bus.elapse(1232 + 960 - bus.now());
    assert_eq!(0x001f, bus.framebuffer()[240]);
}

#[test]
fn mid_scanline_bgcnt_priority_write_changes_only_later_background_pixels() {
    let mut bus = Bus::init(&[]);
    for (addr, value) in [
        (0x0400_0000, (1 << 8) | (1 << 9)),
        (0x0400_0008, 31 << 8),
        (0x0400_000a, (30 << 8) | (1 << 2) | 1),
        (0x0500_0002, 0x001f),
        (0x0500_0004, 0x03e0),
    ] {
        write_half(&mut bus, addr, value);
    }
    write_word(&mut bus, 0x0600_0020, 0x1111_1111);
    write_word(&mut bus, 0x0600_4020, 0x2222_2222);
    for tile_x in 0..30 {
        let offset = tile_x * 2;
        write_half(&mut bus, 0x0600_f800 + offset, 1);
        write_half(&mut bus, 0x0600_f000 + offset, 1);
    }
    bus.elapse(280896 - bus.now());
    bus.elapse(399);
    write_half(&mut bus, 0x0400_0008, (31 << 8) | 2);
    bus.elapse(960 - 400);
    assert_eq!(0x001f, bus.framebuffer()[50]);
    assert_eq!(0x03e0, bus.framebuffer()[200]);
}

#[test]
fn mid_scanline_forced_blank_assertion_whites_only_later_pixels() {
    let mut bus = Bus::init(&[]);
    write_half(&mut bus, 0x0400_0000, 3 | (1 << 10));
    write_half(&mut bus, 0x0600_0064, 0x001f);
    write_half(&mut bus, 0x0600_0190, 0x001f);
    bus.elapse(280896 - bus.now());
    bus.elapse(399);
    write_half(&mut bus, 0x0400_0000, 3 | (1 << 7) | (1 << 10));
    bus.elapse(960 - 400);
    assert_eq!(0x001f, bus.framebuffer()[50]);
    assert_eq!(0x7fff, bus.framebuffer()[200]);
}

#[test]
fn bldy_brightness_decrease_requires_the_top_layer_to_be_selected() {
    let mut selected = Bus::init(&[]);
    write_half(&mut selected, 0x0400_0000, 3 | (1 << 10));
    write_half(&mut selected, 0x0600_0000, 0x0010);
    write_half(&mut selected, 0x0400_0050, (1 << 2) | (3 << 6));
    write_half(&mut selected, 0x0400_0054, 8);
    selected.elapse(956 + 280896);
    assert_eq!(0x0008, selected.framebuffer()[0]);

    let mut unselected = Bus::init(&[]);
    write_half(&mut unselected, 0x0400_0000, 3 | (1 << 10));
    write_half(&mut unselected, 0x0600_0000, 0x0010);
    write_half(&mut unselected, 0x0400_0050, (1 << 0) | (3 << 6));
    write_half(&mut unselected, 0x0400_0054, 8);
    unselected.elapse(956 + 280896);
    assert_eq!(0x0010, unselected.framebuffer()[0]);
}

#[test]
fn mid_scanline_dispcnt_win0_enable_affects_only_pixels_at_and_after_its_completion() {
    let mut bus = Bus::init(&[]);
    for (addr, value) in [
        (0x0400_0000, 1 << 8),
        (0x0400_0008, 31 << 8),
        (0x0400_0040, 0x00f0),
        (0x0400_0044, 0x00a0),
        (0x0400_0048, 0),
        (0x0400_004a, 1 << 0),
        (0x0500_0000, 0x03e0),
        (0x0500_0002, 0x001f),
    ] {
        write_half(&mut bus, addr, value);
    }
    write_word(&mut bus, 0x0600_0020, 0x1111_1111);
    write_half(&mut bus, 0x0600_f818, 1);
    bus.elapse(280896 - bus.now());
    bus.elapse(399);
    write_half(&mut bus, 0x0400_0000, (1 << 8) | (1 << 13));
    bus.elapse(960 - 400);
    assert_eq!(0x001f, bus.framebuffer()[99]);
    assert_eq!(0x03e0, bus.framebuffer()[100]);
}

#[test]
fn bldalpha_uses_an_obj_behind_the_top_bg_as_the_second_visible_target() {
    let mut bus = Bus::init(&[]);
    for (addr, value) in [
        (0x0400_0000, (1 << 8) | (1 << 9) | (1 << 12)),
        (0x0400_0008, 31 << 8),
        (0x0400_000a, (30 << 8) | (1 << 2) | 2),
        (0x0500_0002, 0x001f),
        (0x0500_0004, 0x7c00),
        (0x0500_0206, 0x03e0),
        (0x0600_0020, 0x0001),
        (0x0600_4020, 0x0002),
        (0x0600_f800, 1),
        (0x0600_f000, 1),
        (0x0601_0020, 0x0003),
        (0x0700_0000, 0),
        (0x0700_0002, 0),
        (0x0700_0004, 1 | (1 << 10)),
        (0x0400_0050, (1 << 0) | (1 << 6) | (1 << 12)),
        (0x0400_0052, 8 | (8 << 8)),
    ] {
        write_half(&mut bus, addr, value);
    }
    bus.elapse(960 - bus.now() + 280896);
    assert_eq!(0x01ef, bus.framebuffer()[0]);
}

#[test]
fn wrapped_win0_bounds_take_precedence_over_overlapping_win1() {
    let mut bus = Bus::init(&[]);
    for (addr, value) in [
        (0x0400_0000, 3 | (1 << 10) | (1 << 13) | (1 << 14)),
        (0x0400_0040, 0xef01),
        (0x0400_0042, 0x00f0),
        (0x0400_0044, 0x9f01),
        (0x0400_0046, 0x0001),
        (0x0400_0048, 1 << (8 + 2)),
        (0x0400_004a, 0),
        (0x0500_0000, 0x7c00),
        (0x0600_0000, 0x001f),
        (0x0600_0002, 0x001f),
        (0x0600_01de, 0x001f),
    ] {
        write_half(&mut bus, addr, value);
    }
    bus.elapse(960 + 280896);
    assert_eq!(0x7c00, bus.framebuffer()[0]);
    assert_eq!(0x001f, bus.framebuffer()[1]);
    assert_eq!(0x7c00, bus.framebuffer()[239]);
}

#[test]
fn window_color_effect_bit_gates_brightness_independently_of_its_layer_mask() {
    let mut bus = Bus::init(&[]);
    for (addr, value) in [
        (0x0400_0000, 3 | (1 << 10) | (1 << 13)),
        (0x0400_0040, 0x0001),
        (0x0400_0044, 0x0001),
        (0x0400_0048, 1 << 2),
        (0x0400_004a, (1 << 2) | (1 << 5)),
        (0x0400_0050, (1 << 2) | (2 << 6)),
        (0x0400_0054, 8),
        (0x0600_0000, 0x0010),
        (0x0600_0002, 0x0010),
    ] {
        write_half(&mut bus, addr, value);
    }
    bus.elapse(960 + 280896);
    assert_eq!(0x0010, bus.framebuffer()[0]);
    assert_eq!(0x3df7, bus.framebuffer()[1]);
}

#[test]
fn clipped_equal_and_wrapped_window_bounds_keep_bg2_visible_on_every_pixel() {
    for (h0, h1, winin, outside) in [
        (0x00f0, 0x00f0, 0x003f, 0),
        (0xf0f0, 0x00ff, 0x3f00, 0),
        (0xfaf5, 0, 0x0004, 0),
        (0xf0ff, 0xf5f5, 0, 0x3f),
        (0xef01, 0x00f0, 0x2404, 0),
    ] {
        let mut bus = Bus::init(&[]);
        bus.dispcnt = 3 | (1 << 10) | (1 << 13) | (1 << 14);
        bus.win0h = h0;
        bus.win1h = h1;
        bus.win0v = 160;
        bus.win1v = 160;
        bus.winin = winin;
        bus.winout = outside;
        bus.bg_affine_parameters[0] = [0; 4];
        bus.vram[0..2].copy_from_slice(&0x001fu16.to_le_bytes());
        bus.elapse(960);
        assert!(bus.framebuffer()[0..240].iter().all(|&pixel| pixel == 0x001f));
    }
}

#[test]
fn obj_window_nonzero_texels_select_the_high_winout_mask_only_with_obj_enabled() {
    let mut bus = Bus::init(&[]);
    for (addr, value) in [
        (0x0400_0000, (1 << 8) | (1 << 12) | (1 << 15)),
        (0x0400_0008, 31 << 8),
        (0x0400_004a, 1 << 0),
        (0x0500_0000, 0x7c00),
        (0x0500_0002, 0x001f),
        (0x0600_0020, 0x0011),
        (0x0600_f800, 1),
        (0x0601_0020, 0x0001),
        (0x0700_0000, 2 << 10),
        (0x0700_0002, 0),
        (0x0700_0004, 1),
    ] {
        write_half(&mut bus, addr, value);
    }
    bus.elapse(960 + 280896);
    assert_eq!(0x7c00, bus.framebuffer()[0]);
    assert_eq!(0x001f, bus.framebuffer()[1]);
    write_half(&mut bus, 0x0400_0000, (1 << 8) | (1 << 15));
    bus.elapse(2 * 280896 + 960 - bus.now());
    assert_eq!(0x001f, bus.framebuffer()[0]);
}

#[test]
fn obj_window_mosaic_quantizes_source_rows_but_not_horizontal_mask_samples() {
    let mut bus = Bus::init(&[]);
    for (addr, value) in [
        (0x0400_0000, (1 << 8) | (1 << 12) | (1 << 15)),
        (0x0400_0008, 31 << 8),
        (0x0400_004a, 1 << 0),
        (0x0400_004c, (3 << 8) | (1 << 12)),
        (0x0500_0000, 0x7c00),
        (0x0500_0002, 0x001f),
        (0x0600_0020, 0x0011),
        (0x0600_0024, 0x0011),
        (0x0600_f800, 1),
        (0x0601_0020, 0x0001),
        (0x0601_0024, 0x0010),
        (0x0700_0000, (2 << 10) | (1 << 12)),
        (0x0700_0002, 0),
        (0x0700_0004, 1),
    ] {
        write_half(&mut bus, addr, value);
    }
    bus.elapse(1232 + 960 - bus.now());
    assert_eq!(0x7c00, bus.framebuffer()[240]);
    assert_eq!(0x001f, bus.framebuffer()[241]);
}

#[test]
fn text_tiles_past_the_bg_side_of_vram_are_transparent() {
    for (color_256, first_tile, tiles) in [(false, 512usize, 512usize), (true, 256, 768)] {
        for mosaic in [false, true] {
            let mut bus = Bus::init(&[]);
            bus.dispcnt = 1 << 8;
            bus.bg_controls[0] = (31 << 8)
                | (3 << 2)
                | if color_256 { 1 << 7 } else { 0 }
                | if mosaic { 1 << 6 } else { 0 };
            bus.mosaic = 1;
            bus.palette[..2].copy_from_slice(&0x1234u16.to_le_bytes());
            bus.palette[2..].fill(0xff);
            bus.vram.fill(0x11);
            for entry in 0..1024 {
                let tile = ((first_tile + entry % tiles) | ((entry & 3) << 10)) as u16;
                bus.vram[0xf800 + entry * 2..][..2].copy_from_slice(&tile.to_le_bytes());
            }
            bus.elapse(280896);
            assert!(bus.framebuffer().iter().all(|&pixel| pixel == 0x1234));
        }
    }
}

#[test]
fn text_map_entries_past_the_bg_side_of_vram_read_as_zero() {
    for mosaic in [false, true] {
        let mut bus = Bus::init(&[]);
        bus.dispcnt = 1 << 8;
        bus.bg_controls[0] = (1 << 14) | (31 << 8) | if mosaic { 1 << 6 } else { 0 };
        bus.mosaic = 1;
        bus.bg_horizontal_offsets[0] = 256;
        bus.palette[2..4].copy_from_slice(&0x001fu16.to_le_bytes());
        bus.palette[4..6].copy_from_slice(&0x03e0u16.to_le_bytes());
        bus.vram[0..32].fill(0x11);
        bus.vram[32..64].fill(0x22);
        bus.vram[0x10000..0x10800].fill(0x01);
        bus.elapse(280896);
        assert!(bus.framebuffer().iter().all(|&pixel| pixel == 0x001f));
    }
}

#[test]
fn empty_text_rows_preserve_the_backdrop_and_hidden_rows_do_not_fetch_tile_data() {
    for color_256 in [false, true] {
        let mut bus = Bus::init(&[]);
        bus.dispcnt = 1 << 8;
        bus.bg_controls[0] = (31 << 8) | if color_256 { 1 << 7 } else { 0 };
        bus.palette[0..2].copy_from_slice(&0x1234u16.to_le_bytes());
        bus.palette[3 * 16 * 2..][..2].copy_from_slice(&0x4321u16.to_le_bytes());
        for tile in 0..32 {
            bus.vram[0xf800 + tile * 2..][..2].copy_from_slice(&(3u16 << 12).to_le_bytes());
        }
        bus.elapse(960);
        assert!(bus.framebuffer()[0..240].iter().all(|&pixel| pixel == 0x1234));
        bus.dispcnt |= 1 << 13;
        bus.bg_controls[0] |= (3 << 2) | (1 << 7);
        for tile in 0..32 {
            bus.vram[0xf800 + tile * 2..][..2].copy_from_slice(&1023u16.to_le_bytes());
        }
        bus.scanline_memo.invalidate();
        bus.elapse(280896);
        assert!(bus.framebuffer()[0..240].iter().all(|&pixel| pixel == 0x1234));
    }
}

#[test]
fn one_by_one_text_mosaic_preserves_distinct_pixels_and_rows() {
    for color_256 in [false, true] {
        let mut bus = Bus::init(&[]);
        write_half(&mut bus, 0x0400_0000, 1 << 8);
        write_half(
            &mut bus,
            0x0400_0008,
            (31 << 8) | (1 << 6) | if color_256 { 1 << 7 } else { 0 },
        );
        write_half(&mut bus, 0x0400_004c, 0);
        write_half(&mut bus, 0x0500_0002, 0x001f);
        write_half(&mut bus, 0x0500_0004, 0x03e0);
        let tile = if color_256 { 0x0600_0040 } else { 0x0600_0020 };
        write_half(&mut bus, tile, if color_256 { 0x0201 } else { 0x0021 });
        write_half(
            &mut bus,
            tile + if color_256 { 8 } else { 4 },
            if color_256 { 0x0102 } else { 0x0012 },
        );
        write_half(&mut bus, 0x0600_f800, 1);
        bus.elapse(280896 + 1232 + 960 - bus.now());
        assert_eq!(&[0x001f, 0x03e0, 0], &bus.framebuffer()[0..3]);
        assert_eq!(&[0x03e0, 0x001f, 0], &bus.framebuffer()[240..243]);
    }
}

#[test]
fn affine_background_mosaic_quantizes_screen_coordinates_before_matrix_sampling() {
    let mut bus = Bus::init(&[]);
    for (addr, value) in [
        (0x0400_0000, 2 | (1 << 10)),
        (0x0400_000c, (31 << 8) | (1 << 6)),
        (0x0400_0020, 0x0100),
        (0x0400_0026, 0x0100),
        (0x0400_004c, 2 | (1 << 4)),
        (0x0500_0002, 0x001f),
        (0x0500_0004, 0x03e0),
        (0x0600_0040, 0x0201),
        (0x0600_0048, 0x0202),
    ] {
        write_half(&mut bus, addr, value);
    }
    write_byte(&mut bus, 0x0600_f800, 1);
    bus.elapse(280896 + 1232 + 960 - bus.now());
    for x in 0..3 {
        assert_eq!(0x001f, bus.framebuffer()[x]);
        assert_eq!(0x001f, bus.framebuffer()[240 + x]);
    }
}

#[test]
fn affine_overflow_wrap_maps_a_negative_reference_point_to_the_far_edge() {
    let mut bus = Bus::init(&[]);
    for (addr, value) in [
        (0x0400_0000, 2 | (1 << 10)),
        (0x0400_000c, (31 << 8) | (1 << 13)),
        (0x0400_0020, 0x0100),
        (0x0400_0026, 0x0100),
        (0x0400_0028, 0xff00),
        (0x0400_002a, 0x0fff),
        (0x0500_0012, 0x6318),
    ] {
        write_half(&mut bus, addr, value);
    }
    write_byte(&mut bus, 0x0600_00c7, 9);
    write_byte(&mut bus, 0x0600_f80f, 3);
    bus.elapse(951 + 280896);
    assert_eq!(0x6318, bus.framebuffer()[0]);
}

#[test]
fn regular_obj_size_horizontal_flip_wrapped_x_and_1d_mapping_select_the_source_tile() {
    let mut bus = Bus::init(&[]);
    for (addr, value) in [
        (0x0400_0000, (1 << 12) | (1 << 6)),
        (0x0500_0204, 0x03e0),
        (0x0601_0020, 0x2000),
        (0x0700_0000, 1 << 14),
        (0x0700_0002, 508 | (1 << 12)),
        (0x0700_0004, 0),
    ] {
        write_half(&mut bus, addr, value);
    }
    bus.elapse(954);
    assert_eq!(0x03e0, bus.framebuffer()[0]);
}

#[test]
fn bitmap_modes_suppress_each_obj_tile_below_character_name_512() {
    let mut bus = Bus::init(&[]);
    for (addr, value) in [
        (0x0400_0000, 3 | (1 << 6) | (1 << 12)),
        (0x0500_0000, 0x7c00),
        (0x0500_0202, 0x001f),
        (0x0500_0204, 0x03e0),
        (0x0601_3fe0, 0x0002),
        (0x0601_4000, 0x0001),
        (0x0700_0000, 1 << 14),
        (0x0700_0002, 0),
        (0x0700_0004, 511),
    ] {
        write_half(&mut bus, addr, value);
    }
    bus.elapse(960 - bus.now());
    assert_eq!(0x7c00, bus.framebuffer()[0]);
    assert_eq!(0x001f, bus.framebuffer()[8]);
}

#[test]
fn obj_character_names_wrap_at_1024_in_1d_mapping_and_within_the_row_in_2d_mapping() {
    for (mapping_1d, name, first_tile) in [(true, 1023, 0x0601_7fe0), (false, 31, 0x0601_03e0)] {
        let mut bus = Bus::init(&[]);
        for (addr, value) in [
            (0x0400_0000, (1 << 12) | if mapping_1d { 1 << 6 } else { 0 }),
            (0x0500_0202, 0x001f),
            (0x0500_0204, 0x03e0),
            (first_tile, 0x0002),
            (0x0601_0000, 0x0001),
            (0x0700_0000, 1 << 14),
            (0x0700_0002, 0),
            (0x0700_0004, name),
        ] {
            write_half(&mut bus, addr, value);
        }
        bus.elapse(960 - bus.now());
        assert_eq!(0x03e0, bus.framebuffer()[0]);
        assert_eq!(0x001f, bus.framebuffer()[8]);
    }
}

#[test]
fn lower_oam_index_wins_overlapping_obj_regardless_of_bg_relative_priority() {
    let mut bus = Bus::init(&[]);
    for (addr, value) in [
        (0x0400_0000, 1 << 12),
        (0x0500_0202, 0x001f),
        (0x0500_0204, 0x03e0),
        (0x0601_0020, 0x0001),
        (0x0601_0040, 0x0002),
        (0x0700_0000, 0),
        (0x0700_0002, 0),
        (0x0700_0004, 1 | (3 << 10)),
        (0x0700_0008, 0),
        (0x0700_000a, 0),
        (0x0700_000c, 2),
    ] {
        write_half(&mut bus, addr, value);
    }
    bus.elapse(960 - bus.now());
    assert_eq!(0x001f, bus.framebuffer()[0]);
}

#[test]
fn reused_text_rows_retain_midline_vram_history_and_update_the_following_frame() {
    let mut bus = Bus::init(&[]);
    bus.dispcnt = 1 << 8;
    bus.bg_controls[0] = 31 << 8;
    bus.palette[2..4].copy_from_slice(&0x001fu16.to_le_bytes());
    bus.palette[4..6].copy_from_slice(&0x03e0u16.to_le_bytes());
    bus.vram[0x20..0x24].copy_from_slice(&0x1111_1111u32.to_le_bytes());
    for x in 0..30 {
        bus.vram[0xf800 + x * 2..][..2].copy_from_slice(&1u16.to_le_bytes());
    }
    bus.elapse(280896 + 399);
    write_half(&mut bus, 0x0600_0020, 0x2222);
    bus.elapse(560);
    assert_eq!(0x001f, bus.framebuffer()[96]);
    assert_eq!(0x03e0, bus.framebuffer()[104]);
    assert_eq!(0x001f, bus.framebuffer()[108]);
    bus.elapse(280896);
    assert_eq!(0x03e0, bus.framebuffer()[96]);
    assert_eq!(0x001f, bus.framebuffer()[100]);
}

#[test]
fn lower_overlapping_obj_is_excluded_from_the_color_effects_second_target() {
    let mut bus = Bus::init(&[]);
    for (addr, value) in [
        (0x0400_0000, (1 << 8) | (1 << 12)),
        (0x0400_0008, 31 << 8),
        (0x0500_0002, 0x001f),
        (0x0500_0202, 0x7c00),
        (0x0500_0204, 0x03e0),
        (0x0600_0020, 0x0001),
        (0x0600_f800, 1),
        (0x0601_0020, 0x0001),
        (0x0601_0040, 0x0002),
        (0x0700_0000, 1 << 10),
        (0x0700_0002, 0),
        (0x0700_0004, 1),
        (0x0700_0008, 0),
        (0x0700_000a, 0),
        (0x0700_000c, 2),
        (0x0400_0050, 1 << 8),
        (0x0400_0052, 8 | (8 << 8)),
    ] {
        write_half(&mut bus, addr, value);
    }
    bus.elapse(960 + 280896);
    assert_eq!(0x3c0f, bus.framebuffer()[0]);
}

#[test]
fn affine_obj_mosaic_holds_transformed_source_coordinates_for_each_screen_group() {
    let mut bus = Bus::init(&[]);
    for (addr, value) in [
        (0x0400_0000, (1 << 12) | (1 << 6)),
        (0x0400_004c, (3 << 8) | (1 << 12)),
        (0x0500_0202, 0x001f),
        (0x0500_0204, 0x03e0),
        (0x0601_0000, 0x0201),
        (0x0601_0008, 0x0202),
        (0x0700_0000, (1 << 8) | (1 << 12) | (1 << 13)),
        (0x0700_0002, 0),
        (0x0700_0004, 0),
        (0x0700_0006, 0x0100),
        (0x0700_000e, 0),
        (0x0700_0016, 0),
        (0x0700_001e, 0x0100),
    ] {
        write_half(&mut bus, addr, value);
    }
    bus.elapse(1232 + 960 - bus.now());
    for x in 0..4 {
        assert_eq!(0x001f, bus.framebuffer()[x]);
        assert_eq!(0x001f, bus.framebuffer()[240 + x]);
    }
}

#[test]
fn hblank_free_obj_budget_partially_draws_the_final_evaluated_object() {
    for hblank_free in [false, true] {
        let mut bus = Bus::init(&[]);
        write_half(&mut bus, 0x0400_0000, (1 << 12) | if hblank_free { 1 << 5 } else { 0 });
        write_half(&mut bus, 0x0500_0202, 0x001f);
        write_half(&mut bus, 0x0500_0204, 0x03e0);
        write_word(&mut bus, 0x0601_0020, 0x1111_1111);
        write_word(&mut bus, 0x0601_0040, 0x2222_2222);
        for index in 0..119 {
            write_half(&mut bus, 0x0700_0002 + index * 8, 240);
        }
        write_half(&mut bus, 0x0700_0004 + 119 * 8, 1);
        write_half(&mut bus, 0x0700_0002 + 120 * 8, 8);
        write_half(&mut bus, 0x0700_0004 + 120 * 8, 2);
        bus.elapse(960 - bus.now());
        if hblank_free {
            assert_eq!(0x001f, bus.framebuffer()[0]);
            assert_eq!(0x001f, bus.framebuffer()[1]);
            assert_eq!(0, bus.framebuffer()[2]);
            assert_eq!(0, bus.framebuffer()[8]);
        } else {
            assert_eq!(0x001f, bus.framebuffer()[7]);
            assert_eq!(0x03e0, bus.framebuffer()[8]);
        }
    }
}

#[test]
fn obj_admission_is_recalculated_when_a_scanline_returns_next_frame() {
    let mut bus = Bus::init(&[]);
    for i in 0..128 {
        bus.oam[i * 8..][..2].copy_from_slice(&(1u16 << 9).to_le_bytes());
    }
    for i in 0..16 {
        bus.oam[i * 8..][..2].copy_from_slice(&0u16.to_le_bytes());
        bus.oam[i * 8 + 2..][..2]
            .copy_from_slice(&((3u16 << 14) | if i == 15 { 0 } else { 240 }).to_le_bytes());
    }
    bus.palette[514..516].copy_from_slice(&0x001fu16.to_le_bytes());
    bus.vram[0x10000..0x10020].fill(0x11);
    write_half(&mut bus, 0x0400_0000, 1 << 12);
    bus.elapse(960 - bus.now());
    assert_eq!(0x001f, bus.framebuffer()[0]);
    write_half(&mut bus, 0x0400_0000, 0);
    bus.elapse(280896 - bus.now());
    write_half(&mut bus, 0x0400_0000, (1 << 12) | (1 << 5));
    bus.elapse(280896 + 960 - bus.now());
    assert_eq!(0, bus.framebuffer()[0]);
}

#[test]
fn reused_text_rows_observe_changed_bytes_palette_banks_and_horizontal_flips() {
    let mut bus = Bus::init(&[]);
    bus.dispcnt = 1 << 8;
    bus.bg_controls[0] = 31 << 8;
    for index in 0..48 {
        bus.palette[index * 2..][..2].copy_from_slice(&(index as u16).to_le_bytes());
    }
    bus.vram[0x20..0x24].copy_from_slice(&0x8765_4321u32.to_le_bytes());
    bus.vram[0xf800..0xf802].copy_from_slice(&1u16.to_le_bytes());
    bus.vram[0xf802..0xf804].copy_from_slice(&(1u16 | (1 << 10) | (2 << 12)).to_le_bytes());
    bus.elapse(960);
    assert_eq!(&[1, 2, 3, 4, 5, 6, 7, 8], &bus.framebuffer()[0..8]);
    assert_eq!(&[40, 39, 38, 37, 36, 35, 34, 33], &bus.framebuffer()[8..16]);
    bus.vram[0x20..0x24].copy_from_slice(&0x1234_5678u32.to_le_bytes());
    bus.scanline_memo.invalidate();
    bus.elapse(280896);
    assert_eq!(&[8, 7, 6, 5, 4, 3, 2, 1], &bus.framebuffer()[0..8]);
    assert_eq!(&[33, 34, 35, 36, 37, 38, 39, 40], &bus.framebuffer()[8..16]);
    write_half(&mut bus, 0x0500_0010, 0x4567);
    bus.elapse(280896);
    assert_eq!(0x4567, bus.framebuffer()[0]);
    assert_eq!(40, bus.framebuffer()[15]);
    bus.palette[16..18].copy_from_slice(&0x9234u16.to_le_bytes());
    bus.scanline_memo.invalidate();
    bus.elapse(280896);
    assert_eq!(0x1234, bus.framebuffer()[0]);
    assert_eq!(40, bus.framebuffer()[15]);
    write_word(&mut bus, 0x0200_0000, 0x5555_6666);
    write_word(&mut bus, 0x0400_00b0, 0x0200_0000);
    write_word(&mut bus, 0x0400_00b4, 0x0600_0020);
    write_word(&mut bus, 0x0400_00b8, 0x8400_0001);
    bus.elapse(280896);
    assert_eq!(&[6, 6, 6, 6, 5, 5, 5, 5], &bus.framebuffer()[0..8]);
    bus.register_ram_reset(1 << 3);
    bus.dispcnt = 1 << 8;
    bus.elapse(280896);
    assert_eq!(&[0; 8], &bus.framebuffer()[0..8]);
}

#[test]
fn scanlines_resume_alpha_obj_and_raster_effects_after_opaque_composition() {
    let mut bus = Bus::init(&[]);
    bus.dispcnt = (1 << 8) | (1 << 12);
    bus.bg_controls[0] = 31 << 8;
    bus.palette[0..4].copy_from_slice(&[0, 0x7c, 0x1f, 0]);
    bus.palette[0x202..0x204].copy_from_slice(&[0xe0, 0x03]);
    bus.vram[0x20..0x40].fill(0x11);
    bus.vram[0x10020..0x10040].fill(0x11);
    for tile in 0..32 {
        bus.vram[0xf800 + tile * 2] = 1;
    }
    bus.oam[0..6].copy_from_slice(&[0, 2, 0, 0, 1, 0]);
    bus.bldcnt = 1 << 13;
    bus.bldalpha = 8 | (8 << 8);
    bus.elapse(960);
    assert_eq!(0x001f, bus.framebuffer()[0]);
    write_half(&mut bus, 0x0400_0050, 1 | (1 << 6) | (1 << 13));
    bus.elapse(1232 + 960 - bus.now());
    assert_eq!(0x3c0f, bus.framebuffer()[240]);
    write_half(&mut bus, 0x0400_0050, 1 << 8);
    write_half(&mut bus, 0x0700_0000, 2 | (1 << 10));
    bus.elapse(2 * 1232 + 960 - bus.now());
    assert_eq!(0x01ef, bus.framebuffer()[2 * 240]);
    write_half(&mut bus, 0x0700_0000, 2 | (1 << 9));
    bus.elapse(3 * 1232 + 960 - bus.now());
    assert_eq!(0x001f, bus.framebuffer()[3 * 240]);
    write_half(&mut bus, 0x0400_0050, 1 | (2 << 6));
    write_half(&mut bus, 0x0400_0054, 8);
    bus.elapse(4 * 1232 + 960 - bus.now());
    assert_eq!(0x3dff, bus.framebuffer()[4 * 240]);
    write_half(&mut bus, 0x0400_0050, 1 | (1 << 6) | (1 << 13));
    bus.elapse(5 * 1232 + 480 - bus.now());
    write_half(&mut bus, 0x0400_0050, 0);
    bus.elapse(5 * 1232 + 960 - bus.now());
    assert_eq!(0x3c0f, bus.framebuffer()[5 * 240]);
    assert_eq!(0x001f, bus.framebuffer()[5 * 240 + 239]);
}

#[test]
fn selective_video_latches_whole_frames_and_publishes_only_complete_images() {
    const FRAME: u64 = 280896;
    const VISIBLE_END: u64 = 159 * 1232 + 960;
    let mut bus = Bus::init(&[]);
    bus.dispcnt = 0;
    bus.palette[..2].copy_from_slice(&0x001fu16.to_le_bytes());
    bus.elapse(400);
    bus.set_video_interval(3);
    assert!(bus.is_framebuffer_canonical());
    assert_eq!(0, bus.video_frame_id);
    assert_eq!(VISIBLE_END, bus.next_rendered_frame_cycle());
    bus.elapse(VISIBLE_END - bus.now());
    assert_eq!(1, bus.video_frame_id);
    assert_eq!(VISIBLE_END, bus.video_frame_cycle);
    assert!(bus.published_framebuffer().iter().all(|&p| p == 0x001f));
    bus.elapse(FRAME - bus.now());
    bus.palette[..2].copy_from_slice(&0x03e0u16.to_le_bytes());
    bus.scanline_memo.invalidate();
    bus.elapse(960);
    assert_eq!(0x03e0, bus.framebuffer()[0]);
    assert_eq!(0x001f, bus.framebuffer()[240]);
    assert!(bus.published_framebuffer().iter().all(|&p| p == 0x001f));
    assert_eq!(1, bus.video_frame_id);
    bus.elapse(FRAME + VISIBLE_END - bus.now());
    assert_eq!(2, bus.video_frame_id);
    assert_eq!(4 * FRAME + VISIBLE_END, bus.next_rendered_frame_cycle());
    assert!(bus.published_framebuffer().iter().all(|&p| p == 0x03e0));
    bus.elapse(2 * FRAME - bus.now());
    bus.palette[..2].copy_from_slice(&0x7c00u16.to_le_bytes());
    bus.scanline_memo.invalidate();
    bus.elapse(960);
    assert!(!bus.is_framebuffer_canonical());
    assert_eq!(0x03e0, bus.framebuffer()[0]);
    bus.set_video_interval(16);
    let next_selected = 3 * FRAME + VISIBLE_END;
    assert_eq!(next_selected, bus.next_rendered_frame_cycle());
    bus.set_video_interval(16);
    assert_eq!(next_selected, bus.next_rendered_frame_cycle());
    bus.elapse(3 * FRAME + 960 - bus.now());
    assert!(!bus.is_framebuffer_canonical());
    assert_eq!(0x7c00, bus.framebuffer()[0]);
    assert!(bus.published_framebuffer().iter().all(|&p| p == 0x03e0));
    bus.set_video_interval(1);
    bus.elapse(next_selected - bus.now());
    assert!(bus.is_framebuffer_canonical());
    assert_eq!(4, bus.video_frame_id);
    assert_eq!(4 * FRAME + VISIBLE_END, bus.next_rendered_frame_cycle());
    assert!(bus.published_framebuffer().iter().all(|&p| p == 0x7c00));
    bus.elapse(4 * FRAME + VISIBLE_END - bus.now());
    assert_eq!(bus.framebuffer().as_ptr(), bus.published_framebuffer().as_ptr());
    assert_eq!(5 * FRAME + VISIBLE_END, bus.next_rendered_frame_cycle());
}

#[test]
fn canonical_video_repair_preserves_publication_and_full_rendering_resumes_immediately() {
    const FRAME: u64 = 280896;
    const VISIBLE_END: u64 = 159 * 1232 + 960;
    let mut bus = Bus::init(&[]);
    bus.set_video_interval(2);
    bus.elapse(2 * FRAME + 960);
    assert!(!bus.is_framebuffer_canonical());
    let published_id = bus.video_frame_id;
    let published_cycle = bus.video_frame_cycle;
    bus.accept_canonical_framebuffer(&[0x1234; 240 * 160]);
    assert!(bus.is_framebuffer_canonical());
    assert_eq!(published_id, bus.video_frame_id);
    assert_eq!(published_cycle, bus.video_frame_cycle);
    assert_eq!(0x7fff, bus.published_framebuffer()[0]);
    bus.use_full_video();
    assert_eq!(bus.framebuffer().as_ptr(), bus.published_framebuffer().as_ptr());
    bus.elapse(1232);
    assert_eq!(0x1234, bus.framebuffer()[0]);
    assert_eq!(0x7fff, bus.framebuffer()[240]);
    assert_eq!(2 * FRAME + VISIBLE_END, bus.next_rendered_frame_cycle());
    bus.set_video_interval(16);
    let reset_cycle = bus.now();
    bus.hard_reset();
    assert!(bus.is_framebuffer_canonical());
    assert_eq!(0, bus.video_frame_id);
    assert_eq!(reset_cycle + VISIBLE_END, bus.next_rendered_frame_cycle());
    assert!(bus.published_framebuffer().iter().all(|&p| p == 0));
    bus.elapse(VISIBLE_END);
    assert_eq!(reset_cycle + 16 * FRAME + VISIBLE_END, bus.next_rendered_frame_cycle());
    assert!(bus.published_framebuffer().iter().all(|&p| p == 0x7fff));
}
