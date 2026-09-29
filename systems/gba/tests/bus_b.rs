//! ZiGBA `test/bus_test.zig` integration oracles not covered by `bus.rs`.

use evenfall_gba::bus::{
    Access, AccessKind, AccessSource, AccessWidth, Bus, InstructionFetchRegion, KeyState,
    TimedInput, TimedInputChange,
};
use evenfall_gba::machine::{Config, Machine};

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

fn read_half(bus: &mut Bus, address: u32) -> u16 {
    bus.read(access(address, AccessWidth::Half, AccessKind::Nonsequential, AccessSource::CpuData))
        .value as u16
}

fn fetch_state_parity(reference: &Bus, active: &Bus) {
    assert_eq!(reference.now(), active.now());
    assert_eq!(reference.game_pak_prefetch, active.game_pak_prefetch);
    assert_eq!(reference.duty_valid_until, active.duty_valid_until);
    assert_eq!(reference.open_bus_value, active.open_bus_value);
    assert_eq!(reference.fetch_kind, active.fetch_kind);
    assert_eq!(reference.game_pak_code_active, active.game_pak_code_active);
    assert_eq!(reference.prefetch_disabled_bug_pending, active.prefetch_disabled_bug_pending);
    assert_eq!(reference.prefetch_buffer_was_disabled, active.prefetch_buffer_was_disabled);
    assert_eq!(reference.batch_deadline, active.batch_deadline);
    assert_eq!(reference.irq.interrupt_request_flags, active.irq.interrupt_request_flags);
}

fn bus_state_parity(reference: &Bus, active: &Bus) {
    fetch_state_parity(reference, active);
    let mut expected = Vec::new();
    let mut actual = Vec::new();
    reference.write_state(&mut evenfall_state::Sink::new(Some(&mut expected), None)).unwrap();
    active.write_state(&mut evenfall_state::Sink::new(Some(&mut actual), None)).unwrap();
    assert_eq!(expected, actual);
    assert_eq!(reference.batch_active, active.batch_active);
    assert_eq!(reference.batch_device_cycle, active.batch_device_cycle);
    assert_eq!(reference.timer_cycle, active.timer_cycle);
    assert_eq!(reference.apu_cycle, active.apu_cycle);
    assert_eq!(reference.framebuffer(), active.framebuffer());
    assert_eq!(reference.apu.fifo_a, active.apu.fifo_a);
    assert_eq!(format!("{:?}", reference.apu), format!("{:?}", active.apu));
}

fn game_pak_data_read_parity(
    reference: &mut Bus,
    active: &mut Bus,
    address: u32,
    width: AccessWidth,
) {
    assert_eq!(reference.now(), active.now());
    let expected =
        reference.read(access(address, width, AccessKind::Nonsequential, AccessSource::CpuData));
    assert_eq!(expected.value, active.read_cpu_data(address, width, AccessKind::Nonsequential));
    fetch_state_parity(reference, active);
}

fn region_fetch_parity(
    reference: &mut Bus,
    active: &mut Bus,
    region: &mut InstructionFetchRegion,
    address: u32,
    width: AccessWidth,
    kind: AccessKind,
) {
    assert_eq!(reference.now(), active.now());
    let expected = reference.read(access(address, width, kind, AccessSource::CpuCode));
    active.fetch_kind = kind;
    assert_eq!(expected.value, active.fetch_instruction_in_region(region, address, width));
    fetch_state_parity(reference, active);
}

