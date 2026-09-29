use super::*;
use crate::cartridge::save;

fn arm_cpu(opcode: u32) -> Cpu {
    let mut cpu = Cpu::init(0x0300_0000, 0x1f, true);
    cpu.pipeline = Some(Pipeline {
        instructions: [opcode, 0xef08_0000],
        instruction_set: InstructionSet::Arm,
    });
    cpu
}

/// The engine runs only a pipeline that matches memory: store the latched
/// words where the kernel tests' CPU says they were fetched from.
fn load_pipeline(bus: &mut Bus, cpu: &Cpu) {
    let pc = cpu.core.next_instruction_address;
    for (slot, word) in cpu.pipeline.unwrap().instructions.into_iter().enumerate() {
        let index = (pc as usize + 4 * slot) & 0x7fff;
        bus.iwram[index..index + 4].copy_from_slice(&word.to_le_bytes());
    }
}

fn batched(bus: &mut Bus) {
    bus.begin_device_batch();
    bus.elapse(1);
    begin_arm_iwram_slice(bus);
}

fn equal_bus(reference: &mut Bus, actual: &mut Bus) {
    assert_eq!(reference.now(), actual.now());
    assert_eq!(reference.open_bus_value, actual.open_bus_value);
    assert_eq!(reference.fetch_kind, actual.fetch_kind);
    assert_eq!(reference.game_pak_prefetch, actual.game_pak_prefetch);
    assert_eq!(&reference.iwram[..], &actual.iwram[..]);
    assert_eq!(&reference.ewram[..], &actual.ewram[..]);
    reference.end_device_batch();
    actual.end_device_batch();
    let (mut expected, mut got) = (Vec::new(), Vec::new());
    reference.write_state(&mut evenfall_state::Sink::new(Some(&mut expected), None)).unwrap();
    actual.write_state(&mut evenfall_state::Sink::new(Some(&mut got), None)).unwrap();
    assert_eq!(expected, got);
}

/// Zig: ARM kernel Game Pak ROM loads match ordinary timing values and prefetch state.
#[test]
fn arm_kernel_game_pak_rom_loads_match_ordinary_timing_values_and_prefetch_state() {
    let mut rom = vec![0; 0x1_0000];
    for (i, byte) in rom.iter_mut().enumerate() {
        *byte = (i as u8).wrapping_mul(29).wrapping_add(0x85);
    }
    let opcodes = [0xe1d1_20d0, 0xe1d1_20f0, 0xe1d1_20b0, 0xe1f1_20d1, 0xe591_2000, 0xe5d1_2000];
    let bases =
        [0x0800_0100, 0x0800_0103, 0x0a00_0102, 0x0c00_7ffe, 0x0900_0000, 0x0d00_0100, 0x0dff_ff01];
    for save_type in [save::Type::Sram, save::Type::Eeprom8k] {
        for waitcnt in [0x0000, 0x4317, 0x001b] {
            for opcode in opcodes {
                for base in bases {
                    let mut bus = Bus::init_with_save(&rom, save_type);
                    let mut reference_bus = Bus::init_with_save(&rom, save_type);
                    for b in [&mut bus, &mut reference_bus] {
                        b.write(
                            Access {
                                address: 0x0400_0204,
                                width: AccessWidth::Half,
                                kind: AccessKind::Nonsequential,
                                source: AccessSource::CpuData,
                            },
                            waitcnt,
                        );
                        batched(b);
                    }
                    let mut cpu = arm_cpu(opcode);
                    cpu.registers[1] = base;
                    load_pipeline(&mut bus, &cpu);
                    load_pipeline(&mut reference_bus, &cpu);
                    let before = cpu.clone();
                    let mut reference = cpu.clone();
                    let _ = reference.step(&mut reference_bus);
                    let start = bus.now();
                    let half = opcode & 0x0e00_0000 == 0 && opcode & 0x60 != 0x40;
                    let eeprom = save_type != save::Type::Sram && half && base >> 24 == 0x0d;
                    assert_eq!(
                        cpu.run_arm_iwram_kernel(&mut bus, start + 40),
                        !eeprom,
                        "{opcode:08x} {base:08x} {save_type:?} {waitcnt:04x}"
                    );
                    if eeprom {
                        assert_eq!(cpu, before);
                        assert_eq!(bus.now(), start);
                        bus.end_device_batch();
                        reference_bus.end_device_batch();
                        continue;
                    }
                    assert_eq!(cpu, reference);
                    equal_bus(&mut reference_bus, &mut bus);
                }
            }
        }
    }
}

