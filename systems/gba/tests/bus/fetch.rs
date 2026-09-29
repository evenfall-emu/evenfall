//! Optimized Bus reads and fetches compared with the general Bus paths.
use super::*;
use evenfall_gba::bus::InstructionFetchRegion;

fn assert_same_bus(reference: &Bus, optimized: &Bus) {
    // The canonical codec covers every durable field including RAM, video,
    // device state and timing; compare the ephemeral effects of a fetch too.
    let mut expected = blake3::Hasher::new();
    let mut actual = blake3::Hasher::new();
    reference.write_state(&mut evenfall_state::Sink::new(None, Some(&mut expected))).unwrap();
    optimized.write_state(&mut evenfall_state::Sink::new(None, Some(&mut actual))).unwrap();
    assert_eq!(expected.finalize(), actual.finalize());
    assert_eq!(reference.now(), optimized.now());
    assert_eq!(reference.open_bus_value, optimized.open_bus_value);
    assert_eq!(reference.fetch_kind, optimized.fetch_kind);
    assert_eq!(reference.irq.irq_line, optimized.irq.irq_line);
    assert_eq!(reference.game_pak_timing_key, optimized.game_pak_timing_key);
    assert_eq!(reference.game_pak_n_cycles, optimized.game_pak_n_cycles);
    assert_eq!(reference.game_pak_s_cycles, optimized.game_pak_s_cycles);
    assert_eq!(reference.duty_valid_until, optimized.duty_valid_until);
    assert_eq!(reference.batch_active, optimized.batch_active);
    assert_eq!(reference.batch_deadline, optimized.batch_deadline);
    assert_eq!(reference.batch_device_cycle, optimized.batch_device_cycle);
    assert_eq!(reference.devices_deferred, optimized.devices_deferred);
    assert_eq!(reference.dma_epoch, optimized.dma_epoch);
    assert_eq!(reference.external_input_pointer, optimized.external_input_pointer);
    assert_eq!(reference.external_input_count, optimized.external_input_count);
    assert_eq!(reference.external_input_index, optimized.external_input_index);
    assert_eq!(reference.servicing_dma, optimized.servicing_dma);
    assert_eq!(reference.active_dma_channel, optimized.active_dma_channel);
    assert_eq!(reference.apu_cycle, optimized.apu_cycle);
    assert_eq!(reference.timer_cycle, optimized.timer_cycle);
    assert_eq!(reference.timer_deadline, optimized.timer_deadline);
    assert_eq!(reference.apu, optimized.apu);
    assert_eq!(&reference.frame_priorities[..], &optimized.frame_priorities[..]);
    assert_eq!(&reference.frame_layers[..], &optimized.frame_layers[..]);
    assert_eq!(&reference.frame_second_pixels[..], &optimized.frame_second_pixels[..]);
    assert_eq!(&reference.frame_second_priorities[..], &optimized.frame_second_priorities[..]);
    assert_eq!(&reference.frame_second_layers[..], &optimized.frame_second_layers[..]);
    assert_eq!(&reference.frame_semitransparent[..], &optimized.frame_semitransparent[..]);
    assert_eq!(&reference.frame_obj_window[..], &optimized.frame_obj_window[..]);
    assert_eq!(&reference.video_published_pixels[..], &optimized.video_published_pixels[..]);
    assert_eq!(reference.line_needs_second_surface, optimized.line_needs_second_surface);
    assert_eq!(reference.line_has_color_effects, optimized.line_has_color_effects);
    assert_eq!(reference.line_window_masks, optimized.line_window_masks);
    assert_eq!(reference.line_unmasked_layers, optimized.line_unmasked_layers);
    assert_eq!(reference.line_window_key, optimized.line_window_key);
    assert_eq!(
        std::mem::discriminant(&reference.line_semitransparent_obj),
        std::mem::discriminant(&optimized.line_semitransparent_obj)
    );
    assert_eq!(reference.video_active_interval, optimized.video_active_interval);
    assert_eq!(reference.video_requested_interval, optimized.video_requested_interval);
    assert_eq!(reference.video_skip_remaining, optimized.video_skip_remaining);
    assert_eq!(reference.video_render_current, optimized.video_render_current);
    assert_eq!(reference.video_framebuffer_canonical, optimized.video_framebuffer_canonical);
    assert_eq!(reference.video_publication_active, optimized.video_publication_active);
    assert_eq!(reference.video_frame_id, optimized.video_frame_id);
    assert_eq!(reference.video_frame_cycle, optimized.video_frame_cycle);
}

fn assert_code_fetch_parity(
    reference: &mut Bus,
    scalar: &mut Bus,
    address: u32,
    width: AccessWidth,
    kind: AccessKind,
) {
    let expected = reference.read(access(address, width, kind, AccessSource::CpuCode));
    let start = scalar.now();
    scalar.fetch_kind = kind;
    assert_eq!(expected.value, scalar.fetch_instruction(address, width));
    assert_eq!(expected.cycles, scalar.now() - start);
    assert_same_bus(reference, scalar);
}

