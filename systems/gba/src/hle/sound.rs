//! MP2K SoundArea services. Every access stays on the timed bus.

use crate::bus::{Access, AccessKind, AccessSource, AccessWidth, Bus};
use crate::cpu::Cpu;

pub(super) const HLE_SOUND_JUMP_BASE: u32 = 0x0000_3E00;
pub(super) const HLE_SOUND_JUMP_COUNT: u32 = 36;
pub(super) const SOUNDAREA_ID: u32 = 0x6873_6D53;

pub(super) fn sound_read(bus: &mut Bus, address: u32, width: AccessWidth) -> u32 {
    bus.read(Access {
        address,
        width,
        kind: AccessKind::Nonsequential,
        source: AccessSource::CpuData,
    })
    .value
}

pub(super) fn sound_write(bus: &mut Bus, address: u32, width: AccessWidth, value: u32) {
    bus.write(
        Access { address, width, kind: AccessKind::Nonsequential, source: AccessSource::CpuData },
        value,
    );
}

pub(super) fn signed_sound_byte(value: u32) -> i32 {
    i32::from(value as u8 as i8)
}
pub(super) fn mixed_sound_byte(old: u32, contribution: i32) -> u8 {
    signed_sound_byte(old).wrapping_add(contribution) as u8
}

pub(super) fn hle_sound_jump_index_at(address: u32) -> Option<u8> {
    if !(HLE_SOUND_JUMP_BASE..HLE_SOUND_JUMP_BASE + HLE_SOUND_JUMP_COUNT * 4).contains(&address) {
        return None;
    }
    let offset = address - HLE_SOUND_JUMP_BASE;
    (offset & 3 == 0).then_some((offset / 4) as u8)
}

pub(super) fn is_hle_sound_jump_pointer(pointer: u32) -> bool {
    pointer & 1 != 0 && hle_sound_jump_index_at(pointer & !1).is_some()
}

impl Cpu {
    pub fn hle_sound_jump_index(&self) -> Option<u8> {
        if !self.is_thumb() {
            return None;
        }
        hle_sound_jump_index_at(self.next_instruction_address)
    }

    pub fn execute_hle_sound_get_jump_list(&mut self, bus: &mut Bus) {
        let mut destination = self.registers[0];
        for index in 0..HLE_SOUND_JUMP_COUNT {
            sound_write(bus, destination, AccessWidth::Word, (HLE_SOUND_JUMP_BASE + index * 4) | 1);
            destination = destination.wrapping_add(4);
        }
        self.registers[0] = destination;
        self.registers[1] = 0;
        self.registers[2] = HLE_SOUND_JUMP_BASE + HLE_SOUND_JUMP_COUNT * 4;
        self.registers[3] = (HLE_SOUND_JUMP_BASE + (HLE_SOUND_JUMP_COUNT - 1) * 4) | 1;
        bus.elapse(45);
    }

    pub fn execute_hle_sound_driver_vsync(&mut self, bus: &mut Bus) {
        let sound_area = sound_read(bus, 0x0300_7FF0, AccessWidth::Word);
        let identifier = sound_read(bus, sound_area, AccessWidth::Word);
        self.registers[0] = sound_area;
        self.registers[2] = SOUNDAREA_ID;
        self.registers[3] = identifier;
        if identifier != self.registers[2] {
            bus.elapse(45);
            return;
        }

        let old_countdown = sound_read(bus, sound_area.wrapping_add(4), AccessWidth::Byte) as u8;
        self.registers[1] = u32::from(old_countdown.wrapping_sub(1));
        sound_write(bus, sound_area.wrapping_add(4), AccessWidth::Byte, self.registers[1]);
        if old_countdown > 1 {
            bus.elapse(45);
            return;
        }

        self.registers[1] = sound_read(bus, sound_area.wrapping_add(0x0B), AccessWidth::Byte);
        sound_write(bus, sound_area.wrapping_add(4), AccessWidth::Byte, self.registers[1]);
        self.registers[0] = 0;
        self.registers[1] = 0xB600;
        self.registers[2] = 0x0400_00C6;
        self.registers[3] = 0x0400_00D2;
        sound_write(bus, self.registers[2], AccessWidth::Half, self.registers[0]);
        sound_write(bus, self.registers[3], AccessWidth::Half, self.registers[0]);
        sound_write(bus, self.registers[2], AccessWidth::Half, self.registers[1]);
        sound_write(bus, self.registers[3], AccessWidth::Half, self.registers[1]);
        bus.elapse(45);
    }