/// ZiGBA: "CPU Game Pak data reads match read() in every queue phase and at deadlines"
#[test]
fn cpu_game_pak_data_reads_match_read_in_every_queue_phase_and_at_deadlines() {
    let rom: Vec<u8> = (0..0x20040).map(|index| (index * 37 + 11) as u8).collect();
    for batched in [false, true] {
        for waitcnt in [0x4000, 0x4317, 0x47FC, 0x0317] {
            let mut reference = Bus::init(&rom);
            let mut active = Bus::init(&rom);
            for bus in [&mut reference, &mut active] {
                bus.enable_rtc(0);
                write_half(bus, 0x0400_0204, waitcnt);
                write_half(bus, 0x0800_00C8, 1);
                bus.elapse(900);
                if batched {
                    bus.begin_device_batch();
                }
            }
            for address in [
                0x0800_0100,
                0x0800_0101,
                0x0800_0102,
                0x0800_00C2,
                0x0800_00C4,
                0x0800_00C8,
                0x0800_00CA,
                0x0801_FFFE,
                0x0802_0000,
                0x0802_003C,
                0x0802_003E,
                0x0803_0000,
                0x0A00_0100,
                0x0C00_0102,
                0x0CFF_FFFE,
            ] {
                for width in [AccessWidth::Byte, AccessWidth::Half, AccessWidth::Word] {
                    for opcode_bytes in [2, 4] {
                        for gap in [0, 1, 2, 3, 5, 9] {
                            let code_width = if opcode_bytes == 4 {
                                AccessWidth::Word
                            } else {
                                AccessWidth::Half
                            };
                            for bus in [&mut reference, &mut active] {
                                bus.read(access(
                                    0x0800_0200,
                                    code_width,
                                    AccessKind::Nonsequential,
                                    AccessSource::CpuCode,
                                ));
                                bus.elapse_cpu_internal(gap);
                            }
                            game_pak_data_read_parity(&mut reference, &mut active, address, width);
                        }
                    }
                }
                if batched {
                    for margin in 1..16 {
                        for bus in [&mut reference, &mut active] {
                            bus.read(access(
                                0x0800_0200,
                                AccessWidth::Half,
                                AccessKind::Nonsequential,
                                AccessSource::CpuCode,
                            ));
                            let until_deadline = bus.batch_deadline.saturating_sub(bus.now());
                            if until_deadline > margin {
                                bus.elapse(until_deadline - margin);
                            }
                        }
                        game_pak_data_read_parity(
                            &mut reference,
                            &mut active,
                            address,
                            AccessWidth::Word,
                        );
                    }
                }
            }
            reference.waitcnt = 0x4014;
            active.waitcnt = 0x4014;
            game_pak_data_read_parity(&mut reference, &mut active, 0x0800_0100, AccessWidth::Half);
            if batched {
                reference.end_device_batch();
                active.end_device_batch();
            }
            bus_state_parity(&reference, &active);
        }
    }
}

/// ZiGBA: "Thumb region misses match read() across forced N fetches and deadlines"
#[test]
fn thumb_region_misses_match_read_across_forced_n_fetches_and_deadlines() {
    let rom: Vec<u8> = (0..0x20040).map(|index| (index * 13 + 5) as u8).collect();
    for batched in [false, true] {
        for waitcnt in [0x4000_u16, 0x4317, 0x47FC, 0x0317] {
            let mut reference = Bus::init(&rom);
            let mut active = Bus::init(&rom);
            let mut region = InstructionFetchRegion::default();
            for bus in [&mut reference, &mut active] {
                write_half(bus, 0x0400_0204, waitcnt);
                bus.elapse(900);
                if batched {
                    bus.begin_device_batch();
                }
            }
            for address in
                [0x0800_0100, 0x0801_FFFE, 0x0802_0000, 0x0802_0002, 0x0A00_0100, 0x0C00_0102]
            {
                for setup in 0..6 {
                    for kind in [AccessKind::Nonsequential, AccessKind::Sequential] {
                        for bus in [&mut reference, &mut active] {
                            let code_width =
                                if setup == 4 { AccessWidth::Word } else { AccessWidth::Half };
                            bus.read(access(
                                0x0800_0200,
                                code_width,
                                AccessKind::Nonsequential,
                                AccessSource::CpuCode,
                            ));
                            bus.elapse_cpu_internal(setup);
                            match setup {
                                1 => {
                                    bus.read(access(
                                        0x0800_0300,
                                        AccessWidth::Word,
                                        AccessKind::Nonsequential,
                                        AccessSource::CpuData,
                                    ));
                                }
                                2 => {
                                    write_half(bus, 0x0400_0204, waitcnt & !(1 << 14));
                                    bus.elapse_cpu_internal(1);
                                    write_half(bus, 0x0400_0204, waitcnt);
                                }
                                3 => {
                                    write_half(bus, 0x0400_0204, waitcnt & !(1 << 14));
                                    bus.read(access(
                                        0x0800_0204,
                                        AccessWidth::Half,
                                        AccessKind::Sequential,
                                        AccessSource::CpuCode,
                                    ));
                                    bus.elapse_cpu_internal(1);
                                }
                                _ => {}
                            }
                        }
                        region_fetch_parity(
                            &mut reference,
                            &mut active,
                            &mut region,
                            address,
                            AccessWidth::Half,
                            kind,
                        );
                        if setup == 3 {
                            write_half(&mut reference, 0x0400_0204, waitcnt);
                            write_half(&mut active, 0x0400_0204, waitcnt);
                        }
                    }
                }
                if batched {
                    for margin in 1..16 {
                        for bus in [&mut reference, &mut active] {
                            bus.read(access(
                                0x0800_0300,
                                AccessWidth::Word,
                                AccessKind::Nonsequential,
                                AccessSource::CpuData,
                            ));
                            let until_deadline = bus.batch_deadline.saturating_sub(bus.now());
                            if until_deadline > margin {
                                bus.elapse(until_deadline - margin);
                            }
                        }
                        region_fetch_parity(
                            &mut reference,
                            &mut active,
                            &mut region,
                            address,
                            AccessWidth::Half,
                            AccessKind::Nonsequential,
                        );
                    }
                }
            }
            reference.waitcnt = waitcnt ^ 0x0010;
            active.waitcnt = waitcnt ^ 0x0010;
            for bus in [&mut reference, &mut active] {
                bus.read(access(
                    0x0800_0300,
                    AccessWidth::Word,
                    AccessKind::Nonsequential,
                    AccessSource::CpuData,
                ));
            }
            region_fetch_parity(
                &mut reference,
                &mut active,
                &mut region,
                0x0800_0100,
                AccessWidth::Half,
                AccessKind::Nonsequential,
            );
            if batched {
                reference.end_device_batch();
                active.end_device_batch();
            }
            bus_state_parity(&reference, &active);
        }
    }
}