fn assert_view_fetch_parity(
    reference: &mut Bus,
    active: &mut Bus,
    region: &mut InstructionFetchRegion,
    address: u32,
    width: AccessWidth,
    kind: AccessKind,
) {
    let expected = reference.read(access(address, width, kind, AccessSource::CpuCode));
    let start = active.now();
    active.fetch_kind = kind;
    assert_eq!(expected.value, active.fetch_instruction_in_region(region, address, width));
    assert_eq!(expected.cycles, active.now() - start);
    assert_same_bus(reference, active);
}

/// ZiGBA: "scalar instruction fetches preserve RTC overlaps short ROM tails and exceptional regions"
#[test]
fn scalar_instruction_fetches_preserve_rtc_overlaps_short_rom_tails_and_exceptional_regions() {
    let mut rom = [0xa5; 0xcd];
    rom[..4].copy_from_slice(&[1, 2, 3, 4]);
    for batched in [false, true] {
        for save_type in [SaveType::Sram, SaveType::Eeprom512] {
            let mut reference = Bus::init_with_save(&rom, save_type);
            let mut scalar = Bus::init_with_save(&rom, save_type);
            for bus in [&mut reference, &mut scalar] {
                bus.enable_rtc(0);
                write_half(bus, 0x0800_00c8, 1);
                write_half(bus, 0x0800_00c6, 15);
                write_half(bus, 0x0800_00c4, 15);
                if batched {
                    bus.begin_device_batch();
                }
            }
            for address in [
                0x0800_0000,
                0x0800_00c1,
                0x0800_00c3,
                0x0800_00c4,
                0x0a00_00c6,
                0x0a00_00c8,
                0x0a00_00c9,
                0x0a00_00ca,
                0x0c00_00cc,
                0x0c00_00cd,
                0x09ff_fffe,
                0x0d00_0000,
                0x0dff_ff00,
                0,
                0x0200_0000,
                0x0300_7ffe,
                0x0400_0204,
                0x0600_0000,
                0x0e00_0000,
                0x1000_0000,
            ] {
                for width in [AccessWidth::Half, AccessWidth::Word] {
                    for kind in [AccessKind::Nonsequential, AccessKind::Sequential] {
                        assert_code_fetch_parity(&mut reference, &mut scalar, address, width, kind);
                    }
                }
            }
            if batched {
                reference.end_device_batch();
                scalar.end_device_batch();
            }
            assert_same_bus(&reference, &scalar);
        }
    }
}

/// ZiGBA: "scalar instruction fetches retain full size RTC ROM and EEPROM address discrimination"
#[test]
fn scalar_instruction_fetches_retain_full_size_rtc_rom_and_eeprom_address_discrimination() {
    let rom = vec![0xa5; 32 * 1024 * 1024];
    let mut reference = Bus::init_with_save(&rom, SaveType::Eeprom512);
    let mut scalar = Bus::init_with_save(&rom, SaveType::Eeprom512);
    reference.enable_rtc(0);
    scalar.enable_rtc(0);
    for address in [0x0800_0100, 0x09ff_fffc, 0x0d00_0000, 0x0dff_ff00] {
        for width in [AccessWidth::Half, AccessWidth::Word] {
            assert_code_fetch_parity(
                &mut reference,
                &mut scalar,
                address,
                width,
                AccessKind::Sequential,
            );
        }
    }
}

/// ZiGBA: "active fetch views preserve GPIO tails EEPROM and mutable RAM"
#[test]
fn active_fetch_views_preserve_gpio_tails_eeprom_and_mutable_ram() {
    let rom = vec![0xa5; 32 * 1024 * 1024];
    for rom_size in [0xcd, rom.len()] {
        let mut reference = Bus::init_with_save(&rom[..rom_size], SaveType::Eeprom512);
        let mut active = Bus::init_with_save(&rom[..rom_size], SaveType::Eeprom512);
        for bus in [&mut reference, &mut active] {
            bus.enable_rtc(0);
            write_half(bus, 0x0800_00c8, 1);
            write_half(bus, 0x0800_00c6, 15);
            write_half(bus, 0x0800_00c4, 15);
        }
        let mut region = InstructionFetchRegion::default();
        for address in [
            0x0800_0000,
            0x0800_00c0,
            0x0800_00c1,
            0x0800_00c3,
            0x0800_00c4,
            0x0800_00c8,
            0x0800_00c9,
            0x0800_00ca,
            0x0800_00cb,
            0x0800_00cc,
            0x0800_00cd,
            0x09ff_fffc,
            0x09ff_fffe,
            0x0a00_0000,
            0x0d00_0000,
            0x0dff_fefc,
            0x0dff_fefe,
            0x0dff_ff00,
            0x0300_0000,
            0x0300_7ffe,
            0,
            0x0400_0204,
            0x0e00_0000,
            0x1000_0000,
        ] {
            for width in [AccessWidth::Half, AccessWidth::Word] {
                assert_view_fetch_parity(
                    &mut reference,
                    &mut active,
                    &mut region,
                    address,
                    width,
                    AccessKind::Sequential,
                );
            }
        }
        reference.iwram[..4].copy_from_slice(&[1, 2, 3, 4]);
        active.iwram[..4].copy_from_slice(&[1, 2, 3, 4]);
        assert_view_fetch_parity(
            &mut reference,
            &mut active,
            &mut region,
            0x0300_0000,
            AccessWidth::Word,
            AccessKind::Nonsequential,
        );
        reference.iwram[..4].copy_from_slice(&[5, 6, 7, 8]);
        active.iwram[..4].copy_from_slice(&[5, 6, 7, 8]);
        assert_view_fetch_parity(
            &mut reference,
            &mut active,
            &mut region,
            0x0300_0000,
            AccessWidth::Word,
            AccessKind::Sequential,
        );
    }
}

