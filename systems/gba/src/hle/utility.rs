//! Non-bulk BIOS utilities: waits, sound bias and pitch conversion.

use crate::bus::{Access, AccessKind, AccessSource, AccessWidth, Bus};
use crate::cpu::Cpu;

fn access(address: u32, width: AccessWidth) -> Access {
    Access { address, width, kind: AccessKind::Nonsequential, source: AccessSource::CpuData }
}

impl Cpu {
    /// ZiGBA `executeHleInterruptWait`. The IRQ return path must call
    /// `complete_hle_interrupt_wait` only at depth zero.
    pub fn execute_hle_interrupt_wait(&mut self, bus: &mut Bus, vblank: bool) {
        if vblank {
            self.registers[0] = 1;
            self.registers[1] = 1;
        }
        let mask = self.registers[1] as u16;
        bus.interrupt_master_enabled = true;
        bus.update_irq_line();
        let mut flags = bus.read(access(0x0300_7FF8, AccessWidth::Half)).value as u16;
        if self.registers[0] != 0 {
            flags &= !mask;
        }
        if flags & mask != 0 {
            flags &= !mask;
            self.hle_interrupt_wait_mask = None;
        } else {
            self.hle_interrupt_wait_mask = Some(mask);
            bus.enter_halt();
        }
        bus.write(access(0x0300_7FF8, AccessWidth::Half), u32::from(flags));
        bus.elapse(12);
    }

    pub fn complete_hle_interrupt_wait(&mut self, bus: &mut Bus) {
        let Some(mask) = self.hle_interrupt_wait_mask else { return };
        let mut flags = bus.read(access(0x0300_7FF8, AccessWidth::Half)).value as u16;
        if flags & mask == 0 {
            bus.enter_halt();
            return;
        }
        flags &= !mask;
        bus.write(access(0x0300_7FF8, AccessWidth::Half), u32::from(flags));
        self.hle_interrupt_wait_mask = None;
    }

    /// ZiGBA `executeHleSoundBias`: every unit write is visible to the APU.
    pub fn execute_hle_sound_bias(&mut self, bus: &mut Bus) {
        let target = if self.registers[0] == 0 { 0_u16 } else { 0x0200 };
        let current = bus.read(access(0x0400_0088, AccessWidth::Half)).value as u16;
        let resolution = current & 0xC000;
        let mut bias = current & 0x03FF;
        while bias != target {
            if bias < target {
                bias += 1;
            } else {
                bias -= 1;
            }
            bus.write(access(0x0400_0088, AccessWidth::Half), u32::from(resolution | bias));
            bus.elapse(8);
        }
        bus.elapse(45);
    }

    pub fn execute_hle_midi_key2_freq(&mut self, bus: &mut Bus) {
        let frequency =
            bus.read(access(self.registers[0].wrapping_add(4), AccessWidth::Word)).value;
        let midi_key = self.registers[1] as u8;
        let fine_pitch = self.registers[2] as u8;
        let exponent = (180.0 - f32::from(midi_key) - f32::from(fine_pitch) / 256.0) / 12.0;
        let result = frequency as f32 / exponent.exp2();
        debug_assert!(result.abs() < 2_f32.powi(62));
        self.registers[0] = result as i64 as u32;
        bus.elapse(45);
    }
}
