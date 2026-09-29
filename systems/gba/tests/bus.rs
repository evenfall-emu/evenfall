//! ZiGBA bus integration oracles (`test/bus_test.zig`).

use evenfall_gba::bus::{
    Access, AccessKind, AccessSource, AccessWidth, Bus, JoyBusCommand, JoyBusError, KeyState,
    SioConnection, SioLinkError, TimedInput, TimedInputChange, synchronize_sio,
};
use evenfall_gba::cartridge::save::Type as SaveType;

#[path = "bus/fetch.rs"]
mod fetch;
#[path = "bus/peripherals.rs"]
mod peripherals;
#[path = "bus/video.rs"]
mod video;

const FLASH_SECTOR_ERASE_CYCLES: u64 = 30_000;

fn access(address: u32, width: AccessWidth, kind: AccessKind, source: AccessSource) -> Access {
    Access { address, width, kind, source }
}

fn write_half(bus: &mut Bus, address: u32, value: u16) -> u64 {
    bus.write(
        access(address, AccessWidth::Half, AccessKind::Nonsequential, AccessSource::CpuData),
        u32::from(value),
    )
    .cycles
}

fn write_word(bus: &mut Bus, address: u32, value: u32) -> u64 {
    bus.write(
        access(address, AccessWidth::Word, AccessKind::Nonsequential, AccessSource::CpuData),
        value,
    )
    .cycles
}

fn write_byte(bus: &mut Bus, address: u32, value: u8) -> u64 {
    bus.write(
        access(address, AccessWidth::Byte, AccessKind::Nonsequential, AccessSource::CpuData),
        u32::from(value),
    )
    .cycles
}

fn read_half(bus: &mut Bus, address: u32) -> u16 {
    bus.read(access(address, AccessWidth::Half, AccessKind::Nonsequential, AccessSource::CpuData))
        .value as u16
}

fn read_word(bus: &mut Bus, address: u32) -> u32 {
    bus.read(access(address, AccessWidth::Word, AccessKind::Nonsequential, AccessSource::CpuData))
        .value
}

fn read_byte(bus: &mut Bus, address: u32) -> u8 {
    bus.read(access(address, AccessWidth::Byte, AccessKind::Nonsequential, AccessSource::CpuData))
        .value as u8
}

/// ZiGBA: "BIOS hard reset preserves host audio suppression"
#[test]
fn bios_hard_reset_preserves_host_audio_suppression() {
    let mut bus = Bus::init(&[]);
    bus.set_audio_output(false);
    bus.hard_reset();
    bus.elapse_cpu_internal(1024);
    let mut samples = [Default::default(); 16];
    assert_eq!(0, bus.drain_audio_samples(&mut samples));
    bus.set_audio_output(true);
    bus.elapse_cpu_internal(1024);
    assert_eq!(2, bus.drain_audio_samples(&mut samples));
}

/// ZiGBA: "no-event materialization preserves every prescaler and cascade until the exact overflow"
#[test]
fn no_event_materialization_preserves_every_prescaler_and_cascade_until_the_exact_overflow() {
    for (select, divisor) in [1, 64, 256, 1024].into_iter().enumerate() {
        let mut bus = Bus::init(&[]);
        bus.timers[0].reload = 0xfffc;
        bus.timers[0].counter = 0xfffc;
        bus.timers[0].control = 0x00c4 | select as u16;
        for timer in &mut bus.timers[1..] {
            timer.reload = 0xffff;
            timer.counter = 0xffff;
            timer.control = 0x00c4;
        }
        bus.begin_device_batch();
        for _ in 0..2 * divisor {
            bus.elapse(1);
        }
        bus.set_audio_output(true);
        assert_eq!(0xfffe, bus.timers[0].counter);
        assert_eq!(0, bus.timers[0].prescaler_phase);
        for _ in 0..2 * divisor - 1 {
            bus.elapse(1);
        }
        bus.set_audio_output(true);
        assert_eq!(4 * divisor - 1, bus.now());
        assert_eq!(0xffff, bus.timers[0].counter);
        assert_eq!(divisor - 1, bus.timers[0].prescaler_phase);
        assert_eq!(0, bus.interrupt_request_flags);
        for timer in &bus.timers[1..] {
            assert_eq!(0xffff, timer.counter);
        }
        bus.elapse(1);
        assert_eq!(4 * divisor, bus.now());
        assert_eq!(0xfffc, bus.timers[0].counter);
        assert_eq!(0, bus.timers[0].prescaler_phase);
        assert_eq!(0x0078, bus.interrupt_request_flags);
        for timer in &bus.timers[1..] {
            assert_eq!(0xffff, timer.counter);
        }
        bus.end_device_batch();
    }
}

/// ZiGBA: "no-event materialization precedes register reset and hard reset clock changes"
#[test]
fn no_event_materialization_precedes_register_reset_and_hard_reset_clock_changes() {
    let mut bus = Bus::init(&[]);
    bus.timers[0].control = 0x0080;
    bus.begin_device_batch();
    bus.elapse(1);
    bus.elapse(6);
    bus.register_ram_reset(0);
    assert_eq!(7, bus.now());
    assert_eq!(7, bus.ppu_timing.cycle_in_line);
    assert_eq!(7, bus.timers[0].counter);
    bus.elapse(1);
    bus.elapse(4);
    bus.hard_reset();
    assert_eq!(12, bus.now());
    assert_eq!(0, bus.ppu_timing.cycle_in_line);
    bus.elapse(1);
    assert_eq!(13, bus.now());
    assert_eq!(1, bus.ppu_timing.cycle_in_line);
    assert_eq!(0, bus.timers[0].counter);
    bus.end_device_batch();
}

/// ZiGBA: "RAM accesses preserve byte order, unaligned mirror wrapping and bus timing"
#[test]
fn ram_accesses_preserve_byte_order_unaligned_mirror_wrapping_and_bus_timing() {
    let mut bus = Bus::init(&[]);
    for base in [0x0200_0000, 0x0300_0000] {
        let len = if base == 0x0200_0000 { bus.ewram.len() } else { bus.iwram.len() };
        let mask = len - 1;
        for offset in [0, 1, 2, 3, len - 4, len - 3, len - 2, len - 1, 0xfffffd, 0xfffffe, 0xffffff]
        {
            for (width, count, expected) in [
                (AccessWidth::Byte, 1, 0x11),
                (AccessWidth::Half, 2, 0x2211),
                (AccessWidth::Word, 4, 0x4433_2211),
            ] {
                let cycles = if base == 0x0300_0000 {
                    1
                } else if width == AccessWidth::Word {
                    6
                } else {
                    3
                };
                let addr = access(
                    base + offset as u32,
                    width,
                    AccessKind::Nonsequential,
                    AccessSource::CpuData,
                );
                let ram = if base == 0x0200_0000 { &mut bus.ewram[..] } else { &mut bus.iwram[..] };
                ram.fill(0xa5);
                for (i, byte) in [0x11, 0x22, 0x33, 0x44].into_iter().enumerate() {
                    ram[(offset + i) & mask] = byte;
                }
                let read = bus.read(addr);
                assert_eq!(expected, read.value);
                assert_eq!(expected, bus.open_bus_value);
                assert_eq!(cycles, read.cycles);
                assert_eq!(AccessKind::Nonsequential, bus.fetch_kind);
                let ram = if base == 0x0200_0000 { &mut bus.ewram[..] } else { &mut bus.iwram[..] };
                ram.fill(0xa5);
                let write = bus.write(addr, 0x4433_2211);
                assert_eq!(cycles, write.cycles);
                let ram = if base == 0x0200_0000 { &bus.ewram[..] } else { &bus.iwram[..] };
                for (i, byte) in [0x11, 0x22, 0x33, 0x44][..count].iter().enumerate() {
                    assert_eq!(*byte, ram[(offset + i) & mask]);
                }
                assert_eq!(0xa5, ram[(offset + mask) & mask]);
                assert_eq!(0xa5, ram[(offset + count) & mask]);
            }
        }
    }
}

/// ZiGBA: "default Game Pak, SRAM, and WRAM access costs match the GBATEK totals"
#[test]
fn default_game_pak_sram_and_wram_access_costs_match_the_gbatek_totals() {
    let mut bus = Bus::init(&[0; 8]);
    let read = |bus: &mut Bus, address, width, kind| {
        bus.read(access(address, width, kind, AccessSource::CpuData)).cycles
    };
    assert_eq!(5, read(&mut bus, 0x0800_0000, AccessWidth::Half, AccessKind::Nonsequential));
    assert_eq!(3, read(&mut bus, 0x0800_0002, AccessWidth::Half, AccessKind::Sequential));
    assert_eq!(8, read(&mut bus, 0x0800_0000, AccessWidth::Word, AccessKind::Nonsequential));
    assert_eq!(6, read(&mut bus, 0x0800_0004, AccessWidth::Word, AccessKind::Sequential));
    assert_eq!(5, read(&mut bus, 0x0e00_0000, AccessWidth::Byte, AccessKind::Nonsequential));
    assert_eq!(6, read(&mut bus, 0x0200_0000, AccessWidth::Word, AccessKind::Nonsequential));
    assert_eq!(1, read(&mut bus, 0x0300_0000, AccessWidth::Word, AccessKind::Nonsequential));
    assert_eq!(2, read(&mut bus, 0x0600_0000, AccessWidth::Word, AccessKind::Nonsequential));
}

/// ZiGBA: "cached IRQ line follows IE IF IME writes and keypad edges"
#[test]
fn cached_irq_line_follows_ie_if_ime_writes_and_keypad_edges() {
    let mut bus = Bus::init(&[]);
    write_half(&mut bus, 0x0400_0132, 0x4001);
    write_half(&mut bus, 0x0400_0200, 1 << 12);
    write_half(&mut bus, 0x0400_0208, 1);
    assert!(!bus.irq_line);
    bus.set_keys(KeyState { a: true, ..Default::default() });
    assert!(bus.irq_line);
    assert!(bus.irq_pending());
    write_byte(&mut bus, 0x0400_0203, 1 << 4);
    assert!(!bus.irq_line);
    bus.set_keys(Default::default());
    bus.set_keys(KeyState { a: true, ..Default::default() });
    assert!(bus.irq_line);
    write_half(&mut bus, 0x0400_0200, 0);
    assert!(!bus.irq_line);
    write_half(&mut bus, 0x0400_0200, 1 << 12);
    assert!(bus.irq_line);
    write_half(&mut bus, 0x0400_0208, 0);
    assert!(!bus.irq_line);
    write_half(&mut bus, 0x0400_0208, 1);
    assert!(bus.irq_line);
    write_half(&mut bus, 0x0400_0202, 1 << 12);
    assert!(!bus.irq_line);
}

/// ZiGBA: "LCD timing follows the GBATEK 960/1232-cycle line and 228-line frame"
#[test]
fn lcd_timing_follows_the_gbatek_960_1232_cycle_line_and_228_line_frame() {
    let mut bus = Bus::init(&[]);
    write_half(&mut bus, 0x0400_0004, 1 << 3);
    bus.elapse(959);
    assert_ne!(0, read_half(&mut bus, 0x0400_0004) & (1 << 1));
    bus.elapse(271);
    assert_eq!(1, read_half(&mut bus, 0x0400_0006));
    bus.elapse(160 * 1232 - bus.now());
    assert_eq!(160, read_half(&mut bus, 0x0400_0006));
    assert_ne!(0, read_half(&mut bus, 0x0400_0004) & 1);
    assert_ne!(0, read_half(&mut bus, 0x0400_0202) & 1);
    bus.elapse(228 * 1232 - 1 - bus.now());
    assert_eq!(0, bus.completed_frames());
    bus.elapse(1);
    assert_eq!(1, bus.completed_frames());
}

/// ZiGBA: "aligned MMIO words compose adjacent halfword registers in one cycle"
#[test]
fn aligned_mmio_words_compose_adjacent_halfword_registers_in_one_cycle() {
    let mut bus = Bus::init(&[]);
    assert_eq!(1, write_word(&mut bus, 0x0400_0000, 0x0001_0100));
    assert_eq!(0x0001_0100, read_word(&mut bus, 0x0400_0000));
    bus.elapse(1232 - bus.now());
    let result = bus.read(access(
        0x0400_0004,
        AccessWidth::Word,
        AccessKind::Nonsequential,
        AccessSource::CpuData,
    ));
    assert_eq!(0x0001_0000, result.value);
    assert_eq!(1, result.cycles);
}

/// ZiGBA: "unmapped and unused MMIO reads distinguish open-bus words from paired zero halves"
#[test]
fn unmapped_and_unused_mmio_reads_distinguish_open_bus_words_from_paired_zero_halves() {
    let mut bus = Bus::init(&[0x78, 0x56, 0x34, 0x12]);
    bus.read(access(
        0x0800_0000,
        AccessWidth::Word,
        AccessKind::Nonsequential,
        AccessSource::CpuCode,
    ));
    let unused = bus.read(access(
        0x0400_00e0,
        AccessWidth::Word,
        AccessKind::Nonsequential,
        AccessSource::CpuData,
    ));
    assert_eq!(0x1234_5678, unused.value);
    assert_eq!(1, unused.cycles);
    assert_eq!(0x1234_5678, read_word(&mut bus, 0x8000_0000));
    bus.read(access(
        0x0800_0000,
        AccessWidth::Word,
        AccessKind::Nonsequential,
        AccessSource::CpuCode,
    ));
    let half = bus.read(access(
        0x0400_00e2,
        AccessWidth::Half,
        AccessKind::Nonsequential,
        AccessSource::CpuData,
    ));
    assert_eq!(0x1234, half.value);
    let paired = bus.read(access(
        0x0400_004e,
        AccessWidth::Half,
        AccessKind::Nonsequential,
        AccessSource::CpuData,
    ));
    assert_eq!(0, paired.value);
}