fn expect_thumb_runner_matches_stepping(rom: &[u8], start: u32, waitcnt: u16, span: u64) {
    let mut bus = Bus::init(rom);
    let mut reference_bus = Bus::init(rom);
    for b in [&mut bus, &mut reference_bus] {
        b.write(
            Access {
                address: 0x0400_0204,
                width: AccessWidth::Half,
                kind: AccessKind::Nonsequential,
                source: AccessSource::CpuData,
            },
            u32::from(waitcnt),
        );
        b.begin_device_batch();
        b.elapse(100);
    }
    let mut cpu = Cpu::init(start, 0x3f, true);
    cpu.refill_pipeline(&mut bus);
    let mut reference = Cpu::init(start, 0x3f, true);
    reference.refill_pipeline(&mut reference_bus);
    let yield_at = bus.now() + span;
    while bus.now() < yield_at {
        let mut reason = None;
        let mut fault = None;
        match cpu.run_normal_until(&mut bus, yield_at, &mut reason, &mut fault) {
            ChunkOutcome::Deadline => break,
            ChunkOutcome::Generic => {
                let _ = cpu.step(&mut bus);
            }
            ChunkOutcome::Faulted => panic!("unexpected fault: {fault:?} {reason:?}"),
        }
    }
    while reference_bus.now() < yield_at {
        let _ = reference.step(&mut reference_bus);
    }
    assert_eq!(cpu, reference, "start={start:08x} waitcnt={waitcnt:04x}");
    equal_bus(&mut reference_bus, &mut bus);
}

fn fill_thumb_adds(rom: &mut [u8]) {
    for slot in rom.as_chunks_mut::<2>().0 {
        slot.copy_from_slice(&0x3701_u16.to_le_bytes());
    }
}

/// Zig: Thumb runner with prefetch runs sequentially across the GPIO view split.
#[test]
fn thumb_runner_with_prefetch_runs_sequentially_across_the_gpio_view_split() {
    let mut rom = vec![0; 0x800];
    fill_thumb_adds(&mut rom);
    for start in [0x0800_00c0, 0x0800_00c2, 0x0800_00c4, 0x0800_00c6, 0x0a00_00c0, 0x0c00_00c0] {
        for waitcnt in [0x4000, 0x4317] {
            expect_thumb_runner_matches_stepping(&rom, start, waitcnt, 200);
        }
    }
    let mut bus = Bus::init(&rom);
    bus.write(
        Access {
            address: 0x0400_0204,
            width: AccessWidth::Half,
            kind: AccessKind::Nonsequential,
            source: AccessSource::CpuData,
        },
        0x4000,
    );
    bus.begin_device_batch();
    bus.elapse(100);
    let mut cpu = Cpu::init(0x0800_00c8, 0x3f, true);
    cpu.refill_pipeline(&mut bus);
    let mut view = InstructionFetchRegion::default();
    bus.fetch_instruction_in_region(&mut view, 0x0800_00ca, AccessWidth::Half);
    let before = view;
    assert!(before.bytes.is_some());
    let deadline = bus.now() + 1000;
    assert_eq!(cpu.run_thumb_rom_kernel(&mut bus, &mut view, deadline), 0);
    assert_eq!(before.bytes, view.bytes);
    assert_eq!(before.address, view.address);
    assert_eq!(before.valid_starts, view.valid_starts);
    assert_eq!(before.region_class, view.region_class);
    bus.end_device_batch();
}

