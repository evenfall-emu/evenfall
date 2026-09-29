//! ZiGBA `test/machine_test.zig`, first-half machine integration oracles.

use evenfall_cpu_arm::Pipeline;
use evenfall_gba::bus::{
    Access, AccessKind, AccessSource, AccessWidth, KeyState, PowerState, TimedInput,
    TimedInputChange,
};
use evenfall_gba::cpu::{Cpu, InstructionSet, WaitReason};
use evenfall_gba::machine::{
    BootMode, Config, Machine, RunError, StepOutcome, StepResult, StopReason,
};

fn executed_step(machine: &mut Machine) -> StepResult {
    match machine.step() {
        StepOutcome::Executed(step) => step,
        outcome => panic!("expected executed instruction, got {outcome:?}"),
    }
}

fn write_words(bytes: &mut [u8], offset: usize, words: &[u32]) {
    for (index, word) in words.iter().enumerate() {
        bytes[offset + index * 4..][..4].copy_from_slice(&word.to_le_bytes());
    }
}

fn condition_truth_oracle(condition: u8, n: bool, z: bool, c: bool, v: bool) -> bool {
    match condition {
        0x0 => z,
        0x1 => !z,
        0x2 => c,
        0x3 => !c,
        0x4 => n,
        0x5 => !n,
        0x6 => v,
        0x7 => !v,
        0x8 => c && !z,
        0x9 => !c || z,
        0xA => n == v,
        0xB => n != v,
        0xC => !z && n == v,
        0xD => z || n != v,
        0xE => true,
        0xF => false,
        _ => unreachable!(),
    }
}

/// Zig: `condition truth table ARM covers every NZCV assignment through generic and normal execution`.
#[test]
fn condition_truth_table_arm_covers_every_nzcv_assignment_through_generic_and_normal_execution() {
    for flag_bits in 0..16_u32 {
        let n = flag_bits & 8 != 0;
        let z = flag_bits & 4 != 0;
        let c = flag_bits & 2 != 0;
        let v = flag_bits & 1 != 0;
        let flags =
            u32::from(n) << 31 | u32::from(z) << 30 | u32::from(c) << 29 | u32::from(v) << 28;
        for condition in 0..16_u32 {
            let passed = condition_truth_oracle(condition as u8, n, z, c, v);
            for background in [0x0000_009F, 0x0FFF_FFDF] {
                let cpsr = flags | background;
                let instruction = condition << 28 | 0x03A0_005A;
                let mut machines =
                    [Machine::new(Config::default()), Machine::new(Config::default())];
                for machine in &mut machines {
                    machine.cpu = Cpu::init(0x0300_1000, cpsr, true);
                    machine.cpu.registers[0] = 0xA5A5_5A5A;
                    machine.cpu.registers[1] = 0x1020_3040;
                    machine.cpu.registers[2] = 0x5566_7788;
                    machine.cpu.pipeline = Some(Pipeline {
                        instructions: [instruction, 0xE281_1001],
                        instruction_set: InstructionSet::Arm,
                    });
                    write_words(
                        &mut machine.bus.iwram[..],
                        0x1000,
                        &[instruction, 0xE281_1001, 0xE282_2001],
                    );
                }
                let mut expected_registers = machines[0].cpu.registers;
                expected_registers[0] = if passed { 0x5A } else { 0xA5A5_5A5A };
                expected_registers[15] = 0x0300_100C;
                let step = executed_step(&mut machines[0]);
                let result = machines[1].run_until(1).unwrap();
                assert_eq!(
                    (step.started_at, step.stopped_at, step.instruction),
                    (0, 1, instruction)
                );
                assert_eq!(result.reason, StopReason::Deadline);
                for machine in &machines {
                    assert_eq!(machine.now(), 1);
                    assert_eq!(machine.next_instruction_address(), 0x0300_1004);
                    assert_eq!(machine.read_cpsr(), cpsr);
                    assert_eq!(machine.cpu.registers, expected_registers);
                    assert_eq!(
                        machine.cpu.pipeline.unwrap().instructions,
                        [0xE281_1001, 0xE282_2001]
                    );
                    assert_eq!(machine.cpu.pipeline.unwrap().instruction_set, InstructionSet::Arm);
                }
            }
        }
    }
}

/// Zig: `condition truth table Thumb covers every branch condition and NZCV assignment at a deadline`.
#[test]
fn condition_truth_table_thumb_covers_every_branch_condition_and_nzcv_assignment_at_a_deadline() {
    for flag_bits in 0..16_u32 {
        let n = flag_bits & 8 != 0;
        let z = flag_bits & 4 != 0;
        let c = flag_bits & 2 != 0;
        let v = flag_bits & 1 != 0;
        let flags =
            u32::from(n) << 31 | u32::from(z) << 30 | u32::from(c) << 29 | u32::from(v) << 28;
        for condition in 0..14_u32 {
            let passed = condition_truth_oracle(condition as u8, n, z, c, v);
            for background in [0x0000_00BF, 0x0FFF_FFFF] {
                let cpsr = flags | background;
                let instruction = 0xD004_u16 | (condition as u16) << 8;
                let mut machines =
                    [Machine::new(Config::default()), Machine::new(Config::default())];
                for machine in &mut machines {
                    machine.cpu = Cpu::init(0x0300_1000, cpsr, true);
                    machine.cpu.registers[15] = 0x0300_1004;
                    machine.cpu.registers[0] = 0x1020_3040;
                    machine.cpu.registers[2] = 0x5060_7080;
                    machine.cpu.registers[4] = 0x1122_3344;
                    machine.cpu.registers[5] = 0x5566_7788;
                    machine.cpu.pipeline = Some(Pipeline {
                        instructions: [u32::from(instruction), 0x3001],
                        instruction_set: InstructionSet::Thumb,
                    });
                    for (offset, half) in [
                        (0x1000, instruction),
                        (0x1002, 0x3001),
                        (0x1004, 0x3201),
                        (0x100C, 0x3401),
                        (0x100E, 0x3501),
                    ] {
                        machine.bus.iwram[offset..offset + 2].copy_from_slice(&half.to_le_bytes());
                    }
                }
                let mut expected_registers = machines[0].cpu.registers;
                expected_registers[15] = if passed { 0x0300_1010 } else { 0x0300_1006 };
                let expected_pc = if passed { 0x0300_100C } else { 0x0300_1002 };
                let expected_pipeline = if passed { [0x3401, 0x3501] } else { [0x3001, 0x3201] };
                let expected_cycles = if passed { 3 } else { 1 };
                let step = executed_step(&mut machines[0]);
                let result = machines[1].run_until(1).unwrap();
                assert_eq!(
                    (step.started_at, step.stopped_at, step.instruction),
                    (0, expected_cycles, u32::from(instruction))
                );
                assert_eq!(
                    result.reason,
                    if passed {
                        StopReason::InstructionBoundaryAfterDeadline
                    } else {
                        StopReason::Deadline
                    }
                );
                for machine in &machines {
                    assert_eq!(machine.now(), expected_cycles);
                    assert_eq!(machine.next_instruction_address(), expected_pc);
                    assert_eq!(machine.read_cpsr(), cpsr);
                    assert_eq!(machine.cpu.registers, expected_registers);
                    assert_eq!(machine.cpu.pipeline.unwrap().instructions, expected_pipeline);
                    assert_eq!(
                        machine.cpu.pipeline.unwrap().instruction_set,
                        InstructionSet::Thumb
                    );
                }
            }
        }
    }
}