/// ZiGBA: "unused and read-only MMIO writes are ignored"
#[test]
fn unused_and_read_only_mmio_writes_are_ignored() {
    let mut bus = Bus::init(&[]);
    for (address, width, value) in [
        (0x0400_00e0, AccessWidth::Half, 0xffff),
        (0x0400_0400, AccessWidth::Word, 0xffff_ffff),
        (0x0400_0407, AccessWidth::Byte, 0xff),
    ] {
        assert_eq!(
            1,
            bus.write(
                access(address, width, AccessKind::Nonsequential, AccessSource::CpuData),
                value
            )
            .cycles
        );
    }
    write_half(&mut bus, 0x0400_0006, 0xffff);
    write_half(&mut bus, 0x0400_0130, 0);
    assert_eq!(0x03ff, read_half(&mut bus, 0x0400_0130));
}

/// ZiGBA: "unused Game Pak ROM reads expose the multiplexed halfword address"
#[test]
fn unused_game_pak_rom_reads_expose_the_multiplexed_halfword_address() {
    let mut bus = Bus::init(&[0xaa]);
    assert_eq!(0x1a, read_byte(&mut bus, 0x0800_1234));
    assert_eq!(0x09, read_byte(&mut bus, 0x0a00_1235));
    assert_eq!(0x091a, read_half(&mut bus, 0x0c00_1234));
    assert_eq!(0x091b_091a, read_word(&mut bus, 0x0800_1234));
}

/// ZiGBA: "bitmap modes leave the lower half of mirrored VRAM unmapped"
#[test]
fn bitmap_modes_leave_the_lower_half_of_mirrored_vram_unmapped() {
    let mut bus = Bus::init(&[]);
    write_half(&mut bus, 0x0601_0000, 0xbeef);
    write_half(&mut bus, 0x0400_0000, 3);
    assert_eq!(0, read_half(&mut bus, 0x0601_8000));
    write_half(&mut bus, 0x0601_8000, 0x1234);
    assert_eq!(0xbeef, read_half(&mut bus, 0x0601_0000));
    write_half(&mut bus, 0x0601_c000, 0xcafe);
    assert_eq!(0xcafe, read_half(&mut bus, 0x0601_4000));
}

/// ZiGBA: "Game Pak timing follows every wait field and forces N at 128 KiB boundaries"
#[test]
fn game_pak_timing_follows_every_wait_field_and_forces_n_at_128_kib_boundaries() {
    let mut bus = Bus::init(&[]);
    for window in 0..3u32 {
        for first in 0..4u16 {
            for fast in [false, true] {
                let first_shift = 2 + window * 3;
                let seq_shift = 4 + window * 3;
                let waitcnt = first << first_shift | u16::from(fast) << seq_shift | first;
                write_half(&mut bus, 0x0400_0204, waitcnt ^ 0x07ff);
                bus.read(access(
                    0x0800_0004,
                    AccessWidth::Word,
                    AccessKind::Sequential,
                    AccessSource::CpuData,
                ));
                bus.waitcnt = waitcnt;
                let n = [5, 4, 3, 9][first as usize];
                let s = if fast { 2 } else { [3, 5, 9][window as usize] };
                let sram = bus.read(access(
                    0x0e00_0000,
                    AccessWidth::Byte,
                    AccessKind::Nonsequential,
                    AccessSource::CpuData,
                ));
                assert_eq!(n, sram.cycles);
                for offset in [4, 0x20000] {
                    for kind in [AccessKind::Nonsequential, AccessKind::Sequential] {
                        for width in [AccessWidth::Half, AccessWidth::Word] {
                            let result = bus.read(access(
                                0x0800_0000 + window * 0x0200_0000 + offset,
                                width,
                                kind,
                                AccessSource::CpuData,
                            ));
                            let first_cycles =
                                if offset == 0x20000 || kind == AccessKind::Nonsequential {
                                    n
                                } else {
                                    s
                                };
                            assert_eq!(
                                first_cycles + if width == AccessWidth::Word { s } else { 0 },
                                result.cycles
                            );
                        }
                    }
                }
            }
        }
    }
}

/// ZiGBA: "Thumb Game Pak prefetch fills during internal cycles and serves buffered opcodes"
#[test]
fn thumb_game_pak_prefetch_fills_during_internal_cycles_and_serves_buffered_opcodes() {
    let mut bus = Bus::init(&[0; 16]);
    bus.waitcnt = 1 << 14;
    let first = bus.read(access(
        0x0800_0000,
        AccessWidth::Half,
        AccessKind::Nonsequential,
        AccessSource::CpuCode,
    ));
    bus.elapse(4);
    let second = bus.read(access(
        0x0800_0002,
        AccessWidth::Half,
        AccessKind::Sequential,
        AccessSource::CpuCode,
    ));
    let third = bus.read(access(
        0x0800_0004,
        AccessWidth::Half,
        AccessKind::Sequential,
        AccessSource::CpuCode,
    ));
    assert_eq!(5, first.cycles);
    assert_eq!(1, second.cycles);
    assert_eq!(1, third.cycles);
}

/// ZiGBA: "ARM Game Pak prefetch buffers complete words and completes an in-flight word"
#[test]
fn arm_game_pak_prefetch_buffers_complete_words_and_completes_an_in_flight_word() {
    let mut bus = Bus::init(&[0; 16]);
    bus.waitcnt = 1 << 14;
    let first = bus.read(access(
        0x0800_0000,
        AccessWidth::Word,
        AccessKind::Nonsequential,
        AccessSource::CpuCode,
    ));
    bus.elapse(6);
    let buffered = bus.read(access(
        0x0800_0004,
        AccessWidth::Word,
        AccessKind::Sequential,
        AccessSource::CpuCode,
    ));
    let in_flight = bus.read(access(
        0x0800_0008,
        AccessWidth::Word,
        AccessKind::Sequential,
        AccessSource::CpuCode,
    ));
    assert_eq!(8, first.cycles);
    assert_eq!(1, buffered.cycles);
    assert_eq!(5, in_flight.cycles);
}

/// ZiGBA: "Game Pak data reads and writes cancel prefetched opcodes"
#[test]
fn game_pak_data_reads_and_writes_cancel_prefetched_opcodes() {
    for write in [false, true] {
        let mut bus = Bus::init(&[0; 16]);
        bus.waitcnt = 1 << 14;
        bus.read(access(
            0x0800_0000,
            AccessWidth::Half,
            AccessKind::Nonsequential,
            AccessSource::CpuCode,
        ));
        bus.elapse(4);
        let data = access(
            0x0800_000a,
            AccessWidth::Half,
            AccessKind::Nonsequential,
            AccessSource::CpuData,
        );
        if write {
            bus.write(data, 0xffff);
        } else {
            bus.read(data);
        }
        let code = bus.read(access(
            0x0800_0002,
            AccessWidth::Half,
            AccessKind::Sequential,
            AccessSource::CpuCode,
        ));
        assert_eq!(3, code.cycles);
    }
}

/// ZiGBA: "ordinary Game Pak writes consume wait states without changing ROM"
#[test]
fn ordinary_game_pak_writes_consume_wait_states_without_changing_rom() {
    let rom = [0x34, 0x12, 0x78, 0x56];
    let mut half = Bus::init(&rom);
    let write = write_half(&mut half, 0x0800_0000, 0xbeef);
    let unchanged = read_half(&mut half, 0x0800_0000);
    assert_eq!(5, write);
    assert_eq!(0x1234, unchanged);
    let mut word = Bus::init(&rom);
    assert_eq!(8, write_word(&mut word, 0x0800_0000, 0xdead_beef));
}

/// ZiGBA: "SRAM traffic pays a one-cycle collision when Game Pak prefetch is about to complete"
#[test]
fn sram_traffic_pays_a_one_cycle_collision_when_game_pak_prefetch_is_about_to_complete() {
    let mut bus = Bus::init(&[0; 16]);
    bus.waitcnt = 1 << 14;
    bus.read(access(
        0x0800_0000,
        AccessWidth::Half,
        AccessKind::Nonsequential,
        AccessSource::CpuCode,
    ));
    bus.elapse(1);
    let sram = bus.read(access(
        0x0e00_0000,
        AccessWidth::Byte,
        AccessKind::Nonsequential,
        AccessSource::CpuData,
    ));
    assert_eq!(5, sram.cycles);
}

/// ZiGBA: "disabling prefetch, or an internal cycle while disabled, forces the next opcode fetch N"
#[test]
fn disabling_prefetch_or_an_internal_cycle_while_disabled_forces_the_next_opcode_fetch_n() {
    fn fetch(bus: &mut Bus, address: u32, kind: AccessKind) -> u64 {
        bus.read(access(address, AccessWidth::Half, kind, AccessSource::CpuCode)).cycles
    }
    let mut disabling = Bus::init(&[0; 16]);
    write_half(&mut disabling, 0x0400_0204, 1 << 14);
    fetch(&mut disabling, 0x0800_0000, AccessKind::Nonsequential);
    write_half(&mut disabling, 0x0400_0204, 0);
    assert_eq!(5, fetch(&mut disabling, 0x0800_0002, AccessKind::Sequential));
    let mut disabled = Bus::init(&[0; 16]);
    fetch(&mut disabled, 0x0800_0000, AccessKind::Nonsequential);
    disabled.elapse_cpu_internal(1);
    assert_eq!(5, fetch(&mut disabled, 0x0800_0002, AccessKind::Sequential));
    assert_eq!(3, fetch(&mut disabled, 0x0800_0004, AccessKind::Sequential));
}

/// ZiGBA: "implemented MMIO registers expose their individual byte lanes"
#[test]
fn implemented_mmio_registers_expose_their_individual_byte_lanes() {
    let mut bus = Bus::init(&[]);
    bus.set_keys(KeyState { a: true, left: true, ..Default::default() });
    write_half(&mut bus, 0x0400_0000, 0xa55a);
    assert_eq!(0x5a, read_byte(&mut bus, 0x0400_0000));
    assert_eq!(0xa5, read_byte(&mut bus, 0x0400_0001));
    assert_eq!(0xde, read_byte(&mut bus, 0x0400_0130));
    assert_eq!(0x03, read_byte(&mut bus, 0x0400_0131));
}

/// ZiGBA: "MMIO byte writes preserve the other lane for video timers and DMA"
#[test]
fn mmio_byte_writes_preserve_the_other_lane_for_video_timers_and_dma() {
    let mut bus = Bus::init(&[]);
    write_byte(&mut bus, 0x0400_0000, 0x03);
    write_byte(&mut bus, 0x0400_0001, 0x12);
    assert_eq!(0x1203, read_half(&mut bus, 0x0400_0000));
    write_byte(&mut bus, 0x0400_0100, 0xaa);
    write_byte(&mut bus, 0x0400_0101, 0xbb);
    write_byte(&mut bus, 0x0400_0102, 0x80);
    assert_eq!(0xbbaa, bus.timers[0].reload);
    assert_eq!(0xbbaa, bus.timers[0].counter);
    for (address, value) in [
        (0x0400_00d4, 0x78),
        (0x0400_00d5, 0x56),
        (0x0400_00d6, 0x34),
        (0x0400_00d7, 0xf2),
        (0x0400_00dc, 0x21),
        (0x0400_00dd, 0x43),
    ] {
        write_byte(&mut bus, address, value);
    }
    assert_eq!(0x0234_5678, bus.dma_channels[3].source);
    assert_eq!(0x4321, bus.dma_channels[3].word_count);
}

/// ZiGBA: "MMIO byte writes keep interrupt flag clearing lane-local"
#[test]
fn mmio_byte_writes_keep_interrupt_flag_clearing_lane_local() {
    let mut bus = Bus::init(&[]);
    bus.interrupt_request_flags = 0xa5a5;
    bus.update_irq_line();
    write_byte(&mut bus, 0x0400_0202, 0x05);
    assert_eq!(0xa5a0, bus.interrupt_request_flags);
    write_byte(&mut bus, 0x0400_0203, 0xa0);
    assert_eq!(0x05a0, bus.interrupt_request_flags);
}

/// ZiGBA: "DMA3 immediate starts two cycles after its enabling write, copies halfwords, clears enable, and requests IRQ"
#[test]
fn dma3_immediate_starts_two_cycles_after_its_enabling_write_copies_halfwords_clears_enable_and_requests_irq()
 {
    let mut bus = Bus::init(&[]);
    write_half(&mut bus, 0x0200_0000, 0x1122);
    write_half(&mut bus, 0x0200_0002, 0x3344);
    for (address, value) in [
        (0x0400_00d4, 0),
        (0x0400_00d6, 0x0200),
        (0x0400_00d8, 0),
        (0x0400_00da, 0x0300),
        (0x0400_00dc, 2),
    ] {
        write_half(&mut bus, address, value);
    }
    let enable_cycles = write_half(&mut bus, 0x0400_00de, (1 << 14) | (1 << 15));
    assert_eq!(1, enable_cycles);
    assert_eq!(0, u16::from_le_bytes(bus.iwram[0..2].try_into().unwrap()));
    assert_ne!(0, read_half(&mut bus, 0x0400_00de) & (1 << 15));
    let before_start = bus.now();
    bus.elapse(1);
    assert_eq!(11, bus.now() - before_start);
    assert_eq!(0x1122, read_half(&mut bus, 0x0300_0000));
    assert_eq!(0x3344, read_half(&mut bus, 0x0300_0002));
    assert_eq!(0, read_half(&mut bus, 0x0400_00de) & (1 << 15));
    assert_ne!(0, read_half(&mut bus, 0x0400_0202) & (1 << 11));
}

/// ZiGBA: "disabling immediate DMA before its startup deadline cancels the transfer"
#[test]
fn disabling_immediate_dma_before_its_startup_deadline_cancels_the_transfer() {
    let mut bus = Bus::init(&[]);
    write_half(&mut bus, 0x0300_0100, 0xcafe);
    for (address, value) in [
        (0x0400_00d4, 0x0100),
        (0x0400_00d6, 0x0300),
        (0x0400_00d8, 0x0200),
        (0x0400_00da, 0x0300),
        (0x0400_00dc, 1),
        (0x0400_00de, 1 << 15),
        (0x0400_00de, 0),
    ] {
        write_half(&mut bus, address, value);
    }
    bus.elapse(10);
    assert_eq!(0, read_half(&mut bus, 0x0300_0200));
}

