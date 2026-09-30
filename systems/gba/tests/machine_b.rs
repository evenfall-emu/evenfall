//! Second half of the machine integration oracles. The expected values are
//! fixed oracles, not calculated from the Rust implementation.

use evenfall_gba::bus::{
    Access, AccessKind, AccessSource, AccessWidth, InstructionFetchRegion, JoyBusCommand,
    PowerState, SioConnection,
};
use evenfall_gba::cpu::{Cpu, FaultReason, HleMemoryOperation, InstructionSet};
use evenfall_gba::machine::{
    BootMode, Config, Fault, LinkedStopReason, Machine, StateError, StepOutcome, StepResult,
    run_linked_until,
};
use evenfall_state::Error as CodecError;

fn machine(rom: &[u8]) -> Box<Machine> {
    Machine::new(Config { rom, ..Config::default() })
}

fn executed_step(machine: &mut Machine) -> StepResult {
    match machine.step() {
        StepOutcome::Executed(step) => step,
        outcome => panic!("expected executed instruction, got {outcome:?}"),
    }
}

fn faulted_step(machine: &mut Machine) -> Fault {
    match machine.step() {
        StepOutcome::Faulted(fault) => fault,
        outcome => panic!("expected machine fault, got {outcome:?}"),
    }
}

fn write_machine_half(machine: &mut Machine, address: u32, value: u16) {
    machine.bus.write(
        Access {
            address,
            width: AccessWidth::Half,
            kind: AccessKind::Nonsequential,
            source: AccessSource::CpuData,
        },
        u32::from(value),
    );
}

fn read_machine_half(machine: &mut Machine, address: u32) -> u16 {
    machine
        .bus
        .read(Access {
            address,
            width: AccessWidth::Half,
            kind: AccessKind::Nonsequential,
            source: AccessSource::CpuData,
        })
        .value as u16
}

fn write_u32(bytes: &mut [u8], value: u32) {
    bytes[..4].copy_from_slice(&value.to_le_bytes());
}

fn read_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes[..4].try_into().unwrap())
}

fn encode_machine_state(machine: &Machine) -> Vec<u8> {
    let mut bytes = Vec::new();
    machine.write_state(&mut bytes).unwrap();
    bytes
}

#[test]
fn hle_sound_channel_clear_stops_twelve_initialized_software_channels() {
    let rom = [0x00, 0x00, 0x1e, 0xef];
    let mut machine = machine(&rom);
    let sound_area_address = 0x0200_0100;
    write_u32(&mut machine.bus.iwram[0x7ff0..], sound_area_address);
    write_u32(&mut machine.bus.ewram[0x100..], 0x6873_6d53);
    write_u32(&mut machine.bus.ewram[0x11c..], 0);
    for channel in 0..12 {
        let offset = 0x150 + channel * 0x40;
        machine.bus.ewram[offset] = (channel + 1) as u8;
        machine.bus.ewram[offset + 1] = 0xa5;
    }

    executed_step(&mut machine);

    assert_eq!(read_u32(&machine.bus.ewram[0x100..]), 0x6873_6d53);
    for channel in 0..12 {
        let offset = 0x150 + channel * 0x40;
        assert_eq!(machine.bus.ewram[offset], 0);
        assert_eq!(machine.bus.ewram[offset + 1], 0xa5);
    }
    assert_eq!(machine.read_register(0), sound_area_address + 0x350);
    assert_eq!(machine.read_register(1), 0);
    assert_eq!(machine.read_register(2), 0);
}

#[test]
fn hle_sound_channel_clear_leaves_an_invalid_sound_area_untouched() {
    let rom = [0x00, 0x00, 0x1e, 0xef];
    let mut machine = machine(&rom);
    write_u32(&mut machine.bus.iwram[0x7ff0..], 0x0200_0100);
    write_u32(&mut machine.bus.ewram[0x100..], 0x6873_6d54);
    machine.bus.ewram[0x150] = 0x7f;
    machine.cpu.registers[1] = 0x1111_1111;
    machine.cpu.registers[2] = 0x2222_2222;

    executed_step(&mut machine);

    assert_eq!(machine.bus.ewram[0x150], 0x7f);
    assert_eq!(machine.read_register(0), 0x6873_6d54);
    assert_eq!(machine.read_register(1), 0x1111_1111);
    assert_eq!(machine.read_register(2), 0x2222_2222);
}

/// ZiGBA: `save-state v6 byte schema is locked by a deterministic fixture`.
#[test]
fn save_state_v6_byte_schema_is_locked_by_a_deterministic_fixture() {
    // Exact canonical bytes, not just a round trip. This digest and length
    // come from ZiGBA's fixture in test/machine_test.zig:6493.
    let rom = [0xfe, 0xff, 0xff, 0xea];
    let machine = machine(&rom);
    let bytes = encode_machine_state(&machine);
    let expected_digest = [
        0xd2, 0x20, 0x66, 0xfe, 0x98, 0xc2, 0xb0, 0x88, 0x6b, 0x09, 0x77, 0x09, 0x9d, 0x1f, 0x71,
        0x46, 0xe2, 0x56, 0x3f, 0xc8, 0x3a, 0xb7, 0xb1, 0x09, 0xd9, 0x43, 0x5b, 0xbe, 0x52, 0xac,
        0x93, 0xd5,
    ];
    assert_eq!(bytes.len(), 478_558);
    assert_eq!(blake3::hash(&bytes).as_bytes(), &expected_digest);
}

#[test]
fn canonical_save_state_round_trip_resumes_bit_identical_execution() {
    let words: [u32; 3] = [0xe3a0_0005, 0xe283_3001, 0xeaff_fffd];
    let rom: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
    let mut machine = machine(&rom);
    for _ in 0..10 {
        executed_step(&mut machine);
    }
    let bytes = encode_machine_state(&machine);
    // The large arrays are boxed, so the live size includes their
    // allocations rather than only the pointer fields.
    let live_bytes = std::mem::size_of::<Machine>()
        + std::mem::size_of_val(machine.bus.ewram.as_ref())
        + std::mem::size_of_val(machine.bus.vram.as_ref())
        + std::mem::size_of_val(machine.bus.frame_pixels.as_ref())
        + std::mem::size_of_val(machine.bus.frame_priorities.as_ref());
    assert!(bytes.len() < live_bytes);
    let mut resumed = Machine::new(Config { rom: &rom, ..Config::default() });
    resumed.load_state(&bytes).unwrap();
    assert_eq!(bytes, encode_machine_state(&resumed));

    for _ in 0..10 {
        executed_step(&mut machine);
        executed_step(&mut resumed);
    }
    assert_eq!(machine.now(), resumed.now());
    for index in 0..16 {
        assert_eq!(machine.read_register(index), resumed.read_register(index));
    }
    assert_eq!(machine.read_cpsr(), resumed.read_cpsr());
    assert_eq!(machine.next_instruction_address(), resumed.next_instruction_address());
}

/// ZiGBA: `save-state envelope rejects wrong media version corruption truncation and trailing data`.
#[test]
fn save_state_envelope_rejects_wrong_media_version_corruption_truncation_and_trailing_data() {
    let rom = [0xfe, 0xff, 0xff, 0xea];
    let mut source = machine(&rom);
    for _ in 0..4 {
        executed_step(&mut source);
    }
    let bytes = encode_machine_state(&source);

    let mut other_rom = rom;
    other_rom[0] ^= 1;
    let mut wrong_rom = machine(&other_rom);
    let wrong_rom_before = encode_machine_state(&wrong_rom);
    assert!(matches!(
        wrong_rom.load_state(&bytes),
        Err(StateError::Codec(CodecError::SaveStateRomMismatch))
    ));
    assert_eq!(wrong_rom_before, encode_machine_state(&wrong_rom));

    let mut corrupt = bytes.clone();
    corrupt[Machine::STATE_HEADER_SIZE + 8] ^= 0x40;
    let mut target = machine(&rom);
    let target_before = encode_machine_state(&target);
    assert!(matches!(
        target.load_state(&corrupt),
        Err(StateError::Codec(CodecError::InvalidSaveState))
    ));
    assert_eq!(target_before, encode_machine_state(&target));
    assert!(matches!(
        target.load_state(&bytes[..bytes.len() - 1]),
        Err(StateError::Codec(CodecError::InvalidSaveState))
    ));
    let mut appended = bytes.clone();
    appended.push(0);
    assert!(matches!(
        target.load_state(&appended),
        Err(StateError::Codec(CodecError::InvalidSaveState))
    ));
    let mut old_version = bytes.clone();
    old_version[8..12].copy_from_slice(&4u32.to_le_bytes());
    assert!(matches!(
        target.load_state(&old_version),
        Err(StateError::Codec(CodecError::UnsupportedSaveStateVersion))
    ));
    let mut wrong_schema = bytes;
    wrong_schema[12] ^= 1;
    assert!(matches!(
        target.load_state(&wrong_schema),
        Err(StateError::Codec(CodecError::SaveStateSchemaMismatch))
    ));
}

#[test]
fn save_state_writer_rejects_impossible_ppu_phases_past_deadlines_and_incomplete_joy_bus() {
    let rom = [0xfe, 0xff, 0xff, 0xea];
    for case in 0..9 {
        let mut machine = machine(&rom);
        machine.bus.elapse(10);
        let past = machine.now() - 1;
        match case {
            0 => machine.bus.sio_completion_cycle = Some(past),
            1 => machine.bus.uart_tx_completion_cycle = Some(past),
            2 => machine.bus.uart_rx_arrival_cycle = Some(past),
            3 => {
                machine.bus.joy_pending_command = Some(JoyBusCommand::Status);
                machine.bus.joy_completion_cycle = Some(past);
            }
            4 => machine.bus.dma_start_cycles[2] = Some(past),
            5 => machine.bus.joy_completion_cycle = Some(machine.now() + 10),
            6 => machine.bus.joy_pending_command = Some(JoyBusCommand::Status),
            7 => machine.bus.ppu_timing.line = 228,
            8 => machine.bus.ppu_timing.cycle_in_line = 1232,
            _ => unreachable!(),
        }
        assert!(
            matches!(
                machine.write_state(&mut Vec::new()),
                Err(StateError::Codec(CodecError::InvalidSaveState))
            ),
            "case {case}"
        );
    }
}

/// ZiGBA: `save-state envelope binds BIOS and boot mode`.
#[test]
fn save_state_envelope_binds_bios_and_boot_mode() {
    let rom = [0xfe, 0xff, 0xff, 0xea];
    let bios_a = [0u8; 0x4000];
    let mut bios_b = bios_a;
    bios_b[0x123] = 1;
    let source = Machine::new(Config {
        rom: &rom,
        bios: &bios_a,
        boot: BootMode::SkipBios,
        ..Config::default()
    });
    let bytes = encode_machine_state(&source);
    let mut wrong_bios = Machine::new(Config {
        rom: &rom,
        bios: &bios_b,
        boot: BootMode::SkipBios,
        ..Config::default()
    });
    assert!(matches!(
        wrong_bios.load_state(&bytes),
        Err(StateError::Codec(CodecError::SaveStateBiosMismatch))
    ));
    let mut wrong_boot = Machine::new(Config {
        rom: &rom,
        bios: &bios_a,
        boot: BootMode::ExternalBios,
        ..Config::default()
    });
    assert!(matches!(
        wrong_boot.load_state(&bytes),
        Err(StateError::Codec(CodecError::SaveStateBootModeMismatch))
    ));
}

#[test]
fn save_state_load_drops_already_produced_host_audio_samples() {
    let rom = [0xfe, 0xff, 0xff, 0xea];
    let mut source = machine(&rom);
    source.run_until(5_000).unwrap();
    let bytes = encode_machine_state(&source);
    let mut queued = [evenfall_audio::StereoSample::default(); 32];
    assert!(source.drain_audio_samples(&mut queued) > 0);
    let mut resumed = machine(&rom);
    resumed.load_state(&bytes).unwrap();
    assert_eq!(resumed.drain_audio_samples(&mut queued), 0);
}

#[test]
fn save_state_load_transactionally_rejects_a_digest_valid_impossible_ppu_phase() {
    let rom = [0xfe, 0xff, 0xff, 0xea];
    let baseline = machine(&rom);
    let baseline_bytes = encode_machine_state(&baseline);
    let mut marker = machine(&rom);
    marker.bus.ppu_timing.cycle_in_line = 1;
    let marker_bytes = encode_machine_state(&marker);
    let payload_len = u64::from_le_bytes(baseline_bytes[44..52].try_into().unwrap()) as usize;
    let payload_start = Machine::STATE_HEADER_SIZE;
    let payload_end = payload_start + payload_len;
    assert_eq!(baseline_bytes.len(), payload_end + 32);
    assert_eq!(baseline_bytes.len(), marker_bytes.len());

    // Discover the field from two independently valid public states instead
    // of baking in a private payload offset.
    let offsets: Vec<_> = (payload_start..payload_end)
        .filter(|&offset| baseline_bytes[offset] != marker_bytes[offset])
        .collect();
    assert_eq!(offsets.len(), 1);
    let offset = offsets[0];
    assert_eq!(baseline_bytes[offset], 0);
    assert_eq!(marker_bytes[offset], 1);
    let mut malformed = baseline_bytes;
    malformed[offset..offset + 2].copy_from_slice(&1232u16.to_le_bytes());
    let digest = blake3::hash(&malformed[payload_start..payload_end]);
    malformed[payload_end..].copy_from_slice(digest.as_bytes());

    let mut target = machine(&rom);
    executed_step(&mut target);
    let target_before = encode_machine_state(&target);
    assert!(matches!(
        target.load_state(&malformed),
        Err(StateError::Codec(CodecError::InvalidSaveState))
    ));
    assert_eq!(target_before, encode_machine_state(&target));
}

#[test]
fn save_state_round_trip_keeps_the_prescaler_counter_and_rejects_a_timer_phase_off_it() {
    let rom = [0xfe, 0xff, 0xff, 0xea];
    let baseline_bytes = encode_machine_state(&machine(&rom));
    let mut marker = machine(&rom);
    marker.bus.prescaler_origin = 1;
    let marker_bytes = encode_machine_state(&marker);
    assert_eq!(baseline_bytes.len(), marker_bytes.len());
    let payload_start = Machine::STATE_HEADER_SIZE;
    let payload_end = baseline_bytes.len() - 32;
    let offsets: Vec<_> = (payload_start..payload_end)
        .filter(|&offset| baseline_bytes[offset] != marker_bytes[offset])
        .collect();
    assert_eq!(offsets.len(), 1);
    let offset = offsets[0];

    let mut source = machine(&rom);
    source.bus.prescaler_origin = 37;
    write_machine_half(&mut source, 0x0400_0100, 0xfff0);
    write_machine_half(&mut source, 0x0400_0102, 0x0082);
    let bytes = encode_machine_state(&source);
    assert_eq!(baseline_bytes.len(), bytes.len());
    assert_eq!(bytes[offset], 37);
    let mut resumed = machine(&rom);
    resumed.load_state(&bytes).unwrap();
    for machine in [&mut source, &mut resumed] {
        write_machine_half(machine, 0x0400_0104, 0xfffe);
        write_machine_half(machine, 0x0400_0106, 0x0083);
        machine.run_until(machine.now() + 5_000).unwrap();
    }
    assert_eq!(encode_machine_state(&source), encode_machine_state(&resumed));

    let mut forged = bytes;
    forged[offset] = 38;
    let digest = blake3::hash(&forged[payload_start..payload_end]);
    forged[payload_end..].copy_from_slice(digest.as_bytes());
    let mut target = machine(&rom);
    assert!(matches!(
        target.load_state(&forged),
        Err(StateError::Codec(CodecError::InvalidSaveState))
    ));
}

#[test]
fn hle_sound_driver_v_sync_only_decrements_a_countdown_above_one() {
    let rom = [0, 0, 0x1d, 0xef];
    let mut machine = machine(&rom);
    write_u32(&mut machine.bus.iwram[0x7ff0..], 0x0200_0100);
    write_u32(&mut machine.bus.ewram[0x100..], 0x6873_6d53);
    machine.bus.ewram[0x104] = 2;
    machine.bus.ewram[0x10b] = 7;
    machine.bus.dma_channels[1].control = 0xa600;
    machine.bus.dma_channels[2].control = 0xa600;
    executed_step(&mut machine);
    assert_eq!(machine.bus.ewram[0x104], 1);
    assert_eq!(machine.bus.dma_channels[1].control, 0xa600);
    assert_eq!(machine.bus.dma_channels[2].control, 0xa600);
    assert_eq!(
        (0..4).map(|i| machine.read_register(i)).collect::<Vec<_>>(),
        [0x0200_0100, 1, 0x6873_6d53, 0x6873_6d53]
    );
}

#[test]
fn hle_sound_driver_v_sync_reloads_zero_and_one_countdowns_and_resets_dma() {
    let rom = [0, 0, 0x1d, 0xef];
    for countdown in [0, 1] {
        let mut machine = machine(&rom);
        write_u32(&mut machine.bus.iwram[0x7ff0..], 0x0200_0100);
        write_u32(&mut machine.bus.ewram[0x100..], 0x6873_6d53);
        machine.bus.ewram[0x104] = countdown;
        machine.bus.ewram[0x10b] = 7;
        machine.bus.dma_channels[1].control = 0xa600;
        machine.bus.dma_channels[2].control = 0xa600;
        executed_step(&mut machine);
        assert_eq!(machine.bus.ewram[0x104], 7);
        assert_eq!(machine.bus.dma_channels[1].control, 0xb600);
        assert_eq!(machine.bus.dma_channels[2].control, 0xb600);
        assert_eq!(
            (0..4).map(|i| machine.read_register(i)).collect::<Vec<_>>(),
            [0, 0xb600, 0x0400_00c6, 0x0400_00d2]
        );
    }
}