/// Zig: `HLE boot exposes the post-BIOS System stack pointer and forced blank`.
#[test]
fn hle_boot_exposes_the_post_bios_system_stack_pointer_and_forced_blank() {
    let mut machine = Machine::new(Config { rom: &[0], boot: BootMode::Hle, ..Config::default() });
    assert_eq!(machine.read_register(13), 0x0300_7F00);
    assert_eq!(machine.read_cpsr() & 0x1F, 0x1F);
    assert_eq!(
        machine
            .bus
            .read(Access {
                address: 0x0400_0000,
                width: AccessWidth::Half,
                kind: AccessKind::Nonsequential,
                source: AccessSource::CpuData
            })
            .value,
        0x0080
    );
}

/// Zig: `runUntil stops at an exact deadline or the first instruction boundary after it`.
#[test]
fn run_until_stops_at_an_exact_deadline_or_the_first_instruction_boundary_after_it() {
    let rom = [0x01, 0x00, 0xA0, 0xE3, 0x02, 0x10, 0xA0, 0xE3];
    let mut machine = Machine::new(Config { rom: &rom, ..Config::default() });
    let first = machine.run_until(7).unwrap();
    assert_eq!(
        (first.started_at, first.stopped_at, first.reason),
        (0, 8, StopReason::InstructionBoundaryAfterDeadline)
    );
    assert_eq!(machine.read_register(0), 1);
    assert_eq!(machine.run_until(7), Err(RunError::TargetBeforeCurrentCycle));
    let second = machine.run_until(14).unwrap();
    assert_eq!((second.stopped_at, second.reason), (14, StopReason::Deadline));
    assert_eq!(machine.read_register(1), 2);
    let repeated = machine.run_until(14).unwrap();
    assert_eq!(
        (repeated.started_at, repeated.stopped_at, repeated.reason),
        (14, 14, StopReason::Deadline)
    );
}

/// Zig: `host yield points do not change input timing inside atomic instructions`.
#[test]
fn host_yield_points_do_not_change_input_timing_inside_atomic_instructions() {
    let rom = [0xFE, 0xFF, 0xFF, 0xEA];
    let mut uninterrupted = Machine::new(Config { rom: &rom, ..Config::default() });
    let mut sliced = Machine::new(Config { rom: &rom, ..Config::default() });
    let inputs = [
        TimedInput {
            at: 11,
            change: TimedInputChange::Keys(KeyState { a: true, ..KeyState::default() }),
        },
        TimedInput {
            at: 17,
            change: TimedInputChange::Keys(KeyState { b: true, ..KeyState::default() }),
        },
        TimedInput {
            at: 65,
            change: TimedInputChange::Keys(KeyState { start: true, ..KeyState::default() }),
        },
    ];
    let full = uninterrupted.run_until_with_input(64, &inputs).unwrap();
    let first = sliced.run_slice_with_input(10, 64, &inputs).unwrap();
    assert!(first.stopped_at > 17);
    assert_eq!(first.inputs_consumed, 2);
    let last = sliced.run_slice_with_input(64, 64, &inputs[first.inputs_consumed..]).unwrap();
    assert_eq!(full.stopped_at, last.stopped_at);
    assert_eq!(full.inputs_consumed, first.inputs_consumed + last.inputs_consumed);
    assert_eq!(uninterrupted.inspect_cpu(), sliced.inspect_cpu());
    let mut a = [0; 2];
    let mut b = [0; 2];
    uninterrupted.inspect_memory(0x0400_0130, &mut a).unwrap();
    sliced.inspect_memory(0x0400_0130, &mut b).unwrap();
    assert_eq!(a, b);
    assert_eq!(full.inputs_consumed, 2);
    let before = sliced.inspect_cpu();
    assert_eq!(
        sliced.run_slice_with_input(sliced.now() + 10, sliced.now() + 5, &[]),
        Err(RunError::TargetBeforeCurrentCycle)
    );
    assert_eq!(before, sliced.inspect_cpu());
}

/// Zig: `timestamped input wakes Halt at its exact master cycle`.
#[test]
fn timestamped_input_wakes_halt_at_its_exact_master_cycle() {
    let mut machine = Machine::new(Config::default());
    machine.bus.keypad_control = (1 << 14) | 1;
    machine.bus.enter_halt();
    let inputs = [
        TimedInput {
            at: 20,
            change: TimedInputChange::Keys(KeyState { a: true, ..KeyState::default() }),
        },
        TimedInput { at: 20, change: TimedInputChange::Keys(KeyState::default()) },
    ];
    let result = machine.run_until_with_input(20, &inputs).unwrap();
    assert_eq!(result.inputs_consumed, 2);
    assert_eq!(machine.now(), 20);
    assert_eq!(result.reason, StopReason::Deadline);
    assert_ne!(machine.bus.interrupt_request_flags & (1 << 12), 0);
    assert_eq!(machine.bus.keys, KeyState::default());
}

/// Zig: `timestamped input validates its absolute ordering`.
#[test]
fn timestamped_input_validates_its_absolute_ordering() {
    let mut machine = Machine::new(Config::default());
    machine.run_until(8).unwrap();
    assert_eq!(
        machine.run_until_with_input(
            8,
            &[TimedInput {
                at: 7,
                change: TimedInputChange::Keys(KeyState { a: true, ..KeyState::default() })
            }]
        ),
        Err(RunError::InputBeforeCurrentCycle)
    );
    assert_eq!(
        machine.run_until_with_input(
            20,
            &[
                TimedInput {
                    at: 12,
                    change: TimedInputChange::Keys(KeyState { a: true, ..KeyState::default() })
                },
                TimedInput { at: 11, change: TimedInputChange::Keys(KeyState::default()) },
            ]
        ),
        Err(RunError::InputsOutOfOrder)
    );
}