/// ZiGBA: "DMA register high halves apply channel-specific address widths"
#[test]
fn dma_register_high_halves_apply_channel_specific_address_widths() {
    let mut bus = Bus::init(&[]);
    for (address, value) in [
        (0x0400_00b0, 0xffff),
        (0x0400_00b2, 0xffff),
        (0x0400_00be, 0xffff),
        (0x0400_00cc, 0xffff),
        (0x0400_00ce, 0xffff),
        (0x0400_00d8, 0xffff),
        (0x0400_00da, 0xffff),
    ] {
        write_half(&mut bus, address, value);
    }
    assert_eq!(0x07ff_ffff, bus.dma_channels[0].source);
    assert_eq!(0x0fff_0000, bus.dma_channels[1].source);
    assert_eq!(0x07ff_ffff, bus.dma_channels[2].destination);
    assert_eq!(0x0fff_ffff, bus.dma_channels[3].destination);
}

/// ZiGBA: "DMA aligns latched source and destination to its transfer width"
#[test]
fn dma_aligns_latched_source_and_destination_to_its_transfer_width() {
    let mut half = Bus::init(&[]);
    write_half(&mut half, 0x0300_0100, 0xbeef);
    for (address, value) in [
        (0x0400_00d4, 0x0101),
        (0x0400_00d6, 0x0300),
        (0x0400_00d8, 0x0203),
        (0x0400_00da, 0x0300),
        (0x0400_00dc, 1),
        (0x0400_00de, 1 << 15),
    ] {
        write_half(&mut half, address, value);
    }
    half.elapse(3);
    assert_eq!(0xbeef, read_half(&mut half, 0x0300_0202));
    let mut word = Bus::init(&[]);
    write_word(&mut word, 0x0300_0100, 0xa1b2_c3d4);
    for (address, value) in [
        (0x0400_00d4, 0x0103),
        (0x0400_00d6, 0x0300),
        (0x0400_00d8, 0x0207),
        (0x0400_00da, 0x0300),
        (0x0400_00dc, 1),
        (0x0400_00de, (1 << 10) | (1 << 15)),
    ] {
        write_half(&mut word, address, value);
    }
    word.elapse(3);
    assert_eq!(0xa1b2_c3d4, read_word(&mut word, 0x0300_0204));
}

/// ZiGBA: "Game Pak DMA sources increment despite decrement or fixed control"
#[test]
fn game_pak_dma_sources_increment_despite_decrement_or_fixed_control() {
    for source_control in [1, 2] {
        let mut bus = Bus::init(&[0x11, 0x11, 0x22, 0x22]);
        for (address, value) in [
            (0x0400_00d4, 0),
            (0x0400_00d6, 0x0800),
            (0x0400_00d8, 0x0300),
            (0x0400_00da, 0x0300),
            (0x0400_00dc, 2),
            (0x0400_00de, (source_control << 7) | (1 << 15)),
        ] {
            write_half(&mut bus, address, value);
        }
        bus.elapse(3);
        assert_eq!(0x1111, read_half(&mut bus, 0x0300_0300));
        assert_eq!(0x2222, read_half(&mut bus, 0x0300_0302));
    }
}

fn program_flash(bus: &mut Bus, offset: u16, value: u8) {
    write_byte(bus, 0x0e00_5555, 0xaa);
    write_byte(bus, 0x0e00_2aaa, 0x55);
    write_byte(bus, 0x0e00_5555, 0xa0);
    write_byte(bus, 0x0e00_0000 + u32::from(offset), value);
}

fn switch_flash_bank(bus: &mut Bus, bank: u8) {
    write_byte(bus, 0x0e00_5555, 0xaa);
    write_byte(bus, 0x0e00_2aaa, 0x55);
    write_byte(bus, 0x0e00_5555, 0xb0);
    write_byte(bus, 0x0e00_0000, bank);
}

/// ZiGBA: "runtime SRAM detection replays a broken Flash unlock prefix"
#[test]
fn runtime_sram_detection_replays_a_broken_flash_unlock_prefix() {
    let mut bus = Bus::init_with_save(&[], SaveType::Auto);
    write_byte(&mut bus, 0x0e00_5555, 0xaa);
    assert_eq!(SaveType::Auto, bus.save_type());
    write_byte(&mut bus, 0x0e00_1234, 0xa5);
    assert_eq!(SaveType::Sram, bus.save_type());
    assert_eq!(0xaa, read_byte(&mut bus, 0x0e00_5555));
    assert_eq!(0xa5, read_byte(&mut bus, 0x0e00_1234));

    let mut read_after_prefix = Bus::init_with_save(&[], SaveType::Auto);
    write_byte(&mut read_after_prefix, 0x0e00_5555, 0xaa);
    assert_eq!(0xaa, read_byte(&mut read_after_prefix, 0x0e00_5555));
    assert_eq!(SaveType::Sram, read_after_prefix.save_type());

    let mut invalid_command = Bus::init_with_save(&[], SaveType::Auto);
    write_byte(&mut invalid_command, 0x0e00_5555, 0xaa);
    write_byte(&mut invalid_command, 0x0e00_2aaa, 0x55);
    write_byte(&mut invalid_command, 0x0e00_5555, 0x10);
    assert_eq!(SaveType::Sram, invalid_command.save_type());
    assert_eq!(0x10, read_byte(&mut invalid_command, 0x0e00_5555));
    assert_eq!(0x55, read_byte(&mut invalid_command, 0x0e00_2aaa));
}

/// ZiGBA: "runtime save detection recognizes Flash commands and later bank promotion"
#[test]
fn runtime_save_detection_recognizes_flash_commands_and_later_bank_promotion() {
    let mut bus = Bus::init_with_save(&[], SaveType::Auto);
    write_byte(&mut bus, 0x0e00_5555, 0xaa);
    write_byte(&mut bus, 0x0e00_2aaa, 0x55);
    assert_eq!(SaveType::Auto, bus.save_type());
    write_byte(&mut bus, 0x0e00_5555, 0xa0);
    assert_eq!(SaveType::Flash64k, bus.save_type());
    write_byte(&mut bus, 0x0e00_0123, 0x5a);
    assert_eq!(0x5a, read_byte(&mut bus, 0x0e00_0123));
    assert_eq!(64 * 1024, bus.save_data().len());
    switch_flash_bank(&mut bus, 1);
    assert_eq!(SaveType::Flash128k, bus.save_type());
    assert_eq!(128 * 1024, bus.save_data().len());
    program_flash(&mut bus, 0x0123, 0xa5);
    assert_eq!(0xa5, read_byte(&mut bus, 0x0e00_0123));
    switch_flash_bank(&mut bus, 0);
    assert_eq!(0x5a, read_byte(&mut bus, 0x0e00_0123));
    let mut fixed = Bus::init_with_save(&[], SaveType::Flash64k);
    switch_flash_bank(&mut fixed, 1);
    assert_eq!(SaveType::Flash64k, fixed.save_type());
}

/// ZiGBA: "Flash sector erase resets only the addressed 4 KiB sector"
#[test]
fn flash_sector_erase_resets_only_the_addressed_4_kib_sector() {
    let mut bus = Bus::init_with_save(&[], SaveType::Flash64k);
    program_flash(&mut bus, 0x0123, 0x5a);
    program_flash(&mut bus, 0x1123, 0xa5);
    for (address, value) in [
        (0x0e00_5555, 0xaa),
        (0x0e00_2aaa, 0x55),
        (0x0e00_5555, 0x80),
        (0x0e00_5555, 0xaa),
        (0x0e00_2aaa, 0x55),
        (0x0e00_0000, 0x30),
    ] {
        write_byte(&mut bus, address, value);
    }
    let erased_at = bus.now();
    assert_eq!(0x5f, read_byte(&mut bus, 0x0e00_0123));
    assert_eq!(0xa5, read_byte(&mut bus, 0x0e00_1123));
    bus.elapse(FLASH_SECTOR_ERASE_CYCLES - (bus.now() - erased_at));
    assert_eq!(0xff, read_byte(&mut bus, 0x0e00_0123));
    assert_eq!(0xa5, read_byte(&mut bus, 0x0e00_1123));
}

/// ZiGBA: "Flash identification mode exposes device bytes until the exit command"
#[test]
fn flash_identification_mode_exposes_device_bytes_until_the_exit_command() {
    let mut bus = Bus::init_with_save(&[], SaveType::Flash64k);
    for (address, value) in [(0x0e00_5555, 0xaa), (0x0e00_2aaa, 0x55), (0x0e00_5555, 0x90)] {
        write_byte(&mut bus, address, value);
    }
    assert_eq!(0xc2, read_byte(&mut bus, 0x0e00_0000));
    assert_eq!(0x1c, read_byte(&mut bus, 0x0e00_0001));
    for (address, value) in [(0x0e00_5555, 0xaa), (0x0e00_2aaa, 0x55), (0x0e00_5555, 0xf0)] {
        write_byte(&mut bus, address, value);
    }
    assert_eq!(0xff, read_byte(&mut bus, 0x0e00_0000));
}

/// ZiGBA: "UART keeps transmit and receive data separate and clears errors on SIOCNT read"
#[test]
fn uart_keeps_transmit_and_receive_data_separate_and_clears_errors_on_siocnt_read() {
    let mut bus = Bus::init(&[]);
    write_half(&mut bus, 0x0400_0134, 0);
    write_half(&mut bus, 0x0400_0128, 0x3000);
    write_half(&mut bus, 0x0400_012a, 0x12ab);
    assert_eq!(0, read_half(&mut bus, 0x0400_012a));
    assert_eq!(0x3030, read_half(&mut bus, 0x0400_0128));
    bus.uart_error = true;
    assert_eq!(0x3070, read_half(&mut bus, 0x0400_0128));
    assert_eq!(0x3030, read_half(&mut bus, 0x0400_0128));
}

/// ZiGBA: "UART nominal frame deadlines follow baud, data-length, and parity selection"
#[test]
fn uart_nominal_frame_deadlines_follow_baud_data_length_and_parity_selection() {
    for (control, cycles) in [
        (0x3480, 17_477),
        (0x3481, 4_370),
        (0x3482, 2_913),
        (0x3483, 1_457),
        (0x3403, 1_311),
        (0x3683, 1_602),
    ] {
        let mut bus = Bus::init(&[]);
        write_half(&mut bus, 0x0400_0134, 0);
        write_half(&mut bus, 0x0400_0128, control);
        write_half(&mut bus, 0x0400_012a, 0x005a);
        assert_eq!(Some(bus.now() + cycles), bus.uart_tx_completion_cycle);
    }
}

/// ZiGBA: "normal-mode internal-clock SIO transfers complete at their bit-clock deadline"
#[test]
fn normal_mode_internal_clock_sio_transfers_complete_at_their_bit_clock_deadline() {
    for (control, cycles, normal32) in [
        (0x4081, 8 * 64, false),
        (0x4083, 8 * 8, false),
        (0x5081, 32 * 64, true),
        (0x5083, 32 * 8, true),
    ] {
        let mut bus = Bus::init(&[]);
        write_half(&mut bus, 0x0400_0134, 0);
        write_half(&mut bus, 0x0400_0128, control & !0x0080);
        if normal32 {
            write_word(&mut bus, 0x0400_0120, 0x1234_5678);
        } else {
            write_half(&mut bus, 0x0400_012a, 0x1200);
        }
        write_half(&mut bus, 0x0400_0128, control);
        bus.elapse(cycles - 2);
        assert_ne!(0, read_half(&mut bus, 0x0400_0128) & 0x0080);
        assert_eq!(0, bus.interrupt_request_flags & (1 << 7));
        bus.elapse(1);
        assert_eq!(0, read_half(&mut bus, 0x0400_0128) & 0x0080);
        assert_eq!(1 << 7, bus.interrupt_request_flags & (1 << 7));
        if normal32 {
            assert_eq!(0xffff_ffff, read_word(&mut bus, 0x0400_0120));
        } else {
            assert_eq!(0x12ff, read_half(&mut bus, 0x0400_012a));
        }
    }
}

/// ZiGBA: "external-clock, GPIO and JoyBus selections leave a started SIO transfer pending"
#[test]
fn external_clock_gpio_and_joy_bus_selections_leave_a_started_sio_transfer_pending() {
    for (rcnt, siocnt) in [(0, 0x4080), (0x8000, 0x4081), (0xc000, 0x4081)] {
        let mut bus = Bus::init(&[]);
        write_half(&mut bus, 0x0400_0134, rcnt);
        write_half(&mut bus, 0x0400_0128, siocnt);
        bus.elapse(4096);
        assert_ne!(0, read_half(&mut bus, 0x0400_0128) & 0x0080);
        assert_eq!(0, bus.interrupt_request_flags & (1 << 7));
    }
}

/// ZiGBA: "JoyBus status and reset commands return the GBA type and raise reset IRQ"
#[test]
fn joy_bus_status_and_reset_commands_return_the_gba_type_and_raise_reset_irq() {
    let mut bus = Bus::init(&[]);
    assert_eq!(
        Err(JoyBusError::NotInJoyBusMode),
        bus.schedule_joy_bus(JoyBusCommand::Status, bus.now())
    );
    write_half(&mut bus, 0x0400_0134, 0xc000);
    write_half(&mut bus, 0x0400_0140, 0x0040);
    bus.schedule_joy_bus(JoyBusCommand::Status, bus.now()).unwrap();
    let status = bus.take_joy_bus_response().unwrap();
    assert_eq!(3, status.len);
    assert_eq!(&[0x00, 0x04, 0x00], &status.data[..status.len as usize]);
    assert_eq!(0, bus.interrupt_request_flags & (1 << 7));
    write_half(&mut bus, 0x0400_0200, 1 << 7);
    let deadline = bus.now() + 37;
    bus.schedule_joy_bus(JoyBusCommand::Reset, deadline).unwrap();
    assert_eq!(None, bus.take_joy_bus_response());
    bus.enter_halt();
    assert!(bus.idle_halt_until(deadline));
    let reset = bus.take_joy_bus_response().unwrap();
    assert_eq!(deadline, bus.now());
    assert_eq!(&[0x00, 0x04, 0x00], &reset.data[..reset.len as usize]);
    assert_eq!(0x0041, read_half(&mut bus, 0x0400_0140));
    assert_eq!(1 << 7, bus.interrupt_request_flags & (1 << 7));
    assert!(bus.halt_wake_pending());
    write_half(&mut bus, 0x0400_0140, 0x0041);
    assert_eq!(0x0040, read_half(&mut bus, 0x0400_0140));
}