/// ZiGBA: "scalar CPU reads preserve values and complete bus effects across regions"
#[test]
fn scalar_cpu_reads_preserve_values_and_complete_bus_effects_across_regions() {
    for batched in [false, true] {
        let mut reference = Bus::init(&[0x12, 0x34, 0x56, 0x78]);
        let mut scalar = Bus::init(&[0x12, 0x34, 0x56, 0x78]);
        for bus in [&mut reference, &mut scalar] {
            bus.ewram[0] = 0x91;
            bus.iwram[0] = 0x72;
            bus.palette[0] = 0x53;
            bus.vram[0] = 0x34;
            bus.oam[0] = 0x15;
            write_word(bus, 0x0400_0100, 0x00c0_fffe);
            bus.elapse(956);
            if batched {
                bus.begin_device_batch();
            }
        }
        for address in [
            0,
            0x0200_0000,
            0x02ff_ffff,
            0x0300_0000,
            0x03ff_ffff,
            0x0400_0100,
            0x0500_0000,
            0x0600_0000,
            0x0700_0000,
            0x0800_0000,
            0x09ff_fffe,
            0x0a00_0000,
            0x0e00_0000,
            0x1000_0000,
        ] {
            for width in [AccessWidth::Byte, AccessWidth::Half, AccessWidth::Word] {
                for kind in [AccessKind::Nonsequential, AccessKind::Sequential] {
                    let expected =
                        reference.read(access(address, width, kind, AccessSource::CpuData));
                    assert_eq!(expected.value, scalar.read_cpu_data(address, width, kind));
                    assert_same_bus(&reference, &scalar);
                }
            }
        }
        if batched {
            reference.end_device_batch();
            scalar.end_device_batch();
        }
        assert_same_bus(&reference, &scalar);
    }
}

/// ZiGBA: "scalar CPU word stores preserve RAM timing and complete bus effects across regions"
#[test]
fn scalar_cpu_word_stores_preserve_ram_timing_and_complete_bus_effects_across_regions() {
    for batched in [false, true] {
        let mut reference = Bus::init_with_save(&[0x12, 0x34, 0x56, 0x78], SaveType::Sram);
        let mut scalar = Bus::init_with_save(&[0x12, 0x34, 0x56, 0x78], SaveType::Sram);
        for bus in [&mut reference, &mut scalar] {
            bus.ewram.fill(0xa5);
            bus.iwram.fill(0xa5);
            write_half(bus, 0x0400_0204, 0x4000);
            write_word(bus, 0x0400_0100, 0x00c0_fff0);
            bus.elapse(950);
            bus.read(access(
                0x0800_0000,
                AccessWidth::Word,
                AccessKind::Nonsequential,
                AccessSource::CpuCode,
            ));
            assert!(bus.game_pak_prefetch.active);
            if batched {
                bus.begin_device_batch();
            }
        }
        for base in [0x0200_0000, 0x0300_0000] {
            let len = if base == 0x0200_0000 { scalar.ewram.len() } else { scalar.iwram.len() };
            let mask = len - 1;
            let cycles = if base == 0x0200_0000 { 6 } else { 1 };
            for offset in [0, len - 4, len - 1, 0xfffffc, 0xffffff] {
                for kind in [AccessKind::Nonsequential, AccessKind::Sequential] {
                    let address = base + offset as u32;
                    let value: u32 =
                        if kind == AccessKind::Nonsequential { 0x4433_2211 } else { 0x8877_6655 };
                    let started = scalar.now();
                    let open_bus = scalar.open_bus_value;
                    let expected = reference.write(
                        access(address, AccessWidth::Word, kind, AccessSource::CpuData),
                        value,
                    );
                    scalar.write_cpu_word(address, value, kind);
                    assert_eq!(cycles, expected.cycles);
                    assert_eq!(cycles, scalar.now() - started);
                    assert_eq!(open_bus, scalar.open_bus_value);
                    assert_eq!(AccessKind::Nonsequential, scalar.fetch_kind);
                    let ram =
                        if base == 0x0200_0000 { &scalar.ewram[..] } else { &scalar.iwram[..] };
                    for i in 0..4 {
                        assert_eq!((value >> (i * 8)) as u8, ram[(offset + i) & mask]);
                    }
                    assert_same_bus(&reference, &scalar);
                }
            }
        }
        for (address, value) in [
            (0x0400_0100, 0x00c0_fffe),
            (0x0400_0200, 0xffff_0008),
            (0x0400_0208, 1),
            (0x0400_0204, 0x4014),
            (0x0400_00a0, 0x4433_2211),
            (0x0500_0000, 0x03e0_001f),
            (0x0600_0000, 0x4433_2211),
            (0x0700_0000, 0x4433_2211),
            (0x0e00_0010, 0x8877_6655),
        ] {
            for kind in [AccessKind::Nonsequential, AccessKind::Sequential] {
                reference
                    .write(access(address, AccessWidth::Word, kind, AccessSource::CpuData), value);
                scalar.write_cpu_word(address, value, kind);
                assert_same_bus(&reference, &scalar);
            }
        }
        if batched {
            reference.end_device_batch();
            scalar.end_device_batch();
        }
        assert_same_bus(&reference, &scalar);
    }
}

