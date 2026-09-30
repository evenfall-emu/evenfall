//! HLE SWI dispatch and guest-cycle-bounded bulk service continuation.

use crate::bus::{Access, AccessKind, AccessSource, AccessWidth, Bus};
use crate::cpu::{
    Cpu, ExecuteResult, FaultReason, FaultStep, HleBulkService, HleBulkWork, HleService,
    InstructionSet, StepOutcome, StepResult, hle_bios_fetch,
};

const HLE_BULK_STEP_CYCLES: u64 = 1 << 20;
const HLE_SWI_RETURN_LATCH: u32 = 0xE3A0_2004;

/// The retail SWI dispatcher's cycles besides its read of the service
/// number, which no document lists. The mGBA suite's timing table, recorded
/// on hardware, fixes them: its BIOS Division, Division 2, Sqrt (of 0) and
/// ArcTan rows call from IWRAM, EWRAM and Game Pak ROM under eight wait
/// settings, in ARM and Thumb, and every column matches with this value and
/// the retail call's refills and service-number read.
const HLE_SWI_DISPATCH_CYCLES: u64 = 39;

impl Cpu {
    /// A fault returns as a typed `FaultReason`.
    pub fn execute_hle_software_interrupt(
        &mut self,
        bus: &mut Bus,
        instruction: u32,
        service: u8,
    ) -> Result<ExecuteResult, FaultReason> {
        if let Some(fault) = self.preflight_hle_software_interrupt(service) {
            return Err(fault);
        }
        let bulk_stop_at = bus.now().saturating_add(HLE_BULK_STEP_CYCLES);
        let mut bulk: Option<HleBulkWork> = None;
        let mut branched = false;
        let mut fault = None;
        let retail_call = matches!(service, 0x06..=0x18);
        if retail_call {
            self.enter_hle_swi(bus);
        }
        match service {
            0x00 => {
                self.execute_hle_soft_reset(bus);
                branched = true;
            }
            0x01 => {
                bus.elapse(45);
                bus.register_ram_reset(self.registers[0] as u8);
            }
            0x02 => {
                bus.elapse(45);
                bus.enter_halt();
                self.hle_halt = true;
            }
            0x03 => {
                bus.elapse(45);
                bus.enter_stop();
            }
            0x04 => self.execute_hle_interrupt_wait(bus, false),
            0x05 => self.execute_hle_interrupt_wait(bus, true),
            0x06 => {
                let cycles = self.execute_hle_division(false);
                bus.elapse(cycles);
            }
            0x07 => {
                let cycles = self.execute_hle_division(true);
                bus.elapse(cycles);
            }
            0x08 => {
                self.registers[0] = self.registers[0].isqrt();
                bus.elapse(53);
            }
            0x09 => {
                let cycles = self.execute_hle_arc_tan();
                bus.elapse(cycles);
            }
            0x0A => {
                let cycles = self.execute_hle_arc_tan2();
                bus.elapse(cycles);
            }
            0x0B => bulk = self.begin_hle_cpu_set(bus),
            0x0C => bulk = self.begin_hle_cpu_fast_set(bus),
            0x0D => {
                self.registers[0] = 0xBAAE_187F;
                self.registers[1] = 1;
                self.registers[3] = 0x4000;
                bus.elapse(45);
            }
            0x0E => bulk = Some(self.begin_hle_bg_affine_set(bus)),
            0x0F => bulk = Some(self.begin_hle_obj_affine_set(bus)),
            0x10 => bulk = self.begin_hle_bit_unpack(bus),
            0x11 | 0x12 => bulk = Some(self.begin_hle_lz77(bus, service == 0x12)),
            0x13 => bulk = self.begin_hle_huffman(bus),
            0x14 | 0x15 => bulk = Some(self.begin_hle_run_length(bus, service == 0x15)),
            0x16..=0x18 => {
                bulk =
                    Some(self.begin_hle_differential_filter(bus, service == 0x18, service != 0x16))
            }
            0x19 => self.execute_hle_sound_bias(bus),
            0x1A => self.execute_hle_sound_driver_init(bus),
            0x1B => self.execute_hle_sound_driver_mode(bus),
            0x1C => {
                let result = self.execute_hle_sound_driver_main(bus);
                branched = result.branched;
                fault = result.fault;
            }
            0x1D => self.execute_hle_sound_driver_vsync(bus),
            0x1E => {
                let result = self.execute_hle_sound_channel_clear(bus);
                branched = result.branched;
                fault = result.fault;
            }
            0x1F => self.execute_hle_midi_key2_freq(bus),
            0x20 => self.execute_hle_music_player_open(bus),
            0x21 => fault = self.execute_hle_music_player_start(bus),
            0x22 => fault = self.execute_hle_music_player_stop(bus),
            0x23 => self.execute_hle_music_player_continue(bus),
            0x24 => self.execute_hle_music_player_fade_out(bus),
            0x25 => self.execute_hle_multi_boot(bus),
            0x26 => {
                self.execute_hle_hard_reset(bus);
                branched = true;
            }
            0x27 => {
                bus.elapse(45);
                if self.registers[2] & 0x80 == 0 {
                    bus.enter_halt();
                    self.hle_halt = true;
                } else {
                    bus.enter_stop();
                }
            }
            0x28 => self.execute_hle_sound_driver_vsync_off(bus),
            0x29 => self.execute_hle_sound_driver_vsync_on(bus),
            0x2A => self.execute_hle_sound_get_jump_list(bus),
            _ => fault = Some(FaultReason::UnsupportedHleSwi { service }),
        }
        if let Some(mut work) = bulk {
            match self.resume_hle_bulk_work(bus, &mut work, bulk_stop_at) {
                Ok(true) => {}
                Ok(false) => {
                    self.hle_service = Some(HleService::Bulk(HleBulkService {
                        instruction_address: self.next_instruction_address,
                        instruction,
                        instruction_set: if self.is_thumb() {
                            InstructionSet::Thumb
                        } else {
                            InstructionSet::Arm
                        },
                        work,
                    }));
                    return Ok(ExecuteResult::default());
                }
                Err(reason) => return Err(reason),
            }
        }
        if let Some(reason) = fault {
            return Err(reason);
        }
        bus.set_bios_latch(HLE_SWI_RETURN_LATCH);
        if retail_call {
            self.return_from_hle_swi(bus);
            branched = true;
        }
        Ok(ExecuteResult { branched, ..Default::default() })
    }