/// ZiGBA: "JoyBus write command fills receive data and MMIO reads clear receive status"
#[test]
fn joy_bus_write_command_fills_receive_data_and_mmio_reads_clear_receive_status() {
    let mut bus = Bus::init(&[]);
    write_half(&mut bus, 0x0400_0134, 0xc000);
    write_half(&mut bus, 0x0400_0140, 0x0040);
    bus.schedule_joy_bus(JoyBusCommand::WriteToGba(0x4433_2211), bus.now()).unwrap();
    let response = bus.take_joy_bus_response().unwrap();
    assert_eq!(1, response.len);
    assert_eq!(0x02, response.data[0]);
    assert_eq!(0x0042, read_half(&mut bus, 0x0400_0140));
    assert_eq!(0x0002, read_half(&mut bus, 0x0400_0158));
    assert_eq!(0x2211, read_half(&mut bus, 0x0400_0150));
    assert_eq!(0, read_half(&mut bus, 0x0400_0158) & 0x0002);
    assert_eq!(0x4433, read_half(&mut bus, 0x0400_0152));
    assert_eq!(1 << 7, bus.interrupt_request_flags & (1 << 7));
}

/// ZiGBA: "JoyBus rejects invalid scheduling and mode exit cancels a pending command"
#[test]
fn joy_bus_rejects_invalid_scheduling_and_mode_exit_cancels_a_pending_command() {
    let mut bus = Bus::init(&[]);
    write_half(&mut bus, 0x0400_0134, 0xc000);
    assert_eq!(
        Err(JoyBusError::CompletionBeforeCurrentCycle),
        bus.schedule_joy_bus(JoyBusCommand::Status, 0)
    );
    let deadline = bus.now() + 20;
    bus.schedule_joy_bus(JoyBusCommand::Status, deadline).unwrap();
    assert_eq!(
        Err(JoyBusError::TransactionPending),
        bus.schedule_joy_bus(JoyBusCommand::Reset, deadline)
    );
    write_half(&mut bus, 0x0400_0134, 0x8000);
    bus.elapse(32);
    assert_eq!(None, bus.take_joy_bus_response());
}

/// ZiGBA: "serial IRQ wakes HALT at the exact normal-transfer deadline"
#[test]
fn serial_irq_wakes_halt_at_the_exact_normal_transfer_deadline() {
    let mut bus = Bus::init(&[]);
    write_half(&mut bus, 0x0400_0200, 1 << 7);
    write_half(&mut bus, 0x0400_0134, 0);
    write_half(&mut bus, 0x0400_0128, 0x4081);
    let started_at = bus.now();
    bus.enter_halt();
    assert!(bus.idle_halt_until(started_at + 8 * 64));
    assert_eq!(started_at + 8 * 64, bus.now());
    assert_eq!(1 << 7, bus.interrupt_request_flags & (1 << 7));
}

/// ZiGBA: "APU byte writes preserve write-only frequency lanes"
#[test]
fn apu_byte_writes_preserve_write_only_frequency_lanes() {
    let mut bus = Bus::init(&[]);
    for (addr, value) in
        [(0x0400_0084, 0x80), (0x0400_0069, 0xf0), (0x0400_006c, 0x34), (0x0400_006d, 0x02)]
    {
        write_byte(&mut bus, addr, value);
    }
    assert_eq!(0x0234, bus.apu.square2.frequency);
    write_byte(&mut bus, 0x0400_006c, 0x56);
    assert_eq!(0x0256, bus.apu.square2.frequency);
    write_byte(&mut bus, 0x0400_006d, 0x87);
    assert_eq!(0x0756, bus.apu.square2.frequency);
    assert!(bus.apu.square2.enabled);
}

/// ZiGBA: "PSG registers ignore writes while master sound is disabled"
#[test]
fn psg_registers_ignore_writes_while_master_sound_is_disabled() {
    let mut bus = Bus::init(&[]);
    write_half(&mut bus, 0x0400_0068, 0xf080);
    write_half(&mut bus, 0x0400_006c, 0x8700);
    assert_eq!(0, read_half(&mut bus, 0x0400_0068));
    assert_eq!(0, read_half(&mut bus, 0x0400_006c));
    assert_eq!(0, read_half(&mut bus, 0x0400_0084));
    write_half(&mut bus, 0x0400_0084, 0x0080);
    write_half(&mut bus, 0x0400_0068, 0xf080);
    write_half(&mut bus, 0x0400_006c, 0x8700);
    assert_eq!(0xf080, read_half(&mut bus, 0x0400_0068));
    assert_eq!(0x0082, read_half(&mut bus, 0x0400_0084));
    write_half(&mut bus, 0x0400_0084, 0);
    assert_eq!(0, read_half(&mut bus, 0x0400_0068));
    assert_eq!(0, read_half(&mut bus, 0x0400_0084));
}

/// ZiGBA: "Direct Sound control and bias expose only hardware-backed bits"
#[test]
fn direct_sound_control_and_bias_expose_only_hardware_backed_bits() {
    let mut bus = Bus::init(&[]);
    assert_eq!(0, read_half(&mut bus, 0x0400_0082));
    assert_eq!(0x0200, read_half(&mut bus, 0x0400_0088));
    write_half(&mut bus, 0x0400_0082, 0xffff);
    write_half(&mut bus, 0x0400_0088, 0xffff);
    assert_eq!(0x770f, read_half(&mut bus, 0x0400_0082));
    assert_eq!(0xc3fe, read_half(&mut bus, 0x0400_0088));
}

/// ZiGBA: "FIFO reset clears queued words but preserves the active playback word"
#[test]
fn fifo_reset_clears_queued_words_but_preserves_the_active_playback_word() {
    let mut bus = Bus::init(&[]);
    let routing = (1 << 2) | (1 << 8) | (1 << 9);
    write_half(&mut bus, 0x0400_0084, 1 << 7);
    write_half(&mut bus, 0x0400_0082, routing);
    write_word(&mut bus, 0x0400_00a0, 0x0403_0201);
    write_half(&mut bus, 0x0400_0100, 0xffff);
    write_half(&mut bus, 0x0400_0102, 1 << 7);
    bus.elapse(1);
    assert_eq!(4, bus.current_audio_sample().left);
    write_half(&mut bus, 0x0400_0082, routing | (1 << 11));
    assert_eq!(8, bus.current_audio_sample().left);
    bus.elapse(1);
    assert_eq!(12, bus.current_audio_sample().left);
}

/// ZiGBA: "Direct Sound FIFO does not advance while master sound is disabled"
#[test]
fn direct_sound_fifo_does_not_advance_while_master_sound_is_disabled() {
    let mut bus = Bus::init(&[]);
    let routing = (1 << 2) | (1 << 8) | (1 << 9);
    write_half(&mut bus, 0x0400_0082, routing);
    write_word(&mut bus, 0x0400_00a0, 0x0403_0201);
    write_half(&mut bus, 0x0400_0100, 0xffff);
    write_half(&mut bus, 0x0400_0102, 1 << 7);
    bus.elapse(1);
    write_half(&mut bus, 0x0400_0084, 1 << 7);
    bus.elapse(1);
    assert_eq!(4, bus.current_audio_sample().left);
}

/// ZiGBA: "FIFO halfword write commits a whole word slot with untouched lanes"
#[test]
fn fifo_halfword_write_commits_a_whole_word_slot_with_untouched_lanes() {
    let mut bus = Bus::init(&[]);
    write_half(&mut bus, 0x0400_0084, 1 << 7);
    write_half(&mut bus, 0x0400_0082, (1 << 2) | (1 << 9));
    write_half(&mut bus, 0x0400_00a0, 0x0201);
    write_half(&mut bus, 0x0400_0100, 0xffff);
    write_half(&mut bus, 0x0400_0102, 1 << 7);
    for sample in [4, 8, 0, 0] {
        bus.elapse(1);
        assert_eq!(sample, bus.current_audio_sample().left);
    }
}

/// ZiGBA: "mixed audio is biased and clipped to the ten-bit DAC range"
#[test]
fn mixed_audio_is_biased_and_clipped_to_the_ten_bit_dac_range() {
    let mut bus = Bus::init(&[]);
    write_half(&mut bus, 0x0400_0084, 1 << 7);
    write_half(&mut bus, 0x0400_0082, (1 << 2) | (1 << 3) | (1 << 9) | (1 << 13));
    for fifo in [0x0400_00a0, 0x0400_00a4] {
        write_word(&mut bus, fifo, 0x7f7f_7f7f);
    }
    write_half(&mut bus, 0x0400_0100, 0xffff);
    write_half(&mut bus, 0x0400_0102, 1 << 7);
    bus.elapse(1);
    assert_eq!(510, bus.current_audio_sample().left);
    assert_eq!(0, bus.current_audio_sample().right);
}

/// ZiGBA: "SOUNDBIAS resolution truncates the clipped DAC code to PWM precision"
#[test]
fn soundbias_resolution_truncates_the_clipped_dac_code_to_pwm_precision() {
    let mut bus = Bus::init(&[]);
    write_half(&mut bus, 0x0400_0084, 1 << 7);
    write_half(&mut bus, 0x0400_0082, (1 << 2) | (1 << 9));
    write_word(&mut bus, 0x0400_00a0, 0x0303_0303);
    write_half(&mut bus, 0x0400_0100, 0xffff);
    write_half(&mut bus, 0x0400_0102, 1 << 7);
    bus.elapse(1);
    for (resolution, sample) in [12, 12, 8, 0].into_iter().enumerate() {
        write_half(&mut bus, 0x0400_0088, 0x0200 | (resolution as u16) << 14);
        assert_eq!(sample, bus.current_audio_sample().left);
    }
}

/// ZiGBA: "audio samples are produced deterministically at the default 32768 Hz cadence"
#[test]
fn audio_samples_are_produced_deterministically_at_the_default_32768_hz_cadence() {
    let mut bus = Bus::init(&[]);
    assert_eq!(32_768, bus.audio_sample_rate());
    write_half(&mut bus, 0x0400_0084, 1 << 7);
    write_half(&mut bus, 0x0400_0082, (1 << 2) | (1 << 8) | (1 << 9));
    write_word(&mut bus, 0x0400_00a0, 0x0101_0101);
    write_half(&mut bus, 0x0400_0100, 0xffff);
    write_half(&mut bus, 0x0400_0102, 1 << 7);
    bus.elapse(1);
    write_half(&mut bus, 0x0400_0102, 0);
    let mut samples = [Default::default(); 2];
    assert_eq!(0, bus.drain_audio_samples(&mut samples));
    let until_sample = 512 - (bus.now() % 512);
    bus.elapse(until_sample - 1);
    assert_eq!(0, bus.drain_audio_samples(&mut samples));
    bus.elapse(1);
    assert_eq!(1, bus.drain_audio_samples(&mut samples));
    assert_eq!(4, samples[0].left);
    assert_eq!(4, samples[0].right);
}

/// ZiGBA: "UART FIFOs preserve four bytes and flag overrun on the fifth arrival"
#[test]
fn uart_fifos_preserve_four_bytes_and_flag_overrun_on_the_fifth_arrival() {
    let mut sender = Bus::init(&[]);
    let mut receiver = Bus::init(&[]);
    write_half(&mut sender, 0x0400_0134, 0);
    write_half(&mut receiver, 0x0400_0134, 0);
    write_half(&mut sender, 0x0400_0128, 0x3583);
    write_half(&mut receiver, 0x0400_0128, 0x3983);
    for value in [1, 2, 3, 4, 5] {
        write_half(&mut sender, 0x0400_012a, value);
    }
    for _ in 0..5 {
        assert_eq!(
            Ok(true),
            synchronize_sio(&mut [&mut sender, &mut receiver], SioConnection::Cable2p)
        );
        let deadline = sender.uart_tx_completion_cycle.unwrap();
        sender.elapse(deadline - sender.now());
        receiver.elapse(deadline - receiver.now());
    }
    assert_eq!(0x39c3, read_half(&mut receiver, 0x0400_0128));
    for expected in [1, 2, 3, 4] {
        assert_eq!(expected, read_half(&mut receiver, 0x0400_012a));
    }
    assert_eq!(0, read_half(&mut receiver, 0x0400_012a));
}

/// ZiGBA: "UART overrun requests the serial IRQ and FIFO disable discards received bytes"
#[test]
fn uart_overrun_requests_the_serial_irq_and_fifo_disable_discards_received_bytes() {
    let mut sender = Bus::init(&[]);
    let mut receiver = Bus::init(&[]);
    write_half(&mut sender, 0x0400_0134, 0);
    write_half(&mut receiver, 0x0400_0134, 0);
    write_half(&mut sender, 0x0400_0128, 0x3583);
    write_half(&mut receiver, 0x0400_0128, 0x7983);
    for value in [1, 2, 3, 4, 5] {
        write_half(&mut sender, 0x0400_012a, value);
    }
    for _ in 0..5 {
        assert_eq!(
            Ok(true),
            synchronize_sio(&mut [&mut sender, &mut receiver], SioConnection::Cable2p)
        );
        let deadline = sender.uart_tx_completion_cycle.unwrap();
        sender.elapse(deadline - sender.now());
        receiver.elapse(deadline - receiver.now());
    }
    assert_ne!(0, read_half(&mut receiver, 0x0400_0202) & (1 << 7));
    write_half(&mut receiver, 0x0400_0128, 0x7883);
    assert_eq!(0, read_half(&mut receiver, 0x0400_012a));
    assert_ne!(0, read_half(&mut receiver, 0x0400_0128) & 0x0020);
}