/// ZiGBA: "scalar ROM instruction fetches match general reads through every prefetch phase"
#[test]
fn scalar_rom_instruction_fetches_match_general_reads_through_every_prefetch_phase() {
    for batched in [false, true] {
        for base in [0x0800_0000, 0x0c00_0000] {
            for width in [AccessWidth::Half, AccessWidth::Word] {
                for kind in [AccessKind::Nonsequential, AccessKind::Sequential] {
                    let mut reference = Bus::init(&[1, 2, 3, 4, 5, 6, 7, 8]);
                    let mut scalar = Bus::init(&[1, 2, 3, 4, 5, 6, 7, 8]);
                    for bus in [&mut reference, &mut scalar] {
                        write_half(bus, 0x0400_0204, 0x4000);
                        write_word(bus, 0x0400_0100, 0x00c0_fff0);
                        bus.elapse(950);
                        if batched {
                            bus.begin_device_batch();
                        }
                    }
                    let stride = if width == AccessWidth::Word { 4 } else { 2 };
                    let start = base + 0x100;
                    assert_code_fetch_parity(&mut reference, &mut scalar, start, width, kind);
                    assert_code_fetch_parity(
                        &mut reference,
                        &mut scalar,
                        start + stride,
                        width,
                        kind,
                    );
                    reference.elapse(1);
                    scalar.elapse(1);
                    assert_code_fetch_parity(
                        &mut reference,
                        &mut scalar,
                        start + 2 * stride,
                        width,
                        kind,
                    );
                    reference.elapse(128);
                    scalar.elapse(128);
                    for index in 3..12 {
                        assert_code_fetch_parity(
                            &mut reference,
                            &mut scalar,
                            start + index * stride,
                            width,
                            kind,
                        );
                    }
                    let other = if width == AccessWidth::Word {
                        AccessWidth::Half
                    } else {
                        AccessWidth::Word
                    };
                    assert_code_fetch_parity(
                        &mut reference,
                        &mut scalar,
                        base + 0x400,
                        other,
                        kind,
                    );
                    for bus in [&mut reference, &mut scalar] {
                        write_half(bus, 0x0400_0204, 0);
                    }
                    assert_code_fetch_parity(
                        &mut reference,
                        &mut scalar,
                        base + 0x800,
                        width,
                        AccessKind::Sequential,
                    );
                    reference.elapse_cpu_internal(3);
                    scalar.elapse_cpu_internal(3);
                    assert_code_fetch_parity(
                        &mut reference,
                        &mut scalar,
                        base + 0x800 + stride,
                        width,
                        AccessKind::Sequential,
                    );
                    for bus in [&mut reference, &mut scalar] {
                        write_half(bus, 0x0400_0204, 0x4014);
                    }
                    assert_code_fetch_parity(
                        &mut reference,
                        &mut scalar,
                        base + 0x20000 - stride,
                        width,
                        kind,
                    );
                    assert_code_fetch_parity(
                        &mut reference,
                        &mut scalar,
                        base + 0x20000,
                        width,
                        AccessKind::Sequential,
                    );
                    if batched {
                        reference.end_device_batch();
                        scalar.end_device_batch();
                    }
                    assert_same_bus(&reference, &scalar);
                }
            }
        }
    }
}