    /// The retail SWI's way into the service: the exception's refill at the
    /// SWI vector, from the BIOS (which leaves the Game Pak and discards its
    /// prefetch buffer), then the BIOS SWI dispatcher, which reads the
    /// service number from the caller's code, "the BYTE at [R14-2]" (GBATEK,
    /// ARM CPU Exceptions), and spends [`HLE_SWI_DISPATCH_CYCLES`] more.
    fn enter_hle_swi(&mut self, bus: &mut Bus) {
        hle_bios_fetch(bus, 0x08, AccessKind::Nonsequential);
        hle_bios_fetch(bus, 0x0C, AccessKind::Sequential);
        let comment = if self.is_thumb() {
            self.next_instruction_address
        } else {
            self.next_instruction_address.wrapping_add(2)
        };
        bus.read(Access {
            address: comment,
            width: AccessWidth::Byte,
            kind: AccessKind::Nonsequential,
            source: AccessSource::CpuData,
        });
        bus.elapse(HLE_SWI_DISPATCH_CYCLES);
    }

    /// The retail SWI's return, `movs pc, r14`: its opcode fetch in the
    /// BIOS, then the refill at the instruction after the SWI, which the
    /// caller makes as for any branch.
    fn return_from_hle_swi(&mut self, bus: &mut Bus) {
        hle_bios_fetch(bus, 0x00, AccessKind::Sequential);
        let width = if self.is_thumb() { 2 } else { 4 };
        self.next_instruction_address = self.next_instruction_address.wrapping_add(width);
    }