#[test]
fn hle_sound_driver_v_sync_rejects_an_invalid_sound_area_identifier() {
    let rom = [0, 0, 0x1d, 0xef];
    let mut machine = machine(&rom);
    write_u32(&mut machine.bus.iwram[0x7ff0..], 0x0200_0100);
    write_u32(&mut machine.bus.ewram[0x100..], 0x6873_6d54);
    machine.bus.ewram[0x104] = 2;
    machine.cpu.registers[1] = 0x1111_1111;
    executed_step(&mut machine);
    assert_eq!(machine.bus.ewram[0x104], 2);
    assert_eq!(
        (0..4).map(|i| machine.read_register(i)).collect::<Vec<_>>(),
        [0x0200_0100, 0x1111_1111, 0x6873_6d53, 0x6873_6d54]
    );
}

#[test]
fn hle_sound_driver_v_sync_on_configures_repeating_special_timing_dma() {
    let rom = [0, 0, 0x29, 0xef];
    let mut machine = machine(&rom);
    machine.cpu.registers[2] = 0x2222_2222;
    machine.cpu.registers[3] = 0x3333_3333;
    executed_step(&mut machine);
    assert_eq!(machine.bus.dma_channels[1].control, 0xb600);
    assert_eq!(machine.bus.dma_channels[2].control, 0xb600);
    assert_eq!(
        (0..4).map(|i| machine.read_register(i)).collect::<Vec<_>>(),
        [0x0400_00c0, 0xb600, 0x2222_2222, 0x3333_3333]
    );
}

#[test]
fn hle_custom_halt_selects_halt_or_stop_from_r2_bit_seven() {
    let rom = [0, 0, 0x27, 0xef];
    let mut halt = machine(&rom);
    halt.cpu.registers[0..3].copy_from_slice(&[0x1111_1111, 0x2222_2222, 0x3333_3300]);
    executed_step(&mut halt);
    assert_eq!(halt.bus.power_state(), PowerState::Halted);
    assert_eq!(
        (0..3).map(|i| halt.read_register(i)).collect::<Vec<_>>(),
        [0x1111_1111, 0x2222_2222, 0x3333_3300]
    );
    let mut stop = machine(&rom);
    stop.cpu.registers[2] = 0x80;
    executed_step(&mut stop);
    assert_eq!(stop.bus.power_state(), PowerState::Stopped);
}

#[test]
fn hle_div_and_div_arm_return_quotient_remainder_and_absolute_quotient_including_bios_edges() {
    // GBATEK SWI 06h/07h: quotient, remainder, |quotient|. The 0/0 and
    // INT_MIN/-1 results are the retail BIOS oracles.
    let cases = [
        (6, 100, 7, 14, 2, 14),
        (7, 7, 100, 14, 2, 14),
        (6, (-100i32) as u32, 7, (-14i32) as u32, (-2i32) as u32, 14),
        (6, 0x8000_0000, 0xffff_ffff, 0x8000_0000, 0, 0x8000_0000),
        (6, 0, 0, 1, 0, 1),
    ];
    for (swi, r0, r1, quotient, remainder, absolute) in cases {
        let rom = [0, 0, swi, 0xef];
        let mut machine = machine(&rom);
        machine.cpu.registers[0] = r0;
        machine.cpu.registers[1] = r1;
        executed_step(&mut machine);
        assert_eq!(machine.read_register(0), quotient);
        assert_eq!(machine.read_register(1), remainder);
        assert_eq!(machine.read_register(3), absolute);
    }
}

#[test]
fn hle_get_bios_checksum_returns_the_retail_bios_identity_tuple() {
    let rom = [0, 0, 0x0d, 0xef];
    let mut machine = machine(&rom);
    machine.cpu.registers[2] = 0xef0d_0000;
    executed_step(&mut machine);
    assert_eq!(
        (0..4).map(|i| machine.read_register(i)).collect::<Vec<_>>(),
        [0xbaae_187f, 1, 0xef0d_0000, 0x4000]
    );
}

#[test]
fn hle_cpu_set_copies_halfwords_or_words_and_writes_back_only_word_address_registers() {
    let rom = [0, 0, 0x0b, 0xef];
    for word in [false, true] {
        let mut machine = machine(&rom);
        let source = [0x11, 0, 0x22, 0, 0x33, 0, 0x44, 0];
        machine.bus.ewram[..source.len()].copy_from_slice(&source);
        machine.cpu.registers[0] = 0x0200_0000;
        machine.cpu.registers[1] = 0x0300_0000;
        machine.cpu.registers[2] = if word { 2 | (1 << 26) } else { 3 };
        executed_step(&mut machine);
        assert_eq!(&machine.bus.iwram[..6], &source[..6]);
        assert_eq!(&machine.bus.iwram[6..8], if word { &[0x44, 0][..] } else { &[0, 0][..] });
        assert_eq!(machine.read_register(0), if word { 0x0200_0008 } else { 0x0200_0000 });
        assert_eq!(machine.read_register(1), if word { 0x0300_0008 } else { 0x0300_0000 });
        assert_eq!(machine.read_register(3), 0x170);
    }
}

#[test]
fn hle_bit_un_pack_extracts_low_bits_first_and_applies_its_conditional_bias() {
    for (bias, expected) in [
        (0x8000_0010u32, [16, 17, 18, 19, 19, 18, 17, 16]),
        (0x0000_0010, [0, 17, 18, 19, 19, 18, 17, 0]),
    ] {
        let mut rom = [0, 0, 0x10, 0xef, 0xe4, 0x1b, 0, 0, 2, 0, 2, 8, 0, 0, 0, 0];
        write_u32(&mut rom[12..], bias);
        let mut machine = machine(&rom);
        machine.cpu.registers[..4].copy_from_slice(&[
            0x0800_0004,
            0x0200_0000,
            0x0800_0008,
            0xdead_beef,
        ]);
        executed_step(&mut machine);
        assert_eq!(&machine.bus.ewram[..8], &expected);
        assert_eq!(
            (0..4).map(|i| machine.read_register(i)).collect::<Vec<_>>(),
            [0x0800_0006, 0x0200_0008, 0x0800_0008, 0xdead_beef]
        );
    }
}

#[test]
fn hle_lz77_expands_literals_and_overlapping_back_references_to_wram_and_vram() {
    let expected = b"ABCABCABCX";
    for (swi, destination, vram) in [(0x11, 0x0200_0000, false), (0x12, 0x0600_0000, true)] {
        let rom = [0, 0, swi, 0xef, 0x10, 0x0a, 0, 0, 0x10, b'A', b'B', b'C', 0x30, 2, b'X'];
        let mut machine = machine(&rom);
        machine.cpu.registers[0] = 0x0800_0004;
        machine.cpu.registers[1] = destination;
        machine.cpu.registers[3] = 0xdead_beef;
        executed_step(&mut machine);
        let output = if vram { &machine.bus.vram[..10] } else { &machine.bus.ewram[..10] };
        assert_eq!(output, expected);
        assert_eq!(machine.read_register(0), 0x0800_000f);
        assert_eq!(machine.read_register(1), destination + 10);
        assert_eq!(machine.read_register(3), 0);
    }
}

#[test]
fn hle_lz77_consumes_a_new_control_byte_after_every_eight_tokens() {
    let rom = [
        0, 0, 0x11, 0xef, 0x10, 9, 0, 0, 0, b'1', b'2', b'3', b'4', b'5', b'6', b'7', b'8', 0, b'9',
    ];
    let mut machine = machine(&rom);
    machine.cpu.registers[0] = 0x0800_0004;
    machine.cpu.registers[1] = 0x0200_0000;
    executed_step(&mut machine);
    assert_eq!(&machine.bus.ewram[..9], b"123456789");
    assert_eq!(machine.read_register(0), 0x0800_0013);
    assert_eq!(machine.read_register(1), 0x0200_0009);
    assert_eq!(machine.read_register(3), 0);
}

#[test]
fn hle_run_length_decompression_handles_literal_and_repeated_blocks_in_wram_and_vram() {
    let expected = b"AAAAABCDEFZZZZ\0\0";
    for (swi, destination, vram) in [(0x14, 0x0200_0000, false), (0x15, 0x0600_0000, true)] {
        let rom = [
            0, 0, swi, 0xef, 0x30, 0x0e, 0, 0, 0x82, b'A', 4, b'B', b'C', b'D', b'E', b'F', 0x85,
            b'Z',
        ];
        let mut machine = machine(&rom);
        machine.cpu.registers[0] = 0x0800_0004;
        machine.cpu.registers[1] = destination;
        machine.cpu.registers[3] = 0xdead_beef;
        executed_step(&mut machine);
        let output = if vram { &machine.bus.vram[..16] } else { &machine.bus.ewram[..16] };
        assert_eq!(output, expected);
        assert_eq!(machine.read_register(0), 0x0800_0012);
        assert_eq!(machine.read_register(1), destination + 16);
        assert_eq!(machine.read_register(3), 0xdead_beef);
    }
}

#[test]
fn hle_run_length_vram_output_from_an_odd_destination_ends_its_padding() {
    for (length, expected) in
        [(4, [0, b'A', b'B', b'C', 0xee, 0xee]), (2, [0, b'A', 0xee, 0xee, 0, 0])]
    {
        let rom = [0, 0, 0x15, 0xef, 0x30, length, 0, 0, 3, b'A', b'B', b'C', b'D'];
        let mut machine = machine(&rom);
        machine.bus.vram[..6].fill(0xee);
        machine.cpu.registers[0] = 0x0800_0004;
        machine.cpu.registers[1] = 0x0600_0001;
        executed_step(&mut machine);
        assert_eq!(&machine.bus.vram[..6], &expected);
        assert_eq!(machine.read_register(1), 0x0600_0006);
    }
}

#[test]
fn hle_8_bit_differential_filters_accumulate_deltas_to_wram_and_packed_vram_output() {
    for (swi, destination, vram) in [(0x16, 0x0200_0000, false), (0x17, 0x0600_0000, true)] {
        let rom = [0, 0, swi, 0xef, 0x80, 4, 0, 0, 1, 2, 0xff, 5];
        let mut machine = machine(&rom);
        machine.cpu.registers[0] = 0x0800_0004;
        machine.cpu.registers[1] = destination;
        machine.cpu.registers[3] = 0xdead_beef;
        executed_step(&mut machine);
        let output = if vram { &machine.bus.vram[..4] } else { &machine.bus.ewram[..4] };
        assert_eq!(output, [1, 3, 2, 7]);
        assert_eq!(machine.read_register(0), 0x0800_000c);
        assert_eq!(machine.read_register(1), destination + 4);
        assert_eq!(machine.read_register(3), 0xdead_beef);
    }
}

#[test]
fn hle_16_bit_differential_filter_wraps_halfword_deltas() {
    let rom = [0, 0, 0x18, 0xef, 0x80, 6, 0, 0, 1, 0, 2, 0, 0xff, 0xff];
    let mut machine = machine(&rom);
    machine.cpu.registers[0] = 0x0800_0004;
    machine.cpu.registers[1] = 0x0600_0000;
    machine.cpu.registers[3] = 0xdead_beef;
    executed_step(&mut machine);
    assert_eq!(&machine.bus.vram[..6], &[1, 0, 3, 0, 2, 0]);
    assert_eq!(machine.read_register(0), 0x0800_000e);
    assert_eq!(machine.read_register(1), 0x0600_0006);
    assert_eq!(machine.read_register(3), 0xdead_beef);
}

#[test]
fn m_gba_bios_math_table_arc_tan_cases_preserve_fixed_point_bios_intermediates() {
    let rom = [0, 0, 9, 0xef];
    let cases = [
        (0, 0, 0, 0xa2f9),
        (0x3fff, 0x1fff, 0xffff_c002, 0x8001),
        (0x4000, 0x2000, 0xffff_c000, 0x8000),
        (0x4001, 0x1fff, 0xffff_bffe, 0x7ffe),
        (0x8000, 0x16a2, 0xffff_0000, 0x0002_2d45),
        (0xffff, 0xffff_a2fe, 8, 0xa2ff),
        (0xffff_0000, 0x5d07, 0, 0xa2f9),
        (0xffff_c000, 0xffff_e000, 0xffff_c000, 0x8000),
        (0xffff_ffff, 0xffff_ffff, 0, 0xa2f9),
    ];
    for (input, r0, r1, r3) in cases {
        let mut machine = machine(&rom);
        machine.cpu.registers[0] = input;
        machine.cpu.registers[2] = 0xef09_0000;
        let result = executed_step(&mut machine);
        assert_eq!(
            (0..4).map(|i| machine.read_register(i)).collect::<Vec<_>>(),
            [r0, r1, 0xef09_0000, r3]
        );
        assert_eq!(machine.read_cpsr(), 0x1f);
        if input == 0 {
            assert_eq!(result.stopped_at, 53);
        }
    }
}

#[test]
fn m_gba_bios_math_table_arc_tan2_covers_axes_and_every_quadrant() {
    let rom = [0, 0, 0x0a, 0xef];
    let cases = [
        (0, 0, 0, 0),
        (1, 0, 0, 0),
        (0xffff_ffff, 0, 0x8000, 0),
        (0, 1, 0x4000, 1),
        (0, 0xffff_0000, 0xc000, 0xffff_0000),
        (1, 1, 0x2000, 0xffff_c000),
        (0xffff_ffff, 1, 0x6000, 0xffff_c000),
        (0xffff_0000, 0xffff_0000, 0xa000, 0xffff_c000),
        (0x0000_ffff, 0xffff_0000, 0xe000, 0xffff_c002),
        (0x0000_8000, 0x0000_4000, 0x12e4, 0xffff_f000),
        (0xffff_4000, 0x0000_8000, 0x680b, 0xffff_e390),
    ];
    for (x, y, angle, r1) in cases {
        let mut machine = machine(&rom);
        machine.cpu.registers[0] = x;
        machine.cpu.registers[1] = y;
        machine.cpu.registers[2] = 0xef0a_0000;
        let result = executed_step(&mut machine);
        assert_eq!(
            (0..4).map(|i| machine.read_register(i)).collect::<Vec<_>>(),
            [angle, r1, 0xef0a_0000, 0x170]
        );
        assert_eq!(machine.read_cpsr(), 0x1f);
        if x == 0 || y == 0 {
            assert_eq!(result.stopped_at, 19);
        }
    }
}

#[test]
fn hle_bg_affine_set_applies_fixed_point_rotation_and_ignores_the_low_angle_byte() {
    let rom = [0, 0, 0x0e, 0xef];
    let mut machine = machine(&rom);
    for index in 0..2 {
        let offset = index * 20;
        machine.bus.ewram[offset..offset + 4].copy_from_slice(&(100i32 * 256 + 128).to_le_bytes());
        machine.bus.ewram[offset + 4..offset + 8]
            .copy_from_slice(&(50i32 * 256 + 64).to_le_bytes());
        for (at, value) in [(8, 10i16), (10, 20), (12, 0x0100), (14, 0x0200)] {
            machine.bus.ewram[offset + at..offset + at + 2].copy_from_slice(&value.to_le_bytes());
        }
        machine.bus.ewram[offset + 16..offset + 18]
            .copy_from_slice(&(if index == 0 { 0x4000u16 } else { 0x40ff }).to_le_bytes());
    }
    machine.cpu.registers[..4].copy_from_slice(&[0x0200_0000, 0x0300_0000, 2, 0xdead_beef]);
    executed_step(&mut machine);
    for index in 0..2 {
        let offset = index * 16;
        for (at, expected) in [(0, 0i16), (2, -256), (4, 512), (6, 0)] {
            assert_eq!(
                i16::from_le_bytes(
                    machine.bus.iwram[offset + at..offset + at + 2].try_into().unwrap()
                ),
                expected
            );
        }
        assert_eq!(
            i32::from_le_bytes(machine.bus.iwram[offset + 8..offset + 12].try_into().unwrap()),
            30_848
        );
        assert_eq!(
            i32::from_le_bytes(machine.bus.iwram[offset + 12..offset + 16].try_into().unwrap()),
            7_744
        );
    }
    assert_eq!(
        (0..4).map(|i| machine.read_register(i)).collect::<Vec<_>>(),
        [0x0200_0000, 0x0300_0000, 2, 0xdead_beef]
    );
}