/// ZiGBA: "active IWRAM fetches preserve mirrors wrapping widths and complete bus effects"
#[test]
fn active_iwram_fetches_preserve_mirrors_wrapping_widths_and_complete_bus_effects() {
    for batched in [false, true] {
        let mut reference = Bus::init(&[1, 2, 3, 4]);
        let mut active = Bus::init(&[1, 2, 3, 4]);
        for bus in [&mut reference, &mut active] {
            bus.iwram[0..4].copy_from_slice(&[0x12, 0x34, 0x56, 0x78]);
            bus.iwram[0x7ffc..0x8000].copy_from_slice(&[0x90, 0xab, 0xcd, 0xef]);
            bus.waitcnt = 1 << 14;
            write_word(bus, 0x0400_0100, 0x00c0_fff0);
            bus.elapse(950);
            if batched {
                bus.begin_device_batch();
            }
        }
        let mut region = InstructionFetchRegion::default();
        for base in [0x0300_0000, 0x03ff_8000] {
            for (offset, half, word) in [
                (0, 0x3412, 0x7856_3412),
                (1, 0x5634, 0x0078_5634),
                (0x7ffe, 0xefcd, 0x3412_efcd),
                (0x7fff, 0x12ef, 0x5634_12ef),
            ] {
                for kind in [AccessKind::Nonsequential, AccessKind::Sequential] {
                    assert_view_fetch_parity(
                        &mut reference,
                        &mut active,
                        &mut region,
                        0x0800_0000,
                        AccessWidth::Word,
                        AccessKind::Nonsequential,
                    );
                    for (width, expected) in [(AccessWidth::Half, half), (AccessWidth::Word, word)]
                    {
                        let started = active.now();
                        assert_view_fetch_parity(
                            &mut reference,
                            &mut active,
                            &mut region,
                            base + offset,
                            width,
                            kind,
                        );
                        assert_eq!(started + 1, active.now());
                        assert_eq!(expected, active.open_bus_value);
                        assert_eq!(AccessKind::Sequential, active.fetch_kind);
                        assert!(!active.game_pak_code_active);
                        assert!(!active.game_pak_prefetch.active);
                        assert_eq!(0, active.game_pak_prefetch.head_address);
                    }
                }
            }
        }
        if batched {
            reference.end_device_batch();
            active.end_device_batch();
        }
        assert_same_bus(&reference, &active);
    }
}

/// ZiGBA: "scalar ROM fetches preserve DMA at buffered and in flight completion boundaries"
#[test]
fn scalar_rom_fetches_preserve_dma_at_buffered_and_in_flight_completion_boundaries() {
    for batched in [false, true] {
        for buffered in [false, true] {
            let mut reference = Bus::init(&[1, 2, 3, 4, 5, 6, 7, 8]);
            let mut scalar = Bus::init(&[1, 2, 3, 4, 5, 6, 7, 8]);
            for bus in [&mut reference, &mut scalar] {
                for (address, value) in [
                    (0x0400_00d4, 0),
                    (0x0400_00d6, 0x0800),
                    (0x0400_00d8, 0),
                    (0x0400_00da, 0x0300),
                    (0x0400_00dc, 1),
                    (0x0400_0204, 1 << 14),
                ] {
                    write_half(bus, address, value);
                }
                bus.read(access(
                    0x0800_0000,
                    AccessWidth::Word,
                    AccessKind::Nonsequential,
                    AccessSource::CpuCode,
                ));
                if buffered {
                    bus.elapse(128);
                }
                write_half(bus, 0x0400_00de, 1 << 15);
                if buffered {
                    bus.elapse(1);
                }
                if batched {
                    bus.begin_device_batch();
                }
            }
            assert_code_fetch_parity(
                &mut reference,
                &mut scalar,
                0x0800_0004,
                AccessWidth::Word,
                AccessKind::Sequential,
            );
            assert!(!scalar.game_pak_prefetch.active);
            if !buffered {
                assert_eq!(0x0800_0008, scalar.game_pak_prefetch.head_address);
            }
            assert_code_fetch_parity(
                &mut reference,
                &mut scalar,
                0x0300_0000,
                AccessWidth::Word,
                AccessKind::Nonsequential,
            );
            assert_eq!(0, scalar.game_pak_prefetch.head_address);
            assert_code_fetch_parity(
                &mut reference,
                &mut scalar,
                0x0800_0008,
                AccessWidth::Word,
                AccessKind::Nonsequential,
            );
            if batched {
                reference.end_device_batch();
                scalar.end_device_batch();
            }
            assert_same_bus(&reference, &scalar);
        }
    }
}

