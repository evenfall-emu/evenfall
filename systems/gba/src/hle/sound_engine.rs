//! MP2K per-player track scheduler and command dispatch.

use crate::bus::{AccessWidth, Bus};
use crate::cpu::{Cpu, FaultReason};

use super::sound::{HLE_SOUND_JUMP_BASE, signed_sound_byte, sound_read, sound_write};
use super::sound_events::{
    clear_hle_music_channel_chain, hle_music_fade_out_body, hle_track_volume_pitch_set,
};
use super::sound_note::{
    MP2K_CLOCK_TABLE, hle_mp2k_channel_volume_set, hle_mp2k_event_note,
    hle_mp2k_midi_key_to_cgb_freq, hle_mp2k_midi_key_to_freq,
};
use super::sound_player::clear_hle_music_player;

fn validate_hle_mp2k_command_target(
    bus: &mut Bus,
    sound_area: u32,
    index: u8,
) -> Option<FaultReason> {
    let table = sound_read(bus, sound_area.wrapping_add(0x34), AccessWidth::Word);
    if table == 0 || table == 0x0000_3738 {
        return None;
    }
    let target = sound_read(bus, table.wrapping_add(u32::from(index) * 4), AccessWidth::Word);
    let expected = (HLE_SOUND_JUMP_BASE + u32::from(index) * 4) | 1;
    if target != expected {
        return Some(FaultReason::Mp2kReplacementCallback { command: index, target });
    }
    None
}