/// Zig: `Machine inspection is coherent and has no MMIO or timing side effects`.
#[test]
fn machine_inspection_is_coherent_and_has_no_mmio_or_timing_side_effects() {
    let mut machine = Machine::new(Config::default());
    machine.bus.write(
        Access {
            address: 0x0200_0040,
            width: AccessWidth::Word,
            kind: AccessKind::Nonsequential,
            source: AccessSource::CpuData,
        },
        0xA1B2_C3D4,
    );
    machine.cpu.registers[4] = 0xCAFE_BABE;
    machine.bus.joy_receive = 0x1122_3344;
    machine.bus.joy_status |= 1 << 1;
    let started_at = machine.now();
    let cpu = machine.inspect_cpu();
    let mut ewram = [0; 4];
    machine.inspect_memory(0x0200_0040, &mut ewram).unwrap();
    let mut joy_receive = [0; 4];
    machine.inspect_memory(0x0400_0150, &mut joy_receive).unwrap();
    assert_eq!(cpu.cycle, started_at);
    assert_eq!(cpu.registers[4], 0xCAFE_BABE);
    assert_eq!(ewram, [0xD4, 0xC3, 0xB2, 0xA1]);
    assert_eq!(joy_receive, [0x44, 0x33, 0x22, 0x11]);
    assert_ne!(machine.bus.joy_status & (1 << 1), 0);
    assert_eq!(machine.now(), started_at);
    assert_eq!(
        machine.inspect_memory(0xFFFF_FFFF, &mut [0; 2]),
        Err(evenfall_gba::machine::AddressOverflow)
    );
}

/// Zig: `external BIOS boot begins at reset vector in masked supervisor mode`.
#[test]
fn external_bios_boot_begins_at_reset_vector_in_masked_supervisor_mode() {
    let mut bios = [0; 0x4000];
    write_words(&mut bios, 0, &[0xE3A0_0009]);
    let mut machine =
        Machine::new(Config { bios: &bios, boot: BootMode::ExternalBios, ..Config::default() });
    let reset = executed_step(&mut machine);
    assert_eq!(reset.instruction_address, 0);
    assert_eq!(machine.read_register(0), 9);
    assert_eq!(machine.read_cpsr() & 0x1F, 0x13);
    assert_ne!(machine.read_cpsr() & (1 << 7), 0);
    assert_ne!(machine.read_cpsr() & (1 << 6), 0);
}

/// Zig: `HLE MultiBoot rejects malformed parameters and returns to the following instruction`.
#[test]
fn hle_multi_boot_rejects_malformed_parameters_and_returns_to_the_following_instruction() {
    let mut rom = [0; 8];
    write_words(&mut rom, 0, &[0xEF25_0000, 0xE3A0_205A]);
    let mut machine = Machine::new(Config { rom: &rom, ..Config::default() });
    machine.cpu.registers[0] = 0x0300_0000;
    machine.cpu.registers[1] = 0;
    let call = executed_step(&mut machine);
    let following = executed_step(&mut machine);
    assert_eq!(call.instruction_address, 0x0800_0000);
    assert_eq!(machine.read_register(0), 1);
    assert_eq!(following.instruction_address, 0x0800_0004);
    assert_eq!(machine.read_register(2), 0x5A);
    assert!(machine.cpu.hle_service.is_none());
}

/// Zig: `HLE RegisterRamReset clears selected memories but preserves the BIOS work area`.
#[test]
fn hle_register_ram_reset_clears_selected_memories_but_preserves_the_bios_work_area() {
    let rom = [0, 0, 1, 0xEF];
    let mut machine = Machine::new(Config { rom: &rom, ..Config::default() });
    machine.bus.ewram.fill(0xA5);
    machine.bus.iwram.fill(0xA5);
    machine.bus.palette.fill(0xA5);
    machine.bus.vram.fill(0xA5);
    machine.bus.oam.fill(0xA5);
    machine.cpu.registers[0] = 0x1F;
    executed_step(&mut machine);
    assert!(machine.bus.ewram.iter().all(|&byte| byte == 0));
    assert!(machine.bus.iwram[..0x7E00].iter().all(|&byte| byte == 0));
    assert!(machine.bus.iwram[0x7E00..].iter().all(|&byte| byte == 0xA5));
    assert!(machine.bus.palette.iter().all(|&byte| byte == 0));
    assert!(machine.bus.vram.iter().all(|&byte| byte == 0));
    assert!(machine.bus.oam.iter().all(|&byte| byte == 0));
    assert_eq!(machine.bus.dispcnt, 0x0080);
}

/// Zig: `runUntil bounds Halt with no enabled wake source at its deadline`.
#[test]
fn run_until_bounds_halt_with_no_enabled_wake_source_at_its_deadline() {
    let mut rom = [0; 8];
    write_words(&mut rom, 0, &[0xEF02_0000, 0xE3A0_0007]);
    let mut machine = Machine::new(Config { rom: &rom, ..Config::default() });
    executed_step(&mut machine);
    let target = machine.now() + 100;
    let result = machine.run_until(target).unwrap();
    assert_eq!(
        (result.reason, result.stopped_at, machine.now()),
        (StopReason::Deadline, target, target)
    );
    assert_eq!(machine.read_register(0), 0);
    assert_eq!(machine.next_instruction_address(), 0x0800_0004);
    assert_eq!(machine.bus.power_state(), PowerState::Halted);
}

/// Zig: `HLE IntrWait consumes a selected old BIOS IRQ flag without sleeping`.
#[test]
fn hle_intr_wait_consumes_a_selected_old_bios_irq_flag_without_sleeping() {
    let rom = [0, 0, 4, 0xEF];
    let mut machine = Machine::new(Config { rom: &rom, ..Config::default() });
    machine.cpu.registers[0] = 0;
    machine.cpu.registers[1] = 1 << 3;
    machine.bus.iwram[0x7FF8..0x7FFA].copy_from_slice(&(1_u16 << 3).to_le_bytes());
    executed_step(&mut machine);
    assert_eq!(machine.bus.power_state(), PowerState::Running);
    assert!(machine.bus.interrupt_master_enabled);
    assert_eq!(&machine.bus.iwram[0x7FF8..0x7FFA], &[0, 0]);
}

/// Zig: `HLE VBlankIntrWait discards old VBlank state and enters Halt`.
#[test]
fn hle_vblank_intr_wait_discards_old_vblank_state_and_enters_halt() {
    let rom = [0, 0, 5, 0xEF];
    let mut machine = Machine::new(Config { rom: &rom, ..Config::default() });
    machine.cpu.registers[0] = 0xAAAA_AAAA;
    machine.cpu.registers[1] = 0xBBBB_BBBB;
    machine.bus.iwram[0x7FF8..0x7FFA].copy_from_slice(&1_u16.to_le_bytes());
    executed_step(&mut machine);
    assert_eq!((machine.read_register(0), machine.read_register(1)), (1, 1));
    assert_eq!(machine.bus.power_state(), PowerState::Halted);
    assert!(machine.bus.interrupt_master_enabled);
    assert_eq!(&machine.bus.iwram[0x7FF8..0x7FFA], &[0, 0]);
}

