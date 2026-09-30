//! HLE exception and sound-driver callback return stubs.

use crate::bus::{Access, AccessKind, AccessSource, AccessWidth, Bus};
use crate::cpu::{
    Cpu, FaultReason, FaultStep, HLE_CALLBACK_RETURN_STUB, HLE_IRQ_RETURN_STUB, HleBranchResult,
    HleSoundChannelClearContinuation, InstructionSet, StepOutcome, StepResult, hle_bios_fetch,
};

use super::sound::{SOUNDAREA_ID, sound_read, sound_write};

/// The cycles the retail `IntrWait` spends between the IRQ return that
/// satisfies it and its return from the SWI, beyond the accesses HLE makes:
/// checking and clearing its flag, the SWI epilogue and the refills. No
/// document lists that BIOS code; the mGBA suite's timer count-up table,
/// recorded on hardware, fixes the total. Its prescaled rows start a timer a
/// fixed number of cycles after an `IntrWait` returns, and every one of
/// them matches at 42 and at no neighboring value.
const HLE_INTR_WAIT_RETURN_CYCLES: u64 = 42;

impl Cpu {
    /// Returns from an HLE IRQ with the bus cycles of the retail
    /// trampoline's return (GBATEK, BIOS Interrupt handling):
    /// `ldmfd r13!,{r0-r3,r12,r14}` at 138h loads the six registers the
    /// entry stored below the IRQ stack pointer, then `subs r15,r14,4h` at
    /// 13Ch fetches its opcode and refills at the return address. The
    /// registers come from the HLE frame.
    pub fn complete_hle_irq(&mut self, bus: &mut Bus) -> StepResult {
        if self.pipeline.is_some() {
            bus.read(Access {
                address: HLE_IRQ_RETURN_STUB + 8,
                width: AccessWidth::Word,
                kind: AccessKind::Sequential,
                source: AccessSource::CpuCode,
            });
        } else {
            bus.elapse(1);
        }
        let base = self.registers[13].wrapping_sub(24);
        for index in 0..6 {
            bus.read(Access {
                address: base.wrapping_add(4 * index),
                width: AccessWidth::Word,
                kind: if index == 0 { AccessKind::Nonsequential } else { AccessKind::Sequential },
                source: AccessSource::CpuData,
            });
        }
        bus.elapse(1);
        hle_bios_fetch(bus, HLE_IRQ_RETURN_STUB + 12, AccessKind::Nonsequential);
        debug_assert_ne!(self.hle_irq_depth, 0);
        self.hle_irq_depth -= 1;
        let frame = self.hle_irq_stack[usize::from(self.hle_irq_depth)];
        self.registers[..4].copy_from_slice(&frame.saved_r0_r3);
        self.registers[12] = frame.saved_r12;
        self.next_instruction_address = frame.return_address.wrapping_sub(4);
        self.spsr_irq = frame.saved_spsr_irq;
        self.restore_cpsr_from_spsr();
        self.irq_r14 = frame.return_address;
        self.registers[15] =
            self.next_instruction_address.wrapping_add(if self.is_thumb() { 4 } else { 8 });
        self.refill_pipeline(bus);
        let wait_ended = self.hle_irq_depth == 0 && self.complete_hle_interrupt_wait(bus);
        if wait_ended {
            bus.elapse(HLE_INTR_WAIT_RETURN_CYCLES);
        }
        if self.hle_irq_depth == 0 {
            self.end_hle_halt(bus);
        }
        // The retail trampoline's return leaves its IRQ latch; an IntrWait
        // it ends then returns from its SWI, which leaves the SWI latch
        // (GBATEK, Reading from BIOS Memory).
        bus.set_bios_latch(if wait_ended { 0xE3A0_2004 } else { 0xE55E_C002 });
        StepResult {
            instruction_address: HLE_IRQ_RETURN_STUB,
            instruction: 0xE25E_F004,
            instruction_set: InstructionSet::Arm,
        }
    }

