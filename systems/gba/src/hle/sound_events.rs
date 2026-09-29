//! MP2K command helpers and BIOS SoundGetJumpList entries.

use crate::bus::{AccessWidth, Bus};
use crate::cpu::{Cpu, FaultReason, FaultStep, InstructionSet, StepOutcome, StepResult};

use super::sound::{signed_sound_byte, sound_read, sound_write};
use super::sound_player::{clear_hle_music_player, stop_hle_music_player_track};

pub(super) fn consume_hle_track_byte(bus: &mut Bus, track: u32) -> u8 {
    let pointer = sound_read(bus, track.wrapping_add(0x40), AccessWidth::Word);
    let value = sound_read(bus, pointer, AccessWidth::Byte) as u8;
    sound_write(bus, track.wrapping_add(0x40), AccessWidth::Word, pointer.wrapping_add(1));
    value
}

fn hle_set_track_byte_and_flags(bus: &mut Bus, track: u32, offset: u32, flags: u8, bias: u8) {
    let value = consume_hle_track_byte(bus, track).wrapping_sub(bias);
    sound_write(bus, track.wrapping_add(offset), AccessWidth::Byte, u32::from(value));
    let status = sound_read(bus, track, AccessWidth::Byte);
    sound_write(bus, track, AccessWidth::Byte, status | u32::from(flags));
}

pub(super) fn clear_hle_track_modulation(bus: &mut Bus, track: u32) {
    sound_write(bus, track.wrapping_add(0x1A), AccessWidth::Byte, 0);
    sound_write(bus, track.wrapping_add(0x16), AccessWidth::Byte, 0);
    let flags =
        if sound_read(bus, track.wrapping_add(0x18), AccessWidth::Byte) == 0 { 0x0C } else { 3 };
    let status = sound_read(bus, track, AccessWidth::Byte);
    sound_write(bus, track, AccessWidth::Byte, status | flags);
}

fn hle_music_event_fine(bus: &mut Bus, track: u32) {
    let mut channel = sound_read(bus, track.wrapping_add(0x20), AccessWidth::Word);
    while channel != 0 {
        let next = sound_read(bus, channel.wrapping_add(0x34), AccessWidth::Word);
        let status = sound_read(bus, channel, AccessWidth::Byte);
        if status & 0xC7 != 0 {
            sound_write(bus, channel, AccessWidth::Byte, status | 0x40);
        }
        clear_hle_music_channel_chain(bus, channel);
        channel = next;
    }
    sound_write(bus, track, AccessWidth::Byte, 0);
}

fn hle_music_event_goto(bus: &mut Bus, track: u32) {
    let pointer = sound_read(bus, track.wrapping_add(0x40), AccessWidth::Word);
    let target = sound_read(bus, pointer, AccessWidth::Word);
    sound_write(bus, track.wrapping_add(0x40), AccessWidth::Word, target);
}

fn hle_music_event_pattern(bus: &mut Bus, track: u32) {
    let level = sound_read(bus, track.wrapping_add(2), AccessWidth::Byte);
    if level >= 3 {
        hle_music_event_fine(bus, track);
        return;
    }
    let pointer = sound_read(bus, track.wrapping_add(0x40), AccessWidth::Word);
    sound_write(
        bus,
        track.wrapping_add(0x44).wrapping_add(level * 4),
        AccessWidth::Word,
        pointer.wrapping_add(4),
    );
    sound_write(bus, track.wrapping_add(2), AccessWidth::Byte, level + 1);
    hle_music_event_goto(bus, track);
}

fn hle_music_event_pattern_end(bus: &mut Bus, track: u32) {
    let level = sound_read(bus, track.wrapping_add(2), AccessWidth::Byte);
    if level == 0 {
        return;
    }
    let next_level = level - 1;
    sound_write(bus, track.wrapping_add(2), AccessWidth::Byte, next_level);
    let address =
        sound_read(bus, track.wrapping_add(0x44).wrapping_add(next_level * 4), AccessWidth::Word);
    sound_write(bus, track.wrapping_add(0x40), AccessWidth::Word, address);
}

fn hle_music_event_repeat(bus: &mut Bus, track: u32) {
    let pointer = sound_read(bus, track.wrapping_add(0x40), AccessWidth::Word);
    let limit = sound_read(bus, pointer, AccessWidth::Byte);
    if limit == 0 {
        sound_write(bus, track.wrapping_add(0x40), AccessWidth::Word, pointer.wrapping_add(1));
        hle_music_event_goto(bus, track);
        return;
    }
    let repeat = (sound_read(bus, track.wrapping_add(3), AccessWidth::Byte) + 1) & 0xFF;
    sound_write(bus, track.wrapping_add(3), AccessWidth::Byte, repeat);
    if repeat < limit {
        sound_write(bus, track.wrapping_add(0x40), AccessWidth::Word, pointer.wrapping_add(1));
        hle_music_event_goto(bus, track);
    } else {
        sound_write(bus, track.wrapping_add(3), AccessWidth::Byte, 0);
        sound_write(bus, track.wrapping_add(0x40), AccessWidth::Word, pointer.wrapping_add(5));
    }
}