/// Zig: `direct step while halted or stopped returns without advancing emulated state`.
#[test]
fn direct_step_while_halted_or_stopped_returns_without_advancing_emulated_state() {
    let rom = [7, 0, 0xA0, 0xE3];
    for state in [WaitReason::Halted, WaitReason::Stopped] {
        let mut machine = Machine::new(Config { rom: &rom, ..Config::default() });
        if state == WaitReason::Stopped {
            machine.bus.enter_stop();
            machine.bus.interrupt_enable_mask = 1 << 3;
            machine.bus.interrupt_request_flags = 1 << 3;
            machine.bus.interrupt_master_enabled = true;
            machine.bus.update_irq_line();
        } else {
            machine.bus.enter_halt();
        }
        let waiting_at = machine.now();
        let instruction_address = machine.next_instruction_address();
        let ppu_line = machine.bus.ppu_timing.line;
        let ppu_cycle = machine.bus.ppu_timing.cycle_in_line;
        assert_eq!(machine.step(), StepOutcome::Waiting(state));
        assert_eq!(machine.now(), waiting_at);
        assert_eq!(machine.next_instruction_address(), instruction_address);
        assert_eq!(machine.bus.ppu_timing.line, ppu_line);
        assert_eq!(machine.bus.ppu_timing.cycle_in_line, ppu_cycle);
        assert_eq!(
            machine.bus.power_state(),
            if state == WaitReason::Stopped { PowerState::Stopped } else { PowerState::Halted }
        );
    }
}

/// Zig: `direct step resumes a pending Halt wake independently of IME`.
#[test]
fn direct_step_resumes_a_pending_halt_wake_independently_of_ime() {
    let rom = [7, 0, 0xA0, 0xE3];
    let mut machine = Machine::new(Config { rom: &rom, ..Config::default() });
    machine.bus.interrupt_enable_mask = 1 << 3;
    machine.bus.interrupt_request_flags = 1 << 3;
    machine.bus.interrupt_master_enabled = false;
    machine.bus.update_irq_line();
    machine.bus.enter_halt();
    let step = executed_step(&mut machine);
    assert_eq!(step.started_at, 0);
    assert_eq!(step.instruction_address, 0x0800_0000);
    assert_eq!(machine.read_register(0), 7);
    assert_eq!(machine.bus.power_state(), PowerState::Running);
}

/// Zig: `stepWithInput applies a same-cycle keypad wake before stepping Stop`.
#[test]
fn step_with_input_applies_a_same_cycle_keypad_wake_before_stepping_stop() {
    let rom = [7, 0, 0xA0, 0xE3];
    let mut machine = Machine::new(Config { rom: &rom, ..Config::default() });
    machine.bus.keypad_control = (1 << 14) | 1;
    machine.bus.interrupt_enable_mask = 1 << 12;
    machine.bus.update_irq_line();
    machine.bus.enter_stop();
    let result = machine
        .step_with_input(&[TimedInput {
            at: machine.now(),
            change: TimedInputChange::Keys(KeyState { a: true, ..KeyState::default() }),
        }])
        .unwrap();
    assert_eq!(result.inputs_consumed, 1);
    match result.outcome {
        StepOutcome::Executed(step) => assert_eq!(step.instruction_address, 0x0800_0000),
        other => panic!("expected executed instruction, got {other:?}"),
    }
    assert_eq!(machine.read_register(0), 7);
    assert_eq!(machine.bus.power_state(), PowerState::Running);
}

/// Zig: `stepWithInput retains a future input while Halt is waiting`.
#[test]
fn step_with_input_retains_a_future_input_while_halt_is_waiting() {
    let mut machine = Machine::new(Config::default());
    machine.bus.keypad_control = (1 << 14) | 1;
    machine.bus.interrupt_enable_mask = 1 << 12;
    machine.bus.update_irq_line();
    machine.bus.enter_halt();
    let result = machine
        .step_with_input(&[TimedInput {
            at: machine.now() + 1,
            change: TimedInputChange::Keys(KeyState { a: true, ..KeyState::default() }),
        }])
        .unwrap();
    assert_eq!(result.inputs_consumed, 0);
    assert_eq!(result.outcome, StepOutcome::Waiting(WaitReason::Halted));
    assert_eq!(machine.now(), 0);
    assert!(!machine.bus.keys.a);
}

/// Zig: `waitUntilWithInput reaches a future keypad wake without executing the guest`.
#[test]
fn wait_until_with_input_reaches_a_future_keypad_wake_without_executing_the_guest() {
    let rom = [7, 0, 0xA0, 0xE3];
    let mut machine = Machine::new(Config { rom: &rom, ..Config::default() });
    machine.bus.keypad_control = (1 << 14) | 1;
    machine.bus.interrupt_enable_mask = 1 << 12;
    machine.bus.update_irq_line();
    machine.bus.enter_halt();
    let address = machine.next_instruction_address();
    let input_at = machine.now() + 37;
    let result = machine
        .wait_until_with_input(
            input_at + 100,
            &[TimedInput {
                at: input_at,
                change: TimedInputChange::Keys(KeyState { a: true, ..KeyState::default() }),
            }],
        )
        .unwrap();
    assert_eq!(
        (result.started_at, result.stopped_at, result.reason),
        (0, input_at, WaitReason::Halted)
    );
    assert!(result.woke);
    assert_eq!(result.inputs_consumed, 1);
    assert_eq!(machine.next_instruction_address(), address);
    assert_eq!(machine.read_register(0), 0);
    assert!(machine.bus.keys.a);
    assert_eq!(machine.bus.power_state(), PowerState::Halted);
    let resumed = executed_step(&mut machine);
    assert_eq!(resumed.started_at, input_at);
    assert_eq!(machine.read_register(0), 7);
    assert_eq!(machine.bus.power_state(), PowerState::Running);
}

/// Zig: `waitUntil bounds Halt without a wake and executes no guest instruction`.
#[test]
fn wait_until_bounds_halt_without_a_wake_and_executes_no_guest_instruction() {
    let rom = [7, 0, 0xA0, 0xE3];
    let mut machine = Machine::new(Config { rom: &rom, ..Config::default() });
    machine.bus.enter_halt();
    let address = machine.next_instruction_address();
    let deadline = machine.now() + 100;
    let result = machine.wait_until(deadline).unwrap();
    assert!(!result.woke);
    assert_eq!(result.stopped_at, deadline);
    assert_eq!(machine.next_instruction_address(), address);
    assert_eq!(machine.read_register(0), 0);
    assert_eq!(machine.bus.power_state(), PowerState::Halted);
}