/// ZiGBA: "synchronized reads and FIFO writes keep the device batch armed"
#[test]
fn synchronized_reads_and_fifo_writes_keep_the_device_batch_armed() {
    let mut eager = Bus::init(&[]);
    let mut batched = Bus::init(&[]);
    for bus in [&mut eager, &mut batched] {
        write_half(bus, 0x0400_0084, 1 << 7);
        write_half(bus, 0x0400_0082, 1 << 8);
        write_half(bus, 0x0400_0100, 0xFFF0);
        write_half(bus, 0x0400_0102, 0x0080);
        write_half(bus, 0x0400_0104, 0xFF00);
        write_half(bus, 0x0400_0106, 0x00C4);
        bus.iwram[0x100..0x108].copy_from_slice(&[0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]);
        write_half(bus, 0x0400_00BC, 0x0000);
        write_half(bus, 0x0400_00BE, 0x0300);
        write_half(bus, 0x0400_00C0, 0x00A0);
        write_half(bus, 0x0400_00C2, 0x0400);
        write_half(bus, 0x0400_00C4, 4);
        write_half(bus, 0x0400_00C6, 0xB440);
        bus.keypad_control = 0x4001;
    }
    let inputs = [TimedInput {
        at: eager.now() + 40,
        change: TimedInputChange::Keys(KeyState { a: true, ..Default::default() }),
    }];
    eager.begin_external_inputs(&inputs);
    batched.begin_external_inputs(&inputs);
    batched.begin_device_batch();

    for _ in 0..20 {
        eager.elapse(1);
        batched.elapse(1);
    }
    assert_ne!(eager.apu.fifo_a.write_index, eager.apu.fifo_a.read_index);

    let timer0_counter =
        access(0x0400_0100, AccessWidth::Half, AccessKind::Nonsequential, AccessSource::CpuData);
    let expected_counter = eager.read(timer0_counter);
    let batched_counter = batched.read(timer0_counter);
    assert_ne!(expected_counter.value, 0);
    assert_eq!(expected_counter.value, batched_counter.value);
    assert_eq!(expected_counter.cycles, batched_counter.cycles);

    let fifo_write =
        access(0x0400_00A0, AccessWidth::Word, AccessKind::Nonsequential, AccessSource::CpuData);
    assert_eq!(
        eager.write(fifo_write, 0x0DDC_0FFE).cycles,
        batched.write(fifo_write, 0x0DDC_0FFE).cycles
    );
    let timer1_reload =
        access(0x0400_0104, AccessWidth::Half, AccessKind::Nonsequential, AccessSource::CpuData);
    assert_eq!(
        eager.write(timer1_reload, 0xFFF8).cycles,
        batched.write(timer1_reload, 0xFFF8).cycles
    );

    for cycles in [1, 4, 16, 23, 64, 100, 512, 1232] {
        eager.elapse(cycles);
        batched.elapse(cycles);
        assert_eq!(eager.now(), batched.now());
        assert_eq!(eager.irq.interrupt_request_flags, batched.irq.interrupt_request_flags);
        assert_eq!(eager.external_inputs_consumed(), batched.external_inputs_consumed());
        assert_eq!(eager.apu.fifo_a, batched.apu.fifo_a);
        assert_eq!(eager.apu.sample_a, batched.apu.sample_a);
        if batched.now() == 60 {
            assert_ne!(batched.irq.interrupt_request_flags & (1 << 12), 0);
        }
    }
    let key_input =
        access(0x0400_0130, AccessWidth::Half, AccessKind::Nonsequential, AccessSource::CpuData);
    let expected_keys = eager.read(key_input);
    assert_eq!(expected_keys.value & 1, 0);
    let actual_keys = batched.read(key_input);
    assert_eq!(expected_keys.value, actual_keys.value);
    assert_eq!(expected_keys.cycles, actual_keys.cycles);

    batched.end_device_batch();
    eager.end_external_inputs();
    batched.end_external_inputs();
    bus_state_parity(&eager, &batched);
}