#[test]
fn hle_obj_affine_set_writes_consecutive_records_at_the_requested_oam_stride() {
    let rom = [0, 0, 0x0f, 0xef];
    let mut machine = machine(&rom);
    for (at, value) in
        [(0, 0x0100i16), (2, 0x0200), (4, 0), (8, 0x0080), (10, -0x0100), (12, 0x40ff)]
    {
        machine.bus.ewram[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }
    machine.bus.ewram[0x100..0x140].fill(0xcc);
    machine.cpu.registers[..4].copy_from_slice(&[0x0200_0000, 0x0200_0100, 2, 8]);
    executed_step(&mut machine);
    let expected = [256, 0, 0, 512, 0, -128, -256, 0];
    for (index, value) in expected.into_iter().enumerate() {
        let offset = 0x100 + (index / 4) * 32 + (index % 4) * 8;
        assert_eq!(
            i16::from_le_bytes(machine.bus.ewram[offset..offset + 2].try_into().unwrap()),
            value
        );
        assert_eq!(&machine.bus.ewram[offset + 2..offset + 8], &[0xcc; 6]);
    }
    assert_eq!(
        (0..4).map(|i| machine.read_register(i)).collect::<Vec<_>>(),
        [0x0200_0000, 0x0200_0100, 2, 8]
    );
}

#[test]
fn hle_cpu_fast_set_fills_complete_eight_word_bursts() {
    let words: [u32; 11] = [
        0xe3a0_4402,
        0xe3a0_5403,
        0xe3a0_605a,
        0xe584_6000,
        0xe1a0_0004,
        0xe1a0_1005,
        0xe3a0_2001,
        0xe382_2401,
        0xef0c_0000,
        0xe595_7000,
        0xe595_801c,
    ];
    let rom: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
    let mut machine = machine(&rom);
    for _ in words {
        executed_step(&mut machine);
    }
    assert_eq!(machine.read_register(7), 0x5a);
    assert_eq!(machine.read_register(8), 0x5a);
    assert_eq!(machine.read_register(0), 0x0200_0000);
    assert_eq!(machine.read_register(1), 0x0300_0020);
    assert_eq!(machine.read_register(2), 0x0300_0004);
    assert_eq!(machine.read_register(3), 0x5a);
}

#[test]
fn unsupported_hle_swi_is_a_typed_machine_fault() {
    let rom = [0, 0, 0x2b, 0xef];
    let mut machine = machine(&rom);
    let fault = faulted_step(&mut machine);
    assert_eq!(fault.instruction_address, 0x0800_0000);
    assert_eq!(fault.instruction, 0xef2b_0000);
    assert_eq!(fault.instruction_set, InstructionSet::Arm);
    assert_eq!(fault.reason, FaultReason::UnsupportedHleSwi { service: 0x2b });
    let mut diagnostic = Vec::new();
    fault.write_diagnostic(&mut diagnostic).unwrap();
    assert!(
        String::from_utf8(diagnostic).unwrap().contains("reason=unsupported_hle_swi service=2b\n")
    );
}

#[test]
fn hle_sample_freq_set_mode_zero_is_a_typed_machine_fault() {
    let rom = [0x13, 0xff, 0x2f, 0xe1];
    let mut machine = machine(&rom);
    machine.cpu.registers[3] = 0x0000_3e79;
    machine.cpu.registers[14] = 0x0800_0004;
    executed_step(&mut machine);
    let fault = faulted_step(&mut machine);
    assert_eq!(fault.instruction_address, 0x0000_3e78);
    assert_eq!(fault.instruction, 0x4770);
    assert_eq!(fault.instruction_set, InstructionSet::Thumb);
    assert_eq!(fault.reason, FaultReason::SampleFreqSetModeZero { mode: 0 });
    let mut diagnostic = Vec::new();
    fault.write_diagnostic(&mut diagnostic).unwrap();
    assert!(
        String::from_utf8(diagnostic)
            .unwrap()
            .contains("reason=sample_freq_set_mode_zero mode=0\n")
    );
}

#[test]
fn hle_bg_affine_set_rejects_an_unaligned_mmio_word_read_before_side_effects() {
    let rom = [0, 0, 0x0e, 0xef];
    let mut machine = machine(&rom);
    machine.cpu.registers[..3].copy_from_slice(&[0x0400_0002, 0x0200_0000, 1]);
    machine.bus.ewram[..16].fill(0xa5);
    let fault = faulted_step(&mut machine);
    assert_eq!(fault.instruction_address, 0x0800_0000);
    assert_eq!(fault.instruction, 0xef0e_0000);
    assert_eq!(fault.instruction_set, InstructionSet::Arm);
    assert_eq!(
        fault.reason,
        FaultReason::InvalidHleMemoryAccess {
            service: 0x0e,
            operation: HleMemoryOperation::Read,
            address: 0x0400_0002,
            width: AccessWidth::Word,
        }
    );
    assert_eq!(
        (0..3).map(|i| machine.read_register(i)).collect::<Vec<_>>(),
        [0x0400_0002, 0x0200_0000, 1]
    );
    assert_eq!(&machine.bus.ewram[..16], &[0xa5; 16]);
    let mut diagnostic = Vec::new();
    fault.write_diagnostic(&mut diagnostic).unwrap();
    assert!(String::from_utf8(diagnostic).unwrap().contains(
        "reason=invalid_hle_memory_access service=0e operation=read address=04000002 width=word\n"
    ));
}

#[test]
fn hle_bg_affine_set_rejects_a_later_unaligned_mmio_word_write_before_side_effects() {
    let rom = [0, 0, 0x0e, 0xef];
    let mut machine = machine(&rom);
    machine.cpu.registers[..3].copy_from_slice(&[0x0200_0000, 0x03ff_fffa, 1]);
    machine.bus.iwram[0x7ffa..].fill(0x5a);
    let fault = faulted_step(&mut machine);
    assert_eq!(
        fault.reason,
        FaultReason::InvalidHleMemoryAccess {
            service: 0x0e,
            operation: HleMemoryOperation::Write,
            address: 0x0400_0002,
            width: AccessWidth::Word,
        }
    );
    assert_eq!(
        (0..3).map(|i| machine.read_register(i)).collect::<Vec<_>>(),
        [0x0200_0000, 0x03ff_fffa, 1]
    );
    assert_eq!(&machine.bus.iwram[0x7ffa..], &[0x5a; 6]);
}

#[test]
fn hle_services_preflight_direct_guest_word_pointers_at_the_machine_seam() {
    // (service, r0, r1, r2, r13, operation); each wraps onto 04000002h.
    let cases = [
        (0x10, 0, 0, 0x03ff_fffe, 0x0300_7f00, HleMemoryOperation::Read),
        (0x11, 0x0400_0002, 0, 0, 0x0300_7f00, HleMemoryOperation::Read),
        (0x12, 0x0400_0002, 0, 0, 0x0300_7f00, HleMemoryOperation::Read),
        (0x1a, 0x0200_0000, 0, 0, 0x0400_0002, HleMemoryOperation::Write),
        (0x1f, 0x03ff_fffe, 0, 0, 0x0300_7f00, HleMemoryOperation::Read),
        (0x20, 0x0400_0002, 0, 1, 0x0300_7f00, HleMemoryOperation::Write),
        (0x21, 0x03ff_ffce, 0, 0, 0x0300_7f00, HleMemoryOperation::Read),
        (0x22, 0x03ff_ffce, 0, 0, 0x0300_7f00, HleMemoryOperation::Read),
        (0x23, 0x03ff_ffce, 0, 0, 0x0300_7f00, HleMemoryOperation::Read),
        (0x24, 0x03ff_ffce, 0, 0, 0x0300_7f00, HleMemoryOperation::Read),
        (0x25, 0x03ff_ffe2, 0, 0, 0x0300_7f00, HleMemoryOperation::Read),
        (0x2a, 0x0400_0002, 0, 0, 0x0300_7f00, HleMemoryOperation::Write),
    ];
    for (service, r0, r1, r2, r13, operation) in cases {
        let rom = (0xef00_0000u32 | (service << 16)).to_le_bytes();
        let mut machine = machine(&rom);
        machine.cpu.registers[0] = r0;
        machine.cpu.registers[1] = r1;
        machine.cpu.registers[2] = r2;
        machine.cpu.registers[13] = r13;
        let fault = faulted_step(&mut machine);
        assert_eq!(
            fault.reason,
            FaultReason::InvalidHleMemoryAccess {
                service: service as u8,
                operation,
                address: 0x0400_0002,
                width: AccessWidth::Word,
            }
        );
        assert_eq!((0..3).map(|i| machine.read_register(i)).collect::<Vec<_>>(), [r0, r1, r2]);
        assert_eq!(machine.read_register(13), r13);
    }
}

#[test]
fn hle_sound_driver_init_establishes_the_retail_mode_four_mixer_state() {
    let rom = [0, 0, 0x1a, 0xef];
    let mut machine = machine(&rom);
    machine.bus.ewram[0x100..0x10b1].fill(0xa5);
    machine.bus.apu.sound_bias = 0xc2aa;
    machine.cpu.registers[0] = 0x0200_0100;
    machine.cpu.registers[3] = 0x3333_3333;
    executed_step(&mut machine);

    assert_eq!(read_u32(&machine.bus.iwram[0x7ff0..]), 0x0200_0100);
    let sound_area = &machine.bus.ewram[0x100..];
    assert_eq!(read_u32(sound_area), 0x6873_6d53);
    assert_eq!(&sound_area[4..9], &[0, 0, 8, 15, 4]);
    assert_eq!(sound_area[0x0b], 7);
    for (offset, value) in [
        (0x10, 224),
        (0x14, 13_379),
        (0x18, 627),
        (0x28, 0x1709),
        (0x2c, 0x1709),
        (0x30, 0x1709),
        (0x34, 0x3738),
        (0x38, 0x2425),
        (0x3c, 0x1709),
    ] {
        assert_eq!(read_u32(&sound_area[offset..]), value);
    }
    for offset in [0x40, 0x350, 0x980, 0xfaf] {
        assert_eq!(sound_area[offset], 0);
    }
    assert_eq!(sound_area[0xfb0], 0xa5);
    assert!(machine.bus.apu.master_enabled);
    assert_eq!(machine.bus.apu.sound_control_h, 0x210e);
    assert_eq!(machine.bus.apu.sound_bias, 0x42aa);
    for (index, source, destination) in
        [(1, 0x0200_0450, 0x0400_00a0), (2, 0x0200_0a80, 0x0400_00a4)]
    {
        assert_eq!(machine.bus.dma_channels[index].source, source);
        assert_eq!(machine.bus.dma_channels[index].destination, destination);
        assert_eq!(machine.bus.dma_channels[index].control, 0xb640);
    }
    assert_eq!(machine.bus.timers[0].reload, 0xfb1a);
    assert_eq!(machine.bus.timers[0].control, 0x80);
    assert_eq!(machine.bus.ppu_timing.read_vcount(), 159);
    assert_eq!(machine.read_register(0), 0x6873_6d53);
    assert_eq!(machine.read_register(1), 159);
    assert_eq!(machine.read_register(3), 0x3333_3333);
    assert_eq!(read_u32(&machine.bus.iwram[0x7f00..]), 0);
}

#[test]
fn hle_sound_driver_mode_applies_fields_and_clears_enabled_software_channels() {
    let rom = [0, 0, 0x1b, 0xef];
    let mut machine = machine(&rom);
    write_u32(&mut machine.bus.iwram[0x7ff0..], 0x0200_0100);
    write_u32(&mut machine.bus.ewram[0x100..], 0x6873_6d53);
    machine.bus.ewram[0x105..0x108].copy_from_slice(&[0x55, 8, 15]);
    for channel in 0..12 {
        let offset = 0x150 + channel * 0x40;
        machine.bus.ewram[offset..offset + 2].copy_from_slice(&[0x80, 0xa5]);
    }
    machine.bus.apu.sound_bias = 0xc2aa;
    machine.cpu.registers[0] = 0x0090_4381;
    executed_step(&mut machine);
    assert_eq!(read_u32(&machine.bus.ewram[0x100..]), 0x6873_6d53);
    assert_eq!(&machine.bus.ewram[0x105..0x108], &[1, 3, 4]);
    for channel in 0..12 {
        let offset = 0x150 + channel * 0x40;
        assert_eq!(&machine.bus.ewram[offset..offset + 2], &[0, 0xa5]);
    }
    assert_eq!(machine.bus.apu.sound_bias, 0x42aa);
    assert_eq!(machine.read_register(0), 0x0090_4381);
}

#[test]
fn hle_sound_driver_mode_ignores_zero_fields_and_an_invalid_sound_area() {
    let rom = [0, 0, 0x1b, 0xef];
    let mut zero_fields = machine(&rom);
    write_u32(&mut zero_fields.bus.iwram[0x7ff0..], 0x0200_0100);
    write_u32(&mut zero_fields.bus.ewram[0x100..], 0x6873_6d53);
    zero_fields.bus.ewram[0x105..0x108].copy_from_slice(&[5, 6, 7]);
    zero_fields.bus.ewram[0x150] = 0x80;
    zero_fields.bus.apu.sound_bias = 0xc2aa;
    executed_step(&mut zero_fields);
    assert_eq!(&zero_fields.bus.ewram[0x105..0x108], &[5, 6, 7]);
    assert_eq!(zero_fields.bus.ewram[0x150], 0x80);
    assert_eq!(zero_fields.bus.apu.sound_bias, 0xc2aa);

    let mut invalid = machine(&rom);
    write_u32(&mut invalid.bus.iwram[0x7ff0..], 0x0200_0100);
    write_u32(&mut invalid.bus.ewram[0x100..], 0x6873_6d54);
    invalid.bus.ewram[0x105] = 5;
    invalid.cpu.registers[0] = 0x0090_4381;
    executed_step(&mut invalid);
    assert_eq!(invalid.bus.ewram[0x105], 5);
    assert_eq!(invalid.read_register(0), 0x0090_4381);
    assert_eq!(invalid.read_register(1), 0x6873_6d54);
}

#[test]
fn hle_sound_driver_v_sync_off_disables_dma_and_clears_the_mix_buffer() {
    let rom = [0, 0, 0x28, 0xef];
    for identifier in [0x6873_6d53, 0x6873_6d54] {
        let mut machine = machine(&rom);
        write_u32(&mut machine.bus.iwram[0x7ff0..], 0x0200_0100);
        write_u32(&mut machine.bus.ewram[0x100..], identifier);
        machine.bus.ewram[0x104] = 3;
        machine.bus.ewram[0x44f] = 0x7e;
        machine.bus.ewram[0x450..0x10b0].fill(0xa5);
        machine.bus.ewram[0x10b0] = 0x7e;
        machine.bus.dma_channels[1].control = 0xb600;
        machine.bus.dma_channels[2].control = 0xb600;
        machine.cpu.registers[3] = 0x3333_3333;
        executed_step(&mut machine);
        let area = &machine.bus.ewram[0x100..];
        assert_eq!(read_u32(area), identifier);
        assert_eq!(area[4], 0);
        assert_eq!(area[0x34f], 0x7e);
        assert!(area[0x350..0xfb0].iter().all(|&byte| byte == 0));
        assert_eq!(area[0xfb0], 0x7e);
        assert_eq!(machine.bus.dma_channels[1].control, 0);
        assert_eq!(machine.bus.dma_channels[2].control, 0);
        assert_eq!(
            (0..4).map(|i| machine.read_register(i)).collect::<Vec<_>>(),
            [identifier, 0x0200_10b0, 0x0500_0318, 0x3333_3333]
        );
        assert_eq!(read_u32(&machine.bus.iwram[0x7f00..]), 0);
    }
}

#[test]
fn hle_sound_driver_mode_programs_playback_mode_four_at_scanline_159() {
    let rom = [0, 0, 0x1b, 0xef];
    let mut machine = machine(&rom);
    write_u32(&mut machine.bus.iwram[0x7ff0..], 0x0200_0100);
    write_u32(&mut machine.bus.ewram[0x100..], 0x6873_6d53);
    machine.bus.ewram[0x450..0x10b0].fill(0xa5);
    machine.cpu.registers[0] = 4 << 16;
    executed_step(&mut machine);
    let area = &machine.bus.ewram[0x100..];
    assert_eq!(read_u32(area), 0x6873_6d53);
    assert_eq!(area[8], 4);
    assert_eq!(area[0x0b], 7);
    assert_eq!(
        [read_u32(&area[0x10..]), read_u32(&area[0x14..]), read_u32(&area[0x18..])],
        [224, 13_379, 627]
    );
    assert!(area[0x350..0xfb0].iter().all(|&byte| byte == 0));
    assert_eq!(machine.bus.timers[0].reload, 0xfb1a);
    assert_eq!(machine.bus.timers[0].control, 0x0080);
    assert_eq!(machine.bus.dma_channels[1].control, 0xb600);
    assert_eq!(machine.bus.dma_channels[2].control, 0xb600);
    assert_eq!(machine.bus.ppu_timing.read_vcount(), 159);
    assert_eq!(machine.read_register(0), 0x80);
    assert_eq!(machine.read_register(1), 159);
}

#[test]
fn hle_sound_driver_v_sync_off_rejects_identifiers_beyond_the_busy_state() {
    let rom = [0, 0, 0x28, 0xef];
    let mut machine = machine(&rom);
    write_u32(&mut machine.bus.iwram[0x7ff0..], 0x0200_0100);
    write_u32(&mut machine.bus.ewram[0x100..], 0x6873_6d55);
    machine.bus.ewram[0x104] = 3;
    machine.bus.ewram[0x450] = 0xa5;
    machine.bus.dma_channels[1].control = 0xb600;
    machine.bus.dma_channels[2].control = 0xb600;
    machine.cpu.registers[1..4].copy_from_slice(&[0x1111_1111, 0x2222_2222, 0x3333_3333]);
    executed_step(&mut machine);
    assert_eq!(machine.bus.ewram[0x104], 3);
    assert_eq!(machine.bus.ewram[0x450], 0xa5);
    assert_eq!(machine.bus.dma_channels[1].control, 0xb600);
    assert_eq!(machine.bus.dma_channels[2].control, 0xb600);
    assert_eq!(
        (0..4).map(|i| machine.read_register(i)).collect::<Vec<_>>(),
        [0x6873_6d55, 0x1111_1111, 0x2222_2222, 0x3333_3333]
    );
}

#[test]
fn hle_midi_key2_freq_reads_wave_data_frequency_and_uses_byte_sized_pitch_inputs() {
    let rom = [0, 0, 0x1f, 0xef];
    let mut octave = machine(&rom);
    write_u32(&mut octave.bus.ewram[4..], 0x0100_0000);
    octave.cpu.registers[..4].copy_from_slice(&[0x0200_0000, 168, 0, 0xcafe_babe]);
    executed_step(&mut octave);
    assert_eq!(
        (0..4).map(|i| octave.read_register(i)).collect::<Vec<_>>(),
        [0x0080_0000, 168, 0, 0xcafe_babe]
    );

    let mut byte_inputs = machine(&rom);
    write_u32(&mut byte_inputs.bus.ewram[4..], 0x0100_0000);
    byte_inputs.cpu.registers[..3].copy_from_slice(&[0x0200_0000, 0xffff_ffb4, 0xabcd_0000]);
    executed_step(&mut byte_inputs);
    assert_eq!(byte_inputs.read_register(0), 0x0100_0000);
}

#[test]
fn hle_sound_driver_main_advances_an_attacking_fixed_rate_pcm_channel() {
    let rom = [0, 0, 0x1c, 0xef];
    let mut machine = machine(&rom);
    write_u32(&mut machine.bus.iwram[0x7ff0..], 0x0200_0100);
    write_u32(&mut machine.bus.ewram[0x100..], 0x6873_6d53);
    machine.bus.ewram[0x106] = 1;
    machine.bus.ewram[0x107] = 15;
    write_u32(&mut machine.bus.ewram[0x110..], 4);
    write_u32(&mut machine.bus.ewram[0x128..], 0x1709);
    machine.bus.ewram[0x150..0x155].copy_from_slice(&[0x80, 8, 0xff, 0xff, 0xff]);
    write_u32(&mut machine.bus.ewram[0x174..], 0x0200_1000);
    write_u32(&mut machine.bus.ewram[0x100c..], 4);
    machine.bus.ewram[0x1010..0x1014].copy_from_slice(&[1, 0xfe, 3, 0xfc]);
    executed_step(&mut machine);
    let area = &machine.bus.ewram[0x100..];
    assert_eq!(read_u32(area), 0x6873_6d53);
    assert_eq!(area[0x50], 0);
    assert_eq!(&area[0x59..0x5c], &[0xff, 0xfe, 0xfe]);
    assert_eq!(read_u32(&area[0x68..]), 0);
    assert_eq!(read_u32(&area[0x78..]), 0x0200_1014);
    assert_eq!(&area[0x350..0x354], &[0, 0xfe, 2, 0xfc]);
    assert_eq!(&area[0x980..0x984], &[0, 0xfe, 2, 0xfc]);
}

#[test]
fn hle_sound_driver_main_linearly_interpolates_a_variable_rate_pcm_channel() {
    let rom = [0, 0, 0x1c, 0xef];
    let mut machine = machine(&rom);
    write_u32(&mut machine.bus.iwram[0x7ff0..], 0x0200_0100);
    write_u32(&mut machine.bus.ewram[0x100..], 0x6873_6d53);
    machine.bus.ewram[0x106..0x108].copy_from_slice(&[1, 15]);
    for (at, value) in [
        (0x110, 2),
        (0x114, 4),
        (0x118, 0x0020_0000),
        (0x128, 0x1709),
        (0x170, 2),
        (0x174, 0x0200_1000),
        (0x100c, 2),
    ] {
        write_u32(&mut machine.bus.ewram[at..], value);
    }
    machine.bus.ewram[0x150] = 0x80;
    machine.bus.ewram[0x152..0x155].fill(0xff);
    machine.bus.ewram[0x1010..0x1012].copy_from_slice(&[0, 100]);
    executed_step(&mut machine);
    let area = &machine.bus.ewram[0x100..];
    assert_eq!(area[0x50], 2);
    assert_eq!(read_u32(&area[0x68..]), 1);
    assert_eq!(read_u32(&area[0x6c..]), 0);
    assert_eq!(read_u32(&area[0x78..]), 0x0200_1011);
    assert_eq!(&area[0x350..0x352], &[0, 49]);
    assert_eq!(&area[0x980..0x982], &[0, 49]);
}

#[test]
fn high_speed_audio_preserves_machine_execution_and_fifo_dma_timing() {
    let rom = [0xfe, 0xff, 0xff, 0xea];
    let mut exact = machine(&rom);
    let mut fast = machine(&rom);
    for machine in [&mut exact, &mut fast] {
        machine.set_audio_output(false);
        for (index, byte) in machine.bus.ewram.iter_mut().enumerate() {
            *byte = index as u8;
        }
        for (address, value) in [
            (0x0400_0084, 0x0080),
            (0x0400_0062, 0xf040),
            (0x0400_0064, 0x87ff),
            (0x0400_0078, 0xd900),
            (0x0400_007c, 0x8000),
            (0x0400_0082, 0x0304),
            (0x0400_00bc, 0),
            (0x0400_00be, 0x0200),
            (0x0400_00c0, 0x00a0),
            (0x0400_00c2, 0x0400),
            (0x0400_00c4, 1),
            (0x0400_00c6, 0xf640),
            (0x0400_0100, 0xff80),
            (0x0400_0102, 0x00c0),
        ] {
            write_machine_half(machine, address, value);
        }
    }
    fast.set_high_speed_audio(true);
    let deadline = exact.now() + 300_003;
    assert_eq!(exact.run_until(deadline), fast.run_until(deadline));
    assert_ne!(exact.bus.irq.interrupt_request_flags & (1 << 9), 0);
    assert_eq!(exact.bus.apu.fifo_a, fast.bus.apu.fifo_a);
    assert_eq!(exact.bus.apu.sample_a, fast.bus.apu.sample_a);
    assert_eq!(exact.bus.apu.read_half(0x0400_0084), fast.bus.apu.read_half(0x0400_0084));

    let fast_apu = fast.bus.apu.clone();
    fast.bus.apu = exact.bus.apu.clone();
    let exact_bytes = encode_machine_state(&exact);
    let fast_bytes = encode_machine_state(&fast);
    fast.bus.apu = fast_apu;
    assert_eq!(exact_bytes, fast_bytes);
}

#[test]
fn high_speed_audio_state_round_trip_retains_phases_and_requires_policy_reapplication() {
    let rom = [0xfe, 0xff, 0xff, 0xea];
    let mut machine = machine(&rom);
    for (address, value) in [
        (0x0400_0084, 0x0080),
        (0x0400_0062, 0xf040),
        (0x0400_0064, 0x87ff),
        (0x0400_0078, 0xd900),
        (0x0400_007c, 0x8000),
    ] {
        write_machine_half(&mut machine, address, value);
    }
    machine.run_until(513).unwrap();
    machine.set_audio_output(false);
    machine.set_high_speed_audio(true);
    machine.run_until(123_457).unwrap();
    let bytes = encode_machine_state(&machine);
    let mut resumed = Machine::new(Config { rom: &rom, ..Config::default() });
    resumed.load_state(&bytes).unwrap();
    assert!(!resumed.bus.apu.high_speed_audio_enabled);
    assert!(resumed.bus.apu.sample_output_enabled);
    resumed.set_audio_output(false);
    resumed.set_high_speed_audio(true);
    assert_eq!(machine.bus.apu.square1, resumed.bus.apu.square1);
    assert_eq!(machine.bus.apu.noise, resumed.bus.apu.noise);
    machine.run_until(345_679).unwrap();
    resumed.run_until(345_679).unwrap();
    assert_eq!(encode_machine_state(&machine), encode_machine_state(&resumed));

    machine.set_high_speed_audio(false);
    resumed.set_high_speed_audio(false);
    machine.set_audio_output(true);
    resumed.set_audio_output(true);
    machine.run_until(350_001).unwrap();
    resumed.run_until(350_001).unwrap();
    let mut expected = [evenfall_audio::StereoSample::default(); 32];
    let mut actual = expected;
    let count = machine.drain_audio_samples(&mut expected);
    assert!(count > 0);
    assert_eq!(count, resumed.drain_audio_samples(&mut actual));
    assert_eq!(&expected[..count], &actual[..count]);
}

#[test]
fn high_speed_audio_transitions_flush_device_batches_and_survive_bios_hard_reset() {
    let mut machine = machine(&[]);
    write_machine_half(&mut machine, 0x0400_0084, 0x0080);
    write_machine_half(&mut machine, 0x0400_0062, 0xf040);
    write_machine_half(&mut machine, 0x0400_0064, 0x87ff);
    machine.set_audio_output(false);
    let phase = machine.bus.apu.square1.phase_cycles + machine.bus.apu.pending_square_noise_cycles;
    machine.bus.begin_device_batch();
    machine.bus.elapse(1);
    machine.bus.elapse(3);
    machine.set_high_speed_audio(true);
    assert_eq!(machine.bus.apu.square1.phase_cycles, (phase + 4) % 16);
    assert_eq!(machine.bus.apu.pending_square_noise_cycles, 0);
    machine.bus.end_device_batch();
    machine.bus.hard_reset();
    assert!(machine.bus.apu.high_speed_audio_enabled);
    assert!(!machine.bus.apu.sample_output_enabled);
    machine.set_audio_output(true);
    machine.bus.elapse(1024);
    let mut samples = [evenfall_audio::StereoSample::default(); 16];
    assert_eq!(machine.drain_audio_samples(&mut samples), 0);
    machine.set_high_speed_audio(false);
    machine.bus.elapse(1024);
    assert_eq!(machine.drain_audio_samples(&mut samples), 2);
}

fn bulk_service_machine(service: u8) -> Box<Machine> {
    let words = [0xef00_0000 | (u32::from(service) << 16), 0xe3a0_505a, 0xeaff_fffe, 0xeaff_fffe];
    let rom: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
    machine(&rom)
}

fn fill_pattern(bytes: &mut [u8], seed: u32) {
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = (index as u32).wrapping_mul(0x9e37_79b1).wrapping_add(seed).wrapping_shr(13) as u8;
    }
}