impl Cpu {
    pub fn execute_hle_mp2k_player_body(
        &mut self,
        bus: &mut Bus,
        sound_area: u32,
        player: u32,
    ) -> Option<FaultReason> {
        if sound_read(bus, player.wrapping_add(4), AccessWidth::Word) & 0x8000_0000 != 0 {
            return None;
        }
        if let Some(fault) = hle_music_fade_out_body(bus, player) {
            return Some(fault);
        }
        if sound_read(bus, player.wrapping_add(4), AccessWidth::Word) & 0x8000_0000 != 0 {
            return None;
        }
        let mut tempo = sound_read(bus, player.wrapping_add(0x22), AccessWidth::Half)
            .wrapping_add(sound_read(bus, player.wrapping_add(0x20), AccessWidth::Half))
            & 0xFFFF;
        sound_write(bus, player.wrapping_add(0x22), AccessWidth::Half, tempo);
        while tempo >= 150 {
            let mut track_bits = 0;
            let count = sound_read(bus, player.wrapping_add(8), AccessWidth::Byte);
            let tracks = sound_read(bus, player.wrapping_add(0x2C), AccessWidth::Word);
            for index in 0..count {
                let track = tracks.wrapping_add(index.wrapping_mul(0x50));
                if sound_read(bus, track, AccessWidth::Byte) & 0x80 == 0 {
                    continue;
                }
                if index < 32 {
                    track_bits |= 1_u32 << index;
                }
                let mut channel = sound_read(bus, track.wrapping_add(0x20), AccessWidth::Word);
                while channel != 0 {
                    let next = sound_read(bus, channel.wrapping_add(0x34), AccessWidth::Word);
                    let status = sound_read(bus, channel, AccessWidth::Byte);
                    if status & 0x80 == 0 {
                        clear_hle_music_channel_chain(bus, channel);
                    } else {
                        let gate = sound_read(bus, channel.wrapping_add(0x10), AccessWidth::Byte);
                        if gate != 0 {
                            sound_write(
                                bus,
                                channel.wrapping_add(0x10),
                                AccessWidth::Byte,
                                gate - 1,
                            );
                            if gate == 1 {
                                sound_write(bus, channel, AccessWidth::Byte, status | 0x40);
                            }
                        }
                    }
                    channel = next;
                }
                if sound_read(bus, track, AccessWidth::Byte) & 0x40 != 0 {
                    clear_hle_music_player(bus, track);
                    for (offset, value) in [(0, 0x80), (0x0F, 2), (0x13, 64), (0x19, 22), (0x24, 1)]
                    {
                        sound_write(bus, track.wrapping_add(offset), AccessWidth::Byte, value);
                    }
                }
                while sound_read(bus, track.wrapping_add(1), AccessWidth::Byte) == 0 {
                    let pointer = sound_read(bus, track.wrapping_add(0x40), AccessWidth::Word);
                    let event_byte = sound_read(bus, pointer, AccessWidth::Byte) as u8;
                    let mut event = event_byte;
                    if event < 0x80 {
                        event = sound_read(bus, track.wrapping_add(7), AccessWidth::Byte) as u8;
                        if event < 0x80 {
                            return Some(FaultReason::Mp2kMissingRunningStatus {
                                track,
                                event: event_byte,
                            });
                        }
                    } else {
                        sound_write(
                            bus,
                            track.wrapping_add(0x40),
                            AccessWidth::Word,
                            pointer.wrapping_add(1),
                        );
                        if event >= 0xBD {
                            sound_write(
                                bus,
                                track.wrapping_add(7),
                                AccessWidth::Byte,
                                u32::from(event),
                            );
                        }
                    }
                    if event >= 0xCF {
                        hle_mp2k_event_note(bus, sound_area, player, track, event - 0xCF);
                    } else if event >= 0xB1 {
                        let index = event - 0xB1;
                        if let Some(fault) =
                            validate_hle_mp2k_command_target(bus, sound_area, index)
                        {
                            return Some(fault);
                        }
                        sound_write(
                            bus,
                            player.wrapping_add(0x0A),
                            AccessWidth::Byte,
                            u32::from(index),
                        );
                        self.registers[0] = player;
                        self.registers[1] = track;
                        if let Some(fault) = self.execute_hle_sound_jump_body(bus, index) {
                            return Some(fault);
                        }
                        if sound_read(bus, track, AccessWidth::Byte) == 0 {
                            break;
                        }
                    } else {
                        sound_write(
                            bus,
                            track.wrapping_add(1),
                            AccessWidth::Byte,
                            u32::from(MP2K_CLOCK_TABLE[usize::from(event - 0x80)]),
                        );
                    }
                }
                if sound_read(bus, track, AccessWidth::Byte) == 0 {
                    continue;
                }
                let wait = sound_read(bus, track.wrapping_add(1), AccessWidth::Byte);
                sound_write(bus, track.wrapping_add(1), AccessWidth::Byte, wait.wrapping_sub(1));
                let lfo_speed = sound_read(bus, track.wrapping_add(0x19), AccessWidth::Byte);
                let depth = sound_read(bus, track.wrapping_add(0x17), AccessWidth::Byte);
                if lfo_speed != 0 && depth != 0 {
                    let delay = sound_read(bus, track.wrapping_add(0x1C), AccessWidth::Byte);
                    if delay != 0 {
                        sound_write(bus, track.wrapping_add(0x1C), AccessWidth::Byte, delay - 1);
                    } else {
                        let counter = sound_read(bus, track.wrapping_add(0x1A), AccessWidth::Byte)
                            .wrapping_add(lfo_speed) as u8;
                        sound_write(
                            bus,
                            track.wrapping_add(0x1A),
                            AccessWidth::Byte,
                            u32::from(counter),
                        );
                        let triangle = if (0x40..0xC0).contains(&counter) {
                            128 - i32::from(counter)
                        } else {
                            signed_sound_byte(u32::from(counter))
                        };
                        let modulation = ((depth as i32) * triangle).div_euclid(64);
                        if modulation
                            != signed_sound_byte(sound_read(
                                bus,
                                track.wrapping_add(0x16),
                                AccessWidth::Byte,
                            ))
                        {
                            sound_write(
                                bus,
                                track.wrapping_add(0x16),
                                AccessWidth::Byte,
                                modulation as u8 as u32,
                            );
                            let flag = if sound_read(
                                bus,
                                track.wrapping_add(0x18),
                                AccessWidth::Byte,
                            ) == 0
                            {
                                0x0C
                            } else {
                                3
                            };
                            let status = sound_read(bus, track, AccessWidth::Byte);
                            sound_write(bus, track, AccessWidth::Byte, status | flag);
                        }
                    }
                }
            }
            let tick = sound_read(bus, player.wrapping_add(0x0C), AccessWidth::Word);
            sound_write(bus, player.wrapping_add(0x0C), AccessWidth::Word, tick.wrapping_add(1));
            if track_bits == 0 {
                sound_write(bus, player.wrapping_add(4), AccessWidth::Word, 0x8000_0000);
                return None;
            }
            sound_write(bus, player.wrapping_add(4), AccessWidth::Word, track_bits);
            tempo = tempo.wrapping_sub(150);
            sound_write(bus, player.wrapping_add(0x22), AccessWidth::Half, tempo);
        }
        let count = sound_read(bus, player.wrapping_add(8), AccessWidth::Byte);
        let tracks = sound_read(bus, player.wrapping_add(0x2C), AccessWidth::Word);
        for index in 0..count {
            let track = tracks.wrapping_add(index.wrapping_mul(0x50));
            let changes = sound_read(bus, track, AccessWidth::Byte);
            if changes & 0x80 == 0 || changes & 0x0F == 0 {
                continue;
            }
            hle_track_volume_pitch_set(bus, track);
            let mut channel = sound_read(bus, track.wrapping_add(0x20), AccessWidth::Word);
            while channel != 0 {
                let next = sound_read(bus, channel.wrapping_add(0x34), AccessWidth::Word);
                if sound_read(bus, channel, AccessWidth::Byte) & 0xC7 == 0 {
                    clear_hle_music_channel_chain(bus, channel);
                    channel = next;
                    continue;
                }
                let cgb_type = sound_read(bus, channel.wrapping_add(1), AccessWidth::Byte) & 7;
                if changes & 3 != 0 {
                    hle_mp2k_channel_volume_set(bus, channel, track);
                    if cgb_type != 0 {
                        let flags = sound_read(bus, channel.wrapping_add(0x1D), AccessWidth::Byte);
                        sound_write(bus, channel.wrapping_add(0x1D), AccessWidth::Byte, flags | 1);
                    }
                }
                if changes & 0x0C != 0 {
                    let key = (signed_sound_byte(sound_read(
                        bus,
                        channel.wrapping_add(8),
                        AccessWidth::Byte,
                    )) + signed_sound_byte(sound_read(
                        bus,
                        track.wrapping_add(8),
                        AccessWidth::Byte,
                    )))
                    .max(0);
                    let fine = sound_read(bus, track.wrapping_add(9), AccessWidth::Byte) as u8;
                    if cgb_type != 0 {
                        let frequency = hle_mp2k_midi_key_to_cgb_freq(cgb_type, key, fine);
                        sound_write(bus, channel.wrapping_add(0x20), AccessWidth::Word, frequency);
                        let flags = sound_read(bus, channel.wrapping_add(0x1D), AccessWidth::Byte);
                        sound_write(bus, channel.wrapping_add(0x1D), AccessWidth::Byte, flags | 2);
                    } else {
                        let wave = sound_read(bus, channel.wrapping_add(0x24), AccessWidth::Word);
                        let frequency = hle_mp2k_midi_key_to_freq(bus, wave, key, fine);
                        sound_write(bus, channel.wrapping_add(0x20), AccessWidth::Word, frequency);
                    }
                }
                channel = next;
            }
            let status = sound_read(bus, track, AccessWidth::Byte);
            sound_write(bus, track, AccessWidth::Byte, status & 0xF0);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hle_sound_driver_main_runs_the_internal_mp2k_scheduler_and_starts_a_note() {
        let mut bus = Bus::init(&[]);
        let mut cpu = Cpu::init(0x0800_0000, 0x1F, true);
        let sound_area = 0x0200_0100_u32;
        let player = 0x0200_1000_u32;
        let track = 0x0200_1100_u32;
        let commands = 0x0200_1200_u32;
        let voices = 0x0200_1300_u32;
        let wave = 0x0200_1400_u32;
        bus.iwram[0x7FF0..0x7FF4].copy_from_slice(&sound_area.to_le_bytes());
        bus.ewram[0x100..0x104].copy_from_slice(&0x6873_6D53_u32.to_le_bytes());
        bus.ewram[0x106] = 1;
        bus.ewram[0x120..0x124].copy_from_slice(&0x2149_u32.to_le_bytes());
        bus.ewram[0x124..0x128].copy_from_slice(&player.to_le_bytes());
        bus.ewram[0x128..0x12C].copy_from_slice(&0x1709_u32.to_le_bytes());
        bus.ewram[0x102C..0x1030].copy_from_slice(&track.to_le_bytes());
        bus.ewram[0x1030..0x1034].copy_from_slice(&voices.to_le_bytes());
        bus.ewram[0x1034..0x1038].copy_from_slice(&0x6873_6D53_u32.to_le_bytes());
        bus.ewram[0x1020..0x1022].copy_from_slice(&150_u16.to_le_bytes());
        bus.ewram[0x1008] = 1;
        bus.ewram[0x1009] = 3;
        bus.ewram[0x1100] = 0x80;
        bus.ewram[0x110F] = 2;
        bus.ewram[0x1113] = 64;
        bus.ewram[0x1124] = 1;
        bus.ewram[0x1140..0x1144].copy_from_slice(&commands.to_le_bytes());
        bus.ewram[0x1200..0x1208].copy_from_slice(&[0xBD, 0, 0xBE, 127, 0xD0, 60, 100, 0x81]);
        bus.ewram[0x1304..0x1308].copy_from_slice(&wave.to_le_bytes());
        bus.ewram[0x1308..0x130C].copy_from_slice(&[0xFF, 0xF0, 0xC0, 0xE0]);
        bus.ewram[0x1404..0x1408].copy_from_slice(&0x0100_0000_u32.to_le_bytes());
        // Track echo fields must be transferred to the allocated PCM channel.
        bus.ewram[0x111E] = 0x23;
        bus.ewram[0x111F] = 0x16;

        let result = cpu.execute_hle_sound_driver_main(&mut bus);

        assert_eq!(result.fault, None);
        assert!(!result.branched);
        assert_eq!(&bus.ewram[0x100..0x104], &0x6873_6D53_u32.to_le_bytes());
        assert_eq!(&bus.ewram[0x1034..0x1038], &0x6873_6D53_u32.to_le_bytes());
        assert_eq!(&bus.ewram[0x1004..0x1008], &1_u32.to_le_bytes());
        assert_eq!(&bus.ewram[0x100C..0x1010], &1_u32.to_le_bytes());
        assert_eq!(&bus.ewram[0x1022..0x1024], &0_u16.to_le_bytes());
        assert_eq!(bus.ewram[0x1100], 0x80);
        assert_eq!(bus.ewram[0x1101], 0);
        assert_eq!(&bus.ewram[0x1140..0x1144], &(commands + 8).to_le_bytes());
        assert_eq!(&bus.ewram[0x1120..0x1124], &(sound_area + 0x50).to_le_bytes());
        assert_eq!(bus.ewram[0x150], 0x80);
        assert_eq!(&bus.ewram[0x15C..0x15E], &[0x23, 0x16]);
        assert_eq!(&bus.ewram[0x160..0x164], &[1, 60, 100, 3]);
        assert_eq!(&bus.ewram[0x174..0x178], &wave.to_le_bytes());
        assert_eq!(&bus.ewram[0x17C..0x180], &track.to_le_bytes());
        assert_ne!(&bus.ewram[0x170..0x174], &[0; 4]);
    }
}