/// ZiGBA: "UART CTS waits until the peer receiver has capacity"
#[test]
fn uart_cts_waits_until_the_peer_receiver_has_capacity() {
    let mut sender = Bus::init(&[]);
    let mut receiver = Bus::init(&[]);
    write_half(&mut sender, 0x0400_0134, 0);
    write_half(&mut receiver, 0x0400_0134, 0);
    write_half(&mut sender, 0x0400_0128, 0x3487);
    write_half(&mut receiver, 0x0400_0128, 0x3083);
    write_half(&mut sender, 0x0400_012a, 0x003c);
    assert_eq!(None, sender.uart_tx_completion_cycle);
    assert_eq!(
        Ok(false),
        synchronize_sio(&mut [&mut sender, &mut receiver], SioConnection::Cable2p)
    );
    write_half(&mut receiver, 0x0400_0128, 0x3883);
    assert_eq!(
        Ok(true),
        synchronize_sio(&mut [&mut sender, &mut receiver], SioConnection::Cable2p)
    );
    assert!(sender.uart_tx_completion_cycle.is_some());
}

/// ZiGBA: "normal32 SIO peers shift words through the multiplayer cable"
#[test]
fn normal32_sio_peers_shift_words_through_the_multiplayer_cable() {
    let mut master = Bus::init(&[]);
    let mut slave = Bus::init(&[]);
    for bus in [&mut master, &mut slave] {
        write_half(bus, 0x0400_0134, 0);
        write_half(bus, 0x0400_0128, 0x1000);
    }
    write_word(&mut master, 0x0400_0120, 0x1234_5678);
    write_word(&mut slave, 0x0400_0120, 0xcafe_babe);
    write_half(&mut master, 0x0400_0128, 0x5083);
    write_half(&mut slave, 0x0400_0128, 0x5080);
    assert_eq!(
        Ok(true),
        synchronize_sio(&mut [&mut master, &mut slave], SioConnection::CableMulti)
    );
    master.elapse(32 * 8);
    slave.elapse(32 * 8);
    assert_eq!(0, read_word(&mut master, 0x0400_0120));
    assert_eq!(0x1234_5678, read_word(&mut slave, 0x0400_0120));
}

/// ZiGBA: "SIO synchronization waits for both normal peers and rejects one past the deadline"
#[test]
fn sio_synchronization_waits_for_both_normal_peers_and_rejects_one_past_the_deadline() {
    let mut master = Bus::init(&[]);
    let mut slave = Bus::init(&[]);
    write_half(&mut master, 0x0400_0134, 0);
    write_half(&mut slave, 0x0400_0134, 0);
    write_half(&mut master, 0x0400_0128, 0x0083);
    assert_eq!(Ok(false), synchronize_sio(&mut [&mut master, &mut slave], SioConnection::Cable2p));
    write_half(&mut slave, 0x0400_0128, 0x0080);
    slave.elapse(65);
    assert_eq!(
        Err(SioLinkError::MissedDeadline),
        synchronize_sio(&mut [&mut master, &mut slave], SioConnection::Cable2p)
    );
}

/// ZiGBA: "four-player SIO exchanges every multiplayer word at the parent baud deadline"
#[test]
fn four_player_sio_exchanges_every_multiplayer_word_at_the_parent_baud_deadline() {
    let mut buses = [Bus::init(&[]), Bus::init(&[]), Bus::init(&[]), Bus::init(&[])];
    let sent = [0x0123, 0x4567, 0x89ab, 0xcdef];
    for (index, (bus, value)) in buses.iter_mut().zip(sent).enumerate() {
        write_half(bus, 0x0400_0134, 0);
        write_half(bus, 0x0400_0128, 0x2003);
        write_half(bus, 0x0400_012a, value);
        write_half(bus, 0x0400_0128, if index == 0 { 0x6083 } else { 0x6003 });
    }
    let [a, b, c, d] = &mut buses;
    assert_eq!(Ok(true), synchronize_sio(&mut [a, b, c, d], SioConnection::CableMulti));
    for (index, bus) in buses.iter().enumerate() {
        assert_ne!(0, bus.sio_control & 0x0080);
        assert_eq!(index != 0, bus.sio_control & 0x0004 != 0);
        assert_ne!(0, bus.sio_control & 0x0008);
        assert_eq!([0xffff; 4], bus.sio_data);
    }
    for bus in &mut buses {
        bus.elapse(11_863 - 1);
    }
    for bus in &buses {
        assert_ne!(0, bus.sio_control & 0x0080);
    }
    for bus in &mut buses {
        bus.elapse(1);
    }
    for (index, bus) in buses.iter().enumerate() {
        assert_eq!(sent, bus.sio_data);
        assert_eq!(0, bus.sio_control & 0x0080);
        assert_eq!((index as u16) << 4, bus.sio_control & 0x0030);
        assert_eq!(1 << 7, bus.interrupt_request_flags & (1 << 7));
    }
    let [a, b, c, d] = &mut buses;
    assert_eq!(Ok(false), synchronize_sio(&mut [a, b, c, d], SioConnection::CableMulti));
    for (index, bus) in buses.iter().enumerate() {
        assert_eq!((index as u16) << 4, bus.sio_control & 0x0030);
    }
}

/// ZiGBA: "multiplayer SIO leaves absent receive slots disconnected"
#[test]
fn multiplayer_sio_leaves_absent_receive_slots_disconnected() {
    let mut parent = Bus::init(&[]);
    let mut child = Bus::init(&[]);
    for bus in [&mut parent, &mut child] {
        write_half(bus, 0x0400_0134, 0);
        write_half(bus, 0x0400_0128, 0x2000);
    }
    write_half(&mut parent, 0x0400_012a, 0x1111);
    write_half(&mut child, 0x0400_012a, 0x2222);
    write_half(&mut parent, 0x0400_0128, 0x2080);
    write_half(&mut child, 0x0400_0128, 0x2000);
    assert_eq!(
        Ok(true),
        synchronize_sio(&mut [&mut parent, &mut child], SioConnection::CableMulti)
    );
    parent.elapse(73_003);
    child.elapse(73_003);
    let expected = [0x1111, 0x2222, 0xffff, 0xffff];
    assert_eq!(expected, parent.sio_data);
    assert_eq!(expected, child.sio_data);
}

/// ZiGBA: "multiplayer SIO deadlines match the pinned mGBA 2-4 player timing table"
#[test]
fn multiplayer_sio_deadlines_match_the_pinned_mgba_2_4_player_timing_table() {
    let durations = [
        [73_003, 107_680, 142_356],
        [18_251, 26_920, 35_589],
        [12_167, 17_947, 23_726],
        [6_075, 8_973, 11_863],
    ];
    for (baud, row) in durations.iter().enumerate() {
        for (peer_count, duration) in (2..=4).zip(row) {
            let mut buses = [Bus::init(&[]), Bus::init(&[]), Bus::init(&[]), Bus::init(&[])];
            for (index, bus) in buses.iter_mut().take(peer_count).enumerate() {
                write_half(bus, 0x0400_0134, 0);
                write_half(bus, 0x0400_0128, 0x2000 | baud as u16);
                write_half(bus, 0x0400_012a, index as u16);
                write_half(
                    bus,
                    0x0400_0128,
                    0x2000 | baud as u16 | if index == 0 { 0x0080 } else { 0 },
                );
            }
            let started_at = buses[0].now();
            let [a, b, c, d] = &mut buses;
            let mut peers: [&mut Bus; 4] = [a, b, c, d];
            assert_eq!(
                Ok(true),
                synchronize_sio(&mut peers[..peer_count], SioConnection::CableMulti)
            );
            for bus in buses.iter().take(peer_count) {
                assert_eq!(Some(started_at + duration), bus.sio_completion_cycle);
            }
        }
    }
}

/// ZiGBA: "SIO register masks and disconnected line states match the pinned mGBA suite"
#[test]
fn sio_register_masks_and_disconnected_line_states_match_the_pinned_mgba_suite() {
    let addresses = [
        0x0400_0120,
        0x0400_0122,
        0x0400_0124,
        0x0400_0126,
        0x0400_0128,
        0x0400_012a,
        0x0400_0134,
        0x0400_0136,
        0x0400_0140,
        0x0400_0142,
        0x0400_0150,
        0x0400_0152,
        0x0400_0154,
        0x0400_0156,
        0x0400_015a,
    ];
    let modes = [
        (0x2000, 0x0000, [0, 0, 0, 0, 0x6f8f, 0xffff, 0x01ff, 0, 0x0040, 0, 0, 0, 0, 0, 0]),
        (0x0000, 0x0000, [0, 0, 0, 0, 0x4f8f, 0xffff, 0x01f5, 0, 0x0040, 0, 0, 0, 0, 0, 0]),
        (
            0x1000,
            0x0000,
            [0xffff, 0xffff, 0, 0, 0x5f8f, 0xffff, 0x01f5, 0, 0x0040, 0, 0, 0, 0, 0, 0],
        ),
        (0x3000, 0x0000, [0, 0, 0, 0, 0x7faf, 0, 0x01ff, 0, 0x0040, 0, 0, 0, 0, 0, 0]),
        (0x0000, 0x8000, [0, 0, 0, 0, 0x4f8f, 0xffff, 0x81ff, 0, 0x0040, 0, 0, 0, 0, 0, 0]),
        (0x0000, 0xc000, [0, 0, 0, 0, 0x4f8f, 0xffff, 0xc1fc, 0, 0x0040, 0, 0, 0, 0, 0, 0]),
    ];
    for (siocnt, rcnt, expected) in modes {
        for (address, value) in addresses.into_iter().zip(expected) {
            let mut bus = Bus::init(&[]);
            write_half(&mut bus, 0x0400_0134, rcnt);
            write_half(&mut bus, 0x0400_0128, siocnt);
            let written = if address == 0x0400_0128 {
                (read_half(&mut bus, address) & 0x3000) | 0xcfff
            } else if address == 0x0400_0134 {
                (read_half(&mut bus, address) & 0xc000) | 0x3fff
            } else {
                0xffff
            };
            write_half(&mut bus, address, written);
            assert_eq!(
                value,
                read_half(&mut bus, address),
                "SIOCNT={siocnt:04x}, RCNT={rcnt:04x}, addr={address:08x}"
            );
        }
    }
}

/// ZiGBA: "cartridge GPIO preserves ROM fallback and masks mirrored port registers"
#[test]
fn cartridge_gpio_preserves_rom_fallback_and_masks_mirrored_port_registers() {
    let mut rom = [0; 0xca];
    rom[0xc4..0xca].copy_from_slice(&[0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]);
    let mut bus = Bus::init(&rom);
    bus.enable_rtc(981_205_445);
    assert_eq!(0xbbaa, read_half(&mut bus, 0x0800_00c4));
    assert_eq!(0xddcc, read_half(&mut bus, 0x0a00_00c6));
    assert_eq!(0xffee, read_half(&mut bus, 0x0c00_00c8));
    assert_eq!(5, write_half(&mut bus, 0x0800_00c8, 0xffff));
    write_half(&mut bus, 0x0a00_00c6, 0xffff);
    write_half(&mut bus, 0x0c00_00c4, 0xffff);
    assert_eq!(1, read_half(&mut bus, 0x0800_00c8));
    assert_eq!(0x000f, read_half(&mut bus, 0x0a00_00c6));
    assert_eq!(0x000f, read_half(&mut bus, 0x0c00_00c4));
    write_byte(&mut bus, 0x0800_00c4, 0);
    assert_eq!(0x000f, read_half(&mut bus, 0x0800_00c4));
}

/// ZiGBA: "ROM word reads retain GPIO overlap, short media tails and window wrapping"
#[test]
fn rom_word_reads_retain_gpio_overlap_short_media_tails_and_window_wrapping() {
    let mut rom = [0; 0xcd];
    rom[0..2].copy_from_slice(&[0x12, 0x34]);
    rom[0xc3] = 0x11;
    rom[0xcc] = 0x44;
    let mut bus = Bus::init(&rom);
    bus.enable_rtc(0);
    write_half(&mut bus, 0x0800_00c8, 1);
    write_half(&mut bus, 0x0800_00c6, 15);
    write_half(&mut bus, 0x0800_00c4, 15);
    for (address, expected) in [
        (0x0800_0000, 0x3412),
        (0x0a00_00c3, 0x0f00_0f11),
        (0x0c00_00cc, 0x0067_0044),
        (0x09ff_fffe, 0x3412_ffff),
    ] {
        assert_eq!(
            expected,
            bus.read(access(
                address,
                AccessWidth::Word,
                AccessKind::Nonsequential,
                AccessSource::CpuCode
            ))
            .value
        );
    }
}

/// ZiGBA: "PSG channel one sweep disables after its post-update overflow probe"
#[test]
fn psg_channel_one_sweep_disables_after_its_post_update_overflow_probe() {
    let mut bus = Bus::init(&[]);
    write_half(&mut bus, 0x0400_0084, 1 << 7);
    let enabled_at = bus.now();
    write_half(&mut bus, 0x0400_0060, 0x0011);
    write_half(&mut bus, 0x0400_0062, 0xf040);
    write_half(&mut bus, 0x0400_0064, 0x8400);
    assert_eq!(0x0081, read_half(&mut bus, 0x0400_0084));
    let until_sweep = 3 * 32_768 - (bus.now() - enabled_at);
    bus.elapse(until_sweep - 1);
    assert_eq!(0x0081, read_half(&mut bus, 0x0400_0084));
    assert_eq!(0x0080, read_half(&mut bus, 0x0400_0084));
}

/// ZiGBA: "PSG frame sequencer clocks square channel two length at 256 Hz"
#[test]
fn psg_frame_sequencer_clocks_square_channel_two_length_at_256_hz() {
    let mut bus = Bus::init(&[]);
    write_half(&mut bus, 0x0400_0084, 1 << 7);
    let enabled_at = bus.now();
    write_half(&mut bus, 0x0400_0068, 0xf07f);
    write_half(&mut bus, 0x0400_006c, (1 << 15) | (1 << 14));
    assert_eq!(0x0082, read_half(&mut bus, 0x0400_0084));
    let until_clock = 32_768 - (bus.now() - enabled_at);
    bus.elapse(until_clock - 1);
    assert_eq!(0x0082, read_half(&mut bus, 0x0400_0084));
    assert_eq!(0x0080, read_half(&mut bus, 0x0400_0084));
}