/// ZiGBA: "delegated FIFO timer overflows match eager delivery through refill and schedule changes"
#[test]
fn delegated_fifo_timer_overflows_match_eager_delivery_through_refill_and_schedule_changes() {
    let mut eager = Bus::init(&[]);
    for (i, byte) in eager.ewram[..0x4000].iter_mut().enumerate() {
        *byte = (i.wrapping_mul(37).wrapping_add(i >> 8)) as u8;
    }
    for (i, byte) in eager.iwram[..0x4000].iter_mut().enumerate() {
        *byte = (i.wrapping_mul(91).wrapping_add(5)) as u8;
    }
    let mut batched = Bus::init(&[]);
    batched.ewram.copy_from_slice(&eager.ewram[..]);
    batched.iwram.copy_from_slice(&eager.iwram[..]);
    for bus in [&mut eager, &mut batched] {
        write_half(bus, 0x0400_0084, 1 << 7);
        write_half(bus, 0x0400_0082, 0x730E);
        write_half(bus, 0x0400_0200, (1 << 3) | (1 << 4) | (1 << 9) | (1 << 10));
        for (base, source, destination) in
            [(0x0400_00BC, 0x0200_0000, 0x0400_00A0), (0x0400_00C8, 0x0300_0000, 0x0400_00A4)]
        {
            write_word(bus, base, source);
            write_word(bus, base + 4, destination);
            write_half(bus, base + 10, 0xF200);
        }
        write_half(bus, 0x0400_0100, 0xFFF0);
        write_half(bus, 0x0400_0102, 0x0080);
        write_half(bus, 0x0400_0104, 0xFFC3);
        write_half(bus, 0x0400_0106, 0x0080);
    }
    batched.begin_device_batch();
    assert!(batched.fifo_clock.delegated());
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    let mut delegated_steps = 0;
    let mut eager_samples = [Default::default(); 2048];
    let mut batched_samples = [Default::default(); 2048];
    for _ in 0..3000 {
        state =
            state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        let random = (state >> 33) as u32;
        let choice = random % 100;
        let argument = random >> 8;
        if batched.fifo_clock.delegated() {
            delegated_steps += 1;
        }
        for bus in [&mut eager, &mut batched] {
            match choice {
                0..=39 => bus.elapse(u64::from(1 + argument % 400)),
                40..=49 => bus.elapse(u64::from(1 + argument % 3)),
                50..=59 => {
                    let address = 0x0400_00A0 + (argument & 4) + ((argument >> 3) & 3);
                    let width = match (argument >> 5) & 3 {
                        0 => AccessWidth::Byte,
                        1 => AccessWidth::Half,
                        _ => AccessWidth::Word,
                    };
                    let aligned = match width {
                        AccessWidth::Byte => address,
                        AccessWidth::Half => address & !1,
                        AccessWidth::Word => address & !3,
                    };
                    bus.write(
                        access(aligned, width, AccessKind::Nonsequential, AccessSource::CpuData),
                        argument.wrapping_mul(0x0101_0101),
                    );
                }
                60..=64 => {
                    let resets =
                        if argument & 0x30 == 0 { ((argument >> 6) as u16) & 0x8800 } else { 0 };
                    write_half(bus, 0x0400_0082, ((argument as u16) & 0x770F) | resets);
                }
                65..=69 => {
                    let controls = [0x0080, 0x00C0, 0x0084, 0x0081, 0x0000, 0x0080, 0x0080, 0x0082];
                    let timer = argument & 1;
                    write_half(
                        bus,
                        0x0400_0102 + 4 * timer,
                        controls[((argument >> 1) & 7) as usize],
                    );
                }
                70..=74 => {
                    write_half(
                        bus,
                        0x0400_0100 + 4 * (argument & 1),
                        0xFF80 | ((argument >> 1) as u16),
                    );
                }
                75..=76 => {
                    write_half(bus, 0x0400_0084, if argument & 3 == 0 { 0 } else { 1 << 7 });
                }
                77..=79 => {
                    let control = 0x0400_00C6 + 12 * (argument & 1);
                    write_half(bus, control, if argument & 6 == 0 { 0 } else { 0xF200 });
                }
                80..=86 => {
                    read_half(bus, 0x0400_0100 + 4 * (argument & 1));
                }
                87..=89 => {
                    read_half(bus, 0x0400_0084);
                }
                90..=93 => {
                    write_half(bus, 0x0400_0202, 0xFFFF);
                    bus.enter_halt();
                    bus.idle_halt_until(bus.now() + u64::from(argument % 2000));
                    bus.resume_power();
                }
                _ => bus.set_audio_output(true),
            }
        }
        assert_eq!(eager.now(), batched.now());
        assert_eq!(eager.irq.interrupt_request_flags, batched.irq.interrupt_request_flags);
        assert_eq!(eager.pending_dma, batched.pending_dma);
        for (expected, actual) in eager.dma_channels.iter().zip(batched.dma_channels.iter()) {
            assert_eq!(expected.latched_source, actual.latched_source);
        }
        if choice >= 94 {
            assert_eq!(format!("{:?}", eager.apu), format!("{:?}", batched.apu));
            assert_eq!(
                (eager.apu.sample_a, eager.apu.sample_b),
                (batched.apu.sample_a, batched.apu.sample_b)
            );
            for (expected, actual) in eager.timers.iter().zip(batched.timers.iter()) {
                assert_eq!(
                    (expected.reload, expected.counter, expected.control, expected.prescaler_phase),
                    (actual.reload, actual.counter, actual.control, actual.prescaler_phase)
                );
            }
            let count = eager.drain_audio_samples(&mut eager_samples);
            assert_eq!(count, batched.drain_audio_samples(&mut batched_samples));
            assert_eq!(&eager_samples[..count], &batched_samples[..count]);
        }
    }
    assert!(delegated_steps > 1000);
    batched.end_device_batch();
    bus_state_parity(&eager, &batched);
}

