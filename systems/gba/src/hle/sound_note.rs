//! MP2K note allocation, channel volume and pitch conversion.

use crate::bus::{AccessWidth, Bus};

use super::sound::{signed_sound_byte, sound_read, sound_write};
use super::sound_events::{
    clear_hle_music_channel_chain, clear_hle_track_modulation, hle_track_volume_pitch_set,
};

pub(super) const MP2K_CLOCK_TABLE: [u8; 49] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F,
    0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x1C, 0x1E, 0x20, 0x24, 0x28, 0x2A, 0x2C,
    0x30, 0x34, 0x36, 0x38, 0x3C, 0x40, 0x42, 0x44, 0x48, 0x4C, 0x4E, 0x50, 0x54, 0x58, 0x5A, 0x5C,
    0x60,
];

pub(super) fn hle_mp2k_channel_volume_set(bus: &mut Bus, channel: u32, track: u32) {
    let forced_pan =
        signed_sound_byte(sound_read(bus, channel.wrapping_add(0x14), AccessWidth::Byte));
    let velocity = sound_read(bus, channel.wrapping_add(0x12), AccessWidth::Byte);
    let right = ((forced_pan + 128) as u32
        * velocity
        * sound_read(bus, track.wrapping_add(0x10), AccessWidth::Byte)
        / 0x4000)
        .min(0xFF);
    let left = ((127 - forced_pan) as u32
        * velocity
        * sound_read(bus, track.wrapping_add(0x11), AccessWidth::Byte)
        / 0x4000)
        .min(0xFF);
    sound_write(bus, channel.wrapping_add(2), AccessWidth::Byte, right);
    sound_write(bus, channel.wrapping_add(3), AccessWidth::Byte, left);
}

pub(super) fn hle_mp2k_midi_key_to_freq(
    bus: &mut Bus,
    wave: u32,
    input_key: i32,
    input_fine: u8,
) -> u32 {
    const FREQUENCIES: [u32; 12] = [
        2_147_483_648,
        2_275_179_671,
        2_410_468_894,
        2_553_802_834,
        2_705_659_852,
        2_866_546_760,
        3_037_000_500,
        3_217_589_947,
        3_408_917_802,
        3_611_622_603,
        3_826_380_858,
        4_053_909_305,
    ];
    let mut key = input_key.clamp(0, 178) as u32;
    let fine = if input_key > 178 { 0xFF } else { input_fine };
    let octave = 14 - key / 12;
    let first = FREQUENCIES[(key % 12) as usize] >> octave;
    key += 1;
    let next_octave = 14 - key / 12;
    let second = FREQUENCIES[(key % 12) as usize] >> next_octave;
    let interpolated =
        first + (((u64::from(second - first) * (u64::from(fine) << 24)) >> 32) as u32);
    ((u64::from(sound_read(bus, wave.wrapping_add(4), AccessWidth::Word))
        * u64::from(interpolated))
        >> 32) as u32
}

pub(super) fn hle_mp2k_midi_key_to_cgb_freq(
    channel_type: u32,
    input_key: i32,
    input_fine: u8,
) -> u32 {
    if channel_type == 4 {
        let mut key = if input_key <= 20 { 0 } else { (input_key - 21) as u32 };
        key = key.min(59);
        if key >= 56 {
            return 3 - (key - 56);
        }
        return ((13 - key / 4) << 4) | (7 - key % 4);
    }
    const FREQUENCIES: [i32; 12] =
        [-2004, -1891, -1785, -1685, -1591, -1501, -1417, -1337, -1262, -1192, -1125, -1062];
    let mut key = if input_key > 35 { (input_key - 36) as u32 } else { 0 };
    let mut fine = if input_key > 35 { input_fine } else { 0 };
    if key > 130 {
        key = 130;
        fine = 0xFF;
    }
    let first = FREQUENCIES[(key % 12) as usize] >> (key / 12);
    key += 1;
    let second = FREQUENCIES[(key % 12) as usize] >> (key / 12);
    (first + (i32::from(fine) * (second - first)).div_euclid(256) + 2048) as u32
}