/// ZiGBA: "PSG noise channel clocks its LFSR and routes the envelope volume"
#[test]
fn psg_noise_channel_clocks_its_lfsr_and_routes_the_envelope_volume() {
    let mut bus = Bus::init(&[]);
    write_half(&mut bus, 0x0400_0084, 1 << 7);
    write_half(&mut bus, 0x0400_0080, 0x8877);
    write_half(&mut bus, 0x0400_0082, 2);
    write_half(&mut bus, 0x0400_0078, 0xf000);
    write_half(&mut bus, 0x0400_007c, 1 << 15);
    assert_eq!(0x0088, read_half(&mut bus, 0x0400_0084));
    assert_eq!(0, bus.current_audio_sample().left);
    bus.elapse(32);
    assert_eq!(0, bus.current_audio_sample().left);
    assert_eq!(0, bus.current_audio_sample().right);
    bus.elapse(448);
    assert_eq!(240, bus.current_audio_sample().left);
    assert_eq!(240, bus.current_audio_sample().right);
}

/// ZiGBA: "timer zero unused count-up bit preserves the exact batched IRQ deadline"
#[test]
fn timer_zero_unused_count_up_bit_preserves_the_exact_batched_irq_deadline() {
    for direct_state in [false, true] {
        let mut bus = Bus::init(&[]);
        write_half(&mut bus, 0x0400_0200, 1 << 3);
        write_half(&mut bus, 0x0400_0208, 1);
        if direct_state {
            bus.timers[0].reload = 0xfffe;
            bus.timers[0].counter = 0xfffe;
            bus.timers[0].control = 0x00c4;
        } else {
            write_word(&mut bus, 0x0400_0100, 0x00c4_fffe);
        }
        let started_at = bus.now();
        bus.begin_device_batch();
        bus.elapse(1);
        assert_eq!(0, bus.interrupt_request_flags);
        assert!(!bus.irq_line);
        bus.elapse(1);
        assert_eq!(started_at + 2, bus.now());
        assert_eq!(1 << 3, bus.interrupt_request_flags);
        assert!(bus.irq_line);
        bus.end_device_batch();
    }
}

fn expect_timer_irqs_at(bus: &mut Bus, at: u64, flags: u16) {
    let timer_flags = 0xf << 3;
    bus.elapse(at - 1 - bus.now());
    assert_eq!(0, bus.interrupt_request_flags & timer_flags);
    bus.elapse(1);
    assert_eq!(flags, bus.interrupt_request_flags & timer_flags);
    bus.interrupt_request_flags &= !timer_flags;
    bus.update_irq_line();
}

fn expect_timer_overflows_at(bus: &mut Bus, index: u8, expected: u64, divisor: u64) {
    let flag = 1u16 << (index + 3);
    for distance in [expected, 2 * divisor] {
        assert!(distance >= 1);
        bus.elapse(distance - 1);
        assert_eq!(0, bus.interrupt_request_flags & flag);
        bus.elapse(1);
        assert_eq!(flag, bus.interrupt_request_flags & flag);
        bus.interrupt_request_flags = 0;
        bus.update_irq_line();
    }
}

/// ZiGBA: "prescaled timers tick on one free-running prescaler grid"
#[test]
fn prescaled_timers_tick_on_one_free_running_prescaler_grid() {
    for batch in [false, true] {
        let mut bus = Bus::init(&[]);
        write_half(&mut bus, 0x0400_0200, 0xf << 3);
        if batch {
            bus.begin_device_batch();
        }
        write_word(&mut bus, 0x0400_0100, 0x00c1_ffff);
        bus.elapse(38);
        write_word(&mut bus, 0x0400_0104, 0x00c1_ffff);
        write_word(&mut bus, 0x0400_0108, 0x00c2_ffff);
        write_word(&mut bus, 0x0400_010c, 0x00c3_ffff);
        assert_eq!(43, bus.now());
        for tick in 1..17 {
            let at = 64 * tick;
            let mut flags = 0b0011 << 3;
            if at % 256 == 0 {
                flags |= 1 << 5;
            }
            if at % 1024 == 0 {
                flags |= 1 << 6;
            }
            expect_timer_irqs_at(&mut bus, at, flags);
        }
        if batch {
            bus.end_device_batch();
        }
    }
}

/// ZiGBA: "a /64 enable after a /1024 overflow follows the free-running prescaler"
#[test]
fn a_64_enable_after_a_1024_overflow_follows_the_free_running_prescaler() {
    for (delay, overflow) in [(0, 2176), (5, 2176), (15, 2240)] {
        for batch in [false, true] {
            let mut bus = Bus::init(&[]);
            write_half(&mut bus, 0x0400_0200, 1 << 3);
            if batch {
                bus.begin_device_batch();
            }
            write_word(&mut bus, 0x0400_0100, 0x00c3_ffff);
            expect_timer_irqs_at(&mut bus, 1024, 1 << 3);
            write_half(&mut bus, 0x0400_0102, 0);
            write_half(&mut bus, 0x0400_0100, 0xfff0);
            let enable_at = 1024 + 182 + delay;
            bus.elapse(enable_at - 1 - bus.now());
            write_half(&mut bus, 0x0400_0102, 0x00c1);
            assert_eq!(enable_at, bus.now());
            expect_timer_irqs_at(&mut bus, overflow, 1 << 3);
            if batch {
                bus.end_device_batch();
            }
        }
    }
}

/// ZiGBA: "a count-up stage returns to the master clock on the shared prescaler grid"
#[test]
fn a_count_up_stage_returns_to_the_master_clock_on_the_shared_prescaler_grid() {
    for (return_select, expected) in [(1, 108), (0, 2)] {
        let mut eager = Bus::init(&[]);
        let mut batched = Bus::init(&[]);
        for (bus, batch) in [(&mut eager, false), (&mut batched, true)] {
            write_half(bus, 0x0400_0200, 1 << 4);
            if batch {
                bus.begin_device_batch();
            }
            write_half(bus, 0x0400_0104, 0xfffe);
            write_half(bus, 0x0400_0106, 0x00c1);
            bus.elapse(39);
            write_half(bus, 0x0400_0106, 0x00c4);
            bus.elapse(1000);
            write_half(bus, 0x0400_0106, 0x00c0 | return_select);
            assert_eq!(1044, bus.now());
            expect_timer_overflows_at(bus, 1, expected, if return_select == 1 { 64 } else { 1 });
            if batch {
                bus.end_device_batch();
            }
        }
        assert_eq!(eager.now(), batched.now());
    }
}

fn change_running_prescaler(
    bus: &mut Bus,
    old_select: usize,
    new_select: usize,
    delay: u64,
) -> u64 {
    let old_divisor = 1u64 << [0, 6, 8, 10][old_select];
    let new_divisor = 1u64 << [0, 6, 8, 10][new_select];
    assert!((1..=old_divisor).contains(&delay));
    write_half(bus, 0x0400_0102, 0);
    write_half(bus, 0x0400_0100, 0xfffe);
    write_half(bus, 0x0400_0102, 0x00c0 | old_select as u16);
    let enabled_at = bus.now();
    bus.elapse(delay - 1);
    write_half(bus, 0x0400_0102, 0x00c0 | new_select as u16);
    let written_at = bus.now();
    let ticks_left = 2 - (written_at / old_divisor - enabled_at / old_divisor);
    (written_at / new_divisor + ticks_left) * new_divisor - written_at
}

/// ZiGBA: "a running timer's prescaler change ticks on the shared grid when batched and eager"
#[test]
fn a_running_timers_prescaler_change_ticks_on_the_shared_grid_when_batched_and_eager() {
    let mut eager = Bus::init(&[]);
    let mut batched = Bus::init(&[]);
    write_half(&mut eager, 0x0400_0200, 1 << 3);
    write_half(&mut batched, 0x0400_0200, 1 << 3);
    batched.begin_device_batch();
    for old_select in 0..4 {
        let old_divisor = 1u64 << [0, 6, 8, 10][old_select];
        for new_select in 0..4 {
            let new_divisor = 1u64 << [0, 6, 8, 10][new_select];
            for delay in 1..=old_divisor {
                let expected = change_running_prescaler(&mut eager, old_select, new_select, delay);
                assert_eq!(
                    expected,
                    change_running_prescaler(&mut batched, old_select, new_select, delay)
                );
                assert_eq!(eager.now(), batched.now());
                expect_timer_overflows_at(&mut eager, 0, expected, new_divisor);
                expect_timer_overflows_at(&mut batched, 0, expected, new_divisor);
            }
        }
    }
    batched.end_device_batch();
    for (old_select, new_select, delay, expected) in [(1, 3, 60, 960), (3, 1, 699, 65)] {
        let mut bus = Bus::init(&[]);
        write_half(&mut bus, 0x0400_0200, 1 << 3);
        assert_eq!(expected, change_running_prescaler(&mut bus, old_select, new_select, delay));
        expect_timer_overflows_at(&mut bus, 0, expected, if new_select == 1 { 64 } else { 1024 });
    }
}

/// ZiGBA: "timer zero's unused count-up bit does not exempt a prescaler change"
#[test]
fn timer_zeros_unused_count_up_bit_does_not_exempt_a_prescaler_change() {
    let mut eager = Bus::init(&[]);
    let mut batched = Bus::init(&[]);
    for (bus, batch) in [(&mut eager, false), (&mut batched, true)] {
        write_half(bus, 0x0400_0200, 1 << 3);
        if batch {
            bus.begin_device_batch();
        }
        write_half(bus, 0x0400_0100, 0xfffe);
        write_half(bus, 0x0400_0102, 0x00c7);
        bus.elapse(699);
        write_half(bus, 0x0400_0102, 0x00c5);
        assert_eq!(63, bus.timers[0].prescaler_phase);
        expect_timer_overflows_at(bus, 0, 2 * 64 - 63, 64);
        if batch {
            bus.end_device_batch();
        }
    }
    assert_eq!(eager.now(), batched.now());
    eager.write_state(&mut evenfall_state::Sink::new(None, None)).unwrap();
}

/// ZiGBA: "STOP freezes the prescaler counter with the timers"
#[test]
fn stop_freezes_the_prescaler_counter_with_the_timers() {
    for batch in [false, true] {
        let mut bus = Bus::init(&[]);
        write_half(&mut bus, 0x0400_0134, 0xc000);
        write_half(&mut bus, 0x0400_0140, 0x0040);
        write_half(&mut bus, 0x0400_0200, (1 << 7) | (0b11 << 3));
        if batch {
            bus.begin_device_batch();
        }
        write_word(&mut bus, 0x0400_0100, 0x00c1_ffff);
        bus.elapse(46);
        bus.enter_stop();
        let woken_at = bus.now() + 4133;
        bus.schedule_joy_bus(JoyBusCommand::Reset, woken_at).unwrap();
        assert!(bus.idle_stop_until(woken_at));
        bus.resume_power();
        assert_eq!(4183, bus.now());
        assert_eq!(50, bus.prescaler_counter());
        bus.write_state(&mut evenfall_state::Sink::new(None, None)).unwrap();
        write_word(&mut bus, 0x0400_0104, 0x00c1_ffff);
        expect_timer_irqs_at(&mut bus, woken_at + 14, 0b11 << 3);
        expect_timer_irqs_at(&mut bus, woken_at + 78, 0b11 << 3);
        if batch {
            bus.end_device_batch();
        }
    }
}

/// ZiGBA: "DMA source zero reuses the shared transfer latch without advancing"
#[test]
fn dma_source_zero_reuses_the_shared_transfer_latch_without_advancing() {
    let mut bus = Bus::init(&[]);
    write_half(&mut bus, 0x0300_0100, 0xbeef);
    for (address, value) in [
        (0x0400_00bc, 0x0100),
        (0x0400_00be, 0x0300),
        (0x0400_00c0, 0x0200),
        (0x0400_00c2, 0x0300),
        (0x0400_00c4, 1),
        (0x0400_00c6, 1 << 15),
    ] {
        write_half(&mut bus, address, value);
    }
    bus.elapse(3);
    for (address, value) in [
        (0x0400_00d4, 0),
        (0x0400_00d6, 0),
        (0x0400_00d8, 0x0300),
        (0x0400_00da, 0x0300),
        (0x0400_00dc, 2),
        (0x0400_00de, (1 << 10) | (1 << 15)),
    ] {
        write_half(&mut bus, address, value);
    }
    bus.elapse(3);
    assert_eq!(0xbeef_beef, read_word(&mut bus, 0x0300_0300));
    assert_eq!(0xbeef_beef, read_word(&mut bus, 0x0300_0304));
    assert_eq!(0, bus.dma_channels[3].latched_source);
}

/// ZiGBA: "DMA normalizes every source below EWRAM to the transfer-latch source"
#[test]
fn dma_normalizes_every_source_below_ewram_to_the_transfer_latch_source() {
    let mut bus = Bus::init(&[]);
    write_word(&mut bus, 0x0300_0100, 0xa1b2_c3d4);
    for (address, value) in [
        (0x0400_00d4, 0x0100),
        (0x0400_00d6, 0x0300),
        (0x0400_00d8, 0x0200),
        (0x0400_00da, 0x0300),
        (0x0400_00dc, 1),
        (0x0400_00de, (1 << 10) | (1 << 15)),
    ] {
        write_half(&mut bus, address, value);
    }
    bus.elapse(3);
    for (address, value) in [
        (0x0400_00c8, 0x1234),
        (0x0400_00ca, 0),
        (0x0400_00cc, 0x0300),
        (0x0400_00ce, 0x0300),
        (0x0400_00d0, 1),
        (0x0400_00d2, (1 << 10) | (1 << 15)),
    ] {
        write_half(&mut bus, address, value);
    }
    assert_eq!(0, bus.dma_channels[2].latched_source);
    bus.elapse(3);
    assert_eq!(0xa1b2_c3d4, read_word(&mut bus, 0x0300_0300));
}