    pub fn continue_hle_bulk_service(
        &mut self,
        bus: &mut Bus,
        report_instruction: bool,
    ) -> Option<StepOutcome> {
        let HleService::Bulk(service) = self.hle_service.as_ref()? else {
            unreachable!("bulk continuation")
        };
        let call = StepResult {
            instruction_address: service.instruction_address,
            instruction: service.instruction,
            instruction_set: service.instruction_set,
        };
        let mut work = service.work.clone();
        let stop_at = bus.now().saturating_add(HLE_BULK_STEP_CYCLES);
        match self.resume_hle_bulk_work(bus, &mut work, stop_at) {
            Ok(false) => {
                if let Some(HleService::Bulk(service)) = self.hle_service.as_mut() {
                    service.work = work;
                }
                return report_instruction.then_some(StepOutcome::Executed(call));
            }
            Err(reason) => {
                if let Some(HleService::Bulk(service)) = self.hle_service.as_mut() {
                    service.work = work;
                }
                return Some(StepOutcome::Faulted(FaultStep {
                    instruction_address: call.instruction_address,
                    instruction: call.instruction,
                    instruction_set: call.instruction_set,
                    reason,
                }));
            }
            Ok(true) => {}
        }
        self.hle_service = None;
        bus.set_bios_latch(HLE_SWI_RETURN_LATCH);
        // The retail return, as for a service that ends in its first step;
        // the instruction after the SWI is already next.
        hle_bios_fetch(bus, 0x00, AccessKind::Sequential);
        self.refill_pipeline(bus);
        if bus.irq.irq_line && self.cpsr & (1 << 7) == 0 {
            bus.resume_power();
            if let Err(reason) = self.enter_irq(bus) {
                return Some(StepOutcome::Faulted(FaultStep {
                    instruction_address: call.instruction_address,
                    instruction: call.instruction,
                    instruction_set: call.instruction_set,
                    reason,
                }));
            }
            self.refill_pipeline(bus);
        }
        report_instruction.then_some(StepOutcome::Executed(call))
    }

    pub fn execute_hle_soft_reset(&mut self, bus: &mut Bus) {
        bus.elapse(45);
        let entry = bus.soft_reset_entry_address();
        self.reset_hle_cpu(entry);
        bus.resume_power();
    }

    pub fn execute_hle_hard_reset(&mut self, bus: &mut Bus) {
        bus.elapse(45);
        bus.hard_reset();
        self.reset_hle_cpu(0x0800_0000);
        self.fiq_r8_14.fill(0);
        self.spsr_fiq = 0;
        self.abort_r13 = 0;
        self.abort_r14 = 0;
        self.spsr_abort = 0;
        self.undefined_r13 = 0;
        self.undefined_r14 = 0;
        self.spsr_undefined = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Div and DivArm return the quotient, remainder and absolute quotient,
    /// including the BIOS edge cases, through a CPU step.
    #[test]
    fn hle_division_swi_executes_through_cpu_step() {
        for (service, numerator, denominator, quotient, remainder, absolute) in [
            (0x06, 100, 7, 14, 2, 14),
            (0x07, 7, 100, 14, 2, 14),
            (0x06, (-100_i32) as u32, 7, (-14_i32) as u32, (-2_i32) as u32, 14),
            (0x06, 0x8000_0000, u32::MAX, 0x8000_0000, 0, 0x8000_0000),
            (0x06, 0, 0, 1, 0, 1),
        ] {
            let mut bus = Bus::init(&[0, 0, service, 0xEF]);
            let mut cpu = Cpu::init(0x0800_0000, 0x1F, true);
            cpu.registers[0] = numerator;
            cpu.registers[1] = denominator;

            assert_eq!(
                cpu.step(&mut bus),
                StepOutcome::Executed(StepResult {
                    instruction_address: 0x0800_0000,
                    instruction: 0xEF00_0000 | (u32::from(service) << 16),
                    instruction_set: InstructionSet::Arm,
                })
            );
            assert_eq!(
                [cpu.registers[0], cpu.registers[1], cpu.registers[3]],
                [quotient, remainder, absolute]
            );
            assert_eq!(cpu.next_instruction_address, 0x0800_0004);
            assert_eq!(bus.bios_latch, HLE_SWI_RETURN_LATCH);
        }
    }

    /// An unsupported HLE SWI is a typed fault at the CPU boundary.
    #[test]
    fn unsupported_hle_swi_faults_before_return_latch() {
        let mut bus = Bus::init(&[0, 0, 0x2B, 0xEF]);
        let mut cpu = Cpu::init(0x0800_0000, 0x1F, true);
        let original_latch = bus.bios_latch;

        assert_eq!(
            cpu.step(&mut bus),
            StepOutcome::Faulted(FaultStep {
                instruction_address: 0x0800_0000,
                instruction: 0xEF2B_0000,
                instruction_set: InstructionSet::Arm,
                reason: FaultReason::UnsupportedHleSwi { service: 0x2B },
            })
        );
        assert_eq!(cpu.next_instruction_address, 0x0800_0000);
        assert_eq!(bus.bios_latch, original_latch);
    }
}