/// Zig: Thumb runner with prefetch runs sequentially across a 32 MiB image's window seam.
#[test]
fn thumb_runner_with_prefetch_runs_sequentially_across_a_32_mib_images_window_seam() {
    let mut rom = vec![0; 32 * 1024 * 1024];
    fill_thumb_adds(&mut rom);
    for start in [0x09ff_ffe0, 0x0bff_ffe0] {
        for waitcnt in [0x4000, 0x4317] {
            expect_thumb_runner_matches_stepping(&rom, start, waitcnt, 200);
        }
    }
}

/// Runs the ordinary runner, engine included, to `deadline`.
fn run_to(cpu: &mut Cpu, bus: &mut Bus, deadline: u64) {
    let mut reason = None;
    let mut fault = None;
    while bus.now() < deadline {
        match cpu.run_normal_until(bus, deadline, &mut reason, &mut fault) {
            ChunkOutcome::Deadline => {}
            ChunkOutcome::Generic => assert!(matches!(cpu.step(bus), StepOutcome::Executed(_))),
            ChunkOutcome::Faulted => panic!("unexpected fault: {fault:?} {reason:?}"),
        }
    }
}

/// Zig: ARM kernel stops fall-through before the supplied word leaves IWRAM.
/// The engine never runs in the last IWRAM mirror, so the ordinary runner
/// takes these instructions; it must still match single stepping.
#[test]
fn arm_kernel_stops_fall_through_before_the_supplied_word_leaves_iwram() {
    for branch_first in [false, true] {
        let mut bus = Bus::init(&[]);
        let mut reference_bus = Bus::init(&[]);
        for b in [&mut bus, &mut reference_bus] {
            for word in b.iwram.as_chunks_mut::<4>().0 {
                word.copy_from_slice(&0xe280_0001_u32.to_le_bytes());
            }
            batched(b);
        }
        let start = if branch_first { 0x03ff_ffe0 } else { 0x03ff_ffe8 };
        let mut cpu = Cpu::init(start, 0x1f, true);
        cpu.pipeline = Some(Pipeline {
            instructions: [if branch_first { 0xea00_0000 } else { 0xe280_0001 }, 0xe280_0001],
            instruction_set: InstructionSet::Arm,
        });
        let mut reference = cpu.clone();
        for _ in 0..if branch_first { 5 } else { 4 } {
            let _ = reference.step(&mut reference_bus);
        }
        let deadline = reference_bus.now();
        assert!(!cpu.clone().run_arm_iwram_kernel(&mut bus, deadline));
        run_to(&mut cpu, &mut bus, deadline);
        assert_eq!(cpu.core.next_instruction_address, 0x03ff_fff8);
        assert_eq!(cpu.registers[0], 4);
        assert_eq!(cpu, reference);
        equal_bus(&mut reference_bus, &mut bus);
    }
}

/// Zig: unified kernel failed conditions ignore invalid transfer and branch targets.
#[test]
fn unified_kernel_failed_conditions_ignore_invalid_transfer_and_branch_targets() {
    for opcode in [0x059f_f000, 0x081f_8000, 0x0aff_fffd] {
        let mut bus = Bus::init(&[]);
        let mut reference_bus = Bus::init(&[]);
        batched(&mut bus);
        batched(&mut reference_bus);
        let mut cpu = arm_cpu(opcode);
        load_pipeline(&mut bus, &cpu);
        load_pipeline(&mut reference_bus, &cpu);
        let mut reference = cpu.clone();
        let _ = reference.step(&mut reference_bus);
        let deadline = bus.now() + 1;
        assert!(cpu.run_arm_iwram_kernel(&mut bus, deadline));
        assert_eq!(cpu, reference);
        equal_bus(&mut reference_bus, &mut bus);
    }
}

