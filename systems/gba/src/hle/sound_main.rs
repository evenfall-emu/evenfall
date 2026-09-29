//! SoundDriverMain callback phases and MP2K player-chain continuation.

use crate::bus::{Access, AccessKind, AccessSource, AccessWidth, Bus};
use crate::cpu::{
    Cpu, FaultReason, FaultStep, HLE_CALLBACK_RETURN_STUB, HleBranchResult,
    HleSoundDriverMainContinuation, HleSoundDriverMainPhase, InstructionSet, StepOutcome,
    StepResult,
};

use super::sound::{SOUNDAREA_ID, is_hle_sound_jump_pointer, sound_read, sound_write};

impl Cpu {
    pub fn prepare_hle_mp2k_player_chain(
        &mut self,
        bus: &mut Bus,
        continuation: &mut HleSoundDriverMainContinuation,
        first_player: u32,
    ) -> HleBranchResult {
        let mut player = first_player;
        while sound_read(bus, player.wrapping_add(0x34), AccessWidth::Word) == SOUNDAREA_ID {
            if usize::from(continuation.pending_player_count) >= continuation.pending_players.len()
            {
                return HleBranchResult {
                    fault: Some(FaultReason::Mp2kPlayerChainLimit { player }),
                    ..Default::default()
                };
            }
            sound_write(bus, player.wrapping_add(0x34), AccessWidth::Word, SOUNDAREA_ID + 1);
            continuation.pending_players[usize::from(continuation.pending_player_count)] = player;
            continuation.pending_player_count += 1;
            let callback = sound_read(bus, player.wrapping_add(0x38), AccessWidth::Word);
            if callback == 0 {
                break;
            }
            let argument = sound_read(bus, player.wrapping_add(0x3C), AccessWidth::Word);
            if callback == 0x0000_2149 {
                player = argument;
                continue;
            }
            if callback < 0x4000 {
                return HleBranchResult {
                    fault: Some(FaultReason::Mp2kInternalCallback { target: callback }),
                    ..Default::default()
                };
            }
            continuation.phase = HleSoundDriverMainPhase::AfterMp2kCallback;
            self.hle_sound_driver_main = Some(continuation.clone());
            self.registers[0] = argument;
            self.registers[1] = callback;
            self.branch_to_hle_callback(callback);
            return HleBranchResult { branched: true, fault: None };
        }
        HleBranchResult::default()
    }

    pub fn finish_hle_mp2k_player_chain(
        &mut self,
        bus: &mut Bus,
        continuation: &mut HleSoundDriverMainContinuation,
    ) -> Option<FaultReason> {
        while continuation.pending_player_count != 0 {
            continuation.pending_player_count -= 1;
            let player =
                continuation.pending_players[usize::from(continuation.pending_player_count)];
            if let Some(fault) =
                self.execute_hle_mp2k_player_body(bus, continuation.sound_area, player)
            {
                return Some(fault);
            }
            sound_write(bus, player.wrapping_add(0x34), AccessWidth::Word, SOUNDAREA_ID);
        }
        None
    }

    pub fn execute_hle_sound_driver_main(&mut self, bus: &mut Bus) -> HleBranchResult {
        let sound_area = sound_read(bus, 0x0300_7FF0, AccessWidth::Word);
        let identifier = sound_read(bus, sound_area, AccessWidth::Word);
        self.registers[0] = sound_area;
        self.registers[2] = SOUNDAREA_ID;
        self.registers[3] = identifier;
        if identifier != SOUNDAREA_ID {
            bus.elapse(45);
            return HleBranchResult::default();
        }
        sound_write(bus, sound_area, AccessWidth::Word, identifier + 1);
        let mut scanline_limit = sound_read(bus, sound_area.wrapping_add(0x0C), AccessWidth::Byte);
        if scanline_limit != 0 {
            let mut vcount = sound_read(bus, 0x0400_0006, AccessWidth::Byte);
            if vcount < 160 {
                vcount += 228;
            }
            scanline_limit = scanline_limit.wrapping_add(vcount);
        }
        let continuation = HleSoundDriverMainContinuation {
            sound_area,
            scanline_limit,
            phase: HleSoundDriverMainPhase::AfterPlayerCallback,
            return_address: self.next_instruction_address.wrapping_add(if self.is_thumb() {
                2
            } else {
                4
            }),
            return_cpsr: self.cpsr,
            return_lr: self.registers[14],
            saved_r4_r11: self.registers[4..12].try_into().expect("eight registers"),
            ..Default::default()
        };
        let callback = sound_read(bus, sound_area.wrapping_add(0x20), AccessWidth::Word);
        if callback != 0 {
            if callback == 0x0000_2149 {
                let mut mp2k = continuation;
                let player = sound_read(bus, sound_area.wrapping_add(0x24), AccessWidth::Word);
                let preparation = self.prepare_hle_mp2k_player_chain(bus, &mut mp2k, player);
                if preparation.fault.is_some() || preparation.branched {
                    return preparation;
                }
                if let Some(fault) = self.finish_hle_mp2k_player_chain(bus, &mut mp2k) {
                    return HleBranchResult { fault: Some(fault), ..Default::default() };
                }
                return self.finish_hle_sound_driver_player_phase(bus, mp2k, false);
            }
            self.hle_sound_driver_main = Some(continuation);
            self.registers[0] = sound_read(bus, sound_area.wrapping_add(0x24), AccessWidth::Word);
            self.registers[1] = callback;
            if let Some(fault) = self.branch_to_hle_sound_driver_callback(callback) {
                return HleBranchResult { fault: Some(fault), ..Default::default() };
            }
            return HleBranchResult { branched: true, fault: None };
        }
        self.finish_hle_sound_driver_player_phase(bus, continuation, false)
    }