/// ZiGBA: "text row colors refresh before stable second-surface lines and retain palette history"
#[test]
fn text_row_colors_refresh_before_stable_second_surface_lines_and_retain_palette_history() {
    let mut bus = Bus::init(&[]);
    bus.dispcnt = (1 << 8) | (1 << 9);
    bus.bg_controls[0] = 30 << 8;
    bus.bg_controls[1] = 1 | (31 << 8);
    for row in 0..8 {
        bus.vram[32 + row * 4..36 + row * 4].copy_from_slice(&0x8765_4321_u32.to_le_bytes());
        bus.vram[64 + row * 4..68 + row * 4].copy_from_slice(&0x9999_9999_u32.to_le_bytes());
    }
    for tile in 0..32 {
        bus.vram[0xF000 + tile * 2..0xF002 + tile * 2]
            .copy_from_slice(&((1 | (3 << 12)) as u16).to_le_bytes());
        bus.vram[0xF800 + tile * 2..0xF802 + tile * 2]
            .copy_from_slice(&((2 | (4 << 12)) as u16).to_le_bytes());
    }
    for index in 1..9 {
        bus.palette[(48 + index) * 2..(49 + index) * 2]
            .copy_from_slice(&((index * 2) as u16).to_le_bytes());
    }
    bus.palette[49 * 2..49 * 2 + 2].copy_from_slice(&0x001F_u16.to_le_bytes());
    bus.palette[73 * 2..73 * 2 + 2].copy_from_slice(&0x03E0_u16.to_le_bytes());
    bus.elapse(960);
    assert_eq!(bus.framebuffer()[0], 0x001F);
    assert!(!bus.line_needs_second_surface);

    bus.palette[49 * 2..49 * 2 + 2].copy_from_slice(&0x7C00_u16.to_le_bytes());
    bus.bldcnt = 1 | (1 << 6) | (1 << 9);
    bus.bldalpha = 8 | (8 << 8);
    bus.scanline_memo.invalidate();
    bus.elapse(1232 + 960 - bus.now());
    assert!(bus.line_needs_second_surface);
    for x in 0..240 {
        let expected = if x % 8 == 0 { 0x3DE0 } else { (x % 8 + 1) as u16 | 0x01E0 };
        let at = 240 + x;
        assert_eq!(bus.framebuffer()[at], expected, "x={x}");
        // Line 1 has no raster history: the layer-major compositor drew it.
        let ((_, priority, layer), semi, second) = bus.composed_surfaces(x);
        assert_eq!((priority, layer, semi), (0, 0, false));
        assert_eq!(second, (0x03E0, 1, 1));
    }
    bus.elapse(2 * 1232 + 399 - bus.now());
    write_half(&mut bus, 0x0500_0000 + 49 * 2, 0x001F);
    assert_eq!(bus.now(), 2 * 1232 + 400);
    bus.elapse(3 * 1232 + 960 - bus.now());
    for line in [2, 3] {
        for x in 0..240 {
            let expected = if x % 8 == 0 {
                if line == 2 && x < 100 { 0x3DE0 } else { 0x01EF }
            } else {
                (x % 8 + 1) as u16 | 0x01E0
            };
            let at = line * 240 + x;
            assert_eq!(bus.framebuffer()[at], expected, "line={line} x={x}");
            if line == 2 {
                // The palette write gives line 2 raster history: the
                // front-to-back reference drew it into the frame surfaces.
                assert_eq!(bus.frame_second_pixels[at], 0x03E0);
                assert_eq!(
                    (
                        bus.frame_priorities[at],
                        bus.frame_layers[at],
                        bus.frame_second_priorities[at],
                        bus.frame_second_layers[at],
                        bus.frame_semitransparent[at]
                    ),
                    (0, 0, 1, 1, false)
                );
            } else {
                let ((_, priority, layer), semi, second) = bus.composed_surfaces(x);
                assert_eq!((priority, layer, semi), (0, 0, false));
                assert_eq!(second, (0x03E0, 1, 1));
            }
        }
    }
}