/// Zig: unified kernel rejects register lists crossing RAM domains before writes.
#[test]
fn unified_kernel_rejects_register_lists_crossing_ram_domains_before_writes() {
    for base in [0x02ff_fffc, 0x03ff_fffc, 0xffff_fffc] {
        let mut bus = Bus::init(&[]);
        batched(&mut bus);
        let mut cpu = arm_cpu(0xe881_0005);
        cpu.registers[1] = base;
        let before = cpu.clone();
        let start = bus.now();
        assert!(!cpu.run_arm_iwram_kernel(&mut bus, start + 30));
        assert_eq!(cpu, before);
        assert_eq!(bus.now(), start);
        assert!(bus.iwram.iter().all(|&byte| byte == 0));
        assert!(bus.ewram.iter().all(|&byte| byte == 0));
        bus.end_device_batch();
    }
}

/// Zig: unified kernel retains context through backward branch and link refill.
#[test]
fn unified_kernel_retains_context_through_backward_branch_and_link_refill() {
    for link in [false, true] {
        let branch: u32 = if link { 0xebff_fffd } else { 0xeaff_fffd };
        let mut bus = Bus::init(&[]);
        let mut reference_bus = Bus::init(&[]);
        for b in [&mut bus, &mut reference_bus] {
            b.iwram[0..4].copy_from_slice(&0xe280_0007_u32.to_le_bytes());
            b.iwram[4..8].copy_from_slice(&branch.to_le_bytes());
            b.iwram[8..12].copy_from_slice(&0xe280_0001_u32.to_le_bytes());
            batched(b);
        }
        let mut cpu = arm_cpu(0xe280_0007);
        cpu.pipeline.as_mut().unwrap().instructions[1] = branch;
        let mut reference = cpu.clone();
        for _ in 0..8 {
            let _ = reference.step(&mut reference_bus);
        }
        let deadline = bus.now() + 16;
        assert!(cpu.run_arm_iwram_kernel(&mut bus, deadline));
        assert_eq!(cpu.registers[0], 28);
        assert_eq!(cpu, reference);
        equal_bus(&mut reference_bus, &mut bus);
    }
}

/// Zig: unified kernel stores retain latched opcodes but change later fetches.
#[test]
fn unified_kernel_stores_retain_latched_opcodes_but_change_later_fetches() {
    for offset in [4_u32, 8, 12] {
        let mut bus = Bus::init(&[]);
        let mut reference_bus = Bus::init(&[]);
        for b in [&mut bus, &mut reference_bus] {
            b.iwram[8..12].copy_from_slice(&0xe280_0003_u32.to_le_bytes());
            b.iwram[12..16].copy_from_slice(&0xe280_0005_u32.to_le_bytes());
            b.iwram[16..20].copy_from_slice(&0xef08_0000_u32.to_le_bytes());
            batched(b);
        }
        let mut cpu = arm_cpu(0xe581_2000);
        cpu.pipeline.as_mut().unwrap().instructions[1] = 0xe280_0001;
        load_pipeline(&mut bus, &cpu);
        load_pipeline(&mut reference_bus, &cpu);
        cpu.registers[1] = 0x0300_0000 + offset;
        cpu.registers[2] = 0xe280_0011;
        let mut reference = cpu.clone();
        for _ in 0..4 {
            let _ = reference.step(&mut reference_bus);
        }
        // The engine leaves stores into code pages to the interpreter.
        let deadline = reference_bus.now();
        run_to(&mut cpu, &mut bus, deadline);
        assert_eq!(cpu.registers[0], if offset == 12 { 21 } else { 9 });
        assert_eq!(cpu, reference);
        equal_bus(&mut reference_bus, &mut bus);
    }
}