/// Zig: `Stop retains future master-cycle input for the caller`.
#[test]
fn stop_retains_future_master_cycle_input_for_the_caller() {
    let rom = [0, 0, 3, 0xEF];
    let mut machine = Machine::new(Config { rom: &rom, ..Config::default() });
    executed_step(&mut machine);
    let stopped_at = machine.now();
    let input = [TimedInput {
        at: stopped_at + 50,
        change: TimedInputChange::Keys(KeyState { a: true, ..KeyState::default() }),
    }];
    let result = machine.run_until_with_input(stopped_at + 100, &input).unwrap();
    assert_eq!(result.reason, StopReason::PowerStopped);
    assert_eq!(result.inputs_consumed, 0);
    assert_eq!(machine.now(), stopped_at);
    assert!(!machine.bus.keys.a);
}

/// Zig: `HLE SoundBias ramps the DAC bias while preserving resolution bits`.
#[test]
fn hle_sound_bias_ramps_the_dac_bias_while_preserving_resolution_bits() {
    let rom = [0, 0, 0x19, 0xEF];
    let mut raise = Machine::new(Config { rom: &rom, ..Config::default() });
    raise.bus.apu.sound_bias = 0xC000;
    raise.cpu.registers[0] = 1;
    raise.cpu.registers[1] = 0xA5A5_A5A5;
    executed_step(&mut raise);
    assert_eq!(raise.bus.apu.sound_bias, 0xC200);
    assert_eq!(raise.read_register(0), 1);
    assert_eq!(raise.read_register(1), 0xA5A5_A5A5);
    let mut lower = Machine::new(Config { rom: &rom, ..Config::default() });
    lower.bus.apu.sound_bias = 0x8200;
    lower.cpu.registers[0] = 0;
    executed_step(&mut lower);
    assert_eq!(lower.bus.apu.sound_bias, 0x8000);
}

/// Zig: `HLE SoftReset clears CPU state and the BIOS work area before returning to ROM`.
#[test]
fn hle_soft_reset_clears_cpu_state_and_the_bios_work_area_before_returning_to_rom() {
    let mut rom = [0; 8];
    write_words(&mut rom, 0, &[0xEF00_0000, 0xE1A0_0000]);
    let mut machine = Machine::new(Config { rom: &rom, ..Config::default() });
    for index in 0..13 {
        machine.cpu.registers[index] = 0x1111_0000 + index as u32;
    }
    machine.cpu.irq_r13 = 0xDEAD_0001;
    machine.cpu.irq_r14 = 0xDEAD_0002;
    machine.cpu.spsr_irq = 0xDEAD_0003;
    machine.cpu.supervisor_r13 = 0xBEEF_0001;
    machine.cpu.supervisor_r14 = 0xBEEF_0002;
    machine.cpu.spsr_supervisor = 0xBEEF_0003;
    machine.bus.iwram[0x7E00..].fill(0xA5);
    machine.bus.iwram[0x7FFA] = 0;
    executed_step(&mut machine);
    for index in 0..13 {
        assert_eq!(machine.read_register(index), 0);
    }
    assert_eq!(machine.read_register(13), 0x0300_7F00);
    assert_eq!(machine.read_register(14), 0x0800_0000);
    assert_eq!(
        (machine.cpu.irq_r13, machine.cpu.irq_r14, machine.cpu.spsr_irq),
        (0x0300_7FA0, 0, 0)
    );
    assert_eq!(
        (machine.cpu.supervisor_r13, machine.cpu.supervisor_r14, machine.cpu.spsr_supervisor),
        (0x0300_7FE0, 0, 0)
    );
    assert_eq!(machine.read_cpsr() & 0x3F, 0x1F);
    assert_eq!(machine.next_instruction_address(), 0x0800_0000);
    assert!(machine.bus.iwram[0x7E00..].iter().all(|&byte| byte == 0));
}

/// Zig: `user-mode movs pc, lr branches with a flag update and no SPSR restore`.
#[test]
fn user_mode_movs_pc_lr_branches_with_a_flag_update_and_no_spsr_restore() {
    let mut rom = [0; 8];
    write_words(&mut rom, 0, &[0xE1B0_F00E, 0xE1A0_0000]);
    let mut machine = Machine::new(Config { rom: &rom, ..Config::default() });
    machine.cpu.registers[14] = 0;
    machine.cpu.cpsr = 0x10;
    executed_step(&mut machine);
    assert_eq!(machine.next_instruction_address(), 0);
    assert_eq!(machine.read_cpsr() & 0x3F, 0x10);
    assert_ne!(machine.read_cpsr() & (1 << 30), 0);
}

/// Zig: `HLE SoftReset re-enters EWRAM when the captured BIOS flag is nonzero`.
#[test]
fn hle_soft_reset_re_enters_ewram_when_the_captured_bios_flag_is_nonzero() {
    let rom = [0, 0xDF];
    let mut machine = Machine::new(Config { rom: &rom, ..Config::default() });
    machine.cpu.cpsr |= 1 << 5;
    machine.cpu.registers[15] = 0x0800_0004;
    write_words(&mut machine.bus.ewram[..], 0, &[0xE3A0_4007]);
    machine.bus.iwram[0x7E00..].fill(0xA5);
    machine.bus.iwram[0x7FFA] = 1;
    executed_step(&mut machine);
    let first = executed_step(&mut machine);
    assert_eq!(first.instruction_address, 0x0200_0000);
    assert_eq!(machine.read_register(4), 7);
    assert_eq!(machine.read_register(14), 0x0200_0000);
    assert_eq!(machine.read_cpsr() & (1 << 5), 0);
    assert!(machine.bus.iwram[0x7E00..].iter().all(|&byte| byte == 0));
}

/// Zig: `runUntil resumes Halt at an enabled device event before its deadline`.
#[test]
fn run_until_resumes_halt_at_an_enabled_device_event_before_its_deadline() {
    let mut rom = [0; 8];
    write_words(&mut rom, 0, &[0xEF02_0000, 0xE3A0_0007]);
    let mut machine = Machine::new(Config { rom: &rom, ..Config::default() });
    executed_step(&mut machine);
    machine.bus.interrupt_enable_mask = 1 << 3;
    machine.bus.update_irq_line();
    machine.bus.timers[0].write_reload(0xFFFC);
    machine.bus.timers[0].write_control((1 << 7) | (1 << 6), machine.bus.prescaler_counter());
    let target = machine.now() + 8;
    let result = machine.run_until(target).unwrap();
    assert_eq!(result.reason, StopReason::InstructionBoundaryAfterDeadline);
    assert_eq!(machine.now(), target + 2);
    assert_eq!(machine.read_register(0), 7);
    assert_eq!(machine.bus.interrupt_request_flags, 1 << 3);
    assert_eq!(machine.bus.power_state(), PowerState::Running);
}