/// ZiGBA: "cached text colors preserve palette raster history and portable state"
#[test]
fn cached_text_colors_preserve_palette_raster_history_and_portable_state() {
    let rom = [0xFE, 0xFF, 0xFF, 0xEA];
    let mut machine = Machine::new(Config { rom: &rom, ..Config::default() });
    let bus = &mut machine.bus;
    bus.dispcnt = 1 << 8;
    bus.bg_controls[0] = 31 << 8;
    bus.vram[0x20..0x40].fill(0x11);
    for entry in 0..1024 {
        bus.vram[0xF800 + entry * 2..0xF802 + entry * 2].copy_from_slice(&1_u16.to_le_bytes());
    }
    bus.palette[2..4].copy_from_slice(&0x001F_u16.to_le_bytes());
    bus.elapse(Machine::CYCLES_PER_FRAME + 399);
    write_half(bus, 0x0500_0002, 0x03E0);
    bus.elapse(560);
    for x in 0..240 {
        assert_eq!(bus.framebuffer()[x], if x < 100 { 0x001F } else { 0x03E0 });
    }
    bus.elapse(2 * Machine::CYCLES_PER_FRAME - 960);
    for &pixel in bus.framebuffer() {
        assert_eq!(pixel, 0x03E0);
    }
    let state = machine.write_state_alloc().unwrap();
    let bus = &mut machine.bus;
    bus.palette[2..4].copy_from_slice(&0x7C00_u16.to_le_bytes());
    bus.scanline_memo.invalidate();
    bus.elapse(Machine::CYCLES_PER_FRAME);
    for &pixel in bus.framebuffer() {
        assert_eq!(pixel, 0x7C00);
    }
    machine.load_state(&state).unwrap();
    machine.bus.elapse(Machine::CYCLES_PER_FRAME);
    for &pixel in machine.framebuffer() {
        assert_eq!(pixel, 0x03E0);
    }
    for x in 0..240 {
        let (top, semi, _) = machine.bus.composed_surfaces(x);
        assert_eq!((top, semi), ((0x03E0, 0, 0), false));
    }
}