/// ZiGBA: "HBlank DMA0 preempts an active lower-priority DMA3 between beats"
#[test]
fn hblank_dma0_preempts_an_active_lower_priority_dma3_between_beats() {
    let mut bus = Bus::init(&[]);
    for (addr, value) in [
        (0x0300_0100, 0xaaaa),
        (0x0300_0120, 0x1111),
        (0x0300_0122, 0x2222),
        (0x0300_0124, 0x3333),
        (0x0300_0126, 0x4444),
    ] {
        write_half(&mut bus, addr, value);
    }
    for (addr, value) in [
        (0x0400_00b0, 0x0100),
        (0x0400_00b2, 0x0300),
        (0x0400_00b4, 0x0200),
        (0x0400_00b6, 0x0300),
        (0x0400_00b8, 1),
        (0x0400_00ba, (2 << 5) | (2 << 12) | (1 << 15)),
        (0x0400_00d4, 0x0120),
        (0x0400_00d6, 0x0300),
        (0x0400_00d8, 0x0200),
        (0x0400_00da, 0x0300),
        (0x0400_00dc, 4),
    ] {
        write_half(&mut bus, addr, value);
    }
    bus.elapse(955 - bus.now());
    write_half(&mut bus, 0x0400_00de, (2 << 5) | (1 << 15));
    bus.elapse(3);
    assert_eq!(0x4444, read_half(&mut bus, 0x0300_0200));
    assert_eq!(0, read_half(&mut bus, 0x0400_00ba) & (1 << 15));
    assert_eq!(0, read_half(&mut bus, 0x0400_00de) & (1 << 15));
}

/// ZiGBA: "VBlank DMA starts after the access that crosses into line 160"
#[test]
fn vblank_dma_starts_after_the_access_that_crosses_into_line_160() {
    let mut bus = Bus::init(&[]);
    write_half(&mut bus, 0x0300_0100, 0xcafe);
    for (addr, value) in [
        (0x0400_00b0, 0x0100),
        (0x0400_00b2, 0x0300),
        (0x0400_00b4, 0x0200),
        (0x0400_00b6, 0x0300),
        (0x0400_00b8, 1),
        (0x0400_00ba, (1 << 12) | (1 << 15)),
    ] {
        write_half(&mut bus, addr, value);
    }
    let vblank_start = 1232 * 160;
    bus.elapse(vblank_start - bus.now() - 1);
    let boundary_read = bus.read(access(
        0x0300_0200,
        AccessWidth::Half,
        AccessKind::Nonsequential,
        AccessSource::CpuData,
    ));
    assert_eq!(0, boundary_read.value);
    assert_eq!(1, boundary_read.cycles);
    let scheduled_at = bus.now();
    bus.elapse(3);
    assert_eq!(7, bus.now() - scheduled_at);
    assert_eq!(0xcafe, read_half(&mut bus, 0x0300_0200));
    assert_eq!(0, read_half(&mut bus, 0x0400_00ba) & (1 << 15));
}

/// ZiGBA: "prefetch fill timing refreshes WAITCNT across every window and 128 KiB boundary"
#[test]
fn prefetch_fill_timing_refreshes_waitcnt_across_every_window_and_128_kib_boundary() {
    for window in 0..3u32 {
        for first in 0..4u16 {
            for fast in [false, true] {
                let waitcnt =
                    (first << (2 + window * 3)) | (u16::from(fast) << (4 + window * 3)) | (1 << 14);
                let n = [5, 4, 3, 9][first as usize];
                let s = if fast { 2 } else { [3, 5, 9][window as usize] };
                for offset in [0, 0x0100_0000] {
                    for width in [AccessWidth::Half, AccessWidth::Word] {
                        let mut bus = Bus::init(&[]);
                        let stride = if width == AccessWidth::Word { 4 } else { 2 };
                        let boundary = 0x0800_0000 + window * 0x0200_0000 + offset + 0x20000;
                        write_half(&mut bus, 0x0400_0204, waitcnt ^ 0x07fc);
                        bus.fetch_kind = AccessKind::Nonsequential;
                        bus.fetch_instruction(boundary - 2 * stride, width);
                        let in_flight = bus.game_pak_prefetch.countdown;
                        bus.waitcnt = waitcnt;
                        bus.elapse(in_flight);
                        let boundary_duty = n + if width == AccessWidth::Word { s } else { 0 };
                        let sequential_duty = s + if width == AccessWidth::Word { s } else { 0 };
                        assert_eq!(boundary, bus.game_pak_prefetch.last_address);
                        assert_eq!(boundary_duty, bus.game_pak_prefetch.duty);
                        assert_eq!(boundary_duty, bus.game_pak_prefetch.countdown);
                        assert_eq!(1, bus.game_pak_prefetch.count);
                        bus.elapse(boundary_duty - 1);
                        assert_eq!(1, bus.game_pak_prefetch.countdown);
                        assert_eq!(1, bus.game_pak_prefetch.count);
                        bus.elapse(1);
                        assert_eq!(boundary + stride, bus.game_pak_prefetch.last_address);
                        assert_eq!(sequential_duty, bus.game_pak_prefetch.duty);
                        assert_eq!(sequential_duty, bus.game_pak_prefetch.countdown);
                        assert_eq!(2, bus.game_pak_prefetch.count);
                    }
                }
            }
        }
    }
}

/// ZiGBA: "Game Pak prefetch batching matches single-cycle advancement through capacity"
#[test]
fn game_pak_prefetch_batching_matches_single_cycle_advancement_through_capacity() {
    let mut batched = Bus::init(&[0; 64]);
    let mut scalar = Bus::init(&[0; 64]);
    for bus in [&mut batched, &mut scalar] {
        bus.waitcnt = 1 << 14;
        bus.fetch_kind = AccessKind::Nonsequential;
        bus.fetch_instruction(0x0800_0000, AccessWidth::Half);
    }
    batched.elapse(24);
    for _ in 0..24 {
        scalar.elapse(1);
    }
    assert_eq!(scalar.now(), batched.now());
    let a = &scalar.game_pak_prefetch;
    let b = &batched.game_pak_prefetch;
    assert_eq!(
        (
            a.active,
            a.head_address,
            a.last_address,
            a.count,
            a.capacity,
            a.opcode_bytes,
            a.countdown,
            a.duty
        ),
        (
            b.active,
            b.head_address,
            b.last_address,
            b.count,
            b.capacity,
            b.opcode_bytes,
            b.countdown,
            b.duty
        )
    );
    assert_eq!(8, batched.game_pak_prefetch.count);
    assert_eq!(0, batched.game_pak_prefetch.countdown);
}

/// ZiGBA: "WAITCNT changes preserve the in flight fetch and reprice the next unit"
#[test]
fn waitcnt_changes_preserve_the_in_flight_fetch_and_reprice_the_next_unit() {
    let rom = [0xa5; 0x140];
    for batched in [false, true] {
        for width in [AccessWidth::Half, AccessWidth::Word] {
            let stride = if width == AccessWidth::Word { 4 } else { 2 };
            let old_remaining = if width == AccessWidth::Word { 5 } else { 2 };
            let new_duty = if width == AccessWidth::Word { 4 } else { 2 };
            let value = if width == AccessWidth::Word { 0xa5a5_a5a5 } else { 0xa5a5 };
            let mut bus = Bus::init(&rom);
            write_half(&mut bus, 0x0400_0204, 0x4000);
            let mut region = Default::default();
            bus.fetch_instruction_in_region(&mut region, 0x0800_0100, width);
            if batched {
                bus.begin_device_batch();
            }
            write_half(&mut bus, 0x0400_0204, 0x4014);
            assert_eq!(old_remaining, bus.game_pak_prefetch.countdown);
            let first = bus.now();
            assert_eq!(
                value,
                bus.fetch_instruction_in_region(&mut region, 0x0800_0100 + stride, width)
            );
            assert_eq!(first + old_remaining, bus.now());
            assert_eq!(new_duty, bus.game_pak_prefetch.countdown);
            assert_eq!(0, bus.game_pak_prefetch.count);
            let second = bus.now();
            assert_eq!(
                value,
                bus.fetch_instruction_in_region(&mut region, 0x0800_0100 + 2 * stride, width)
            );
            assert_eq!(second + new_duty, bus.now());
            assert_eq!(new_duty, bus.game_pak_prefetch.countdown);
            if batched {
                bus.end_device_batch();
            }
        }
    }
}

/// ZiGBA: "repeating DMA reloads a misaligned MMIO destination to the first unit"
#[test]
fn repeating_dma_reloads_a_misaligned_mmio_destination_to_the_first_unit() {
    for (word, destination, latched, first, reloaded) in [
        (true, 0x0400_0012, 0x0400_0010, [0x045, 0x123], [0x067, 0x155]),
        (false, 0x0400_0013, 0x0400_0012, [0, 0x045], [0, 0x123]),
    ] {
        let mut bus = Bus::init(&[]);
        write_word(&mut bus, 0x0300_4000, 0x0123_0045);
        write_word(&mut bus, 0x0300_4004, 0x0155_0067);
        start_reloading_hblank_dma0(&mut bus, 0x0300_4000, destination, word);
        finish_hblank_dma(&mut bus, 0);
        assert_eq!(first, [bus.bg_horizontal_offsets[0], bus.bg_vertical_offsets[0]]);
        finish_hblank_dma(&mut bus, 1);
        assert_eq!(reloaded, [bus.bg_horizontal_offsets[0], bus.bg_vertical_offsets[0]]);
        assert_eq!(latched, bus.dma_channels[0].latched_destination);
        assert_eq!(destination, bus.dma_channels[0].destination);
    }
}

/// ZiGBA: "repeating DMA reloads a misaligned RAM destination to the first unit"
#[test]
fn repeating_dma_reloads_a_misaligned_ram_destination_to_the_first_unit() {
    for (word, destination, first, reloaded) in [
        (true, 0x0300_0201, 0xa1b2_c3d4, 0x1122_3344),
        (true, 0x0300_0202, 0xa1b2_c3d4, 0x1122_3344),
        (true, 0x0300_0203, 0xa1b2_c3d4, 0x1122_3344),
        (false, 0x0300_0201, 0xeeee_c3d4, 0xeeee_a1b2),
    ] {
        let mut bus = Bus::init(&[]);
        write_word(&mut bus, 0x0300_0100, 0xa1b2_c3d4);
        write_word(&mut bus, 0x0300_0104, 0x1122_3344);
        for addr in [0x0300_01fc, 0x0300_0200, 0x0300_0204] {
            write_word(&mut bus, addr, 0xeeee_eeee);
        }
        start_reloading_hblank_dma0(&mut bus, 0x0300_0100, destination, word);
        finish_hblank_dma(&mut bus, 0);
        assert_eq!(first, read_word(&mut bus, 0x0300_0200));
        finish_hblank_dma(&mut bus, 1);
        assert_eq!(reloaded, read_word(&mut bus, 0x0300_0200));
        assert_eq!(0xeeee_eeee, read_word(&mut bus, 0x0300_01fc));
        assert_eq!(0xeeee_eeee, read_word(&mut bus, 0x0300_0204));
        assert_eq!(0x0300_0200, bus.dma_channels[0].latched_destination);
        assert_eq!(destination, bus.dma_channels[0].destination);
    }
}

fn start_reloading_hblank_dma0(bus: &mut Bus, source: u32, destination: u32, word: bool) {
    write_half(bus, 0x0400_00b0, source as u16);
    write_half(bus, 0x0400_00b2, (source >> 16) as u16);
    write_half(bus, 0x0400_00b4, destination as u16);
    write_half(bus, 0x0400_00b6, (destination >> 16) as u16);
    write_half(bus, 0x0400_00b8, 1);
    let width = if word { 1 << 10 } else { 0 };
    write_half(bus, 0x0400_00ba, (3 << 5) | (1 << 9) | width | (2 << 12) | (1 << 15));
}

fn finish_hblank_dma(bus: &mut Bus, line: u64) {
    bus.elapse(line * 1232 + 960 - bus.now());
    bus.elapse(3);
}

/// ZiGBA: "timer zero unused count-up bit cannot bypass state phase validation"
#[test]
fn timer_zero_unused_count_up_bit_cannot_bypass_state_phase_validation() {
    let mut bus = Bus::init(&[]);
    bus.timers[0].control = 0x0085;
    bus.timers[0].prescaler_phase = 64;
    assert_eq!(
        Err(evenfall_state::Error::InvalidSaveState),
        bus.write_state(&mut evenfall_state::Sink::new(None, None))
    );
    bus.timers[0].prescaler_phase = 63;
    assert_eq!(
        Err(evenfall_state::Error::InvalidSaveState),
        bus.write_state(&mut evenfall_state::Sink::new(None, None))
    );
    bus.timers[0].prescaler_phase = 0;
    bus.timers[1].control = 0x0084;
    bus.timers[1].prescaler_phase = 63;
    bus.write_state(&mut evenfall_state::Sink::new(None, None)).unwrap();
}

/// ZiGBA: "no-event materialization keeps a simultaneous audio sample before FIFO consumption"
#[test]
fn no_event_materialization_keeps_a_simultaneous_audio_sample_before_fifo_consumption() {
    let mut bus = Bus::init(&[]);
    write_half(&mut bus, 0x0400_0084, 1 << 7);
    write_half(&mut bus, 0x0400_0082, (1 << 2) | (1 << 8) | (1 << 9));
    write_word(&mut bus, 0x0400_00a0, 0x0403_0201);
    bus.elapse(505);
    bus.timers[0].reload = 0xfffc;
    bus.timers[0].counter = 0xfffc;
    bus.timers[0].control = 0x0084;
    bus.begin_device_batch();
    bus.elapse(1);
    bus.elapse(2);
    bus.set_video_interval(1);
    let mut samples = [Default::default(); 2];
    assert_eq!(0, bus.drain_audio_samples(&mut samples));
    assert_eq!(0xffff, bus.timers[0].counter);
    bus.elapse(1);
    assert_eq!(1, bus.drain_audio_samples(&mut samples));
    assert_eq!((0, 0), (samples[0].left, samples[0].right));
    assert_eq!((4, 4), (bus.current_audio_sample().left, bus.current_audio_sample().right));
    write_half(&mut bus, 0x0400_0102, 0);
    for _ in 0..511 {
        bus.elapse(1);
    }
    bus.set_audio_output(true);
    assert_eq!(1024, bus.now());
    assert_eq!(1, bus.drain_audio_samples(&mut samples));
    assert_eq!((4, 4), (samples[0].left, samples[0].right));
    bus.end_device_batch();
}