/// Zig: `Halt wake enters an enabled IRQ before executing the interrupted instruction`.
#[test]
fn halt_wake_enters_an_enabled_irq_before_executing_the_interrupted_instruction() {
    let mut rom = [0; 12];
    write_words(&mut rom, 0, &[0xEF02_0000, 0xE3A0_0007, 0xE3A0_3009]);
    let mut machine = Machine::new(Config { rom: &rom, ..Config::default() });
    executed_step(&mut machine);
    write_words(&mut machine.bus.iwram[..], 0x7FFC, &[0x0800_0008]);
    machine.bus.interrupt_enable_mask = 1 << 3;
    machine.bus.interrupt_master_enabled = true;
    machine.bus.update_irq_line();
    machine.bus.timers[0].write_reload(0xFFFC);
    machine.bus.timers[0].write_control((1 << 7) | (1 << 6), machine.bus.prescaler_counter());
    machine.run_until(machine.now() + 4).unwrap();
    let handler = executed_step(&mut machine);
    assert_eq!(handler.instruction_address, 0x0800_0008);
    assert_eq!(machine.read_register(0), 0);
    assert_eq!(machine.read_register(3), 9);
    assert_eq!(machine.read_cpsr() & 0x1F, 0x12);
    assert_ne!(machine.read_cpsr() & (1 << 7), 0);
    assert_eq!(machine.bus.power_state(), PowerState::Running);
}

/// Zig: `HLE Stop freezes system clocks and runUntil reports the stopped state`.
#[test]
fn hle_stop_freezes_system_clocks_and_run_until_reports_the_stopped_state() {
    let rom = [0, 0, 3, 0xEF];
    let mut machine = Machine::new(Config { rom: &rom, ..Config::default() });
    executed_step(&mut machine);
    let stopped_at = machine.now();
    let line = machine.bus.ppu_timing.line;
    let cycle = machine.bus.ppu_timing.cycle_in_line;
    machine.bus.interrupt_enable_mask = 1 << 3;
    machine.bus.update_irq_line();
    machine.bus.timers[0].write_reload(0xFFFE);
    machine.bus.timers[0].write_control((1 << 7) | (1 << 6), machine.bus.prescaler_counter());
    let result = machine.run_until(stopped_at + 100).unwrap();
    assert_eq!(result.reason, StopReason::PowerStopped);
    assert_eq!((result.stopped_at, machine.now()), (stopped_at, stopped_at));
    assert_eq!(machine.bus.timers[0].read_counter(), 0xFFFE);
    assert_eq!((machine.bus.ppu_timing.line, machine.bus.ppu_timing.cycle_in_line), (line, cycle));
    assert_eq!(machine.bus.power_state(), PowerState::Stopped);
}

/// Zig: `HLE RegisterRamReset resets sound and other devices without clearing RAM`.
#[test]
fn hle_register_ram_reset_resets_sound_and_other_devices_without_clearing_ram() {
    let rom = [0, 0, 1, 0xEF];
    let mut machine = Machine::new(Config { rom: &rom, ..Config::default() });
    machine.bus.ewram.fill(0xA5);
    machine.bus.iwram.fill(0x5A);
    machine.bus.apu.sound1_control_h = 0xFFC0;
    machine.bus.apu.sound_control_h = 0x770F;
    machine.bus.apu.master_enabled = true;
    machine.bus.apu.sound_bias = 0xC3FE;
    machine.bus.apu.wave.ram.fill(0xA5);
    machine.bus.timers[0] =
        evenfall_gba::timer::Timer { reload: 1, counter: 2, control: 0x80, prescaler_phase: 3 };
    machine.bus.dma_channels[0].source = 0x0200_1234;
    machine.bus.dma_channels[0].destination = 0x0300_1234;
    machine.bus.dma_channels[0].word_count = 7;
    machine.bus.dma_channels[0].control = 0x8400;
    machine.bus.interrupt_enable_mask = 0x3FFF;
    machine.bus.interrupt_request_flags = 0x3FFF;
    machine.bus.interrupt_master_enabled = true;
    machine.bus.update_irq_line();
    machine.bus.waitcnt = 0x7FFF;
    machine.bus.ppu_timing.dispstat_control = 0xFF38;
    machine.bus.bg_controls = [1, 2, 3, 4];
    machine.bus.bg_affine_parameters = [[1, 2, 3, 4], [5, 6, 7, 8]];
    machine.bus.win0h = 0x1234;
    machine.bus.bldcnt = 0x3FFF;
    machine.cpu.registers[0] = 0xC0;
    executed_step(&mut machine);
    assert!(machine.bus.ewram.iter().all(|&byte| byte == 0xA5));
    assert!(machine.bus.iwram.iter().all(|&byte| byte == 0x5A));
    assert_eq!(machine.bus.apu.sound1_control_h, 0);
    assert_eq!(machine.bus.apu.sound_control_h, 0);
    assert!(!machine.bus.apu.master_enabled);
    assert_eq!(machine.bus.apu.sound_bias, 0x0200);
    assert!(machine.bus.apu.wave.ram.iter().all(|&byte| byte == 0));
    assert_eq!(machine.bus.timers[0].reload, 0);
    assert_eq!(machine.bus.timers[0].read_control(), 0);
    assert_eq!(machine.bus.dma_channels[0].source, 0);
    assert_eq!(machine.bus.dma_channels[0].control, 0);
    assert_eq!(machine.bus.interrupt_enable_mask, 0);
    assert_eq!(machine.bus.interrupt_request_flags, 0);
    assert!(!machine.bus.interrupt_master_enabled);
    assert_eq!(machine.bus.waitcnt, 0);
    assert_eq!(machine.bus.ppu_timing.dispstat_control, 0);
    assert_eq!(machine.bus.bg_controls, [0; 4]);
    assert_eq!(machine.bus.bg_affine_parameters, [[0x100, 0, 0, 0x100]; 2]);
    assert_eq!((machine.bus.win0h, machine.bus.bldcnt, machine.bus.dispcnt), (0, 0, 0x0080));
}