    pub fn execute_hle_sound_driver_mode(&mut self, bus: &mut Bus) {
        let input = self.registers[0];
        let sound_area = sound_read(bus, 0x0300_7FF0, AccessWidth::Word);
        let identifier = sound_read(bus, sound_area, AccessWidth::Word);
        self.registers[1] = identifier;
        if identifier != SOUNDAREA_ID {
            bus.elapse(45);
            return;
        }
        sound_write(bus, sound_area, AccessWidth::Word, identifier + 1);

        let reverb = input as u8;
        if reverb != 0 {
            sound_write(
                bus,
                sound_area.wrapping_add(5),
                AccessWidth::Byte,
                u32::from(reverb & 0x7F),
            );
        }
        let channel_count = (input >> 8) as u8;
        if channel_count & 0x0F != 0 {
            sound_write(
                bus,
                sound_area.wrapping_add(6),
                AccessWidth::Byte,
                u32::from(channel_count & 0x0F),
            );
            let mut channel_address = sound_area.wrapping_add(0x50);
            for _ in 0..12 {
                sound_write(bus, channel_address, AccessWidth::Byte, 0);
                channel_address = channel_address.wrapping_add(0x40);
            }
        }
        let master_volume = (input >> 12) as u8;
        if master_volume & 0x0F != 0 {
            sound_write(
                bus,
                sound_area.wrapping_add(7),
                AccessWidth::Byte,
                u32::from(master_volume & 0x0F),
            );
        }
        if input & 0x00B0_0000 != 0 {
            let bias_high = sound_read(bus, 0x0400_0089, AccessWidth::Byte) as u8;
            sound_write(
                bus,
                0x0400_0089,
                AccessWidth::Byte,
                u32::from((bias_high & 0x3F) | ((input & 0x0030_0000) >> 14) as u8),
            );
        }
        let playback_mode = input & 0x000F_0000;
        if playback_mode != 0 {
            self.execute_hle_sound_driver_vsync_off(bus);
            self.registers[0] = playback_mode;
            self.execute_hle_sound_driver_frequency(bus, sound_area, (playback_mode >> 16) as u8);
        }
        sound_write(bus, sound_area, AccessWidth::Word, SOUNDAREA_ID);
        bus.elapse(45);
    }

    pub fn execute_hle_sound_driver_init(&mut self, bus: &mut Bus) {
        let sound_area = self.registers[0];
        let saved_r3 = self.registers[3];
        sound_write(bus, 0x0400_00C6, AccessWidth::Half, 0);
        sound_write(bus, 0x0400_00D2, AccessWidth::Half, 0);
        sound_write(bus, 0x0400_0084, AccessWidth::Half, 0x008F);
        sound_write(bus, 0x0400_0082, AccessWidth::Half, 0xA90E);
        let bias_high = sound_read(bus, 0x0400_0089, AccessWidth::Byte) as u8;
        sound_write(bus, 0x0400_0089, AccessWidth::Byte, u32::from((bias_high & 0x3F) | 0x40));
        sound_write(bus, 0x0400_00BC, AccessWidth::Word, sound_area.wrapping_add(0x350));
        sound_write(bus, 0x0400_00C0, AccessWidth::Word, 0x0400_00A0);
        sound_write(bus, 0x0400_00C8, AccessWidth::Word, sound_area.wrapping_add(0x980));
        sound_write(bus, 0x0400_00CC, AccessWidth::Word, 0x0400_00A4);
        sound_write(bus, 0x0300_7FF0, AccessWidth::Word, sound_area);

        sound_write(bus, self.registers[13], AccessWidth::Word, 0);
        self.registers[0] = self.registers[13];
        self.registers[1] = sound_area;
        self.registers[2] = 0x0500_03EC;
        self.execute_hle_cpu_set(bus);

        sound_write(bus, sound_area.wrapping_add(6), AccessWidth::Byte, 8);
        sound_write(bus, sound_area.wrapping_add(7), AccessWidth::Byte, 15);
        sound_write(bus, sound_area.wrapping_add(0x38), AccessWidth::Word, 0x2425);
        for offset in [0x28, 0x2C, 0x30, 0x3C] {
            sound_write(bus, sound_area.wrapping_add(offset), AccessWidth::Word, 0x1709);
        }
        sound_write(bus, sound_area.wrapping_add(0x34), AccessWidth::Word, 0x3738);
        self.registers[0] = 4 << 16;
        self.execute_hle_sound_driver_frequency(bus, sound_area, 4);
        self.registers[0] = SOUNDAREA_ID;
        sound_write(bus, sound_area, AccessWidth::Word, self.registers[0]);
        self.registers[3] = saved_r3;
        bus.elapse(45);
    }