/// Zig: execution region signed loads preserve indexing sign and last bus access.
#[test]
fn execution_region_signed_loads_preserve_indexing_sign_and_last_bus_access() {
    let cases = [
        (0xe1d1_20d0, 0x0300_0100, 0x23, 0x0300_0100, 0x23),
        (0xe1d1_20d1, 0x0300_0100, 0xffff_ff81, 0x0300_0100, 0x81),
        (0xe1d1_20f0, 0x03ff_8100, 0xffff_8123, 0x03ff_8100, 0x8123),
        (0xe1f1_20f1, 0x0300_0100, 0xffff_ff81, 0x0300_0101, 0x8123),
        (0xe011_20d3, 0x0300_0101, 0xffff_ff81, 0x0300_00fe, 0x81),
        (0xe1f1_10f1, 0x0300_0100, 0xffff_ff81, 0xffff_ff81, 0x8123),
    ];
    for (opcode, base, value, adjusted, raw) in cases {
        for passed in [false, true] {
            for following_alu in [false, true] {
                let mut bus = Bus::init(&[]);
                let mut reference_bus = Bus::init(&[]);
                for b in [&mut bus, &mut reference_bus] {
                    b.iwram[0x100] = 0x23;
                    b.iwram[0x101] = 0x81;
                    b.iwram[8..12].copy_from_slice(&0xef08_0000_u32.to_le_bytes());
                    b.iwram[12..16].copy_from_slice(&0xe287_7005_u32.to_le_bytes());
                    batched(b);
                }
                let mut cpu = Cpu::init(0x0300_0000, 0xa000_001f, true);
                cpu.pipeline = Some(Pipeline {
                    instructions: [
                        if passed { opcode } else { opcode & 0x0fff_ffff },
                        if following_alu { 0xe286_6007 } else { 0xef08_0000 },
                    ],
                    instruction_set: InstructionSet::Arm,
                });
                cpu.registers[1] = base;
                cpu.registers[2] = 0x1234;
                cpu.registers[3] = 3;
                load_pipeline(&mut bus, &cpu);
                load_pipeline(&mut reference_bus, &cpu);
                let before = cpu.clone();
                let mut reference = cpu.clone();
                let _ = reference.step(&mut reference_bus);
                if following_alu {
                    let _ = reference.step(&mut reference_bus);
                }
                let started = bus.now();
                assert!(cpu.run_arm_iwram_kernel(&mut bus, started + 10));
                assert_eq!(cpu, reference);
                let dest = ((opcode >> 12) & 15) as usize;
                assert_eq!(
                    cpu.registers[dest],
                    if passed { value } else { before.registers[dest] }
                );
                assert_eq!(cpu.registers[1], if passed { adjusted } else { base });
                assert_eq!(cpu.cpsr, 0xa000_001f);
                assert_eq!(
                    bus.now() - started,
                    if passed { 3 } else { 1 } + u64::from(following_alu)
                );
                assert_eq!(
                    bus.open_bus_value,
                    if following_alu {
                        0xe287_7005
                    } else if passed {
                        raw
                    } else {
                        0xef08_0000
                    }
                );
                equal_bus(&mut reference_bus, &mut bus);
            }
        }
    }
}

/// Zig: execution region signed loads refuse short budgets devices and invalid registers.
#[test]
fn execution_region_signed_loads_refuse_short_budgets_devices_and_invalid_registers() {
    let cases = [
        (0xe1d1_20d0, 0x0300_0100, 2),
        (0xe1d1_20d0, 0x0400_0000, 10),
        (0xe1d1_f0d0, 0x0300_0100, 10),
        (0xe191_20df, 0x0300_0100, 10),
        (0xe1ff_20f0, 0x0300_0100, 10),
    ];
    for (opcode, base, budget) in cases {
        let mut bus = Bus::init(&[]);
        batched(&mut bus);
        let mut cpu = arm_cpu(opcode);
        cpu.registers[1] = base;
        let before = cpu.clone();
        let started = bus.now();
        let open = bus.open_bus_value;
        let kind = bus.fetch_kind;
        assert!(!cpu.run_arm_iwram_kernel(&mut bus, started + budget));
        assert_eq!(cpu, before);
        assert_eq!(bus.now(), started);
        assert_eq!(bus.open_bus_value, open);
        assert_eq!(bus.fetch_kind, kind);
        bus.end_device_batch();
    }
}