fn obj_affine_workload(count: u32, stride: u32) -> Box<Machine> {
    let mut machine = bulk_service_machine(0x0f);
    for offset in (0..machine.bus.ewram.len()).step_by(8) {
        let record = (offset / 8) as u16;
        machine.bus.ewram[offset..offset + 2].copy_from_slice(&0x0100i16.to_le_bytes());
        machine.bus.ewram[offset + 2..offset + 4].copy_from_slice(&0x0100i16.to_le_bytes());
        machine.bus.ewram[offset + 4..offset + 6]
            .copy_from_slice(&((record % 4) << 14).to_le_bytes());
    }
    machine.cpu.registers[..4].copy_from_slice(&[0x0200_0000, 0x0700_0000, count, stride]);
    machine
}

const QUARTER_TURN_MATRICES: [[i16; 4]; 4] =
    [[256, 0, 0, 256], [0, -256, 256, 0], [-256, 0, 0, -256], [0, 256, -256, 0]];

fn expect_quarter_turn_oam(machine: &Machine) {
    for slot in 0..machine.bus.oam.len() / 8 {
        for (index, value) in QUARTER_TURN_MATRICES[slot % 4].iter().enumerate() {
            let offset = slot * 8 + index * 2;
            assert_eq!(
                i16::from_le_bytes(machine.bus.oam[offset..offset + 2].try_into().unwrap()),
                *value
            );
        }
    }
}

#[test]
fn hle_obj_affine_set_spanning_several_steps_matches_the_single_step_cost_and_output() {
    let mut first = obj_affine_workload(1, 2);
    let first_step = executed_step(&mut first);
    assert!(first.cpu.hle_service.is_none());
    let first_record = first_step.stopped_at - first_step.started_at;
    let mut two = obj_affine_workload(2, 2);
    let second_step = executed_step(&mut two);
    assert!(two.cpu.hle_service.is_none());
    let next_record = second_step.stopped_at - second_step.started_at - first_record;

    let count = 250_000;
    let mut machine = obj_affine_workload(count, 2);
    let started_at = machine.now();
    let mut steps = 0;
    loop {
        let step = executed_step(&mut machine);
        steps += 1;
        assert_eq!(step.instruction_address, 0x0800_0000);
        assert_eq!(step.instruction, 0xef0f_0000);
        if machine.cpu.hle_service.is_none() {
            break;
        }
        assert_eq!(machine.next_instruction_address(), 0x0800_0004);
        assert_eq!(machine.read_register(2), count);
        assert_eq!(machine.read_register(5), 0);
    }
    assert!(steps >= 3);
    assert_eq!(machine.now(), started_at + first_record + u64::from(count - 1) * next_record);
    assert_eq!(
        (0..4).map(|i| machine.read_register(i)).collect::<Vec<_>>(),
        [0x0200_0000, 0x0700_0000, count, 2]
    );
    expect_quarter_turn_oam(&machine);
    let following = executed_step(&mut machine);
    assert_eq!(following.instruction_address, 0x0800_0004);
    assert_eq!(machine.read_register(5), 0x5a);
}

#[test]
fn hle_huff_un_comp_without_a_terminal_returns_to_run_until_while_video_advances() {
    let mut machine = bulk_service_machine(0x13);
    machine.bus.iwram.fill(0);
    write_u32(&mut machine.bus.iwram[..], 0x0000_0428);
    machine.cpu.registers[0] = 0x0300_0000;
    machine.cpu.registers[1] = 0x0200_0000;
    executed_step(&mut machine);
    assert!(machine.cpu.hle_service.is_some());
    let mut frames = machine.bus.completed_frames();
    for _ in 0..4 {
        let deadline = machine.now() + Machine::CYCLES_PER_FRAME;
        let result = machine.run_until(deadline).unwrap();
        assert!(matches!(
            result.reason,
            evenfall_gba::machine::StopReason::Deadline
                | evenfall_gba::machine::StopReason::InstructionBoundaryAfterDeadline
        ));
        assert!(result.stopped_at - deadline < (1 << 20) + 256);
        assert!(machine.cpu.hle_service.is_some());
        assert_eq!(machine.next_instruction_address(), 0x0800_0004);
        assert_eq!(machine.read_register(5), 0);
        assert!(machine.bus.completed_frames() > frames);
        frames = machine.bus.completed_frames();
    }
}

fn long_bulk_machine(service: u8) -> Box<Machine> {
    let mut machine = if service == 0x0f {
        obj_affine_workload(250_000, 2)
    } else {
        bulk_service_machine(service)
    };
    if service != 0x0f {
        fill_pattern(&mut machine.bus.ewram[..], u32::from(service));
    }
    match service {
        0x0b => machine.cpu.registers[..3].copy_from_slice(&[0x0200_0002, 0x0600_0000, 0x0a_0000]),
        0x0c => machine.cpu.registers[..4].copy_from_slice(&[
            0x0200_0000,
            0x0600_0000,
            0x6_0003,
            0x1234_5678,
        ]),
        0x0e => machine.cpu.registers[..3].copy_from_slice(&[0x0200_0000, 0x0700_0000, 80_000]),
        0x0f => {}
        0x10 => {
            machine.bus.iwram[..2].copy_from_slice(&0x8000u16.to_le_bytes());
            machine.bus.iwram[2..4].copy_from_slice(&[1, 32]);
            write_u32(&mut machine.bus.iwram[4..], 0x8000_0005);
            machine.cpu.registers[..3].copy_from_slice(&[0x0200_0000, 0x0600_0000, 0x0300_0000]);
        }
        0x12 => {
            let length = 0x2_8000;
            write_u32(&mut machine.bus.ewram[..], 0x10 | (length << 8));
            let mut offset = 4;
            let mut produced = 0;
            while produced < length {
                machine.bus.ewram[offset] = 0;
                for index in 1..9 {
                    machine.bus.ewram[offset + index] = (offset + index) as u8;
                }
                offset += 9;
                machine.bus.ewram[offset] = 0xff;
                for token in 0..8 {
                    machine.bus.ewram[offset + 1 + token * 2] = 0xf0;
                    machine.bus.ewram[offset + 2 + token * 2] = 1;
                }
                offset += 17;
                produced += 8 + 8 * 18;
            }
            machine.cpu.registers[..2].copy_from_slice(&[0x0200_0000, 0x0600_0000]);
        }
        0x13 => {
            write_u32(&mut machine.bus.ewram[..], 0x28 | (0x6_0000 << 8));
            machine.bus.ewram[4..8].copy_from_slice(&[1, 0xc0, b'A', b'B']);
            machine.cpu.registers[..2].copy_from_slice(&[0x0200_0000, 0x0600_0000]);
        }
        0x15 => {
            let length = 0x4_0001;
            write_u32(&mut machine.bus.ewram[..], 0x30 | (length << 8));
            let mut offset = 4;
            let mut produced = 0;
            while produced < length {
                machine.bus.ewram[offset] = 0xff;
                offset += 2;
                machine.bus.ewram[offset] = 0x7f;
                offset += 129;
                produced += 130 + 128;
            }
            machine.cpu.registers[..2].copy_from_slice(&[0x0200_0000, 0x0600_0001]);
        }
        0x17 => {
            write_u32(&mut machine.bus.ewram[..], 0x81 | (0x4_0000 << 8));
            machine.cpu.registers[..2].copy_from_slice(&[0x0200_0000, 0x0600_0000]);
        }
        _ => unreachable!(),
    }
    machine
}

