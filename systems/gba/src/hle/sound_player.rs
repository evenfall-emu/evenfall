//! MP2K music-player state and track command primitives.

use crate::bus::{AccessWidth, Bus};
use crate::cpu::{Cpu, FaultReason};

use super::sound::{SOUNDAREA_ID, sound_read, sound_write};

pub(super) fn clear_hle_music_player(bus: &mut Bus, player: u32) {
    for offset in (0..0x40).step_by(4) {
        sound_write(bus, player.wrapping_add(offset), AccessWidth::Word, 0);
    }
}

/// Returns the unsupported CGB stop callback, after all earlier writes.
pub(super) fn stop_hle_music_player_track(
    bus: &mut Bus,
    sound_area: u32,
    track: u32,
) -> Option<FaultReason> {
    if sound_read(bus, track, AccessWidth::Byte) & 0x80 == 0 {
        return None;
    }
    let mut channel = sound_read(bus, track.wrapping_add(0x20), AccessWidth::Word);
    while channel != 0 {
        if sound_read(bus, channel, AccessWidth::Byte) != 0 {
            let cgb_type = sound_read(bus, channel.wrapping_add(1), AccessWidth::Byte) & 7;
            if cgb_type != 0 {
                let callback = sound_read(bus, sound_area.wrapping_add(0x2C), AccessWidth::Word);
                if callback != 0x0000_1709 {
                    return Some(FaultReason::TrackStopCgbCallback { target: callback });
                }
            }
            sound_write(bus, channel, AccessWidth::Byte, 0);
        }
        sound_write(bus, channel.wrapping_add(0x2C), AccessWidth::Word, 0);
        channel = sound_read(bus, channel.wrapping_add(0x34), AccessWidth::Word);
    }
    sound_write(bus, track.wrapping_add(0x20), AccessWidth::Word, 0);
    None
}

impl Cpu {
    pub fn execute_hle_music_player_open(&mut self, bus: &mut Bus) {
        let player = self.registers[0];
        let tracks = self.registers[1];
        let requested_count = self.registers[2] as i32;
        if requested_count < 1 {
            bus.elapse(45);
            return;
        }
        let track_count = requested_count.min(16) as u8;
        clear_hle_music_player(bus, player);
        sound_write(bus, player.wrapping_add(0x2C), AccessWidth::Word, tracks);
        sound_write(bus, player.wrapping_add(8), AccessWidth::Byte, u32::from(track_count));
        sound_write(bus, player.wrapping_add(4), AccessWidth::Word, 0x8000_0000);
        for index in 0..u32::from(track_count) {
            sound_write(bus, tracks.wrapping_add(index * 0x50), AccessWidth::Byte, 0);
        }
        let sound_area = sound_read(bus, 0x0300_7FF0, AccessWidth::Word);
        if sound_read(bus, sound_area, AccessWidth::Word) == SOUNDAREA_ID {
            sound_write(bus, sound_area, AccessWidth::Word, SOUNDAREA_ID + 1);
            let old_main = sound_read(bus, sound_area.wrapping_add(0x20), AccessWidth::Word);
            if old_main != 0 {
                sound_write(bus, player.wrapping_add(0x38), AccessWidth::Word, old_main);
                let previous_player =
                    sound_read(bus, sound_area.wrapping_add(0x24), AccessWidth::Word);
                sound_write(bus, player.wrapping_add(0x3C), AccessWidth::Word, previous_player);
                sound_write(bus, sound_area.wrapping_add(0x20), AccessWidth::Word, 0);
            }
            sound_write(bus, sound_area.wrapping_add(0x24), AccessWidth::Word, player);
            sound_write(bus, sound_area.wrapping_add(0x20), AccessWidth::Word, 0x0000_2149);
            sound_write(bus, sound_area, AccessWidth::Word, SOUNDAREA_ID);
        }
        sound_write(bus, player.wrapping_add(0x34), AccessWidth::Word, SOUNDAREA_ID);
        self.registers[0] = 0x0000_2149;
        bus.elapse(45);
    }