    pub fn branch_to_hle_sound_driver_callback(&mut self, callback: u32) -> Option<FaultReason> {
        if callback < 0x4000 && !is_hle_sound_jump_pointer(callback) {
            return Some(FaultReason::SoundDriverMainInternalCallback { target: callback });
        }
        self.branch_to_hle_callback(callback);
        None
    }

    pub fn start_hle_sound_driver_cgb_callback(
        &mut self,
        mut continuation: HleSoundDriverMainContinuation,
        callback: u32,
    ) -> HleBranchResult {
        continuation.phase = HleSoundDriverMainPhase::AfterCgbCallback;
        self.hle_sound_driver_main = Some(continuation.clone());
        self.registers[0] = continuation.sound_area;
        self.registers[1] = callback;
        if let Some(fault) = self.branch_to_hle_sound_driver_callback(callback) {
            return HleBranchResult { fault: Some(fault), ..Default::default() };
        }
        HleBranchResult { branched: true, fault: None }
    }

    pub fn continue_hle_sound_driver_main(&mut self, bus: &mut Bus) -> StepOutcome {
        let thumb = self.is_thumb();
        if self.pipeline.is_some() {
            bus.read(Access {
                address: HLE_CALLBACK_RETURN_STUB + if thumb { 4 } else { 8 },
                width: if thumb { AccessWidth::Half } else { AccessWidth::Word },
                kind: AccessKind::Sequential,
                source: AccessSource::CpuCode,
            });
        } else {
            bus.elapse(1);
        }
        let continuation = self.hle_sound_driver_main.clone().expect("sound driver continuation");
        let instruction = if thumb { 0x4708 } else { 0xE12F_FF11 };
        let instruction_set = if thumb { InstructionSet::Thumb } else { InstructionSet::Arm };
        let mut fault = None;
        match continuation.phase {
            HleSoundDriverMainPhase::AfterPlayerCallback => {
                self.hle_sound_driver_main = None;
                let result = self.finish_hle_sound_driver_player_phase(bus, continuation, true);
                fault = result.fault;
                if fault.is_none() && result.branched {
                    self.refill_pipeline(bus);
                }
            }
            HleSoundDriverMainPhase::AfterMp2kCallback => {
                let mut resumed = continuation;
                self.hle_sound_driver_main = None;
                fault = self.finish_hle_mp2k_player_chain(bus, &mut resumed);
                if fault.is_none() {
                    let result = self.finish_hle_sound_driver_player_phase(bus, resumed, true);
                    fault = result.fault;
                    if fault.is_none() && result.branched {
                        self.refill_pipeline(bus);
                    }
                }
            }
            HleSoundDriverMainPhase::AfterCgbCallback => {
                self.finish_hle_sound_driver_main(bus, continuation)
            }
        }
        if let Some(reason) = fault {
            return StepOutcome::Faulted(FaultStep {
                instruction_address: HLE_CALLBACK_RETURN_STUB,
                instruction,
                instruction_set,
                reason,
            });
        }
        StepOutcome::Executed(StepResult {
            instruction_address: HLE_CALLBACK_RETURN_STUB,
            instruction,
            instruction_set,
        })
    }

    pub fn finish_hle_sound_driver_player_phase(
        &mut self,
        bus: &mut Bus,
        continuation: HleSoundDriverMainContinuation,
        returning_from_callback: bool,
    ) -> HleBranchResult {
        let callback =
            sound_read(bus, continuation.sound_area.wrapping_add(0x28), AccessWidth::Word);
        if callback != 0 && callback != 0x0000_1709 {
            return self.start_hle_sound_driver_cgb_callback(continuation, callback);
        }
        if returning_from_callback {
            self.finish_hle_sound_driver_main(bus, continuation);
        } else {
            self.mix_hle_sound_driver_main(
                bus,
                continuation.sound_area,
                continuation.scanline_limit,
            );
            sound_write(bus, continuation.sound_area, AccessWidth::Word, SOUNDAREA_ID);
            bus.elapse(45);
        }
        HleBranchResult::default()
    }

    pub fn finish_hle_sound_driver_main(
        &mut self,
        bus: &mut Bus,
        continuation: HleSoundDriverMainContinuation,
    ) {
        self.mix_hle_sound_driver_main(bus, continuation.sound_area, continuation.scanline_limit);
        sound_write(bus, continuation.sound_area, AccessWidth::Word, SOUNDAREA_ID);
        self.registers[4..12].copy_from_slice(&continuation.saved_r4_r11);
        self.registers[14] = continuation.return_lr;
        self.cpsr = continuation.return_cpsr;
        self.next_instruction_address = continuation.return_address;
        self.hle_sound_driver_main = None;
        self.refill_pipeline(bus);
        bus.set_bios_latch(0xE3A0_2004);
        bus.elapse(45);
    }
}