const LONG_BULK_SERVICES: [u8; 9] = [0x0b, 0x0c, 0x0e, 0x0f, 0x10, 0x12, 0x13, 0x15, 0x17];

fn finish_bulk_call(machine: &mut Machine) {
    while machine.cpu.hle_service.is_some() {
        executed_step(machine);
    }
    executed_step(machine);
}

#[test]
fn a_state_saved_inside_a_multi_step_hle_service_resumes_identically() {
    for service in LONG_BULK_SERVICES {
        let mut uninterrupted = long_bulk_machine(service);
        executed_step(&mut uninterrupted);
        executed_step(&mut uninterrupted);
        assert!(uninterrupted.cpu.hle_service.is_some(), "service {service:x}");
        let saved = encode_machine_state(&uninterrupted);
        let mut resumed = long_bulk_machine(service);
        resumed.load_state(&saved).unwrap();
        assert_eq!(saved, encode_machine_state(&resumed));
        finish_bulk_call(&mut uninterrupted);
        finish_bulk_call(&mut resumed);
        assert_eq!(resumed.read_register(5), 0x5a);
        assert_eq!(
            encode_machine_state(&uninterrupted),
            encode_machine_state(&resumed),
            "service {service:x}"
        );
    }
}

#[test]
fn multi_step_hle_services_reach_the_same_state_under_any_host_slicing() {
    for service in LONG_BULK_SERVICES {
        let mut stepped = long_bulk_machine(service);
        executed_step(&mut stepped);
        assert!(stepped.cpu.hle_service.is_some(), "service {service:x}");
        finish_bulk_call(&mut stepped);
        let target = stepped.now() + 5000;

        let mut whole = long_bulk_machine(service);
        whole.run_until(target).unwrap();
        let mut sliced = long_bulk_machine(service);
        let mut deadline = sliced.now();
        while deadline < target {
            deadline = target.min(deadline + 7919);
            if deadline > sliced.now() {
                sliced.run_until(deadline).unwrap();
            }
        }
        assert_eq!(whole.read_register(5), 0x5a);
        assert_eq!(
            encode_machine_state(&whole),
            encode_machine_state(&sliced),
            "service {service:x}"
        );
    }
}

#[test]
fn arm_software_interrupt_enters_supervisor_at_the_bios_vector_and_movs_pc_lr_returns() {
    let rom = [0, 0, 0, 0xef, 7, 0, 0xa0, 0xe3];
    let mut bios = [0u8; 0x4000];
    write_u32(&mut bios[8..], 0xe1b0_f00e);
    let mut machine = Machine::new(Config {
        rom: &rom,
        bios: &bios,
        boot: BootMode::SkipBios,
        ..Config::default()
    });
    executed_step(&mut machine);
    assert_eq!(machine.read_cpsr() & 0x1f, 0x13);
    assert_ne!(machine.read_cpsr() & (1 << 7), 0);
    assert_eq!(machine.read_register(14), 0x0800_0004);
    assert_eq!(machine.next_instruction_address(), 8);
    executed_step(&mut machine);
    assert_eq!(machine.read_cpsr() & 0x1f, 0x1f);
    assert_eq!(machine.next_instruction_address(), 0x0800_0004);
    assert_eq!(machine.read_register(14), 0);
    executed_step(&mut machine);
    assert_eq!(machine.read_register(0), 7);
}

#[test]
fn software_interrupt_from_fiq_saves_the_fiq_bank_and_enters_supervisor_mode() {
    let words: [u32; 4] = [0xe3a0_8022, 0xe329_f011, 0xe3a0_8044, 0xef00_0000];
    let rom: Vec<_> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
    let bios = [0u8; 0x4000];
    let mut machine = Machine::new(Config {
        rom: &rom,
        bios: &bios,
        boot: BootMode::SkipBios,
        ..Config::default()
    });
    for _ in words {
        executed_step(&mut machine);
    }
    assert_eq!(machine.read_cpsr() & 0x1f, 0x13);
    assert_ne!(machine.read_cpsr() & (1 << 7), 0);
    assert_eq!(machine.cpu.spsr_supervisor, 0x11);
    assert_eq!(machine.read_register(14), 0x0800_0010);
    assert_eq!(machine.read_register(8), 0x22);
    assert_eq!(machine.cpu.fiq_r8_14[0], 0x44);
    assert_eq!(machine.next_instruction_address(), 8);
}

#[test]
fn pending_irq_can_interrupt_supervisor_mode_without_losing_its_bank() {
    let rom = [0x13, 0xf0, 0x29, 0xe3];
    let bios = [0u8; 0x4000];
    let mut machine = Machine::new(Config {
        rom: &rom,
        bios: &bios,
        boot: BootMode::SkipBios,
        ..Config::default()
    });
    machine.cpu.supervisor_r13 = 0x0300_7e00;
    machine.bus.irq.interrupt_enable_mask = 1;
    machine.bus.irq.interrupt_request_flags = 1;
    machine.bus.irq.interrupt_master_enabled = true;
    machine.bus.update_irq_line();
    executed_step(&mut machine);
    assert_eq!(machine.read_cpsr() & 0x1f, 0x12);
    assert_eq!(machine.cpu.spsr_irq & 0x3f, 0x13);
    assert_eq!(machine.cpu.supervisor_r13, 0x0300_7e00);
    assert_eq!(machine.read_register(13), 0x0300_7fa0);
    assert_eq!(machine.read_register(14), 0x0800_0008);
    assert_eq!(machine.next_instruction_address(), 0x18);
}

#[test]
fn ldm_exception_return_loads_the_privileged_bank_then_restores_thumb_system_state() {
    let words: [u32; 5] = [0xe3a0_0403, 0xe329_f012, 0xe369_f03f, 0xe8d0_a000, 0x0000_2107];
    let rom: Vec<_> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
    let mut machine = machine(&rom);
    write_u32(&mut machine.bus.iwram[..], 0x0300_7e40);
    write_u32(&mut machine.bus.iwram[4..], 0x0800_0010);
    for _ in 0..4 {
        executed_step(&mut machine);
    }
    assert_eq!(machine.read_cpsr() & 0x3f, 0x3f);
    assert_eq!(machine.next_instruction_address(), 0x0800_0010);
    assert_eq!(machine.read_register(13), 0x0300_7f00);
    assert_eq!(machine.cpu.irq_r13, 0x0300_7e40);
    executed_step(&mut machine);
    assert_eq!(machine.read_register(1), 7);
}

#[test]
fn arm_coprocessor_encoding_enters_undefined_mode_and_returns_through_movs_pc_lr() {
    let words: [u32; 2] = [0xee00_0010, 0xe3a0_1007];
    let rom: Vec<_> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
    let mut bios = [0u8; 0x4000];
    write_u32(&mut bios[4..], 0xe1b0_f00e);
    let mut machine = Machine::new(Config {
        rom: &rom,
        bios: &bios,
        boot: BootMode::SkipBios,
        ..Config::default()
    });
    executed_step(&mut machine);
    assert_eq!(machine.read_cpsr() & 0x1f, 0x1b);
    assert_eq!(machine.cpu.spsr_undefined, 0x1f);
    assert_eq!(machine.read_register(14), 0x0800_0004);
    assert_eq!(machine.next_instruction_address(), 4);
    executed_step(&mut machine);
    assert_eq!(machine.read_cpsr() & 0x1f, 0x1f);
    assert_eq!(machine.next_instruction_address(), 0x0800_0004);
    executed_step(&mut machine);
    assert_eq!(machine.read_register(1), 7);
}

#[test]
fn arm_class_011_bit_four_reserved_encoding_takes_undefined_only_when_condition_passes() {
    let words: [u32; 5] = [0xe1a0_0000, 0x07f0_00f0, 0xe3a0_1007, 0xe7f0_00f0, 0xe3a0_2009];
    let rom: Vec<_> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
    let mut bios = [0u8; 0x4000];
    write_u32(&mut bios[4..], 0xe1b0_f00e);
    let mut machine = Machine::new(Config {
        rom: &rom,
        bios: &bios,
        boot: BootMode::SkipBios,
        ..Config::default()
    });
    executed_step(&mut machine);
    let failed = executed_step(&mut machine);
    assert_eq!(failed.stopped_at - failed.started_at, 6);
    assert_eq!(machine.read_cpsr(), 0x1f);
    assert_eq!(machine.cpu.spsr_undefined, 0);
    assert_eq!(machine.next_instruction_address(), 0x0800_0008);
    executed_step(&mut machine);
    let undefined = executed_step(&mut machine);
    assert_eq!(undefined.stopped_at - undefined.started_at, 9);
    assert_eq!(machine.read_cpsr() & 0x1f, 0x1b);
    assert_ne!(machine.read_cpsr() & (1 << 7), 0);
    assert_eq!(machine.cpu.spsr_undefined, 0x1f);
    assert_eq!(machine.read_register(14), 0x0800_0010);
    assert_eq!(machine.next_instruction_address(), 4);
    executed_step(&mut machine);
    assert_eq!(machine.read_cpsr(), 0x1f);
    assert_eq!(machine.next_instruction_address(), 0x0800_0010);
    executed_step(&mut machine);
    assert_eq!(machine.read_register(2), 9);
}

#[test]
fn armv5_doubleword_encodings_take_undefined_on_arm7tdmi() {
    for encoding in [0xe1e0_20d4, 0xe1e0_20f4] {
        let words = [0xe1a0_0000u32, encoding, 0xe3a0_4009];
        let rom: Vec<_> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
        let mut bios = [0u8; 0x4000];
        write_u32(&mut bios[4..], 0xe1b0_f00e);
        let mut machine = Machine::new(Config {
            rom: &rom,
            bios: &bios,
            boot: BootMode::SkipBios,
            ..Config::default()
        });
        machine.cpu.registers[0] = 0x0200_0000;
        machine.cpu.registers[2] = 0x1122_3344;
        machine.cpu.registers[3] = 0x5566_7788;
        let memory_before = [0xa5, 0x5a, 0xc3, 0x3c, 0x69, 0x96, 0xf0, 0x0f];
        machine.bus.ewram[..8].copy_from_slice(&memory_before);
        executed_step(&mut machine);
        let undefined = executed_step(&mut machine);
        assert_eq!(undefined.stopped_at - undefined.started_at, 9);
        assert_eq!(machine.read_cpsr() & 0x1f, 0x1b);
        assert_ne!(machine.read_cpsr() & (1 << 7), 0);
        assert_eq!(machine.cpu.spsr_undefined, 0x1f);
        assert_eq!(machine.read_register(14), 0x0800_0008);
        assert_eq!(machine.next_instruction_address(), 4);
        assert_eq!(machine.read_register(0), 0x0200_0000);
        assert_eq!(machine.read_register(2), 0x1122_3344);
        assert_eq!(machine.read_register(3), 0x5566_7788);
        assert_eq!(&machine.bus.ewram[..8], &memory_before);
        executed_step(&mut machine);
        assert_eq!(machine.read_cpsr(), 0x1f);
        assert_eq!(machine.next_instruction_address(), 0x0800_0008);
        executed_step(&mut machine);
        assert_eq!(machine.read_register(4), 9);
    }
}

#[test]
fn thumb_primary_decode_preserves_unallocated_stack_and_branch_encodings() {
    for instruction in [
        0xb100u16, 0xb1ff, 0xb200, 0xb3ff, 0xb600, 0xb7ff, 0xb800, 0xb9ff, 0xba00, 0xbbff, 0xbe00,
        0xbfff, 0xde00, 0xdeff, 0xe800, 0xefff,
    ] {
        let mut machine = machine(&[]);
        machine.cpu = Cpu::init(0x0300_0000, 0xa000_003f, false);
        machine.cpu.registers[0] = 0x1234_5678;
        machine.cpu.registers[15] = 0x0300_0004;
        machine.bus.iwram[..2].copy_from_slice(&instruction.to_le_bytes());
        let step = executed_step(&mut machine);
        assert_eq!(step.stopped_at - step.started_at, 3);
        assert_eq!(machine.read_cpsr(), 0xa000_009b);
        assert_eq!(machine.cpu.spsr_undefined, 0xa000_003f);
        assert_eq!(machine.read_register(14), 0x0300_0002);
        assert_eq!(machine.next_instruction_address(), 4);
        assert_eq!(machine.read_register(15), 12);
        assert_eq!(machine.read_register(0), 0x1234_5678);
        assert_eq!(machine.cpu.pipeline.unwrap().instruction_set, InstructionSet::Arm);
    }
}

#[test]
fn thumb_primary_decode_routes_swi_boundary_immediates_to_the_supervisor_vector() {
    for instruction in [0xdf00u16, 0xdf08, 0xdf7f, 0xdfff] {
        let mut machine = machine(&[]);
        machine.cpu = Cpu::init(0x0300_0000, 0x5000_003f, false);
        machine.cpu.registers[0] = 0x1234_5678;
        machine.cpu.registers[15] = 0x0300_0004;
        machine.bus.iwram[..2].copy_from_slice(&instruction.to_le_bytes());
        let step = executed_step(&mut machine);
        assert_eq!(step.stopped_at - step.started_at, 3);
        assert_eq!(machine.read_cpsr(), 0x5000_0093);
        assert_eq!(machine.cpu.spsr_supervisor, 0x5000_003f);
        assert_eq!(machine.read_register(14), 0x0300_0002);
        assert_eq!(machine.next_instruction_address(), 8);
        assert_eq!(machine.read_register(15), 16);
        assert_eq!(machine.read_register(0), 0x1234_5678);
        assert_eq!(machine.cpu.pipeline.unwrap().instruction_set, InstructionSet::Arm);
    }
}

#[test]
fn linked_scheduler_reports_the_exact_faulted_peer_without_advancing_siblings() {
    let healthy_rom = [0xfe, 0xff, 0xff, 0xea];
    let faulty_rom = [0, 0, 0x1c, 0xef];
    let mut healthy = machine(&healthy_rom);
    let mut faulty = machine(&faulty_rom);
    write_u32(&mut faulty.bus.iwram[0x7ff0..], 0x0200_0100);
    write_u32(&mut faulty.bus.ewram[0x100..], 0x6873_6d53);
    write_u32(&mut faulty.bus.ewram[0x120..], 0x1235);
    let healthy_cycle = healthy.now();
    let result =
        run_linked_until(&mut [&mut faulty, &mut healthy], 100, SioConnection::Cable2p).unwrap();
    match result.reason {
        LinkedStopReason::Faulted(linked) => {
            assert_eq!(linked.machine_index, 0);
            assert_eq!(faulty.fault(), Some(linked.fault));
            assert_eq!(
                linked.fault.reason,
                FaultReason::SoundDriverMainInternalCallback { target: 0x1235 }
            );
        }
        reason => panic!("expected linked fault, got {reason:?}"),
    }
    assert_eq!(healthy.now(), healthy_cycle);
    let faulty_cycle = faulty.now();
    assert!(faulty_cycle > 0);
    run_linked_until(&mut [&mut faulty, &mut healthy], 100, SioConnection::Cable2p).unwrap();
    assert_eq!(healthy.now(), healthy_cycle);
    assert_eq!(faulty.now(), faulty_cycle);
}

#[test]
fn linked_scheduler_advances_a_halted_child_to_the_serial_irq_and_resumes_it() {
    let rom = [0xfe, 0xff, 0xff, 0xea];
    let mut parent = machine(&rom);
    let mut child = machine(&rom);
    for (machine, data, internal) in [(&mut parent, 0x0012, true), (&mut child, 0x0034, false)] {
        write_machine_half(machine, 0x0400_0134, 0);
        write_machine_half(machine, 0x0400_012a, data);
        write_machine_half(machine, 0x0400_0128, 0x4080 | u16::from(internal));
    }
    write_machine_half(&mut child, 0x0400_0200, 1 << 7);
    child.bus.enter_halt();
    let result =
        run_linked_until(&mut [&mut parent, &mut child], 520, SioConnection::Cable2p).unwrap();
    assert_eq!(result.reason, LinkedStopReason::Deadline);
    assert_eq!(child.bus.power_state(), PowerState::Running);
    assert_eq!(child.bus.irq.interrupt_request_flags & (1 << 7), 1 << 7);
    assert_eq!(read_machine_half(&mut child, 0x0400_012a), 0x0012);
}

#[test]
fn linked_scheduler_reports_a_stopped_machine_without_advancing_its_clock() {
    let rom = [0xfe, 0xff, 0xff, 0xea];
    let mut running = machine(&rom);
    let mut stopped = machine(&rom);
    stopped.bus.enter_stop();
    let result =
        run_linked_until(&mut [&mut running, &mut stopped], 20, SioConnection::Cable2p).unwrap();
    assert_eq!(result.reason, LinkedStopReason::PowerStopped);
    assert!(running.now() >= 20);
    assert_eq!(stopped.now(), 0);
}