    pub fn execute_hle_sound_channel_clear(&mut self, bus: &mut Bus) -> HleBranchResult {
        let sound_area = sound_read(bus, 0x0300_7FF0, AccessWidth::Word);
        let identifier = sound_read(bus, sound_area, AccessWidth::Word);
        self.registers[0] = identifier;
        if identifier != SOUNDAREA_ID {
            bus.elapse(45);
            return HleBranchResult::default();
        }
        sound_write(bus, sound_area, AccessWidth::Word, identifier + 1);
        self.registers[0] = sound_area.wrapping_add(0x50);
        self.registers[1] = 12;
        self.registers[2] = 0;
        while self.registers[1] != 0 {
            sound_write(bus, self.registers[0], AccessWidth::Byte, 0);
            self.registers[0] = self.registers[0].wrapping_add(0x40);
            self.registers[1] -= 1;
        }
        let extension = sound_read(bus, sound_area.wrapping_add(0x1C), AccessWidth::Word);
        if extension != 0 {
            let callback = sound_read(bus, sound_area.wrapping_add(0x2C), AccessWidth::Word);
            if callback < 0x4000 {
                return HleBranchResult {
                    fault: Some(FaultReason::SoundChannelClearInternalCallback {
                        target: callback,
                    }),
                    ..Default::default()
                };
            }
            self.hle_sound_channel_clear =
                Some(HleSoundChannelClearContinuation {
                    sound_area,
                    identifier,
                    extension_cursor: extension,
                    channel: 1,
                    return_address: self
                        .next_instruction_address
                        .wrapping_add(if self.is_thumb() { 2 } else { 4 }),
                    return_cpsr: self.cpsr,
                    return_lr: self.registers[14],
                    saved_r4_r7: self.registers[4..8].try_into().expect("four registers"),
                });
            self.registers[0] = 1;
            self.registers[1] = callback;
            self.registers[4] = 1;
            self.registers[5] = extension;
            self.registers[6] = identifier;
            self.registers[7] = sound_area;
            self.branch_to_hle_callback(callback);
            return HleBranchResult { branched: true, fault: None };
        }
        sound_write(bus, sound_area, AccessWidth::Word, identifier);
        bus.elapse(45);
        HleBranchResult::default()
    }

    pub fn branch_to_hle_callback(&mut self, target: u32) {
        self.registers[14] = HLE_CALLBACK_RETURN_STUB | 1;
        if target & 1 != 0 {
            self.cpsr |= 1 << 5;
            self.next_instruction_address = target & !1;
        } else {
            self.cpsr &= !(1 << 5);
            self.next_instruction_address = target & !3;
        }
    }

    pub fn continue_hle_sound_channel_clear(&mut self, bus: &mut Bus) -> StepOutcome {
        let returned_thumb = self.is_thumb();
        if self.pipeline.is_some() {
            bus.read(Access {
                address: HLE_CALLBACK_RETURN_STUB + if returned_thumb { 4 } else { 8 },
                width: if returned_thumb { AccessWidth::Half } else { AccessWidth::Word },
                kind: AccessKind::Sequential,
                source: AccessSource::CpuCode,
            });
        } else {
            bus.elapse(1);
        }

        let mut continuation =
            self.hle_sound_channel_clear.clone().expect("channel clear continuation");
        continuation.channel += 1;
        continuation.extension_cursor = continuation.extension_cursor.wrapping_add(0x40);
        if continuation.channel <= 4 {
            let callback =
                sound_read(bus, continuation.sound_area.wrapping_add(0x2C), AccessWidth::Word);
            if callback < 0x4000 {
                return StepOutcome::Faulted(FaultStep {
                    instruction_address: HLE_CALLBACK_RETURN_STUB,
                    instruction: if returned_thumb { 0x4708 } else { 0xE12F_FF11 },
                    instruction_set: if returned_thumb {
                        InstructionSet::Thumb
                    } else {
                        InstructionSet::Arm
                    },
                    reason: FaultReason::SoundChannelClearInternalCallback { target: callback },
                });
            }
            self.hle_sound_channel_clear = Some(continuation.clone());
            self.registers[0] = u32::from(continuation.channel);
            self.registers[1] = callback;
            self.registers[4] = u32::from(continuation.channel);
            self.registers[5] = continuation.extension_cursor;
            self.registers[6] = continuation.identifier;
            self.registers[7] = continuation.sound_area;
            self.branch_to_hle_callback(callback);
            self.refill_pipeline(bus);
        } else {
            self.registers[2] = 0;
            sound_write(bus, continuation.extension_cursor, AccessWidth::Byte, 0);
            sound_write(bus, continuation.sound_area, AccessWidth::Word, continuation.identifier);
            self.registers[4..8].copy_from_slice(&continuation.saved_r4_r7);
            self.registers[14] = continuation.return_lr;
            self.cpsr = continuation.return_cpsr;
            self.next_instruction_address = continuation.return_address;
            self.hle_sound_channel_clear = None;
            self.refill_pipeline(bus);
            bus.set_bios_latch(0xE3A0_2004);
            bus.elapse(45);
        }

        StepOutcome::Executed(StepResult {
            instruction_address: HLE_CALLBACK_RETURN_STUB,
            instruction: if returned_thumb { 0x4708 } else { 0xE12F_FF11 },
            instruction_set: if returned_thumb {
                InstructionSet::Thumb
            } else {
                InstructionSet::Arm
            },
        })
    }
}