pub(super) fn hle_mp2k_event_note(
    bus: &mut Bus,
    sound_area: u32,
    player: u32,
    track: u32,
    clock: u8,
) {
    sound_write(
        bus,
        track.wrapping_add(4),
        AccessWidth::Byte,
        u32::from(MP2K_CLOCK_TABLE[usize::from(clock)]),
    );
    let mut pointer = sound_read(bus, track.wrapping_add(0x40), AccessWidth::Word);
    if sound_read(bus, pointer, AccessWidth::Byte) < 0x80 {
        let key = sound_read(bus, pointer, AccessWidth::Byte);
        sound_write(bus, track.wrapping_add(5), AccessWidth::Byte, key);
        pointer = pointer.wrapping_add(1);
        if sound_read(bus, pointer, AccessWidth::Byte) < 0x80 {
            let velocity = sound_read(bus, pointer, AccessWidth::Byte);
            sound_write(bus, track.wrapping_add(6), AccessWidth::Byte, velocity);
            pointer = pointer.wrapping_add(1);
            if sound_read(bus, pointer, AccessWidth::Byte) < 0x80 {
                let length = sound_read(bus, track.wrapping_add(4), AccessWidth::Byte)
                    .wrapping_add(sound_read(bus, pointer, AccessWidth::Byte));
                sound_write(bus, track.wrapping_add(4), AccessWidth::Byte, length);
                pointer = pointer.wrapping_add(1);
            }
        }
        sound_write(bus, track.wrapping_add(0x40), AccessWidth::Word, pointer);
    }
    let mut forced_pan = 0_i32;
    let mut key = sound_read(bus, track.wrapping_add(5), AccessWidth::Byte);
    let mut voice = track.wrapping_add(0x24);
    let root_type = sound_read(bus, voice, AccessWidth::Byte);
    if root_type & 0xC0 != 0 {
        let voice_index = if root_type & 0x40 != 0 {
            let key_map = sound_read(bus, voice.wrapping_add(8), AccessWidth::Word);
            sound_read(bus, key_map.wrapping_add(key), AccessWidth::Byte)
        } else {
            key
        };
        voice = sound_read(bus, voice.wrapping_add(4), AccessWidth::Word)
            .wrapping_add(voice_index.wrapping_mul(12));
        if sound_read(bus, voice, AccessWidth::Byte) & 0xC0 != 0 {
            return;
        }
        if root_type & 0x80 != 0 {
            let pan_sweep = sound_read(bus, voice.wrapping_add(3), AccessWidth::Byte);
            if pan_sweep & 0x80 != 0 {
                forced_pan = signed_sound_byte((pan_sweep & 0x7F).wrapping_sub(0x40)) * 2;
            }
            key = sound_read(bus, voice.wrapping_add(1), AccessWidth::Byte);
        }
    }
    let priority = (sound_read(bus, player.wrapping_add(9), AccessWidth::Byte)
        + sound_read(bus, track.wrapping_add(0x1D), AccessWidth::Byte))
    .min(0xFF);
    let cgb_type = sound_read(bus, voice, AccessWidth::Byte) & 7;
    let mut selected = 0;
    if cgb_type != 0 {
        let cgb_channels = sound_read(bus, sound_area.wrapping_add(0x1C), AccessWidth::Word);
        if cgb_channels == 0 {
            return;
        }
        selected = cgb_channels.wrapping_add((cgb_type - 1).wrapping_mul(0x40));
        let status = sound_read(bus, selected, AccessWidth::Byte);
        if status & 0x80 != 0 && status & 0x40 == 0 {
            let old_priority = sound_read(bus, selected.wrapping_add(0x13), AccessWidth::Byte);
            let old_track = sound_read(bus, selected.wrapping_add(0x2C), AccessWidth::Word);
            if old_priority > priority || old_priority == priority && old_track < track {
                return;
            }
        }
    } else {
        let mut best_priority = priority;
        let mut best_track = track;
        let mut found_stopping = false;
        let count = sound_read(bus, sound_area.wrapping_add(6), AccessWidth::Byte);
        for index in 0..count {
            let channel = sound_area.wrapping_add(0x50).wrapping_add(index.wrapping_mul(0x40));
            let status = sound_read(bus, channel, AccessWidth::Byte);
            if status & 0x80 == 0 {
                selected = channel;
                break;
            }
            let stopping = status & 0x40 != 0;
            if stopping && !found_stopping {
                found_stopping = true;
                best_priority = sound_read(bus, channel.wrapping_add(0x13), AccessWidth::Byte);
                best_track = sound_read(bus, channel.wrapping_add(0x2C), AccessWidth::Word);
                selected = channel;
            } else if stopping == found_stopping {
                let channel_priority =
                    sound_read(bus, channel.wrapping_add(0x13), AccessWidth::Byte);
                let channel_track = sound_read(bus, channel.wrapping_add(0x2C), AccessWidth::Word);
                if channel_priority < best_priority
                    || channel_priority == best_priority && channel_track >= best_track
                {
                    best_priority = channel_priority;
                    best_track = channel_track;
                    selected = channel;
                }
            }
        }
    }
    if selected == 0 {
        return;
    }
    clear_hle_music_channel_chain(bus, selected);
    let old_head = sound_read(bus, track.wrapping_add(0x20), AccessWidth::Word);
    sound_write(bus, selected.wrapping_add(0x30), AccessWidth::Word, 0);
    sound_write(bus, selected.wrapping_add(0x34), AccessWidth::Word, old_head);
    if old_head != 0 {
        sound_write(bus, old_head.wrapping_add(0x30), AccessWidth::Word, selected);
    }
    sound_write(bus, track.wrapping_add(0x20), AccessWidth::Word, selected);
    sound_write(bus, selected.wrapping_add(0x2C), AccessWidth::Word, track);
    let lfo_delay = sound_read(bus, track.wrapping_add(0x1B), AccessWidth::Byte);
    sound_write(bus, track.wrapping_add(0x1C), AccessWidth::Byte, lfo_delay);
    if lfo_delay != 0 {
        clear_hle_track_modulation(bus, track);
    }
    hle_track_volume_pitch_set(bus, track);
    for (destination, source) in [(0x10, 4), (0x11, 5), (0x12, 6)] {
        let value = sound_read(bus, track.wrapping_add(source), AccessWidth::Byte);
        sound_write(bus, selected.wrapping_add(destination), AccessWidth::Byte, value);
    }
    sound_write(bus, selected.wrapping_add(0x13), AccessWidth::Byte, priority);
    sound_write(bus, selected.wrapping_add(8), AccessWidth::Byte, key);
    sound_write(bus, selected.wrapping_add(0x14), AccessWidth::Byte, forced_pan as u8 as u32);
    for (destination, source, width) in [
        (1, 0, AccessWidth::Byte),
        (0x24, 4, AccessWidth::Word),
        (4, 8, AccessWidth::Byte),
        (5, 9, AccessWidth::Byte),
        (6, 0x0A, AccessWidth::Byte),
        (7, 0x0B, AccessWidth::Byte),
    ] {
        let value = sound_read(bus, voice.wrapping_add(source), width);
        sound_write(bus, selected.wrapping_add(destination), width, value);
    }
    for (destination, source) in [(0x0C, 0x1E), (0x0D, 0x1F)] {
        let value = sound_read(bus, track.wrapping_add(source), AccessWidth::Byte);
        sound_write(bus, selected.wrapping_add(destination), AccessWidth::Byte, value);
    }
    hle_mp2k_channel_volume_set(bus, selected, track);
    let transposed_key = (key as i32
        + signed_sound_byte(sound_read(bus, track.wrapping_add(8), AccessWidth::Byte)))
    .max(0);
    let fine = sound_read(bus, track.wrapping_add(9), AccessWidth::Byte) as u8;
    if cgb_type != 0 {
        let pan_sweep = sound_read(bus, voice.wrapping_add(3), AccessWidth::Byte);
        let envelope = sound_read(bus, voice.wrapping_add(2), AccessWidth::Byte);
        sound_write(bus, selected.wrapping_add(0x1E), AccessWidth::Byte, envelope);
        sound_write(
            bus,
            selected.wrapping_add(0x1F),
            AccessWidth::Byte,
            if pan_sweep & 0x80 != 0 || pan_sweep & 0x70 == 0 { 8 } else { pan_sweep },
        );
        let frequency = hle_mp2k_midi_key_to_cgb_freq(cgb_type, transposed_key, fine);
        sound_write(bus, selected.wrapping_add(0x20), AccessWidth::Word, frequency);
    } else {
        let wave = sound_read(bus, selected.wrapping_add(0x24), AccessWidth::Word);
        let frequency = hle_mp2k_midi_key_to_freq(bus, wave, transposed_key, fine);
        sound_write(bus, selected.wrapping_add(0x20), AccessWidth::Word, frequency);
    }
    sound_write(bus, selected, AccessWidth::Byte, 0x80);
    let status = sound_read(bus, track, AccessWidth::Byte);
    sound_write(bus, track, AccessWidth::Byte, status & 0xF0);
}