#[test]
fn hle_affine_and_midi_key2_freq_results_beyond_their_stored_width_keep_their_low_bits() {
    let mut obj = machine(&[0, 0, 0x0f, 0xef]);
    obj.bus.ewram[..2].copy_from_slice(&(-0x8000i16).to_le_bytes());
    obj.bus.ewram[2..4].copy_from_slice(&0x0100i16.to_le_bytes());
    obj.bus.ewram[4..6].copy_from_slice(&0x8000u16.to_le_bytes());
    obj.cpu.registers[..4].copy_from_slice(&[0x0200_0000, 0x0200_0100, 1, 2]);
    executed_step(&mut obj);
    for (index, value) in [-0x8000i16, 0, 0, -256].iter().enumerate() {
        let offset = 0x100 + index * 2;
        assert_eq!(
            i16::from_le_bytes(obj.bus.ewram[offset..offset + 2].try_into().unwrap()),
            *value
        );
    }

    let mut bg = machine(&[0, 0, 0x0e, 0xef]);
    write_u32(&mut bg.bus.ewram[..], 0x7fff_ff00);
    bg.bus.ewram[8..10].copy_from_slice(&0x7fffi16.to_le_bytes());
    bg.bus.ewram[12..14].copy_from_slice(&(-0x8000i16).to_le_bytes());
    bg.bus.ewram[14..16].copy_from_slice(&0x0100i16.to_le_bytes());
    bg.cpu.registers[..3].copy_from_slice(&[0x0200_0000, 0x0300_0000, 1]);
    executed_step(&mut bg);
    assert_eq!(i16::from_le_bytes(bg.bus.iwram[..2].try_into().unwrap()), -0x8000);
    assert_eq!(read_u32(&bg.bus.iwram[8..]), 0xbfff_7f00);
    assert_eq!(read_u32(&bg.bus.iwram[12..]), 0);

    let mut midi = machine(&[0, 0, 0x1f, 0xef]);
    write_u32(&mut midi.bus.ewram[4..], 0xc000_0000);
    midi.cpu.registers[..3].copy_from_slice(&[0x0200_0000, 192, 0]);
    executed_step(&mut midi);
    assert_eq!(midi.read_register(0), 0x8000_0000);
}

#[test]
fn hle_huff_un_comp_reads_a_halfword_aligned_bitstream_as_rotated_words() {
    let rom = [0, 0, 0x13, 0xef, 0x28, 4, 0, 0, 2, 0xc0, b'A', b'B', 0, 0x60, 0, 0, 0, 0];
    let mut machine = machine(&rom);
    machine.cpu.registers[..2].copy_from_slice(&[0x0800_0004, 0x0200_0000]);
    executed_step(&mut machine);
    assert_eq!(&machine.bus.ewram[..4], b"ABBA");
    assert_eq!(machine.read_register(0), 0x0800_0012);
}

#[test]
fn hle_huff_un_comp_traverses_nested_8_bit_nodes_from_the_bitstream_msb() {
    let rom =
        [0, 0, 0x13, 0xef, 0x28, 4, 0, 0, 3, 0x80, b'f', 0xc0, b'H', b'u', 0, 0, 0, 0, 0, 0xb0];
    let mut machine = machine(&rom);
    machine.cpu.registers[..4].copy_from_slice(&[
        0x0800_0004,
        0x0200_0000,
        0xcafe_babe,
        0xdead_beef,
    ]);
    executed_step(&mut machine);
    assert_eq!(&machine.bus.ewram[..4], b"Huff");
    assert_eq!(
        (0..4).map(|i| machine.read_register(i)).collect::<Vec<_>>(),
        [0x0800_0014, 0x0200_0004, 0xcafe_babe, 0xdead_beef]
    );
}

#[test]
fn hle_huff_un_comp_packs_4_bit_terminal_symbols_low_nibble_first() {
    let rom = [0, 0, 0x13, 0xef, 0x24, 4, 0, 0, 1, 0xc0, 0x0a, 3, 0, 0, 0, 0x4d];
    let mut machine = machine(&rom);
    machine.cpu.registers[..2].copy_from_slice(&[0x0800_0004, 0x0200_0000]);
    executed_step(&mut machine);
    assert_eq!(&machine.bus.ewram[..4], &[0x3a, 0xaa, 0x33, 0x3a]);
    assert_eq!(machine.read_register(0), 0x0800_0010);
    assert_eq!(machine.read_register(1), 0x0200_0004);
}

#[test]
fn hle_huff_un_comp_treats_zero_symbol_width_as_eight_and_writes_a_padded_word() {
    let rom = [0, 0, 0x13, 0xef, 0x20, 1, 0, 0, 1, 0xc0, b'X', b'Y', 0, 0, 0, 0];
    let mut machine = machine(&rom);
    machine.cpu.registers[..2].copy_from_slice(&[0x0800_0004, 0x0200_0000]);
    executed_step(&mut machine);
    assert_eq!(&machine.bus.ewram[..4], b"XXXX");
    assert_eq!(machine.read_register(0), 0x0800_0010);
    assert_eq!(machine.read_register(1), 0x0200_0004);
}

#[test]
fn save_state_restores_an_in_flight_prefetch_across_waitcnt_and_boundary_repricing() {
    let rom = vec![0xa5; 0x20040];
    // The durable prefetch record does not derive Eq, so compare its eight
    // stored fields explicitly.
    macro_rules! assert_prefetch_eq {
        ($original:expr, $restored:expr) => {{
            let a = &$original.bus.game_pak_prefetch;
            let b = &$restored.bus.game_pak_prefetch;
            assert_eq!(a.active, b.active);
            assert_eq!(a.head_address, b.head_address);
            assert_eq!(a.last_address, b.last_address);
            assert_eq!(a.count, b.count);
            assert_eq!(a.capacity, b.capacity);
            assert_eq!(a.opcode_bytes, b.opcode_bytes);
            assert_eq!(a.countdown, b.countdown);
            assert_eq!(a.duty, b.duty);
        }};
    }
    for batched in [false, true] {
        for width in [AccessWidth::Half, AccessWidth::Word] {
            let (stride, value, old_remaining, new_sequential, new_boundary) =
                if width == AccessWidth::Word {
                    (4, 0xa5a5_a5a5, 3, 6, 8)
                } else {
                    (2, 0xa5a5, 1, 3, 5)
                };
            let boundary = 0x0802_0000;
            let mut original = machine(&rom);
            write_machine_half(&mut original, 0x0400_0204, 0x4014);
            original.bus.fetch_instruction(boundary - 3 * stride, width);
            write_machine_half(&mut original, 0x0400_0204, 0x4000);
            assert_eq!(original.bus.game_pak_prefetch.countdown, old_remaining);
            let saved = encode_machine_state(&original);
            let mut restored = machine(&rom);
            restored.load_state(&saved).unwrap();
            assert_eq!(original.now(), restored.now());
            assert_prefetch_eq!(original, restored);
            let mut original_region = InstructionFetchRegion::default();
            let mut restored_region = InstructionFetchRegion::default();
            if batched {
                original.bus.begin_device_batch();
                restored.bus.begin_device_batch();
            }
            for (address, cycles, next_cycles) in [
                (boundary - 2 * stride, old_remaining, new_sequential),
                (boundary - stride, new_sequential, new_boundary),
                (boundary, new_boundary, new_sequential),
            ] {
                let started = restored.now();
                assert_eq!(
                    original.bus.fetch_instruction_in_region(&mut original_region, address, width),
                    value
                );
                assert_eq!(
                    restored.bus.fetch_instruction_in_region(&mut restored_region, address, width),
                    value
                );
                assert_eq!(restored.now(), started + cycles);
                assert_eq!(original.now(), restored.now());
                assert_eq!(restored.bus.game_pak_prefetch.countdown, next_cycles);
                assert_prefetch_eq!(original, restored);
            }
            if batched {
                original.bus.end_device_batch();
                restored.bus.end_device_batch();
            }
        }
    }
}

#[test]
fn hle_sound_channel_clear_resumes_arm_and_thumb_extension_callbacks() {
    for thumb in [false, true] {
        let mut rom = [0u8; 0x30];
        write_u32(&mut rom, 0xef1e_0000);
        if thumb {
            rom[0x20..0x24].copy_from_slice(&[0x28, 0x70, 0x70, 0x47]);
        } else {
            write_u32(&mut rom[0x20..], 0xe5c5_0000);
            write_u32(&mut rom[0x24..], 0xe1a0_f00e);
        }
        let mut machine = machine(&rom);
        write_u32(&mut machine.bus.iwram[0x7ff0..], 0x0200_0100);
        write_u32(&mut machine.bus.ewram[0x100..], 0x6873_6d53);
        write_u32(&mut machine.bus.ewram[0x11c..], 0x0200_2000);
        write_u32(&mut machine.bus.ewram[0x12c..], 0x0800_0020 | u32::from(thumb));
        for channel in 0..12 {
            machine.bus.ewram[0x150 + channel * 0x40] = 0x80;
        }
        machine.bus.ewram[0x2100] = 0xa5;
        machine.cpu.registers[4..8].copy_from_slice(&[
            0x4444_4444,
            0x5555_5555,
            0x6666_6666,
            0x7777_7777,
        ]);
        machine.cpu.registers[14] = 0xeeee_eeee;
        machine.cpu.cpsr = 0xa000_001f;
        executed_step(&mut machine);
        let mut steps = 0;
        while machine.next_instruction_address() != 0x0800_0004 {
            executed_step(&mut machine);
            steps += 1;
            assert!(steps < 32);
        }
        for channel in 1..5 {
            assert_eq!(machine.bus.ewram[0x2000 + (channel - 1) * 0x40], channel as u8);
        }
        assert_eq!(machine.bus.ewram[0x2100], 0);
        assert_eq!(read_u32(&machine.bus.ewram[0x100..]), 0x6873_6d53);
        for channel in 0..12 {
            assert_eq!(machine.bus.ewram[0x150 + channel * 0x40], 0);
        }
        assert_eq!(machine.read_register(0), 4);
        assert_eq!(machine.read_register(2), 0);
        for index in 4..8 {
            assert_eq!(machine.read_register(index), 0x1111_1111 * (index as u32));
        }
        assert_eq!(machine.read_register(14), 0xeeee_eeee);
        assert_eq!(machine.read_cpsr(), 0xa000_001f);
    }
}

#[test]
fn hle_sound_driver_main_resumes_player_and_cgb_callbacks_in_order() {
    let mut rom = [0u8; 0x48];
    write_u32(&mut rom, 0xef1c_0000);
    write_u32(&mut rom[0x40..], 0xe48c_0004);
    write_u32(&mut rom[0x44..], 0xe1a0_f00e);
    let mut machine = machine(&rom);
    write_u32(&mut machine.bus.iwram[0x7ff0..], 0x0200_0100);
    for (at, value) in [
        (0x100, 0x6873_6d53),
        (0x110, 4),
        (0x120, 0x0800_0040),
        (0x124, 0x0200_2800),
        (0x128, 0x0800_0040),
    ] {
        write_u32(&mut machine.bus.ewram[at..], value);
    }
    machine.cpu.registers[12] = 0x0200_3000;
    for index in 4..12 {
        machine.cpu.registers[index] = 0x1111_0000 + index as u32;
    }
    machine.cpu.registers[14] = 0xeeee_eeee;
    machine.cpu.cpsr = 0xa000_001f;
    executed_step(&mut machine);
    let mut steps = 0;
    while machine.next_instruction_address() != 0x0800_0004 {
        executed_step(&mut machine);
        steps += 1;
        assert!(steps < 24);
    }
    assert_eq!(read_u32(&machine.bus.ewram[0x3000..]), 0x0200_2800);
    assert_eq!(read_u32(&machine.bus.ewram[0x3004..]), 0x0200_0100);
    assert_eq!(read_u32(&machine.bus.ewram[0x100..]), 0x6873_6d53);
    for index in 4..12 {
        assert_eq!(machine.read_register(index as u8), 0x1111_0000 + index as u32);
    }
    assert_eq!(machine.read_register(14), 0xeeee_eeee);
    assert_eq!(machine.read_cpsr(), 0xa000_001f);
}

#[test]
fn hle_sound_driver_main_samples_cgb_callback_once_across_hblank_dma() {
    let mut rom = [0u8; 0x48];
    write_u32(&mut rom, 0xef1c_0000);
    write_u32(&mut rom[0x40..], 0xe1a0_f00e);
    let mut machine = machine(&rom);
    write_u32(&mut machine.bus.iwram[0x7ff0..], 0x0200_0100);
    write_u32(&mut machine.bus.ewram[0x100..], 0x6873_6d53);
    write_u32(&mut machine.bus.ewram[0x120..], 0);
    write_u32(&mut machine.bus.ewram[0x128..], 0x0800_0040);
    let dma = &mut machine.bus.dma_channels[0];
    dma.source = 0x0200_0200;
    dma.destination = 0x0200_0128;
    dma.word_count = 1;
    dma.control = 0xa400;
    dma.latched_source = 0x0200_0200;
    dma.latched_destination = 0x0200_0128;
    machine.bus.elapse(927);
    let step = executed_step(&mut machine);
    assert_eq!(step.instruction_address, 0x0800_0000);
    assert_eq!(step.instruction, 0xef1c_0000);
    assert_eq!(machine.next_instruction_address(), 0x0800_0040);
    assert_eq!(machine.fault(), None);
    assert_eq!(read_u32(&machine.bus.ewram[0x128..]), 0);
    let continuation = machine.cpu.hle_sound_driver_main.unwrap();
    assert_eq!(format!("{:?}", continuation.phase), "AfterCgbCallback");
    assert_eq!(continuation.return_address, 0x0800_0004);
}

#[test]
fn hle_irq_nesting_limit_faults_before_exception_state_changes() {
    let mut machine = machine(&[0, 0, 0xa0, 0xe1]);
    machine.cpu.hle_irq_depth = 8;
    machine.cpu.cpsr = 0xa000_001f;
    machine.cpu.registers[13] = 0x0200_1234;
    machine.cpu.registers[14] = 0x0800_5678;
    machine.cpu.user_r13 = 0x0300_4321;
    machine.cpu.irq_r13 = 0x0300_7fa0;
    machine.cpu.irq_r14 = 0x1111_2222;
    machine.cpu.spsr_irq = 0x3333_4444;
    machine.bus.irq.interrupt_enable_mask = 1;
    machine.bus.irq.interrupt_request_flags = 1;
    machine.bus.irq.interrupt_master_enabled = true;
    machine.bus.update_irq_line();
    let fault = faulted_step(&mut machine);
    assert_eq!(fault.instruction_address, 0x0800_0000);
    assert_eq!(fault.instruction, 0xe1a0_0000);
    assert_eq!(fault.reason, FaultReason::HleIrqNestingLimit { depth: 8 });
    assert_eq!(machine.next_instruction_address(), 0x0800_0004);
    assert_eq!(machine.read_cpsr(), 0xa000_001f);
    assert_eq!(machine.read_register(13), 0x0200_1234);
    assert_eq!(machine.read_register(14), 0x0800_5678);
    assert_eq!(machine.cpu.user_r13, 0x0300_4321);
    assert_eq!(machine.cpu.irq_r14, 0x1111_2222);
    assert_eq!(machine.cpu.spsr_irq, 0x3333_4444);
    assert_eq!(machine.cpu.hle_irq_depth, 8);
    let mut diagnostic = Vec::new();
    fault.write_diagnostic(&mut diagnostic).unwrap();
    assert!(
        String::from_utf8(diagnostic).unwrap().contains("reason=hle_irq_nesting_limit depth=8\n")
    );
}

#[test]
fn hle_mp2k_missing_running_status_is_a_typed_machine_fault() {
    let mut machine = machine(&[0, 0, 0x1c, 0xef]);
    write_u32(&mut machine.bus.iwram[0x7ff0..], 0x0200_0100);
    for (at, value) in [
        (0x100, 0x6873_6d53),
        (0x120, 0x2149),
        (0x124, 0x0200_1000),
        (0x102c, 0x0200_1100),
        (0x1034, 0x6873_6d53),
        (0x1140, 0x0200_1200),
    ] {
        write_u32(&mut machine.bus.ewram[at..], value);
    }
    machine.bus.ewram[0x1020..0x1022].copy_from_slice(&150u16.to_le_bytes());
    machine.bus.ewram[0x1008] = 1;
    machine.bus.ewram[0x1100] = 0x80;
    machine.bus.ewram[0x1200] = 0x20;
    let fault = faulted_step(&mut machine);
    assert_eq!(fault.instruction_address, 0x0800_0000);
    assert_eq!(fault.instruction, 0xef1c_0000);
    assert_eq!(fault.instruction_set, InstructionSet::Arm);
    assert_eq!(
        fault.reason,
        FaultReason::Mp2kMissingRunningStatus { track: 0x0200_1100, event: 0x20 }
    );
    let mut diagnostic = Vec::new();
    fault.write_diagnostic(&mut diagnostic).unwrap();
    assert!(
        String::from_utf8(diagnostic)
            .unwrap()
            .contains("reason=mp2k_missing_running_status track=02001100 event=20\n")
    );
}