    pub fn execute_hle_music_player_start(&mut self, bus: &mut Bus) -> Option<FaultReason> {
        let player = self.registers[0];
        let song = self.registers[1];
        if sound_read(bus, player.wrapping_add(0x34), AccessWidth::Word) != SOUNDAREA_ID {
            bus.elapse(45);
            return None;
        }
        sound_write(bus, player.wrapping_add(0x34), AccessWidth::Word, SOUNDAREA_ID + 1);
        sound_write(bus, player.wrapping_add(4), AccessWidth::Word, 0);
        sound_write(bus, player, AccessWidth::Word, song);
        let song_voicegroup = sound_read(bus, song.wrapping_add(4), AccessWidth::Word);
        sound_write(bus, player.wrapping_add(0x30), AccessWidth::Word, song_voicegroup);
        let song_priority = sound_read(bus, song.wrapping_add(2), AccessWidth::Byte);
        sound_write(bus, player.wrapping_add(9), AccessWidth::Byte, song_priority);
        sound_write(bus, player.wrapping_add(0x1C), AccessWidth::Half, 150);
        sound_write(bus, player.wrapping_add(0x20), AccessWidth::Half, 150);
        sound_write(bus, player.wrapping_add(0x1E), AccessWidth::Half, 0x100);
        sound_write(bus, player.wrapping_add(0x22), AccessWidth::Half, 0);
        sound_write(bus, player.wrapping_add(0x24), AccessWidth::Half, 0);
        let sound_area = sound_read(bus, 0x0300_7FF0, AccessWidth::Word);
        let tracks = sound_read(bus, player.wrapping_add(0x2C), AccessWidth::Word);
        let player_track_count = sound_read(bus, player.wrapping_add(8), AccessWidth::Byte);
        let song_track_count = sound_read(bus, song, AccessWidth::Byte);
        let mut index = 0;
        while index < song_track_count && index < player_track_count {
            let track = tracks.wrapping_add(index.wrapping_mul(0x50));
            if let Some(fault) = stop_hle_music_player_track(bus, sound_area, track) {
                return Some(fault);
            }
            sound_write(bus, track, AccessWidth::Byte, 0xC0);
            let command = sound_read(
                bus,
                song.wrapping_add(8).wrapping_add(index.wrapping_mul(4)),
                AccessWidth::Word,
            );
            sound_write(bus, track.wrapping_add(0x40), AccessWidth::Word, command);
            index += 1;
        }
        while index < player_track_count {
            let track = tracks.wrapping_add(index.wrapping_mul(0x50));
            if let Some(fault) = stop_hle_music_player_track(bus, sound_area, track) {
                return Some(fault);
            }
            sound_write(bus, track, AccessWidth::Byte, 0);
            index += 1;
        }
        let reverb = sound_read(bus, song.wrapping_add(3), AccessWidth::Byte);
        if reverb & 0x80 != 0 {
            self.registers[0] = reverb;
            self.execute_hle_sound_driver_mode(bus);
        }
        sound_write(bus, player.wrapping_add(0x34), AccessWidth::Word, SOUNDAREA_ID);
        bus.elapse(45);
        None
    }

    pub fn execute_hle_music_player_stop(&mut self, bus: &mut Bus) -> Option<FaultReason> {
        let player = self.registers[0];
        if sound_read(bus, player.wrapping_add(0x34), AccessWidth::Word) != SOUNDAREA_ID {
            bus.elapse(45);
            return None;
        }
        sound_write(bus, player.wrapping_add(0x34), AccessWidth::Word, SOUNDAREA_ID + 1);
        let state = sound_read(bus, player.wrapping_add(4), AccessWidth::Word);
        sound_write(bus, player.wrapping_add(4), AccessWidth::Word, state | 0x8000_0000);
        let sound_area = sound_read(bus, 0x0300_7FF0, AccessWidth::Word);
        let tracks = sound_read(bus, player.wrapping_add(0x2C), AccessWidth::Word);
        let track_count = sound_read(bus, player.wrapping_add(8), AccessWidth::Byte);
        for index in 0..track_count {
            if let Some(fault) = stop_hle_music_player_track(
                bus,
                sound_area,
                tracks.wrapping_add(index.wrapping_mul(0x50)),
            ) {
                return Some(fault);
            }
        }
        sound_write(bus, player.wrapping_add(0x34), AccessWidth::Word, SOUNDAREA_ID);
        bus.elapse(45);
        None
    }

    pub fn execute_hle_music_player_continue(&mut self, bus: &mut Bus) {
        let player = self.registers[0];
        if sound_read(bus, player.wrapping_add(0x34), AccessWidth::Word) == SOUNDAREA_ID {
            let state = sound_read(bus, player.wrapping_add(4), AccessWidth::Word);
            sound_write(bus, player.wrapping_add(4), AccessWidth::Word, state & 0x7FFF_FFFF);
            sound_write(bus, player.wrapping_add(0x34), AccessWidth::Word, SOUNDAREA_ID);
        }
        bus.elapse(45);
    }

    pub fn execute_hle_music_player_fade_out(&mut self, bus: &mut Bus) {
        let player = self.registers[0];
        if sound_read(bus, player.wrapping_add(0x34), AccessWidth::Word) == SOUNDAREA_ID {
            let speed = self.registers[1] as u16;
            sound_write(bus, player.wrapping_add(0x26), AccessWidth::Half, u32::from(speed));
            sound_write(bus, player.wrapping_add(0x24), AccessWidth::Half, u32::from(speed));
            sound_write(bus, player.wrapping_add(0x28), AccessWidth::Half, 0x100);
            sound_write(bus, player.wrapping_add(0x34), AccessWidth::Word, SOUNDAREA_ID);
        }
        bus.elapse(45);
    }
}