/// Zig: `HLE RegisterRamReset exposes the SIO reset group and unconditional data-low bug`.
#[test]
fn hle_register_ram_reset_exposes_the_sio_reset_group_and_unconditional_data_low_bug() {
    let rom = [0, 0, 1, 0xEF];
    let mut without = Machine::new(Config { rom: &rom, ..Config::default() });
    without.bus.sio_data = [0x1111, 0x2222, 0x3333, 0x4444];
    without.bus.sio_control = 0x5555;
    without.bus.sio_completion_cycle = Some(9999);
    without.cpu.registers[0] = 0;
    executed_step(&mut without);
    assert_eq!(without.bus.sio_data, [0, 0x2222, 0x3333, 0x4444]);
    assert_eq!(without.bus.sio_control, 0x5555);
    assert_eq!(without.bus.sio_completion_cycle, Some(9999));

    let mut with = Machine::new(Config { rom: &rom, ..Config::default() });
    with.bus.sio_data = [0x1111, 0x2222, 0x3333, 0x4444];
    with.bus.sio_control = 0x5555;
    with.bus.sio_send = 0x6666;
    with.bus.sio_start_cycle = Some(7777);
    with.bus.sio_completion_cycle = Some(9999);
    with.bus.sio_receive_value = 0x1234_5678;
    with.bus.sio_multiplayer_receive = [1, 2, 3, 4];
    with.bus.sio_multiplayer_id = 3;
    with.bus.uart_tx_active = Some(0x12);
    with.bus.uart_tx_completion_cycle = Some(8888);
    with.bus.uart_tx_peer_scheduled = true;
    with.bus.uart_rx_arrival_cycle = Some(9998);
    with.bus.uart_rx_arrival_value = 0x34;
    with.bus.uart_error = true;
    with.bus.uart_cts_ready = true;
    with.bus.rcnt = 0x7777;
    with.bus.joy_control = 0x8888;
    with.bus.joy_receive = 0x9999_AAAA;
    with.bus.joy_transmit = 0xBBBB_CCCC;
    with.bus.joy_status = 0x003A;
    with.bus.joy_pending_command = Some(evenfall_gba::bus::JoyBusCommand::Status);
    with.bus.joy_completion_cycle = Some(9997);
    with.bus.joy_response =
        Some(evenfall_gba::bus::JoyBusResponse { len: 3, ..Default::default() });
    with.cpu.registers[0] = 1 << 5;
    executed_step(&mut with);
    assert_eq!(with.bus.sio_data, [0, 0x2222, 0x3333, 0x4444]);
    assert_eq!((with.bus.sio_control, with.bus.sio_send), (0, 0));
    assert_eq!((with.bus.sio_start_cycle, with.bus.sio_completion_cycle), (None, None));
    assert_eq!(with.bus.sio_receive_value, 0xFFFF_FFFF);
    assert_eq!(with.bus.sio_multiplayer_receive, [0xFFFF; 4]);
    assert_eq!(with.bus.sio_multiplayer_id, 0);
    assert_eq!((with.bus.uart_tx_active, with.bus.uart_tx_completion_cycle), (None, None));
    assert!(!with.bus.uart_tx_peer_scheduled);
    assert_eq!(with.bus.uart_rx_arrival_cycle, None);
    assert_eq!(with.bus.uart_rx_arrival_value, 0);
    assert!(!with.bus.uart_error);
    assert!(!with.bus.uart_cts_ready);
    assert_eq!(with.bus.rcnt, 0x8000);
    assert_eq!(
        (with.bus.joy_control, with.bus.joy_receive, with.bus.joy_transmit, with.bus.joy_status),
        (0, 0, 0, 0)
    );
    assert_eq!(with.bus.joy_pending_command, None);
    assert_eq!(with.bus.joy_completion_cycle, None);
    assert_eq!(with.bus.joy_response, None);
}

/// Zig: `HLE HardReset resets console state and reboots without erasing cartridge save`.
#[test]
fn hle_hard_reset_resets_console_state_and_reboots_without_erasing_cartridge_save() {
    let rom = [0, 0, 0x26, 0xEF, 7, 0, 0xA0, 0xE3];
    let mut save = [0xFF; 32 * 1024];
    save[0x1234] = 0x5A;
    let mut machine = Machine::new(Config {
        rom: &rom,
        save_type: evenfall_gba::cartridge::save::Type::Sram,
        save_data: Some(&save),
        ..Config::default()
    });
    machine.bus.ewram.fill(0xA5);
    machine.bus.iwram.fill(0x5A);
    machine.bus.palette.fill(0x33);
    machine.bus.vram.fill(0x44);
    machine.bus.oam.fill(0x55);
    machine.bus.dispcnt = 3;
    machine.bus.green_swap_enabled = true;
    machine.bus.ppu_timing.line = 123;
    machine.bus.ppu_timing.cycle_in_line = 777;
    machine.bus.apu.master_enabled = true;
    machine.bus.apu.sound_control_h = 0x770F;
    machine.bus.timers[0] =
        evenfall_gba::timer::Timer { reload: 1, counter: 2, control: 0xC0, ..Default::default() };
    machine.bus.dma_channels[0].control = 0x8400;
    machine.bus.interrupt_enable_mask = 0x3FFF;
    machine.bus.interrupt_request_flags = 0x3FFF;
    machine.bus.interrupt_master_enabled = true;
    machine.bus.update_irq_line();
    machine.bus.post_boot = 0;
    machine.bus.keys.a = true;
    machine.bus.keypad_control = 0xC001;
    for index in 0..15 {
        machine.cpu.registers[index] = 0x1111_0000 + index as u32;
    }
    machine.cpu.fiq_r8_14 = [0xF1F1_F1F1; 7];
    machine.cpu.abort_r13 = 0xABAB_ABAB;
    machine.cpu.undefined_r13 = 0xCDCD_CDCD;
    executed_step(&mut machine);
    assert_eq!(machine.next_instruction_address(), 0x0800_0000);
    for index in 0..13 {
        assert_eq!(machine.read_register(index as u8), 0);
    }
    assert_eq!((machine.read_register(13), machine.read_register(14)), (0x0300_7F00, 0x0800_0000));
    assert_eq!(machine.read_cpsr() & 0x3F, 0x1F);
    assert_eq!(machine.cpu.fiq_r8_14, [0; 7]);
    assert_eq!((machine.cpu.abort_r13, machine.cpu.undefined_r13), (0, 0));
    for memory in [
        &machine.bus.ewram[..],
        &machine.bus.iwram[..],
        &machine.bus.palette[..],
        &machine.bus.vram[..],
        &machine.bus.oam[..],
    ] {
        assert!(memory.iter().all(|&byte| byte == 0));
    }
    assert_eq!(machine.bus.dispcnt, 0x0080);
    assert!(!machine.bus.green_swap_enabled);
    assert_eq!((machine.bus.ppu_timing.line, machine.bus.ppu_timing.cycle_in_line), (0, 14));
    assert!(!machine.bus.apu.master_enabled);
    assert_eq!((machine.bus.apu.sound_control_h, machine.bus.apu.sound_bias), (0x000E, 0x0200));
    assert_eq!((machine.bus.timers[0].read_control(), machine.bus.dma_channels[0].control), (0, 0));
    assert_eq!((machine.bus.interrupt_enable_mask, machine.bus.interrupt_request_flags), (0, 0));
    assert!(!machine.bus.interrupt_master_enabled);
    assert_eq!((machine.bus.post_boot, machine.bus.keypad_control), (1, 0));
    assert!(machine.bus.keys.a);
    assert_eq!(machine.save_data()[0x1234], 0x5A);
}