/// Zig: execution region multiply keeps original alias timing and conditional effects.
#[test]
fn execution_region_multiply_keeps_original_alias_timing_and_conditional_effects() {
    let cases = [
        (0x7f_u32, 0x7f00_017d_u32, 1_u64),
        (0x100, 0x300, 2),
        (0xffff_7fff, 0xfefe_7ffd, 2),
        (0x0100_0000, 0x0300_0000, 4),
    ];
    for (multiplier, expected_product, iterations) in cases {
        for accumulate in [false, true] {
            for passed in [false, true] {
                for budget in [5, 6] {
                    let mut bus = Bus::init(&[]);
                    batched(&mut bus);
                    let opcode = (if accumulate { 0x0032_3291 } else { 0x0012_0291 })
                        | if passed { 0xe000_0000 } else { 0 };
                    let mut cpu = Cpu::init(0x0300_0000, 0xa000_001f, true);
                    cpu.pipeline = Some(Pipeline {
                        instructions: [opcode, 0xef08_0000],
                        instruction_set: InstructionSet::Arm,
                    });
                    cpu.registers[1] = 0x0100_0003;
                    cpu.registers[2] = multiplier;
                    cpu.registers[3] = 11;
                    load_pipeline(&mut bus, &cpu);
                    let before = cpu.clone();
                    let start = bus.now();
                    let ran = cpu.run_arm_iwram_kernel(&mut bus, start + budget);
                    let required = if passed { 1 + iterations + u64::from(accumulate) } else { 1 };
                    assert_eq!(ran, budget >= required);
                    if !ran {
                        assert_eq!(cpu, before);
                        assert_eq!(bus.now(), start);
                    } else {
                        let internal = if passed { iterations + u64::from(accumulate) } else { 0 };
                        let product = if passed {
                            expected_product.wrapping_add(if accumulate { 11 } else { 0 })
                        } else {
                            multiplier
                        };
                        assert_eq!(bus.now(), start + 1 + internal);
                        assert_eq!(cpu.registers[2], product);
                        assert_eq!(
                            cpu.cpsr,
                            if passed { 0x2000_001f | product & 0x8000_0000 } else { 0xa000_001f }
                        );
                        assert_eq!(cpu.pipeline.unwrap().instructions[0], 0xef08_0000);
                    }
                    bus.end_device_batch();
                }
            }
        }
    }
}

/// Zig: execution region admission distinguishes eligible IRQ and masked IRQ and DMA ownership.
#[test]
fn execution_region_admission_distinguishes_eligible_irq_and_masked_irq_and_dma_ownership() {
    for case in 0..5 {
        let mut bus = Bus::init(&[]);
        batched(&mut bus);
        let mut cpu = Cpu::init(0x0300_0000, 0x1f, true);
        cpu.pipeline =
            Some(Pipeline { instructions: [0xe280_0003; 2], instruction_set: InstructionSet::Arm });
        for slot in bus.iwram[..24].as_chunks_mut::<4>().0 {
            slot.copy_from_slice(&0xe280_0003_u32.to_le_bytes());
        }
        match case {
            0 => bus.irq_line = true,
            1 => {
                bus.irq_line = true;
                cpu.cpsr |= 1 << 7;
            }
            2 => bus.pending_dma = 1,
            3 => bus.servicing_dma = true,
            _ => {}
        }
        let before = cpu.clone();
        let started = bus.now();
        let deadline = if case == 4 { started } else { started + 3 };
        let executed = cpu.run_arm_iwram_kernel(&mut bus, deadline);
        assert_eq!(executed, case == 1, "case={case}");
        if case == 1 {
            assert_eq!(bus.now(), started + 3);
            assert_eq!(cpu.registers[0], 9);
        } else {
            assert_eq!(bus.now(), started);
            assert_eq!(cpu, before);
        }
        bus.end_device_batch();
    }
}