/// ZiGBA: "selected frames preserve affine mosaic raster histories DMA and durable state"
#[test]
fn selected_frames_preserve_affine_mosaic_raster_histories_dma_and_durable_state() {
    let rom = [0xFE, 0xFF, 0xFF, 0xEA];
    let mut full = Machine::new(Config { rom: &rom, ..Config::default() });
    let mut selected = Machine::new(Config { rom: &rom, ..Config::default() });
    for machine in [&mut full, &mut selected] {
        let bus = &mut machine.bus;
        bus.dispcnt = 4 | (1 << 10) | (1 << 13);
        bus.bg_controls[2] = 1 << 6;
        bus.mosaic = 3 | (1 << 4);
        bus.bg_affine_parameters[0] = [0x100, 0x80, 0, 0x100];
        bus.win0h = (16 << 8) | 224;
        bus.win0v = 160;
        bus.winin = (1 << 2) | (1 << 5);
        bus.winout = 1 << 5;
        bus.ppu_timing.dispstat_control = (80 << 8) | (7 << 3);
        bus.timers[0].counter = 0xFF00;
        bus.timers[0].reload = 0xFF00;
        bus.timers[0].control = 0xC0;
        bus.ewram.fill(2);
        bus.dma_channels[0].source = 0x0200_0000;
        bus.dma_channels[0].destination = 0x0600_0100;
        bus.dma_channels[0].latched_source = 0x0200_0000;
        bus.dma_channels[0].latched_destination = 0x0600_0100;
        bus.dma_channels[0].word_count = 1;
        bus.dma_channels[0].control = (1 << 15) | (2 << 12) | (1 << 9) | (2 << 5);
        for index in 0..256 {
            bus.palette[index * 2..index * 2 + 2]
                .copy_from_slice(&(((index * 73 + 0x1234) & 0x7FFF) as u16).to_le_bytes());
        }
        for (index, pixel) in bus.vram[..240 * 160].iter_mut().enumerate() {
            *pixel = (index % 4 + 1) as u8;
        }
    }
    selected.bus.set_video_interval(3);
    let visible_end = 159 * 1232 + 960;
    for frame_index in 0..8 {
        let start = frame_index * Machine::CYCLES_PER_FRAME;
        let writes = [
            (319, 0x0500_0002, (0x0100 + frame_index) as u16),
            (639, 0x0600_0000, 0x0303),
            (8 * 1232 + 319, 0x0400_0040, (32 << 8) | 208),
            (9 * 1232 + 479, 0x0400_004C, 2 | (2 << 4)),
            (10 * 1232 + 479, 0x0400_0020, 0x80),
            (20 * 1232 + 399, 0x0400_0028, 0x300),
            (21 * 1232 + 399, 0x0400_0018, 3),
            (22 * 1232 + 399, 0x0400_0050, (1 << 2) | (2 << 6)),
        ];
        for (at, address, value) in writes {
            for machine in [&mut full, &mut selected] {
                machine.bus.elapse(start + at - machine.now());
                write_half(&mut machine.bus, address, value);
            }
        }
        for machine in [&mut full, &mut selected] {
            machine.bus.elapse(start + visible_end - machine.now());
        }
        assert_eq!(full.now(), selected.now());
        assert_eq!(full.bus.bg_affine_current, selected.bus.bg_affine_current);
        assert_eq!(full.bus.ppu_timing, selected.bus.ppu_timing);
        for (a, b) in full.bus.dma_channels.iter().zip(selected.bus.dma_channels.iter()) {
            assert_eq!(
                (
                    a.source,
                    a.destination,
                    a.latched_source,
                    a.latched_destination,
                    a.word_count,
                    a.control
                ),
                (
                    b.source,
                    b.destination,
                    b.latched_source,
                    b.latched_destination,
                    b.word_count,
                    b.control
                )
            );
        }
        assert_eq!(full.bus.irq.interrupt_request_flags, selected.bus.irq.interrupt_request_flags);
        let rendered = frame_index == 0 || (frame_index - 1) % 3 == 0;
        assert_eq!(selected.bus.is_framebuffer_canonical(), rendered);
        if rendered {
            assert_eq!(full.framebuffer(), selected.framebuffer());
            assert_eq!(full.framebuffer(), selected.bus.published_framebuffer());
            assert_eq!(selected.bus.video_frame_cycle, start + visible_end);
            assert_eq!(full.write_state_alloc().unwrap(), selected.write_state_alloc().unwrap());
        }
    }
    let state = selected.write_state_alloc().unwrap();
    selected.load_state(&state).unwrap();
    assert!(selected.bus.is_framebuffer_canonical());
    assert_eq!(selected.framebuffer().as_ptr(), selected.bus.published_framebuffer().as_ptr());
    assert_eq!(selected.bus.video_frame_id, 0);
    assert_eq!(
        selected.bus.next_rendered_frame_cycle(),
        8 * Machine::CYCLES_PER_FRAME + visible_end
    );
}