fn hle_music_event_voice(bus: &mut Bus, player: u32, track: u32) {
    let voice = consume_hle_track_byte(bus, track);
    let group = sound_read(bus, player.wrapping_add(0x30), AccessWidth::Word);
    let source = group.wrapping_add(u32::from(voice) * 12);
    for offset in [0, 4, 8] {
        let word = sound_read(bus, source.wrapping_add(offset), AccessWidth::Word);
        sound_write(bus, track.wrapping_add(0x24).wrapping_add(offset), AccessWidth::Word, word);
    }
}

fn hle_music_event_end_tie(bus: &mut Bus, track: u32) {
    let pointer = sound_read(bus, track.wrapping_add(0x40), AccessWidth::Word);
    let mut key = sound_read(bus, pointer, AccessWidth::Byte);
    if key < 0x80 {
        sound_write(bus, track.wrapping_add(5), AccessWidth::Byte, key);
        sound_write(bus, track.wrapping_add(0x40), AccessWidth::Word, pointer.wrapping_add(1));
    } else {
        key = sound_read(bus, track.wrapping_add(5), AccessWidth::Byte);
    }
    let mut channel = sound_read(bus, track.wrapping_add(0x20), AccessWidth::Word);
    while channel != 0 {
        let status = sound_read(bus, channel, AccessWidth::Byte);
        if status & 0x83 != 0
            && status & 0x40 == 0
            && sound_read(bus, channel.wrapping_add(0x11), AccessWidth::Byte) == key
        {
            sound_write(bus, channel, AccessWidth::Byte, status | 0x40);
            return;
        }
        channel = sound_read(bus, channel.wrapping_add(0x34), AccessWidth::Word);
    }
}

pub(super) fn clear_hle_music_channel_chain(bus: &mut Bus, channel: u32) {
    let track = sound_read(bus, channel.wrapping_add(0x2C), AccessWidth::Word);
    if track == 0 {
        return;
    }
    let next = sound_read(bus, channel.wrapping_add(0x34), AccessWidth::Word);
    let previous = sound_read(bus, channel.wrapping_add(0x30), AccessWidth::Word);
    if previous != 0 {
        sound_write(bus, previous.wrapping_add(0x34), AccessWidth::Word, next);
    } else {
        sound_write(bus, track.wrapping_add(0x20), AccessWidth::Word, next);
    }
    if next != 0 {
        sound_write(bus, next.wrapping_add(0x30), AccessWidth::Word, previous);
    }
    sound_write(bus, channel.wrapping_add(0x2C), AccessWidth::Word, 0);
}