/// ZiGBA: "full prefetch queue restarts with boundary and sequential wait states"
#[test]
fn full_prefetch_queue_restarts_with_boundary_and_sequential_wait_states() {
    let rom = vec![0xa5; 0x20040];
    for batched in [false, true] {
        for width in [AccessWidth::Half, AccessWidth::Word] {
            let stride = if width == AccessWidth::Word { 4 } else { 2 };
            let capacity = if width == AccessWidth::Word { 4 } else { 8 };
            let sequential = if width == AccessWidth::Word { 4 } else { 2 };
            let boundary_duty = if width == AccessWidth::Word { 6 } else { 4 };
            let boundary = 0x0802_0000;
            let start = boundary - (capacity + 1) * stride;
            let value = if width == AccessWidth::Word { 0xa5a5_a5a5 } else { 0xa5a5 };
            let mut bus = Bus::init(&rom);
            write_half(&mut bus, 0x0400_0204, 0x4014);
            let mut region = Default::default();
            bus.fetch_instruction_in_region(&mut region, start, width);
            bus.elapse(u64::from(capacity * sequential));
            assert_eq!(capacity, u32::from(bus.game_pak_prefetch.count));
            assert_eq!(0, bus.game_pak_prefetch.countdown);
            assert_eq!(boundary - stride, bus.game_pak_prefetch.last_address);
            if batched {
                bus.begin_device_batch();
            }
            let first = bus.now();
            assert_eq!(value, bus.fetch_instruction_in_region(&mut region, start + stride, width));
            assert_eq!(first + 1, bus.now());
            assert_eq!(capacity - 1, u32::from(bus.game_pak_prefetch.count));
            assert_eq!(boundary, bus.game_pak_prefetch.last_address);
            assert_eq!(boundary_duty - 1, bus.game_pak_prefetch.countdown);
            bus.elapse(boundary_duty - 1);
            assert_eq!(capacity, u32::from(bus.game_pak_prefetch.count));
            assert_eq!(0, bus.game_pak_prefetch.countdown);
            let second = bus.now();
            assert_eq!(
                value,
                bus.fetch_instruction_in_region(&mut region, start + 2 * stride, width)
            );
            assert_eq!(second + 1, bus.now());
            assert_eq!(boundary + stride, bus.game_pak_prefetch.last_address);
            assert_eq!(u64::from(sequential - 1), bus.game_pak_prefetch.countdown);
            if batched {
                bus.end_device_batch();
            }
        }
    }
}

/// ZiGBA: "batched instruction fetch latches before DMA at the exact access boundary"
#[test]
fn batched_instruction_fetch_latches_before_dma_at_the_exact_access_boundary() {
    let mut bus = Bus::init(&[]);
    for (addr, value) in [
        (0x0200_0000, 0x1122),
        (0x0200_0002, 0x3344),
        (0x0400_00d4, 0),
        (0x0400_00d6, 0x0200),
        (0x0400_00d8, 0),
        (0x0400_00da, 0x0300),
        (0x0400_00dc, 2),
        (0x0400_00de, (1 << 14) | (1 << 15)),
    ] {
        write_half(&mut bus, addr, value);
    }
    let started = bus.now();
    bus.begin_device_batch();
    bus.fetch_kind = AccessKind::Sequential;
    for _ in 0..2 {
        assert_eq!(0, bus.fetch_instruction(0x0300_0000, AccessWidth::Word));
    }
    bus.end_device_batch();
    assert_eq!(started + 12, bus.now());
    assert_eq!(0x3344_1122, bus.fetch_instruction(0x0300_0000, AccessWidth::Word));
    assert_ne!(0, bus.interrupt_request_flags & (1 << 11));
}

/// ZiGBA: "no-event materialization retains simultaneous input serial LCD timer and DMA delivery"
#[test]
fn no_event_materialization_retains_simultaneous_input_serial_lcd_timer_and_dma_delivery() {
    let mut bus = Bus::init(&[]);
    bus.elapse(953);
    bus.ppu_timing.write_dispstat(1 << 4);
    bus.timers[0].reload = 0;
    bus.timers[0].counter = 0xfff9;
    bus.timers[0].control = 0x00c4;
    bus.keypad_control = 0x4001;
    bus.rcnt = 0xc000;
    bus.joy_control = 0x0040;
    bus.schedule_joy_bus(JoyBusCommand::Reset, 960).unwrap();
    bus.dma_channels[3].source = 0x0400_0202;
    bus.dma_channels[3].destination = 0x0300_0000;
    bus.dma_channels[3].word_count = 1;
    bus.dma_channels[3].control = 0xc000;
    bus.dma_channels[3].latched_source = 0x0400_0202;
    bus.dma_channels[3].latched_destination = 0x0300_0000;
    bus.dma_start_cycles[3] = Some(960);
    let inputs = [TimedInput {
        at: 960,
        change: TimedInputChange::Keys(KeyState { a: true, ..Default::default() }),
    }];
    // SAFETY: `inputs` remains immutable and in scope through `end_external_inputs` below.
    unsafe {
        bus.begin_external_inputs(&inputs);
    }
    bus.begin_device_batch();
    bus.elapse(1);
    bus.elapse(5);
    bus.set_video_interval(1);
    assert_eq!(959, bus.now());
    assert_eq!(959, bus.ppu_timing.cycle_in_line);
    assert_eq!(0xffff, bus.timers[0].counter);
    assert_eq!(0, bus.interrupt_request_flags);
    assert_eq!(0, bus.external_inputs_consumed());
    assert_eq!(None, bus.take_joy_bus_response());
    bus.elapse(1);
    assert_eq!(964, bus.now());
    assert_eq!(0x108a, u16::from_le_bytes(bus.iwram[0..2].try_into().unwrap()));
    assert_eq!(0x188a, bus.interrupt_request_flags);
    assert_eq!(1, bus.external_inputs_consumed());
    assert!(bus.take_joy_bus_response().is_some());
    bus.end_device_batch();
    bus.end_external_inputs();
}

/// ZiGBA: "audio DMA refill timing is invariant to elapsed-cycle chunking"
#[test]
fn audio_dma_refill_timing_is_invariant_to_elapsed_cycle_chunking() {
    let mut coarse = Bus::init(&[]);
    let mut fine = Bus::init(&[]);
    for bus in [&mut coarse, &mut fine] {
        let source = 0x0300_0000;
        for index in 0..4 {
            write_word(bus, source + index * 4, 0x1111_1111);
        }
        for (addr, value) in [
            (0x0400_0084, 1 << 7),
            (0x0400_0082, (1 << 2) | (1 << 9)),
            (0x0400_00bc, source as u16),
            (0x0400_00be, (source >> 16) as u16),
            (0x0400_00c0, 0x00a0),
            (0x0400_00c2, 0x0400),
            (0x0400_00c4, 1),
            (0x0400_00c6, (1 << 15) | (3 << 12) | (1 << 9)),
            (0x0400_0100, 0xffff),
            (0x0400_0102, (1 << 7) | 2),
        ] {
            write_half(bus, addr, value);
        }
    }
    coarse.elapse(1024);
    for _ in 0..1024 {
        fine.elapse(1);
    }
    let mut coarse_samples = [Default::default(); 4];
    let mut fine_samples = [Default::default(); 4];
    let coarse_count = coarse.drain_audio_samples(&mut coarse_samples);
    let fine_count = fine.drain_audio_samples(&mut fine_samples);
    assert_eq!(fine_count, coarse_count);
    assert_eq!(&fine_samples[..fine_count], &coarse_samples[..coarse_count]);
    assert_eq!(0, coarse_samples[0].left);
    assert_eq!(0x11 * 4, coarse_samples[1].left);
}

/// ZiGBA: "a DMA burst that never releases the bus latches a livelock and returns"
#[test]
fn a_dma_burst_that_never_releases_the_bus_latches_a_livelock_and_returns() {
    let mut bus = Bus::init(&[]);
    write_half(&mut bus, 0x0400_0084, 0x0080);
    write_half(&mut bus, 0x0400_0082, 0xfb00);
    write_word(&mut bus, 0x0400_00bc, 0x0300_2000);
    write_word(&mut bus, 0x0400_00c0, 0x0400_00a0);
    write_half(&mut bus, 0x0400_00c6, 0xb740);
    write_word(&mut bus, 0x0400_00c8, 0x0300_2000);
    write_word(&mut bus, 0x0400_00cc, 0x0400_00a4);
    write_half(&mut bus, 0x0400_00d2, 0xb740);
    write_word(&mut bus, 0x0400_0100, 0x0080_ffff);
    write_word(&mut bus, 0x0400_0104, 0x0080_ffff);
    let started_at = bus.now();
    bus.elapse(1);
    let livelock = bus.dma_livelock().expect("DMA livelock");
    assert!(livelock.channel == 1 || livelock.channel == 2);
    assert!(livelock.started_at >= started_at);
    assert!(bus.now() - livelock.started_at >= Bus::MAX_DMA_BURST_CYCLES);
    let after = bus.now();
    bus.elapse(100);
    assert_eq!(after + 100, bus.now());
}

/// ZiGBA: "batched STOP serial time does not accrue system device clocks"
#[test]
fn batched_stop_serial_time_does_not_accrue_system_device_clocks() {
    let mut bus = Bus::init(&[]);
    write_half(&mut bus, 0x0400_0134, 0xc000);
    write_half(&mut bus, 0x0400_0140, 0x0040);
    write_half(&mut bus, 0x0400_0200, 1 << 7);
    bus.begin_device_batch();
    bus.elapse(1);
    bus.elapse(6);
    bus.enter_stop();
    let ppu = bus.ppu_timing;
    let apu = bus.apu.clone();
    let timers = bus
        .timers
        .each_ref()
        .map(|timer| (timer.reload, timer.counter, timer.control, timer.prescaler_phase));
    let deadline = bus.now() + 4096;
    bus.schedule_joy_bus(JoyBusCommand::Reset, deadline).unwrap();
    assert!(bus.idle_stop_until(deadline));
    bus.end_device_batch();
    assert_eq!(deadline, bus.now());
    assert_eq!(ppu, bus.ppu_timing);
    assert_eq!(apu, bus.apu);
    for (before, after) in timers.iter().zip(bus.timers.iter()) {
        assert_eq!(*before, (after.reload, after.counter, after.control, after.prescaler_phase));
    }
    assert!(bus.take_joy_bus_response().is_some());
}

/// ZiGBA: "a halted batch wakes at the LCD or timer event after post-halt charges and cut slices"
#[test]
fn a_halted_batch_wakes_at_the_lcd_or_timer_event_after_post_halt_charges_and_cut_slices() {
    for timer in [true, false] {
        for lag in [0, 12] {
            for split in [0, 100] {
                for batched in [false, true] {
                    let mut bus = Bus::init(&[]);
                    if timer {
                        write_half(&mut bus, 0x0400_0200, 1 << 3);
                        write_word(&mut bus, 0x0400_0100, 0x00c0_0000 | (0x10000 - 700));
                    } else {
                        write_half(&mut bus, 0x0400_0200, 1 << 1);
                        write_half(&mut bus, 0x0400_0004, 1 << 4);
                    }
                    if batched {
                        bus.begin_device_batch();
                    }
                    bus.enter_halt();
                    write_word(&mut bus, 0x0300_7ff8, 0);
                    bus.elapse(lag);
                    if split != 0 {
                        assert!(!bus.idle_halt_until(bus.now() + split));
                    }
                    assert!(bus.idle_halt_until(bus.now() + 5000));
                    assert_eq!(if timer { 702 } else { 960 }, bus.now());
                    assert_eq!(if timer { 1 << 3 } else { 1 << 1 }, bus.interrupt_request_flags);
                    if batched {
                        bus.end_device_batch();
                    }
                }
            }
        }
    }
}

/// ZiGBA: "one-cycle endpoint preserves simultaneous timer HBlank and DMA ordering"
#[test]
fn one_cycle_endpoint_preserves_simultaneous_timer_hblank_and_dma_ordering() {
    let mut bus = Bus::init(&[]);
    bus.elapse(959);
    bus.ppu_timing.write_dispstat(1 << 4);
    bus.timers[0].reload = 0xfffe;
    bus.timers[0].counter = 0xffff;
    bus.timers[0].control = (1 << 7) | (1 << 6);
    bus.timers[1].reload = 0xfffd;
    bus.timers[1].counter = 0xffff;
    bus.timers[1].control = (1 << 7) | (1 << 6) | (1 << 2);
    bus.dma_channels[0].source = 0x0300_0100;
    bus.dma_channels[0].destination = 0x0300_0200;
    bus.dma_channels[0].word_count = 1;
    bus.dma_channels[0].control = (2 << 12) | (1 << 15);
    bus.dma_channels[0].latched_source = 0x0300_0100;
    bus.dma_channels[0].latched_destination = 0x0300_0200;
    let boundary = bus.read(access(
        0x0300_0000,
        AccessWidth::Word,
        AccessKind::Nonsequential,
        AccessSource::CpuData,
    ));
    assert_eq!(1, boundary.cycles);
    assert_eq!(960, bus.now());
    assert_eq!(0xfffe, bus.timers[0].read_counter());
    assert_eq!(0xfffd, bus.timers[1].read_counter());
    assert_eq!((1 << 1) | (1 << 3) | (1 << 4), bus.interrupt_request_flags);
    assert_eq!(0x7fff, bus.framebuffer()[0]);
    assert_eq!(Some(963), bus.dma_start_cycles[0]);
}