    pub fn execute_hle_sound_driver_frequency(&mut self, bus: &mut Bus, sound_area: u32, mode: u8) {
        let sample_counts: [u16; 15] = [
            0x0060, 0x0084, 0x00B0, 0x00E0, 0x0108, 0x0130, 0x0160, 0x01C0, 0x0210, 0x0260, 0x02A0,
            0x02C0, 0xFFFF, 0xFFFF, 0x001F,
        ];
        let sample_count = u32::from(sample_counts[usize::from(mode) - 1]);
        sound_write(bus, sound_area.wrapping_add(8), AccessWidth::Byte, u32::from(mode));
        sound_write(bus, sound_area.wrapping_add(0x10), AccessWidth::Word, sample_count);
        let dma_count = 0x630 / sample_count;
        sound_write(bus, sound_area.wrapping_add(0x0B), AccessWidth::Byte, dma_count);
        let frequency = (0x0009_1D1B_u32.wrapping_mul(sample_count).wrapping_add(0x1388)) / 0x2710;
        sound_write(bus, sound_area.wrapping_add(0x14), AccessWidth::Word, frequency);
        let phase_step = (((1_u32 << 24) / frequency) + 1) / 2;
        sound_write(bus, sound_area.wrapping_add(0x18), AccessWidth::Word, phase_step);
        sound_write(bus, 0x0400_0102, AccessWidth::Half, 0);
        sound_write(bus, 0x0400_0100, AccessWidth::Half, 0x1_0000 - (0x0004_4940 / sample_count));
        self.execute_hle_sound_driver_vsync_on(bus);

        loop {
            self.registers[1] = sound_read(bus, 0x0400_0006, AccessWidth::Byte);
            if self.registers[1] != 159 {
                break;
            }
            bus.elapse(2);
        }
        loop {
            self.registers[1] = sound_read(bus, 0x0400_0006, AccessWidth::Byte);
            if self.registers[1] == 159 {
                break;
            }
            bus.elapse(2);
        }
        self.registers[0] = 0x80;
        sound_write(bus, 0x0400_0102, AccessWidth::Half, self.registers[0]);
    }

    pub fn execute_hle_sound_driver_vsync_off(&mut self, bus: &mut Bus) {
        let sound_area = sound_read(bus, 0x0300_7FF0, AccessWidth::Word);
        let identifier = sound_read(bus, sound_area, AccessWidth::Word);
        self.registers[0] = identifier;
        if identifier != SOUNDAREA_ID && identifier != SOUNDAREA_ID + 1 {
            bus.elapse(45);
            return;
        }
        sound_write(bus, sound_area, AccessWidth::Word, identifier + 1);
        sound_write(bus, 0x0400_00C6, AccessWidth::Half, 0);
        sound_write(bus, 0x0400_00D2, AccessWidth::Half, 0);
        sound_write(bus, sound_area.wrapping_add(4), AccessWidth::Byte, 0);
        let saved_r3 = self.registers[3];
        sound_write(bus, self.registers[13], AccessWidth::Word, 0);
        self.registers[0] = self.registers[13];
        self.registers[1] = sound_area.wrapping_add(0x350);
        self.registers[2] = 0x0500_0318;
        self.execute_hle_cpu_set(bus);
        self.registers[3] = saved_r3;
        let busy_identifier = sound_read(bus, sound_area, AccessWidth::Word);
        self.registers[0] = busy_identifier.wrapping_sub(1);
        sound_write(bus, sound_area, AccessWidth::Word, self.registers[0]);
        bus.elapse(45);
    }

    pub fn execute_hle_sound_driver_vsync_on(&mut self, bus: &mut Bus) {
        self.registers[0] = 0x0400_00C0;
        self.registers[1] = 0xB600;
        sound_write(bus, self.registers[0].wrapping_add(6), AccessWidth::Half, self.registers[1]);
        sound_write(
            bus,
            self.registers[0].wrapping_add(0x12),
            AccessWidth::Half,
            self.registers[1],
        );
        bus.elapse(45);
    }
}