/// Zig: `HALTCNT byte writes select Halt or Stop and Halt resumes at the following instruction`.
#[test]
fn haltcnt_byte_writes_select_halt_or_stop_and_halt_resumes_at_the_following_instruction() {
    for haltcnt in [0, 0x80] {
        let mut rom = [0; 16];
        write_words(&mut rom, 0, &[0xE3A0_0301, 0xE3A0_1000 | haltcnt, 0xE5C0_1301, 0xE3A0_2009]);
        let mut machine = Machine::new(Config { rom: &rom, ..Config::default() });
        for _ in 0..3 {
            executed_step(&mut machine);
        }
        if haltcnt != 0 {
            assert_eq!(machine.bus.power_state(), PowerState::Stopped);
            continue;
        }
        assert_eq!(machine.bus.power_state(), PowerState::Halted);
        machine.bus.interrupt_enable_mask = 1 << 3;
        machine.bus.update_irq_line();
        machine.bus.timers[0].write_reload(0xFFFE);
        machine.bus.timers[0].write_control((1 << 7) | (1 << 6), machine.bus.prescaler_counter());
        machine.run_until(machine.now() + 2).unwrap();
        let resumed = executed_step(&mut machine);
        assert_eq!(resumed.instruction_address, 0x0800_000C);
        assert_eq!(machine.read_register(2), 9);
        assert_eq!(machine.bus.power_state(), PowerState::Running);
    }
}

/// Zig: `mGBA timing suite single loads include the ARM7 internal result cycle`.
#[test]
fn mgba_timing_suite_single_loads_include_the_arm7_internal_result_cycle() {
    let code_address = 0x0300_0000;
    let data_address = 0x0300_0100;
    let mut machine = Machine::new(Config::default());
    let code = [0xE1A0_0000, 0xE580_1000, 0xE590_2000];
    for (index, instruction) in code.iter().enumerate() {
        machine.bus.write(
            Access {
                address: code_address + index as u32 * 4,
                width: AccessWidth::Word,
                kind: AccessKind::Nonsequential,
                source: AccessSource::CpuData,
            },
            *instruction,
        );
    }
    machine.cpu.next_instruction_address = code_address;
    machine.cpu.registers[15] = code_address + 8;
    machine.bus.fetch_kind = AccessKind::Nonsequential;
    machine.cpu.registers[0] = data_address;
    machine.cpu.registers[1] = 0xA5A5_5A5A;
    let nop = executed_step(&mut machine);
    let store = executed_step(&mut machine);
    let load = executed_step(&mut machine);
    assert_eq!(nop.stopped_at - nop.started_at, 1);
    assert_eq!(store.stopped_at - store.started_at, 2);
    assert_eq!(load.stopped_at - load.started_at, 3);
    assert_eq!(machine.read_register(2), 0xA5A5_5A5A);
}

/// Zig: `ARM7 defers a pending IRQ until after a taken branch refill`.
#[test]
fn arm7_defers_a_pending_irq_until_after_a_taken_branch_refill() {
    let words =
        [0xEA00_0000, 0xE1A0_0000, 0xE1A0_0000, 0xE1A0_0000, 0xE1A0_0000, 0xE1A0_0000, 0xE1A0_0000];
    let mut rom = [0; 28];
    write_words(&mut rom, 0, &words);
    let mut machine = Machine::new(Config { rom: &rom, ..Config::default() });
    machine.bus.write(
        Access {
            address: 0x0300_7FFC,
            width: AccessWidth::Word,
            kind: AccessKind::Nonsequential,
            source: AccessSource::CpuData,
        },
        0x0800_0010,
    );
    machine.bus.interrupt_enable_mask = 1;
    machine.bus.interrupt_request_flags = 1;
    machine.bus.interrupt_master_enabled = true;
    machine.bus.update_irq_line();
    executed_step(&mut machine);
    assert_eq!(machine.read_cpsr() & 0x1F, 0x1F);
    assert_eq!(machine.next_instruction_address(), 0x0800_0008);
    let target = executed_step(&mut machine);
    assert_eq!(target.instruction_address, 0x0800_0008);
    assert_eq!(machine.read_cpsr() & 0x1F, 0x12);
    assert_eq!(machine.next_instruction_address(), 0x0800_0010);
}

/// Zig: `timer IRQ enters ARM IRQ mode with the architectural return address`.
#[test]
fn timer_irq_enters_arm_irq_mode_with_the_architectural_return_address() {
    let rom = [
        0x01, 0x03, 0xA0, 0xE3, 0x01, 0x0C, 0x80, 0xE2, 0x01, 0x4C, 0x80, 0xE2, 0x08, 0x10, 0xA0,
        0xE3, 0xB0, 0x10, 0xC4, 0xE1, 0x01, 0x10, 0xA0, 0xE3, 0xB8, 0x10, 0xC4, 0xE1, 0x00, 0x10,
        0xE0, 0xE3, 0xB0, 0x10, 0xC0, 0xE1, 0xC0, 0x10, 0xA0, 0xE3, 0xB2, 0x10, 0xC0, 0xE1, 0x00,
        0x20, 0xA0, 0xE3,
    ];
    let mut bios = [0; 0x4000];
    write_words(&mut bios, 0x18, &[0xE3A0_5007]);
    let mut machine = Machine::new(Config {
        rom: &rom,
        bios: &bios,
        boot: BootMode::SkipBios,
        ..Config::default()
    });
    for _ in 0..12 {
        executed_step(&mut machine);
    }
    assert_eq!(machine.read_cpsr() & 0x1F, 0x12);
    assert_ne!(machine.read_cpsr() & (1 << 7), 0);
    assert_eq!(machine.read_cpsr() & (1 << 5), 0);
    assert_eq!(machine.read_register(14), 0x0800_0034);
    assert_eq!(machine.next_instruction_address(), 0x0000_0018);
    let vector = executed_step(&mut machine);
    assert_eq!(vector.instruction_address, 0x0000_0018);
    assert_eq!(machine.read_register(5), 7);
}