#[test]
fn unsupported_hle_sound_driver_main_callback_stops_at_the_machine_boundary() {
    let rom = [0, 0, 0x1c, 0xef];
    let mut machine = machine(&rom);
    write_u32(&mut machine.bus.iwram[0x7ff0..], 0x0200_0100);
    write_u32(&mut machine.bus.ewram[0x100..], 0x6873_6d53);
    write_u32(&mut machine.bus.ewram[0x120..], 0x1235);
    let first = faulted_step(&mut machine);
    assert_eq!(machine.now(), machine.bus.now());
    assert_eq!(machine.now(), first.cycle);
    assert_eq!(first.instruction_address, 0x0800_0000);
    assert_eq!(first.instruction, 0xef1c_0000);
    assert_eq!(first.instruction_set, InstructionSet::Arm);
    assert_eq!(first.reason, FaultReason::SoundDriverMainInternalCallback { target: 0x1235 });
    let fault_cycle = machine.now();
    assert_eq!(faulted_step(&mut machine), first);
    assert_eq!(machine.now(), fault_cycle);
    let run = machine.run_until(fault_cycle + 100).unwrap();
    assert_eq!(
        (run.started_at, run.stopped_at, run.inputs_consumed),
        (fault_cycle, fault_cycle, 0)
    );
    assert_eq!(run.reason, evenfall_gba::machine::StopReason::Faulted(first));
    let mut key_before = [0; 2];
    machine.inspect_memory(0x0400_0130, &mut key_before).unwrap();
    let input = evenfall_gba::bus::TimedInput {
        at: fault_cycle,
        change: evenfall_gba::bus::TimedInputChange::Keys(evenfall_gba::bus::KeyState {
            a: true,
            ..Default::default()
        }),
    };
    let input_result = machine.step_with_input(&[input]).unwrap();
    assert_eq!(input_result.inputs_consumed, 0);
    assert_eq!(input_result.outcome, StepOutcome::Faulted(first));
    let mut key_after = [0; 2];
    machine.inspect_memory(0x0400_0130, &mut key_after).unwrap();
    assert_eq!(key_before, key_after);
    assert!(matches!(
        machine.wait_until(fault_cycle + 1),
        Err(evenfall_gba::machine::RunError::MachineFaulted)
    ));
    let mut rejected_state = Vec::new();
    assert!(matches!(machine.write_state(&mut rejected_state), Err(StateError::MachineFaulted)));
    assert!(rejected_state.is_empty());
    assert!(matches!(
        machine.load_state(b"not a state"),
        Err(StateError::Codec(CodecError::InvalidSaveState))
    ));
    assert_eq!(machine.fault(), Some(first));
    let pristine = Machine::new(Config { rom: &rom, ..Config::default() });
    machine.load_state(&encode_machine_state(&pristine)).unwrap();
    assert_eq!(machine.fault(), None);
    let mut diagnostic = Vec::new();
    first.write_diagnostic(&mut diagnostic).unwrap();
    assert!(
        String::from_utf8(diagnostic)
            .unwrap()
            .contains("reason=sound_driver_main_internal_callback target=00001235\n")
    );
}

#[test]
fn unsupported_sound_channel_clear_callbacks_fault_on_entry_and_continuation() {
    let rom = 0xef1e_0000u32.to_le_bytes();
    let mut entry = machine(&rom);
    write_u32(&mut entry.bus.iwram[0x7ff0..], 0x0200_0100);
    write_u32(&mut entry.bus.ewram[0x100..], 0x6873_6d53);
    write_u32(&mut entry.bus.ewram[0x11c..], 0x0200_2000);
    write_u32(&mut entry.bus.ewram[0x12c..], 0x1235);
    assert_eq!(
        faulted_step(&mut entry).reason,
        FaultReason::SoundChannelClearInternalCallback { target: 0x1235 }
    );

    let mut rom = [0u8; 0x24];
    write_u32(&mut rom, 0xef1e_0000);
    write_u32(&mut rom[0x20..], 0xe1a0_f00e);
    let mut continuation = machine(&rom);
    write_u32(&mut continuation.bus.iwram[0x7ff0..], 0x0200_0100);
    write_u32(&mut continuation.bus.ewram[0x100..], 0x6873_6d53);
    write_u32(&mut continuation.bus.ewram[0x11c..], 0x0200_2000);
    write_u32(&mut continuation.bus.ewram[0x12c..], 0x0800_0020);
    executed_step(&mut continuation);
    executed_step(&mut continuation);
    write_u32(&mut continuation.bus.ewram[0x12c..], 0x2345);
    let fault = faulted_step(&mut continuation);
    assert_eq!(fault.instruction_address, 0x0000_013c);
    assert_eq!(fault.reason, FaultReason::SoundChannelClearInternalCallback { target: 0x2345 });
}

#[test]
fn hle_track_stop_cgb_callback_incompatibility_is_a_typed_fault() {
    let mut machine = machine(&[0, 0, 0x22, 0xef]);
    write_u32(&mut machine.bus.iwram[0x7ff0..], 0x0200_0100);
    for (at, value) in
        [(0x12c, 0x3457), (0x1034, 0x6873_6d53), (0x102c, 0x0200_1100), (0x1120, 0x0200_1200)]
    {
        write_u32(&mut machine.bus.ewram[at..], value);
    }
    machine.bus.ewram[0x1008] = 1;
    machine.bus.ewram[0x1100] = 0x80;
    machine.bus.ewram[0x1200..0x1202].copy_from_slice(&[1, 1]);
    machine.cpu.registers[0] = 0x0200_1000;
    assert_eq!(
        faulted_step(&mut machine).reason,
        FaultReason::TrackStopCgbCallback { target: 0x3457 }
    );
}

#[test]
fn hle_mp2k_internal_callback_and_chain_limit_are_typed_faults() {
    let rom = [0, 0, 0x1c, 0xef];
    let mut internal = machine(&rom);
    write_u32(&mut internal.bus.iwram[0x7ff0..], 0x0200_0100);
    for (at, value) in [
        (0x100, 0x6873_6d53),
        (0x120, 0x2149),
        (0x124, 0x0200_1000),
        (0x1034, 0x6873_6d53),
        (0x1038, 0x3211),
    ] {
        write_u32(&mut internal.bus.ewram[at..], value);
    }
    assert_eq!(
        faulted_step(&mut internal).reason,
        FaultReason::Mp2kInternalCallback { target: 0x3211 }
    );

    let mut limited = machine(&rom);
    write_u32(&mut limited.bus.iwram[0x7ff0..], 0x0200_0100);
    for (at, value) in [(0x100, 0x6873_6d53), (0x120, 0x2149), (0x124, 0x0200_1000)] {
        write_u32(&mut limited.bus.ewram[at..], value);
    }
    for index in 0..17 {
        let at = 0x1000 + index * 0x40;
        write_u32(&mut limited.bus.ewram[at + 0x34..], 0x6873_6d53);
        if index < 16 {
            write_u32(&mut limited.bus.ewram[at + 0x38..], 0x2149);
            write_u32(&mut limited.bus.ewram[at + 0x3c..], 0x0200_1040 + (index as u32) * 0x40);
        }
    }
    assert_eq!(
        faulted_step(&mut limited).reason,
        FaultReason::Mp2kPlayerChainLimit { player: 0x0200_1400 }
    );
}

#[test]
fn hle_mp2k_replacement_command_incompatibility_is_a_typed_fault() {
    let mut machine = machine(&[0, 0, 0x1c, 0xef]);
    write_u32(&mut machine.bus.iwram[0x7ff0..], 0x0200_0100);
    for (at, value) in [
        (0x100, 0x6873_6d53),
        (0x120, 0x2149),
        (0x124, 0x0200_1000),
        (0x134, 0x0200_3000),
        (0x1034, 0x6873_6d53),
        (0x102c, 0x0200_1100),
        (0x1140, 0x0200_2000),
        (0x3000, 0x0800_1234),
    ] {
        write_u32(&mut machine.bus.ewram[at..], value);
    }
    machine.bus.ewram[0x1020..0x1022].copy_from_slice(&150u16.to_le_bytes());
    machine.bus.ewram[0x1008] = 1;
    machine.bus.ewram[0x1100] = 0x80;
    machine.bus.ewram[0x2000] = 0xb1;
    assert_eq!(
        faulted_step(&mut machine).reason,
        FaultReason::Mp2kReplacementCallback { command: 0, target: 0x0800_1234 }
    );
}

#[test]
fn hle_sound_driver_main_runs_the_internal_mp2k_scheduler_and_starts_a_note() {
    let mut machine = machine(&[0, 0, 0x1c, 0xef]);
    write_u32(&mut machine.bus.iwram[0x7ff0..], 0x0200_0100);
    for (at, value) in [
        (0x100, 0x6873_6d53),
        (0x120, 0x2149),
        (0x124, 0x0200_1000),
        (0x128, 0x1709),
        (0x102c, 0x0200_1100),
        (0x1030, 0x0200_1300),
        (0x1034, 0x6873_6d53),
        (0x1140, 0x0200_1200),
        (0x1304, 0x0200_1400),
        (0x1404, 0x0100_0000),
    ] {
        write_u32(&mut machine.bus.ewram[at..], value);
    }
    machine.bus.ewram[0x106] = 1;
    machine.bus.ewram[0x1008..0x100a].copy_from_slice(&[1, 3]);
    machine.bus.ewram[0x1020..0x1022].copy_from_slice(&150u16.to_le_bytes());
    machine.bus.ewram[0x1100] = 0xc0;
    machine.bus.ewram[0x1200..0x1208].copy_from_slice(&[0xbd, 0, 0xbe, 127, 0xd0, 60, 100, 0x81]);
    machine.bus.ewram[0x1308..0x130c].copy_from_slice(&[0xff, 0xf0, 0xc0, 0xe0]);
    executed_step(&mut machine);
    let ram = &machine.bus.ewram;
    assert_eq!(read_u32(&ram[0x100..]), 0x6873_6d53);
    assert_eq!(read_u32(&ram[0x1034..]), 0x6873_6d53);
    assert_eq!(read_u32(&ram[0x1004..]), 1);
    assert_eq!(read_u32(&ram[0x100c..]), 1);
    assert_eq!(u16::from_le_bytes(ram[0x1022..0x1024].try_into().unwrap()), 0);
    assert_eq!(&ram[0x1100..0x1102], &[0x80, 0]);
    assert_eq!(read_u32(&ram[0x1140..]), 0x0200_1208);
    assert_eq!(read_u32(&ram[0x1120..]), 0x0200_0150);
    assert_eq!(ram[0x150], 0x80);
    assert_eq!(&ram[0x160..0x164], &[1, 60, 100, 3]);
    assert_eq!(read_u32(&ram[0x17c..]), 0x0200_1100);
    assert_eq!(read_u32(&ram[0x174..]), 0x0200_1400);
    assert_ne!(read_u32(&ram[0x170..]), 0);
}