pub(super) fn hle_music_fade_out_body(bus: &mut Bus, player: u32) -> Option<FaultReason> {
    let interval = sound_read(bus, player.wrapping_add(0x24), AccessWidth::Half);
    if interval == 0 {
        return None;
    }
    let counter =
        sound_read(bus, player.wrapping_add(0x26), AccessWidth::Half).wrapping_sub(1) & 0xFFFF;
    sound_write(bus, player.wrapping_add(0x26), AccessWidth::Half, counter);
    if counter != 0 {
        return None;
    }
    sound_write(bus, player.wrapping_add(0x26), AccessWidth::Half, interval);
    let mut fade_volume = sound_read(bus, player.wrapping_add(0x28), AccessWidth::Half);
    if fade_volume & 2 != 0 {
        fade_volume = fade_volume.wrapping_add(16) & 0xFFFF;
        if fade_volume >= 0x100 {
            fade_volume = 0x100;
            sound_write(bus, player.wrapping_add(0x24), AccessWidth::Half, 0);
        }
        sound_write(bus, player.wrapping_add(0x28), AccessWidth::Half, fade_volume);
    } else {
        fade_volume = fade_volume.wrapping_sub(16) & 0xFFFF;
        sound_write(bus, player.wrapping_add(0x28), AccessWidth::Half, fade_volume);
        if (fade_volume as u16 as i16) <= 0 {
            let sound_area = sound_read(bus, 0x0300_7FF0, AccessWidth::Word);
            let count = sound_read(bus, player.wrapping_add(8), AccessWidth::Byte);
            let tracks = sound_read(bus, player.wrapping_add(0x2C), AccessWidth::Word);
            for index in 0..count {
                let track = tracks.wrapping_add(index.wrapping_mul(0x50));
                if let Some(fault) = stop_hle_music_player_track(bus, sound_area, track) {
                    return Some(fault);
                }
                if fade_volume & 1 == 0 {
                    sound_write(bus, track, AccessWidth::Byte, 0);
                }
            }
            let status = sound_read(bus, player.wrapping_add(4), AccessWidth::Word);
            sound_write(
                bus,
                player.wrapping_add(4),
                AccessWidth::Word,
                if fade_volume & 1 != 0 { status | 0x8000_0000 } else { 0x8000_0000 },
            );
            sound_write(bus, player.wrapping_add(0x24), AccessWidth::Half, 0);
            return None;
        }
    }
    let count = sound_read(bus, player.wrapping_add(8), AccessWidth::Byte);
    let tracks = sound_read(bus, player.wrapping_add(0x2C), AccessWidth::Word);
    for index in 0..count {
        let track = tracks.wrapping_add(index.wrapping_mul(0x50));
        if sound_read(bus, track, AccessWidth::Byte) & 0x80 != 0 {
            sound_write(bus, track.wrapping_add(0x13), AccessWidth::Byte, fade_volume >> 2);
            let status = sound_read(bus, track, AccessWidth::Byte);
            sound_write(bus, track, AccessWidth::Byte, status | 3);
        }
    }
    None
}

pub(super) fn hle_track_volume_pitch_set(bus: &mut Bus, track: u32) {
    let status = sound_read(bus, track, AccessWidth::Byte);
    let modulation_type = sound_read(bus, track.wrapping_add(0x18), AccessWidth::Byte);
    let modulation =
        signed_sound_byte(sound_read(bus, track.wrapping_add(0x16), AccessWidth::Byte));
    if status & 3 != 0 {
        let left = sound_read(bus, track.wrapping_add(0x12), AccessWidth::Byte);
        let right = sound_read(bus, track.wrapping_add(0x13), AccessWidth::Byte);
        let mut volume = ((left * right) >> 5) as i32;
        if modulation_type == 1 {
            volume = volume * (modulation + 128) / 128;
        }
        let pan_base =
            signed_sound_byte(sound_read(bus, track.wrapping_add(0x14), AccessWidth::Byte));
        let pan_offset =
            signed_sound_byte(sound_read(bus, track.wrapping_add(0x15), AccessWidth::Byte));
        let mut pan = 2 * pan_base + pan_offset;
        if modulation_type == 2 {
            pan += modulation;
        }
        pan = pan.clamp(-128, 127);
        sound_write(
            bus,
            track.wrapping_add(0x10),
            AccessWidth::Byte,
            (((pan + 128) * volume) / 256) as u32,
        );
        sound_write(
            bus,
            track.wrapping_add(0x11),
            AccessWidth::Byte,
            (((127 - pan) * volume) / 256) as u32,
        );
    }
    if status & 0x0C != 0 {
        let bend = signed_sound_byte(sound_read(bus, track.wrapping_add(0x0E), AccessWidth::Byte))
            * sound_read(bus, track.wrapping_add(0x0F), AccessWidth::Byte) as i32;
        let mut pitch =
            (signed_sound_byte(sound_read(bus, track.wrapping_add(0x0C), AccessWidth::Byte))
                + bend)
                * 4
                + signed_sound_byte(sound_read(bus, track.wrapping_add(0x0A), AccessWidth::Byte))
                    * 256
                + signed_sound_byte(sound_read(bus, track.wrapping_add(0x0B), AccessWidth::Byte))
                    * 256
                + sound_read(bus, track.wrapping_add(0x0D), AccessWidth::Byte) as i32;
        if modulation_type == 0 {
            pitch += 16 * modulation;
        }
        sound_write(bus, track.wrapping_add(8), AccessWidth::Byte, (pitch >> 8) as u32 & 0xFF);
        sound_write(bus, track.wrapping_add(9), AccessWidth::Byte, pitch as u32 & 0xFF);
    }
    sound_write(bus, track, AccessWidth::Byte, status & !0x05);
}