/// ZiGBA: "complete mixed text rows retain layered surfaces through masks flips and semi OBJ"
#[test]
fn complete_mixed_text_rows_retain_layered_surfaces_through_masks_flips_and_semi_obj() {
    let rows: [[u8; 8]; 4] = [
        [1, 2, 3, 4, 5, 6, 7, 8],
        [1, 0, 2, 0, 3, 0, 4, 0],
        [0, 5, 6, 0, 0, 7, 0, 8],
        [9, 10, 0, 11, 0, 12, 13, 0],
    ];
    let vertical_row = [0_u8, 14, 15, 0, 1, 0, 2, 3];
    let foreground: [[u8; 8]; 4] = [
        [9, 10, 0, 11, 0, 12, 13, 0],
        [0, 13, 12, 0, 11, 0, 10, 9],
        [0, 14, 15, 0, 1, 0, 2, 3],
        [3, 2, 0, 1, 0, 15, 14, 0],
    ];
    let under_entries = [33, 53, 54, 20, 35, 55, 36, 56];
    let under_layers = [2, 1, 1, 3, 2, 1, 2, 1];
    let under_priorities = [2, 0, 0, 3, 2, 0, 2, 0];
    let behind_entries = [17, 18, 34, 0, 21, 22, 23, 24];
    let behind_layers = [3, 3, 2, 5, 3, 3, 3, 3];
    let behind_priorities = [3, 3, 2, 4, 3, 3, 3, 3];
    for second_surface in [false, true] {
        for masked in [false, true] {
            let mut bus = Bus::init(&[]);
            bus.dispcnt = 0x0F00;
            for (background, priority) in [0, 0, 2, 3].into_iter().enumerate() {
                bus.bg_controls[background] = priority | (((28 + background) as u16) << 8);
            }
            for entry in 0..128 {
                bus.palette[entry * 2..entry * 2 + 2]
                    .copy_from_slice(&((0x8300 + entry * 0x41) as u16).to_le_bytes());
            }
            bus.palette[0..2].copy_from_slice(&0x0246_u16.to_le_bytes());
            for (tile, row) in rows.iter().enumerate() {
                for pair in 0..4 {
                    bus.vram[(tile + 1) * 32 + pair] = row[pair * 2] | (row[pair * 2 + 1] << 4);
                }
            }
            for pair in 0..4 {
                bus.vram[4 * 32 + 7 * 4 + pair] =
                    vertical_row[pair * 2] | (vertical_row[pair * 2 + 1] << 4);
            }
            for tile_x in 0..4 {
                for (offset, entry) in [
                    (0xF800, 1 | (1 << 12)),
                    (0xF000, 2 | (2 << 12)),
                    (0xE800, 3 | (3 << 12)),
                    (0xE000, 4 | (tile_x << 10) | ((4 + tile_x) << 12)),
                ] {
                    bus.vram[offset + tile_x * 2..offset + tile_x * 2 + 2]
                        .copy_from_slice(&(entry as u16).to_le_bytes());
                }
            }
            if masked {
                bus.dispcnt |= 1 << 13;
                bus.win0h = (3 << 8) | 29;
                bus.win0v = 160;
                bus.winin = 0x3F;
                bus.winout = 0x3E;
            }
            if second_surface {
                bus.bldcnt = 1 | (1 << 6) | (0x2F << 8);
                bus.bldalpha = 16;
                bus.dispcnt |= 1 << 12;
                for object in 0..128 {
                    bus.oam[object * 8..object * 8 + 2]
                        .copy_from_slice(&(1_u16 << 9).to_le_bytes());
                }
                bus.oam[0..2].copy_from_slice(&(1_u16 << 10).to_le_bytes());
                bus.oam[2..4].copy_from_slice(&16_u16.to_le_bytes());
                bus.oam[4..6].copy_from_slice(&1_u16.to_le_bytes());
                bus.vram[0x10020..0x10024].copy_from_slice(&0x1010_1010_u32.to_le_bytes());
                bus.palette[0x202..0x204].copy_from_slice(&0x4210_u16.to_le_bytes());
            }
            bus.elapse(960);
            assert_eq!(bus.line_needs_second_surface, second_surface);
            for x in 0..32 {
                let lane = x % 8;
                let index = foreground[x / 8][lane] as usize;
                let visible = index != 0 && (!masked || (3..29).contains(&x));
                let top_entry =
                    if visible { (4 + x / 8) * 16 + index } else { under_entries[lane] };
                let mut top_color = (0x300 + top_entry * 0x41) as u16;
                let mut top_priority = if visible { 0 } else { under_priorities[lane] };
                let mut top_layer = if visible { 0 } else { under_layers[lane] };
                let second_entry = if visible { under_entries[lane] } else { behind_entries[lane] };
                let mut second_color =
                    if second_entry == 0 { 0x0246 } else { (0x300 + second_entry * 0x41) as u16 };
                let mut second_priority =
                    if visible { under_priorities[lane] } else { behind_priorities[lane] };
                let mut second_layer =
                    if visible { under_layers[lane] } else { behind_layers[lane] };
                let semi_obj = second_surface && (16..24).contains(&x) && x % 2 == 1;
                if semi_obj {
                    second_color = top_color;
                    second_priority = top_priority;
                    second_layer = top_layer;
                    top_color = 0x4210;
                    top_priority = 0;
                    top_layer = 4;
                }
                assert_eq!(
                    bus.framebuffer()[x],
                    top_color,
                    "x={x} second={second_surface} masked={masked}"
                );
                let ((_, priority, layer), semi, second) = bus.composed_surfaces(x);
                assert_eq!((priority, layer, semi), (top_priority, top_layer, semi_obj));
                // Only blending lines track the second surface.
                if second_surface {
                    assert_eq!(second, (second_color, second_priority, second_layer));
                }
            }
        }
    }
}