#[test]
fn hle_mp2k_scheduler_resumes_a_wrapped_guest_player_callback() {
    let mut rom = [0u8; 0x108];
    write_u32(&mut rom, 0xef1c_0000);
    for (at, value) in [(0x100, 0x225a_u16), (0x102, 0x7002), (0x104, 0x4770)] {
        rom[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }
    let mut machine = machine(&rom);
    write_u32(&mut machine.bus.iwram[0x7ff0..], 0x0200_0100);
    for (at, value) in [
        (0x100, 0x6873_6d53),
        (0x120, 0x2149),
        (0x124, 0x0200_1000),
        (0x128, 0x1709),
        (0x1004, 0x8000_0000),
        (0x1034, 0x6873_6d53),
        (0x1038, 0x0800_0101),
        (0x103c, 0x0200_2000),
    ] {
        write_u32(&mut machine.bus.ewram[at..], value);
    }
    executed_step(&mut machine);
    assert_eq!(machine.next_instruction_address(), 0x0800_0100);
    assert_eq!(read_u32(&machine.bus.ewram[0x1034..]), 0x6873_6d54);
    for _ in 0..4 {
        executed_step(&mut machine);
    }
    assert_eq!(machine.bus.ewram[0x2000], 0x5a);
    assert_eq!(read_u32(&machine.bus.ewram[0x1034..]), 0x6873_6d53);
    assert_eq!(read_u32(&machine.bus.ewram[0x100..]), 0x6873_6d53);
    assert_eq!(machine.next_instruction_address(), 0x0800_0004);
}

#[test]
fn hle_music_player_swis_open_start_stop_continue_and_initialize_fade() {
    let words = [0xef20_0000u32, 0xef21_0000, 0xef22_0000, 0xef23_0000, 0xef24_0000];
    let rom: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
    let mut machine = machine(&rom);
    write_u32(&mut machine.bus.iwram[0x7ff0..], 0x0200_0100);
    for (at, value) in
        [(0x100, 0x6873_6d53), (0x120, 0x0800_0101), (0x124, 0x0200_4000), (0x12c, 0x1709)]
    {
        write_u32(&mut machine.bus.ewram[at..], value);
    }
    machine.bus.ewram[0x1000..0x1040].fill(0xa5);
    machine.bus.ewram[0x1100] = 0xa5;
    machine.bus.ewram[0x1150] = 0xa5;
    machine.cpu.registers[..3].copy_from_slice(&[0x0200_1000, 0x0200_1100, 0x100]);
    executed_step(&mut machine);
    let ram = &machine.bus.ewram;
    assert_eq!(read_u32(&ram[0x102c..]), 0x0200_1100);
    assert_eq!(ram[0x1008], 16);
    assert_eq!(read_u32(&ram[0x1004..]), 0x8000_0000);
    assert_eq!((ram[0x1100], ram[0x1150]), (0, 0));
    assert_eq!(read_u32(&ram[0x1038..]), 0x0800_0101);
    assert_eq!(read_u32(&ram[0x103c..]), 0x0200_4000);
    assert_eq!(read_u32(&ram[0x120..]), 0x2149);
    assert_eq!(read_u32(&ram[0x124..]), 0x0200_1000);
    assert_eq!(read_u32(&ram[0x1034..]), 0x6873_6d53);

    machine.bus.ewram[0x2000..0x2004].copy_from_slice(&[1, 0, 7, 0]);
    write_u32(&mut machine.bus.ewram[0x2004..], 0x0800_9000);
    write_u32(&mut machine.bus.ewram[0x2008..], 0x0800_a000);
    machine.bus.ewram[0x1100] = 0x80;
    write_u32(&mut machine.bus.ewram[0x1120..], 0x0200_3000);
    machine.bus.ewram[0x3000] = 1;
    write_u32(&mut machine.bus.ewram[0x302c..], 0x0200_1100);
    machine.bus.ewram[0x1150] = 0x80;
    machine.cpu.registers[..2].copy_from_slice(&[0x0200_1000, 0x0200_2000]);
    executed_step(&mut machine);
    let ram = &machine.bus.ewram;
    assert_eq!(read_u32(&ram[0x1000..]), 0x0200_2000);
    assert_eq!(read_u32(&ram[0x1004..]), 0);
    assert_eq!(ram[0x1009], 7);
    assert_eq!(&ram[0x101c..0x1022], &[150, 0, 0, 1, 150, 0]);
    assert_eq!(ram[0x1100], 0xc0);
    assert_eq!(read_u32(&ram[0x1140..]), 0x0800_a000);
    assert_eq!((ram[0x1150], ram[0x3000]), (0, 0));
    assert_eq!(read_u32(&ram[0x302c..]), 0);
    machine.bus.ewram[0x1100] = 0xc0;
    write_u32(&mut machine.bus.ewram[0x1120..], 0x0200_3000);
    machine.bus.ewram[0x3000] = 3;
    write_u32(&mut machine.bus.ewram[0x302c..], 0x0200_1100);
    machine.cpu.registers[0] = 0x0200_1000;
    executed_step(&mut machine);
    assert_eq!(read_u32(&machine.bus.ewram[0x1004..]), 0x8000_0000);
    assert_eq!(machine.bus.ewram[0x3000], 0);
    assert_eq!(read_u32(&machine.bus.ewram[0x1120..]), 0);
    machine.cpu.registers[0] = 0x0200_1000;
    executed_step(&mut machine);
    assert_eq!(read_u32(&machine.bus.ewram[0x1004..]), 0);
    machine.cpu.registers[..2].copy_from_slice(&[0x0200_1000, 7]);
    executed_step(&mut machine);
    let ram = &machine.bus.ewram;
    assert_eq!(&ram[0x1024..0x102a], &[7, 0, 7, 0, 0, 1]);
    assert_eq!(read_u32(&ram[0x1034..]), 0x6873_6d53);
}

#[test]
fn hle_music_player_jump_table_copy_publishes_36_callable_thumb_targets() {
    let words = [0xef2a_0000u32, 0xe12f_ff13, 0xe1a0_0000];
    let rom: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
    let mut machine = machine(&rom);
    machine.bus.ewram[0x0fff] = 0xa5;
    machine.bus.ewram[0x1090] = 0x5a;
    machine.bus.ewram[0x2000..0x2041].fill(0xcc);
    machine.cpu.registers[0] = 0x0200_1000;
    executed_step(&mut machine);
    for index in 0..36 {
        assert_eq!(read_u32(&machine.bus.ewram[0x1000 + index * 4..]), 0x3e01 + index as u32 * 4);
    }
    assert_eq!((machine.bus.ewram[0x0fff], machine.bus.ewram[0x1090]), (0xa5, 0x5a));
    assert_eq!(
        (0..4).map(|i| machine.read_register(i)).collect::<Vec<_>>(),
        [0x0200_1090, 0, 0x3e90, 0x3e8d]
    );
    machine.cpu.registers[0] = 0x0200_2000;
    machine.cpu.registers[14] = 0x0800_0008;
    executed_step(&mut machine);
    assert_eq!(machine.next_instruction_address(), 0x3e8c);
    let call = executed_step(&mut machine);
    assert_eq!(
        (call.instruction_address, call.instruction, call.instruction_set),
        (0x3e8c, 0x4770, InstructionSet::Thumb)
    );
    assert_eq!(machine.next_instruction_address(), 0x0800_0008);
    assert_eq!(machine.read_cpsr() & (1 << 5), 0);
    assert_eq!(&machine.bus.ewram[0x2000..0x2040], &[0; 0x40]);
    assert_eq!(machine.bus.ewram[0x2040], 0xcc);
}

#[test]
fn hle_sound_driver_main_can_call_a_function_returned_by_its_hle_jump_list() {
    let words = [0xef2a_0000u32, 0xef1c_0000, 0xe1a0_0000];
    let rom: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
    let mut machine = machine(&rom);
    write_u32(&mut machine.bus.iwram[0x7ff0..], 0x0200_0100);
    write_u32(&mut machine.bus.ewram[0x100..], 0x6873_6d53);
    write_u32(&mut machine.bus.ewram[0x124..], 0x0200_2000);
    machine.bus.ewram[0x2000..0x2040].fill(0xa5);
    machine.cpu.registers[0] = 0x0200_1000;
    executed_step(&mut machine);
    let callback = read_u32(&machine.bus.ewram[0x108c..]);
    assert_eq!(callback, 0x3e8d);
    write_u32(&mut machine.bus.ewram[0x120..], callback);
    executed_step(&mut machine);
    assert_eq!(machine.next_instruction_address(), 0x3e8c);
    executed_step(&mut machine);
    executed_step(&mut machine);
    assert_eq!(machine.next_instruction_address(), 0x0800_0008);
    assert_eq!(&machine.bus.ewram[0x2000..0x2040], &[0; 0x40]);
    assert_eq!(read_u32(&machine.bus.ewram[0x100..]), 0x6873_6d53);
}

#[test]
fn hle_music_event_targets_consume_commands_and_unlink_channels() {
    let words = [0xe12f_ff13u32, 0xe12f_ff13, 0xe1a0_0000];
    let rom: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
    let mut machine = machine(&rom);
    machine.bus.ewram[0x1000] = 0x80;
    machine.bus.ewram[0x1100] = 0x20;
    write_u32(&mut machine.bus.ewram[0x1040..], 0x0200_1100);
    machine.cpu.registers[1] = 0x0200_1000;
    machine.cpu.registers[3] = 0x3e39;
    machine.cpu.registers[14] = 0x0800_0004;
    executed_step(&mut machine);
    executed_step(&mut machine);
    assert_eq!(machine.bus.ewram[0x1014], 0xe0);
    assert_eq!(machine.bus.ewram[0x1000], 0x83);
    assert_eq!(read_u32(&machine.bus.ewram[0x1040..]), 0x0200_1101);
    for (at, value) in [
        (0x1020, 0x0200_2000),
        (0x202c, 0x0200_1000),
        (0x2030, 0),
        (0x2034, 0x0200_2100),
        (0x2130, 0x0200_2000),
    ] {
        write_u32(&mut machine.bus.ewram[at..], value);
    }
    machine.cpu.registers[0] = 0x0200_2000;
    machine.cpu.registers[3] = 0x3e89;
    machine.cpu.registers[14] = 0x0800_0008;
    executed_step(&mut machine);
    executed_step(&mut machine);
    assert_eq!(read_u32(&machine.bus.ewram[0x1020..]), 0x0200_2100);
    assert_eq!(read_u32(&machine.bus.ewram[0x2130..]), 0);
    assert_eq!(read_u32(&machine.bus.ewram[0x202c..]), 0);
}

#[test]
fn hle_music_helpers_calculate_track_output_and_advance_fades() {
    let words = [0xe12f_ff13u32, 0xe12f_ff13, 0xe1a0_0000];
    let rom: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
    let mut machine = machine(&rom);
    for (at, value) in [
        (0x1000, 0x8f),
        (0x100a, 1),
        (0x100b, 0xff),
        (0x100c, 1),
        (0x100d, 5),
        (0x100e, 2),
        (0x100f, 3),
        (0x1012, 64),
        (0x1013, 128),
        (0x1014, 0xf6),
        (0x1015, 4),
        (0x1016, 2),
    ] {
        machine.bus.ewram[at] = value;
    }
    machine.cpu.registers[1] = 0x0200_1000;
    machine.cpu.registers[3] = 0x3e85;
    machine.cpu.registers[14] = 0x0800_0004;
    executed_step(&mut machine);
    executed_step(&mut machine);
    assert_eq!(machine.bus.ewram[0x1000], 0x8a);
    assert_eq!(&machine.bus.ewram[0x1008..0x100a], &[0, 65]);
    assert_eq!(&machine.bus.ewram[0x1010..0x1012], &[112, 143]);
    machine.bus.ewram[0x808] = 1;
    machine.bus.ewram[0x824..0x82a].copy_from_slice(&[1, 0, 1, 0, 32, 0]);
    write_u32(&mut machine.bus.ewram[0x82c..], 0x0200_1000);
    machine.cpu.registers[0] = 0x0200_0800;
    machine.cpu.registers[3] = 0x3e81;
    machine.cpu.registers[14] = 0x0800_0008;
    executed_step(&mut machine);
    executed_step(&mut machine);
    assert_eq!(&machine.bus.ewram[0x826..0x82a], &[1, 0, 16, 0]);
    assert_eq!(machine.bus.ewram[0x1013], 4);
    assert_eq!(machine.bus.ewram[0x1000], 0x8b);
}

#[test]
fn hle_huff_un_comp_and_bit_un_pack_fault_before_an_unaligned_mmio_output_word() {
    let cases = [
        (0x13, 0x0400_0001, false),
        (0x10, 0x0400_0001, false),
        (0x13, 0x0400_0001u32.wrapping_sub(4 * 60_000), true),
        (0x10, 0x0400_0001u32.wrapping_sub(4 * 200_000), true),
    ];
    for (service, destination, later_step) in cases {
        let mut machine = long_bulk_machine(service);
        machine.cpu.registers[1] = destination;
        let mut steps = 0;
        let fault = loop {
            match machine.step() {
                StepOutcome::Executed(_) => assert!(machine.cpu.hle_service.is_some()),
                StepOutcome::Faulted(value) => break value,
                StepOutcome::Waiting(_) => panic!("unexpected wait"),
            }
            steps += 1;
        };
        assert_eq!(later_step, steps != 0);
        assert_eq!(fault.instruction_address, 0x0800_0000);
        assert_eq!(fault.instruction, 0xef00_0000 | u32::from(service) << 16);
        assert_eq!(
            fault.reason,
            FaultReason::InvalidHleMemoryAccess {
                service,
                operation: HleMemoryOperation::Write,
                address: 0x0400_0001,
                width: AccessWidth::Word,
            }
        );
        assert_eq!(machine.read_register(1), destination);
    }
}

#[test]
fn hle_bg_affine_set_preflight_faults_on_distant_and_wrapped_records_before_side_effects() {
    let cases = [
        (0x0300_0001, 0x0200_0000, 0xffff_ffff, HleMemoryOperation::Read, 0x0400_0005),
        (0x0500_0002, 0x0200_0000, 0xffff_ffff, HleMemoryOperation::Read, 0x0400_0002),
        (0x03ff_fffd, 0x0200_0000, 1, HleMemoryOperation::Read, 0x0400_0001),
        (0x04ff_ffff, 0x0200_0000, 1, HleMemoryOperation::Read, 0x04ff_ffff),
        (0x0200_0000, 0xffff_fff2, 0x0040_0006, HleMemoryOperation::Write, 0x0400_000a),
    ];
    for (r0, r1, r2, operation, address) in cases {
        let mut machine = bulk_service_machine(0x0e);
        machine.bus.oam.fill(0xa5);
        machine.cpu.registers[..3].copy_from_slice(&[r0, r1, r2]);
        let fault = faulted_step(&mut machine);
        assert_eq!(fault.instruction, 0xef0e_0000);
        assert_eq!(
            fault.reason,
            FaultReason::InvalidHleMemoryAccess {
                service: 0x0e,
                operation,
                address,
                width: AccessWidth::Word
            }
        );
        assert_eq!((0..3).map(|i| machine.read_register(i)).collect::<Vec<_>>(), [r0, r1, r2]);
        assert!(machine.cpu.hle_service.is_none());
    }
    let mut machine = bulk_service_machine(0x0e);
    machine.cpu.registers[..3].copy_from_slice(&[0x0500_0002, 0x0700_0000, 213_909_504]);
    assert_eq!(executed_step(&mut machine).instruction_address, 0x0800_0000);
    assert!(machine.cpu.hle_service.is_some());
    executed_step(&mut machine);
    assert!(machine.cpu.hle_service.is_some());
}

#[test]
fn a_dma_livelock_is_a_terminal_fault_at_the_same_cycle_under_any_host_slicing() {
    let rom = [
        0x10, 0x00, 0x9f, 0xe5, 0x10, 0x10, 0x9f, 0xe5, 0x00, 0x10, 0x80, 0xe5, 0x0c, 0x10, 0x9f,
        0xe5, 0xb0, 0x10, 0xc0, 0xe1, 0xfe, 0xff, 0xff, 0xea, 0x04, 0x01, 0x00, 0x04, 0x00, 0xff,
        0x80, 0x00, 0xfd, 0xff, 0x00, 0x00,
    ];
    let limit = evenfall_gba::bus::Bus::MAX_DMA_BURST_CYCLES;
    let mut faults = Vec::new();
    for policy in 0..2 {
        let mut machine = machine(&rom);
        write_machine_half(&mut machine, 0x0400_0084, 0x0080);
        write_machine_half(&mut machine, 0x0400_0082, 0xfb00);
        for (address, value) in [(0x0400_00bc, 0x0200_2000), (0x0400_00c0, 0x0400_00a0)] {
            machine.bus.write(
                Access {
                    address,
                    width: AccessWidth::Word,
                    kind: AccessKind::Nonsequential,
                    source: AccessSource::CpuData,
                },
                value,
            );
        }
        write_machine_half(&mut machine, 0x0400_00c6, 0xb740);
        for (address, value) in [(0x0400_00c8, 0x0200_2000), (0x0400_00cc, 0x0400_00a4)] {
            machine.bus.write(
                Access {
                    address,
                    width: AccessWidth::Word,
                    kind: AccessKind::Nonsequential,
                    source: AccessSource::CpuData,
                },
                value,
            );
        }
        write_machine_half(&mut machine, 0x0400_00d2, 0xb740);
        machine.bus.write(
            Access {
                address: 0x0400_0100,
                width: AccessWidth::Word,
                kind: AccessKind::Nonsequential,
                source: AccessSource::CpuData,
            },
            0x0080_fffd,
        );
        let result = if policy == 0 {
            machine.run_until(machine.now() + 4 * limit).unwrap()
        } else {
            let mut result = None;
            for slice in 0..200 {
                let next = machine.run_until(machine.now() + 64).unwrap();
                if matches!(next.reason, evenfall_gba::machine::StopReason::Faulted(_)) {
                    assert_ne!(slice, 0);
                    result = Some(next);
                    break;
                }
            }
            result.expect("expected machine fault")
        };
        let evenfall_gba::machine::StopReason::Faulted(fault) = result.reason else {
            panic!("expected machine fault: {:?}", result.reason)
        };
        let FaultReason::DmaLivelock { started_at, .. } = fault.reason else {
            panic!("expected DMA livelock: {:?}", fault.reason)
        };
        assert!(started_at > 256);
        assert!(fault.cycle >= started_at + limit);
        assert!(fault.cycle < started_at + limit + 256);
        assert_eq!(result.stopped_at, fault.cycle);
        assert_eq!(machine.step(), StepOutcome::Faulted(fault));
        faults.push(fault);
    }
    assert_eq!(faults[0], faults[1]);
}

fn irq_obj_affine_workload(count: u32, enable: u16) -> Box<Machine> {
    let mut machine = obj_affine_workload(count, 2);
    machine.bus.write(
        Access {
            address: 0x0300_7ffc,
            width: AccessWidth::Word,
            kind: AccessKind::Nonsequential,
            source: AccessSource::CpuData,
        },
        0x0800_000c,
    );
    write_machine_half(&mut machine, 0x0400_0004, 1 << 3);
    write_machine_half(&mut machine, 0x0400_0200, enable);
    write_machine_half(&mut machine, 0x0400_0208, 1);
    machine
}

#[test]
fn an_irq_raised_inside_a_multi_step_hle_service_is_taken_when_the_service_returns() {
    let mut one = irq_obj_affine_workload(1, 0);
    let step = executed_step(&mut one);
    assert!(one.cpu.hle_service.is_none());
    let first_record = step.stopped_at - step.started_at;
    let mut two = irq_obj_affine_workload(2, 0);
    let step = executed_step(&mut two);
    assert!(two.cpu.hle_service.is_none());
    let next_record = step.stopped_at - step.started_at - first_record;
    let mut pending = irq_obj_affine_workload(1, 1);
    pending.bus.irq.interrupt_request_flags = 1;
    pending.bus.update_irq_line();
    let step = executed_step(&mut pending);
    assert!(pending.cpu.hle_service.is_none());
    let irq_entry = step.stopped_at - step.started_at - first_record;
    assert_eq!(pending.read_cpsr() & 0x1f, 0x12);

    let count = 250_000;
    let mut machine = irq_obj_affine_workload(count, 1);
    let started_at = machine.now();
    let mut pending_steps = 0;
    loop {
        executed_step(&mut machine);
        if machine.cpu.hle_service.is_none() {
            break;
        }
        assert_eq!(machine.read_cpsr() & 0x1f, 0x1f);
        assert_eq!(machine.next_instruction_address(), 0x0800_0004);
        assert_eq!(machine.cpu.hle_irq_depth, 0);
        if machine.bus.irq.irq_line {
            pending_steps += 1;
        }
    }
    assert!(pending_steps >= 2);
    assert_eq!(machine.read_cpsr() & 0x1f, 0x12);
    assert_eq!(machine.next_instruction_address(), 0x0800_000c);
    assert_eq!(
        machine.now(),
        started_at + first_record + u64::from(count - 1) * next_record + irq_entry
    );
    assert_eq!(machine.cpu.hle_irq_depth, 1);
    assert_eq!(machine.cpu.hle_irq_stack[0].return_address, 0x0800_0008);
    expect_quarter_turn_oam(&machine);
}

#[test]
fn a_bulk_hle_service_runs_on_through_a_halt_it_writes_then_halts() {
    let mut one = obj_affine_workload(1, 0);
    one.cpu.registers[1] = 0x0400_0300;
    let step = executed_step(&mut one);
    assert!(one.cpu.hle_service.is_none());
    let first_record = step.stopped_at - step.started_at;
    let mut two = obj_affine_workload(2, 0);
    two.cpu.registers[1] = 0x0400_0300;
    let step = executed_step(&mut two);
    assert!(two.cpu.hle_service.is_none());
    let next_record = step.stopped_at - step.started_at - first_record;

    let count = 250_000;
    let mut stepped = obj_affine_workload(count, 0);
    stepped.cpu.registers[1] = 0x0400_0300;
    let started_at = stepped.now();
    loop {
        executed_step(&mut stepped);
        if stepped.cpu.hle_service.is_none() {
            break;
        }
        assert_eq!(stepped.bus.power_state(), PowerState::Halted);
    }
    let finished_at = started_at + first_record + u64::from(count - 1) * next_record;
    assert_eq!(stepped.now(), finished_at);
    assert_eq!(stepped.step(), StepOutcome::Waiting(evenfall_gba::cpu::WaitReason::Halted));

    let mut sliced = obj_affine_workload(count, 0);
    sliced.cpu.registers[1] = 0x0400_0300;
    let target = finished_at + 1000;
    let mut deadline = sliced.now();
    while deadline < target {
        deadline = target.min(deadline + 50_000);
        if deadline > sliced.now() {
            sliced.run_until(deadline).unwrap();
        }
    }
    stepped.wait_until(target).unwrap();
    assert_eq!(sliced.now(), target);

    let mut linked = obj_affine_workload(count, 0);
    linked.cpu.registers[1] = 0x0400_0300;
    let mut peer = bulk_service_machine(0x0f);
    peer.cpu.registers[2] = 0;
    run_linked_until(&mut [&mut linked, &mut peer], target, SioConnection::Cable2p).unwrap();
    assert_eq!(linked.now(), target);
    let expected = encode_machine_state(&stepped);
    assert_eq!(expected, encode_machine_state(&sliced));
    assert_eq!(expected, encode_machine_state(&linked));
}