/// ZiGBA: "active ROM fetches preserve DMA at buffered and in flight completion boundaries"
#[test]
fn active_rom_fetches_preserve_dma_at_buffered_and_in_flight_completion_boundaries() {
    for batched in [false, true] {
        for buffered in [false, true] {
            let mut reference = Bus::init(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
            let mut active = Bus::init(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
            for bus in [&mut reference, &mut active] {
                write_word(bus, 0x0400_00d4, 0x0800_0000);
                write_word(bus, 0x0400_00d8, 0x0300_0000);
                write_half(bus, 0x0400_00dc, 1);
                write_half(bus, 0x0400_0204, 1 << 14);
                bus.read(access(
                    0x0800_0000,
                    AccessWidth::Word,
                    AccessKind::Nonsequential,
                    AccessSource::CpuCode,
                ));
                if buffered {
                    bus.elapse(128);
                }
                write_half(bus, 0x0400_00de, 1 << 15);
                if buffered {
                    bus.elapse(1);
                }
                if batched {
                    bus.begin_device_batch();
                }
            }
            let mut region = InstructionFetchRegion::default();
            assert_view_fetch_parity(
                &mut reference,
                &mut active,
                &mut region,
                0x0800_0004,
                AccessWidth::Word,
                AccessKind::Sequential,
            );
            assert!(!active.game_pak_prefetch.active);
            if !buffered {
                assert_eq!(0x0800_0008, active.game_pak_prefetch.head_address);
            }
            assert_view_fetch_parity(
                &mut reference,
                &mut active,
                &mut region,
                0x0300_0000,
                AccessWidth::Word,
                AccessKind::Nonsequential,
            );
            assert_eq!(0, active.game_pak_prefetch.head_address);
            assert_view_fetch_parity(
                &mut reference,
                &mut active,
                &mut region,
                0x0800_0008,
                AccessWidth::Word,
                AccessKind::Nonsequential,
            );
            if batched {
                reference.end_device_batch();
                active.end_device_batch();
            }
            assert_same_bus(&reference, &active);
        }
    }
}

/// ZiGBA: "active ROM views preserve live WAITCNT and every prefetch completion phase"
#[test]
fn active_rom_views_preserve_live_waitcnt_and_every_prefetch_completion_phase() {
    let rom: Vec<u8> = (0..0x20020).map(|index| (index * 17) as u8).collect();
    for batched in [false, true] {
        for width in [AccessWidth::Half, AccessWidth::Word] {
            for base in [0x0800_0000, 0x0a00_0000, 0x0c00_0000] {
                let mut reference = Bus::init(&rom);
                let mut active = Bus::init(&rom);
                for bus in [&mut reference, &mut active] {
                    bus.waitcnt = 0x4000;
                    bus.elapse(950);
                    if batched {
                        bus.begin_device_batch();
                    }
                }
                let mut region = InstructionFetchRegion::default();
                let stride = if width == AccessWidth::Word { 4 } else { 2 };
                let start = base + 0x100;
                assert_view_fetch_parity(
                    &mut reference,
                    &mut active,
                    &mut region,
                    start,
                    width,
                    AccessKind::Nonsequential,
                );
                assert_view_fetch_parity(
                    &mut reference,
                    &mut active,
                    &mut region,
                    start + stride,
                    width,
                    AccessKind::Sequential,
                );
                reference.elapse(1);
                active.elapse(1);
                assert_view_fetch_parity(
                    &mut reference,
                    &mut active,
                    &mut region,
                    start + 2 * stride,
                    width,
                    AccessKind::Sequential,
                );
                reference.elapse(128);
                active.elapse(128);
                for index in 3..12 {
                    assert_view_fetch_parity(
                        &mut reference,
                        &mut active,
                        &mut region,
                        start + index * stride,
                        width,
                        AccessKind::Sequential,
                    );
                }
                assert_view_fetch_parity(
                    &mut reference,
                    &mut active,
                    &mut region,
                    base + 0x20000 - 2 * stride,
                    width,
                    AccessKind::Nonsequential,
                );
                reference.waitcnt = 0x47fc;
                active.waitcnt = 0x47fc;
                assert_view_fetch_parity(
                    &mut reference,
                    &mut active,
                    &mut region,
                    base + 0x20000 - stride,
                    width,
                    AccessKind::Sequential,
                );
                assert_view_fetch_parity(
                    &mut reference,
                    &mut active,
                    &mut region,
                    base + 0x20000,
                    width,
                    AccessKind::Sequential,
                );
                for bus in [&mut reference, &mut active] {
                    write_half(bus, 0x0400_0204, 0);
                }
                assert_view_fetch_parity(
                    &mut reference,
                    &mut active,
                    &mut region,
                    start,
                    width,
                    AccessKind::Sequential,
                );
                reference.elapse_cpu_internal(3);
                active.elapse_cpu_internal(3);
                assert_view_fetch_parity(
                    &mut reference,
                    &mut active,
                    &mut region,
                    start + stride,
                    width,
                    AccessKind::Sequential,
                );
                if batched {
                    reference.end_device_batch();
                    active.end_device_batch();
                }
                assert_same_bus(&reference, &active);
            }
        }
    }
}

/// ZiGBA: "active IWRAM fetches latch before DMA and observe later writes"
#[test]
fn active_iwram_fetches_latch_before_dma_and_observe_later_writes() {
    for batched in [false, true] {
        for width in [AccessWidth::Half, AccessWidth::Word] {
            let mut reference = Bus::init(&[]);
            let mut active = Bus::init(&[]);
            for bus in [&mut reference, &mut active] {
                write_half(bus, 0x0200_0000, 0x1122);
                write_half(bus, 0x0200_0002, 0x3344);
                write_word(bus, 0x0400_00d4, 0x0200_0000);
                write_word(bus, 0x0400_00d8, 0x0300_0000);
                write_half(bus, 0x0400_00dc, 2);
            }
            let mut region = InstructionFetchRegion::default();
            assert_view_fetch_parity(
                &mut reference,
                &mut active,
                &mut region,
                0x0300_8000,
                width,
                AccessKind::Nonsequential,
            );
            for bus in [&mut reference, &mut active] {
                write_half(bus, 0x0400_00de, (1 << 14) | (1 << 15));
            }
            let started = active.now();
            if batched {
                reference.begin_device_batch();
                active.begin_device_batch();
            }
            for _ in 0..2 {
                assert_view_fetch_parity(
                    &mut reference,
                    &mut active,
                    &mut region,
                    0x0300_8000,
                    width,
                    AccessKind::Sequential,
                );
            }
            assert_eq!(started + 12, active.now());
            assert_ne!(0, active.irq.interrupt_request_flags & (1 << 11));
            assert_view_fetch_parity(
                &mut reference,
                &mut active,
                &mut region,
                0x0300_8000,
                width,
                AccessKind::Sequential,
            );
            assert_eq!(
                if width == AccessWidth::Half { 0x1122 } else { 0x3344_1122 },
                active.open_bus_value
            );
            for bus in [&mut reference, &mut active] {
                write_half(bus, 0x0300_0000, 0xaabb);
            }
            assert_view_fetch_parity(
                &mut reference,
                &mut active,
                &mut region,
                0x0300_8000,
                width,
                AccessKind::Sequential,
            );
            assert_eq!(
                if width == AccessWidth::Half { 0xaabb } else { 0x3344_aabb },
                active.open_bus_value
            );
            if batched {
                reference.end_device_batch();
                active.end_device_batch();
            }
            assert_same_bus(&reference, &active);
        }
    }
}

/// ZiGBA: "write classification keeps RAM batched and MMIO flush ordering exact"
#[test]
fn write_classification_keeps_ram_batched_and_mmio_flush_ordering_exact() {
    let mut eager = Bus::init(&[]);
    let mut batched = Bus::init(&[]);
    for bus in [&mut eager, &mut batched] {
        write_half(bus, 0x0400_0100, 0xfff0);
        write_half(bus, 0x0400_0102, 0x0080);
        bus.ewram[..8].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        bus.iwram[..8].copy_from_slice(&[9, 10, 11, 12, 13, 14, 15, 16]);
        bus.elapse(1228);
    }
    batched.begin_device_batch();
    enum Step {
        Elapse(u64),
        Write(u32, AccessWidth, u32),
        Read(u32, AccessWidth),
    }
    let steps = [
        Step::Elapse(1),
        Step::Write(0x0200_0000, AccessWidth::Half, 0x1234),
        Step::Write(0x0200_0004, AccessWidth::Word, 0x89ab_cdef),
        Step::Elapse(2),
        Step::Write(0x0300_0000, AccessWidth::Byte, 0x5a),
        Step::Write(0x0300_0002, AccessWidth::Word, 0x11aa_22bb),
        Step::Write(0x0400_0004, AccessWidth::Half, 0x0008),
        Step::Elapse(3),
        Step::Read(0x0400_0006, AccessWidth::Half),
        Step::Write(0x0500_0000, AccessWidth::Word, 0x03e0_001f),
        Step::Elapse(1),
        Step::Write(0x0600_0000, AccessWidth::Word, 0x4433_2211),
        Step::Write(0x0700_0000, AccessWidth::Half, 0x0100),
        Step::Write(0x0400_0082, AccessWidth::Half, 0x0008),
        Step::Elapse(17),
        Step::Write(0x1000_0000, AccessWidth::Word, 0xdead_beef),
        Step::Read(0x0400_0004, AccessWidth::Half),
    ];
    for step in steps {
        match step {
            Step::Elapse(cycles) => {
                eager.elapse(cycles);
                batched.elapse(cycles);
            }
            Step::Write(address, width, value) => {
                let request =
                    access(address, width, AccessKind::Nonsequential, AccessSource::CpuData);
                assert_eq!(eager.write(request, value), batched.write(request, value));
            }
            Step::Read(address, width) => {
                let request =
                    access(address, width, AccessKind::Nonsequential, AccessSource::CpuData);
                assert_eq!(eager.read(request), batched.read(request));
            }
        }
    }
    batched.end_device_batch();
    assert_same_bus(&eager, &batched);
}

fn assert_fetch_state(reference: &Bus, active: &Bus) {
    let r = &reference.game_pak_prefetch;
    let a = &active.game_pak_prefetch;
    assert_eq!(reference.now(), active.now());
    assert_eq!(
        (
            r.active,
            r.head_address,
            r.last_address,
            r.count,
            r.capacity,
            r.opcode_bytes,
            r.countdown,
            r.duty
        ),
        (
            a.active,
            a.head_address,
            a.last_address,
            a.count,
            a.capacity,
            a.opcode_bytes,
            a.countdown,
            a.duty
        )
    );
    assert_eq!(reference.duty_valid_until, active.duty_valid_until);
    assert_eq!(reference.open_bus_value, active.open_bus_value);
    assert_eq!(reference.fetch_kind, active.fetch_kind);
    assert_eq!(reference.game_pak_code_active, active.game_pak_code_active);
    assert_eq!(reference.prefetch_disabled_bug_pending, active.prefetch_disabled_bug_pending);
    assert_eq!(reference.prefetch_buffer_was_disabled, active.prefetch_buffer_was_disabled);
    assert_eq!(reference.batch_deadline, active.batch_deadline);
    assert_eq!(reference.irq.interrupt_request_flags, active.irq.interrupt_request_flags);
}

fn assert_thumb_refill_parity(
    reference: &mut Bus,
    reference_region: &mut InstructionFetchRegion,
    active: &mut Bus,
    active_region: &mut InstructionFetchRegion,
    target: u32,
) {
    assert_eq!(reference.now(), active.now());
    reference.fetch_kind = AccessKind::Nonsequential;
    let first = reference.fetch_instruction_in_region(reference_region, target, AccessWidth::Half);
    reference.fetch_kind = AccessKind::Sequential;
    let second = reference.fetch_instruction_in_region(
        reference_region,
        target.wrapping_add(2),
        AccessWidth::Half,
    );
    let words = active.fetch_thumb_refill_in_region(active_region, target);
    assert_eq!([first, second], words);
    assert_fetch_state(reference, active);
}

/// ZiGBA: "Thumb refills match two ordinary fetches in every queue phase and at deadlines"
#[test]
fn thumb_refills_match_two_ordinary_fetches_in_every_queue_phase_and_at_deadlines() {
    let rom: Vec<u8> = (0..0x20040).map(|index| (index * 29 + 3) as u8).collect();
    for batched in [false, true] {
        for waitcnt in [0x4000, 0x4317, 0x47fc, 0x0317] {
            let mut reference = Bus::init(&rom);
            let mut active = Bus::init(&rom);
            for bus in [&mut reference, &mut active] {
                write_half(bus, 0x0400_0204, waitcnt);
                bus.elapse(900);
                if batched {
                    bus.begin_device_batch();
                }
            }
            let mut reference_region = InstructionFetchRegion::default();
            let mut active_region = InstructionFetchRegion::default();
            for target in [
                0x0800_0100u32,
                0x0800_0101,
                0x0800_00be,
                0x0800_00c2,
                0x0800_00c8,
                0x0801_fffa,
                0x0801_fffc,
                0x0801_fffe,
                0x0802_0000,
                0x0802_003a,
                0x0802_003c,
                0x0a00_0100,
                0x0c00_0100,
                0x0d00_0000,
                0x0300_0100,
            ] {
                for lead in [0u32, 2, 4, 6, 8, 64] {
                    for gap in [0u64, 1, 2, 3, 5, 9, 17] {
                        let stream = target.wrapping_sub(lead);
                        for bus in [&mut reference, &mut active] {
                            bus.read(access(
                                stream,
                                AccessWidth::Half,
                                AccessKind::Nonsequential,
                                AccessSource::CpuCode,
                            ));
                            bus.read(access(
                                stream.wrapping_add(2),
                                AccessWidth::Half,
                                AccessKind::Sequential,
                                AccessSource::CpuCode,
                            ));
                            if gap == 9 {
                                write_half(bus, 0x0400_0204, waitcnt & !(1 << 14));
                                write_half(bus, 0x0400_0204, waitcnt);
                            }
                            bus.elapse_cpu_internal(gap);
                        }
                        assert_thumb_refill_parity(
                            &mut reference,
                            &mut reference_region,
                            &mut active,
                            &mut active_region,
                            target,
                        );
                    }
                }
                if batched {
                    for margin in 1..24 {
                        for bus in [&mut reference, &mut active] {
                            bus.read(access(
                                target.wrapping_add(0x40),
                                AccessWidth::Half,
                                AccessKind::Nonsequential,
                                AccessSource::CpuCode,
                            ));
                            let until_deadline = bus.batch_deadline.saturating_sub(bus.now());
                            if until_deadline > margin {
                                bus.elapse(until_deadline - margin);
                            }
                        }
                        assert_thumb_refill_parity(
                            &mut reference,
                            &mut reference_region,
                            &mut active,
                            &mut active_region,
                            target,
                        );
                    }
                }
            }
            if batched {
                reference.end_device_batch();
                active.end_device_batch();
            }
            assert_same_bus(&reference, &active);
        }
    }
}