impl Cpu {
    pub fn execute_hle_sound_jump(&mut self, bus: &mut Bus, index: u8) -> StepOutcome {
        let address = self.next_instruction_address;
        if let Some(reason) = self.execute_hle_sound_jump_body(bus, index) {
            return StepOutcome::Faulted(FaultStep {
                instruction_address: address,
                instruction: 0x4770,
                instruction_set: InstructionSet::Thumb,
                reason,
            });
        }
        bus.elapse(45);
        let target = self.registers[14];
        if target & 1 != 0 {
            self.cpsr |= 1 << 5;
            self.next_instruction_address = target & !1;
        } else {
            self.cpsr &= !(1 << 5);
            self.next_instruction_address = target & !3;
        }
        self.refill_pipeline(bus);
        bus.set_bios_latch(0xE12F_FF1E);
        StepOutcome::Executed(StepResult {
            instruction_address: address,
            instruction: 0x4770,
            instruction_set: InstructionSet::Thumb,
        })
    }

    pub fn execute_hle_sound_jump_body(&mut self, bus: &mut Bus, index: u8) -> Option<FaultReason> {
        let player = self.registers[0];
        let track = self.registers[1];
        match index {
            0 | 5..=8 | 21 | 22 | 24..=26 | 28 => hle_music_event_fine(bus, track),
            1 => hle_music_event_goto(bus, track),
            2 => hle_music_event_pattern(bus, track),
            3 => hle_music_event_pattern_end(bus, track),
            4 => hle_music_event_repeat(bus, track),
            9 => {
                let value = consume_hle_track_byte(bus, track);
                sound_write(bus, track.wrapping_add(0x1D), AccessWidth::Byte, u32::from(value));
            }
            10 => {
                let tempo = u32::from(consume_hle_track_byte(bus, track)) * 2;
                sound_write(bus, player.wrapping_add(0x1C), AccessWidth::Half, tempo);
                let scale = sound_read(bus, player.wrapping_add(0x1E), AccessWidth::Half);
                sound_write(
                    bus,
                    player.wrapping_add(0x20),
                    AccessWidth::Half,
                    (tempo * scale) >> 8,
                );
            }
            11 => hle_set_track_byte_and_flags(bus, track, 0x0A, 0x0C, 0x40),
            12 => hle_music_event_voice(bus, player, track),
            13 => hle_set_track_byte_and_flags(bus, track, 0x12, 3, 0),
            14 => hle_set_track_byte_and_flags(bus, track, 0x14, 3, 0x40),
            15 => hle_set_track_byte_and_flags(bus, track, 0x0E, 0x0C, 0x40),
            16 => hle_set_track_byte_and_flags(bus, track, 0x0F, 0x0C, 0),
            17 | 19 => {
                let value = consume_hle_track_byte(bus, track);
                sound_write(
                    bus,
                    track.wrapping_add(if index == 17 { 0x19 } else { 0x17 }),
                    AccessWidth::Byte,
                    u32::from(value),
                );
                if value == 0 {
                    clear_hle_track_modulation(bus, track);
                }
            }
            18 => {
                let value = consume_hle_track_byte(bus, track);
                sound_write(bus, track.wrapping_add(0x1B), AccessWidth::Byte, u32::from(value));
            }
            20 => {
                let value = consume_hle_track_byte(bus, track);
                if u32::from(value) != sound_read(bus, track.wrapping_add(0x18), AccessWidth::Byte)
                {
                    sound_write(bus, track.wrapping_add(0x18), AccessWidth::Byte, u32::from(value));
                    let status = sound_read(bus, track, AccessWidth::Byte);
                    sound_write(bus, track, AccessWidth::Byte, status | 0x0F);
                }
            }
            23 => hle_set_track_byte_and_flags(bus, track, 0x0C, 0x0C, 0x40),
            27 => {
                let offset = consume_hle_track_byte(bus, track);
                let value = consume_hle_track_byte(bus, track);
                sound_write(
                    bus,
                    0x0400_0060 + u32::from(offset),
                    AccessWidth::Byte,
                    u32::from(value),
                );
            }
            29 => hle_music_event_end_tie(bus, track),
            30 => {
                let mode = (player >> 16) as u8 & 15;
                if mode == 0 {
                    return Some(FaultReason::SampleFreqSetModeZero { mode });
                }
                let sound_area = sound_read(bus, 0x0300_7FF0, AccessWidth::Word);
                self.execute_hle_sound_driver_frequency(bus, sound_area, mode);
            }
            31 => {
                let sound_area = sound_read(bus, 0x0300_7FF0, AccessWidth::Word);
                return stop_hle_music_player_track(bus, sound_area, track);
            }
            32 => return hle_music_fade_out_body(bus, player),
            33 => hle_track_volume_pitch_set(bus, track),
            34 => clear_hle_music_channel_chain(bus, player),
            35 => clear_hle_music_player(bus, player),
            _ => unreachable!("sound jump index"),
        }
        None
    }
}
