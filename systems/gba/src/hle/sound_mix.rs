//! SoundDriverMain PCM envelope and mixer, in timed guest access order.

use crate::bus::{AccessWidth, Bus};
use crate::cpu::Cpu;

use super::sound::{SOUNDAREA_ID, mixed_sound_byte, signed_sound_byte, sound_read, sound_write};

impl Cpu {
    pub fn mix_hle_sound_driver_main(
        &mut self,
        bus: &mut Bus,
        sound_area: u32,
        scanline_limit: u32,
    ) {
        let samples = sound_read(bus, sound_area.wrapping_add(0x10), AccessWidth::Word);
        if samples == 0 || samples > 0x630 {
            return;
        }
        let dma_counter = sound_read(bus, sound_area.wrapping_add(4), AccessWidth::Byte);
        let mut pcm = sound_area.wrapping_add(0x350);
        if dma_counter > 1 {
            let period = sound_read(bus, sound_area.wrapping_add(0x0B), AccessWidth::Byte);
            pcm = pcm.wrapping_add(samples.wrapping_mul(period.wrapping_sub(dma_counter - 1)));
        }
        let pcm_other = pcm.wrapping_add(0x630);
        let reverb = sound_read(bus, sound_area.wrapping_add(5), AccessWidth::Byte);
        if reverb == 0 {
            for index in 0..samples {
                sound_write(bus, pcm.wrapping_add(index), AccessWidth::Byte, 0);
                sound_write(bus, pcm_other.wrapping_add(index), AccessWidth::Byte, 0);
            }
        } else {
            let echo = if dma_counter == 2 {
                sound_area.wrapping_add(0x350)
            } else {
                pcm.wrapping_add(samples)
            };
            for index in 0..samples {
                let sum =
                    signed_sound_byte(sound_read(bus, pcm.wrapping_add(index), AccessWidth::Byte))
                        + signed_sound_byte(sound_read(
                            bus,
                            pcm_other.wrapping_add(index),
                            AccessWidth::Byte,
                        ))
                        + signed_sound_byte(sound_read(
                            bus,
                            echo.wrapping_add(index),
                            AccessWidth::Byte,
                        ))
                        + signed_sound_byte(sound_read(
                            bus,
                            echo.wrapping_add(0x630).wrapping_add(index),
                            AccessWidth::Byte,
                        ));
                let mut mixed = (sum * reverb as i32).div_euclid(512);
                if mixed & 0x80 != 0 {
                    mixed += 1;
                }
                let byte = mixed as u8;
                sound_write(bus, pcm.wrapping_add(index), AccessWidth::Byte, u32::from(byte));
                sound_write(bus, pcm_other.wrapping_add(index), AccessWidth::Byte, u32::from(byte));
            }
        }
        let sample_rate = sound_read(bus, sound_area.wrapping_add(0x14), AccessWidth::Word);
        let reciprocal = sound_read(bus, sound_area.wrapping_add(0x18), AccessWidth::Word);
        let master_volume = sound_read(bus, sound_area.wrapping_add(7), AccessWidth::Byte);
        let channel_count = sound_read(bus, sound_area.wrapping_add(6), AccessWidth::Byte);
        for channel_index in 0..channel_count {
            if scanline_limit != 0 {
                let mut vcount = sound_read(bus, 0x0400_0006, AccessWidth::Byte);
                if vcount < 160 {
                    vcount += 228;
                }
                if vcount >= scanline_limit {
                    break;
                }
            }
            let channel =
                sound_area.wrapping_add(0x50).wrapping_add(channel_index.wrapping_mul(0x40));
            let wave = sound_read(bus, channel.wrapping_add(0x24), AccessWidth::Word);
            let mut status = sound_read(bus, channel, AccessWidth::Byte) as u8;
            if status & 0xC7 == 0 {
                continue;
            }

            let mut envelope = 0;
            if status & 0x80 != 0 {
                if status & 0x40 != 0 {
                    sound_write(bus, channel, AccessWidth::Byte, 0);
                    continue;
                }
                status = 3;
                sound_write(bus, channel, AccessWidth::Byte, u32::from(status));
                sound_write(
                    bus,
                    channel.wrapping_add(0x28),
                    AccessWidth::Word,
                    wave.wrapping_add(0x10),
                );
                let length = sound_read(bus, wave.wrapping_add(0x0C), AccessWidth::Word);
                sound_write(bus, channel.wrapping_add(0x18), AccessWidth::Word, length);
                sound_write(bus, channel.wrapping_add(9), AccessWidth::Byte, 0);
                sound_write(bus, channel.wrapping_add(0x1C), AccessWidth::Word, 0);
                if sound_read(bus, wave.wrapping_add(3), AccessWidth::Byte) & 0xC0 != 0 {
                    status |= 0x10;
                    sound_write(bus, channel, AccessWidth::Byte, u32::from(status));
                }
            } else {
                envelope = sound_read(bus, channel.wrapping_add(9), AccessWidth::Byte);
                if status & 4 != 0 {
                    let echo_length =
                        sound_read(bus, channel.wrapping_add(0x0D), AccessWidth::Byte) as u8;
                    let next = echo_length.wrapping_sub(1);
                    sound_write(
                        bus,
                        channel.wrapping_add(0x0D),
                        AccessWidth::Byte,
                        u32::from(next),
                    );
                    if echo_length <= 1 {
                        sound_write(bus, channel, AccessWidth::Byte, 0);
                        continue;
                    }
                } else if status & 0x40 != 0 {
                    let decay = sound_read(bus, channel.wrapping_add(7), AccessWidth::Byte);
                    envelope = envelope.wrapping_mul(decay) >> 8;
                    let echo_volume =
                        sound_read(bus, channel.wrapping_add(0x0C), AccessWidth::Byte);
                    if envelope <= echo_volume {
                        if echo_volume == 0 {
                            sound_write(bus, channel, AccessWidth::Byte, 0);
                            continue;
                        }
                        envelope = echo_volume;
                        status |= 4;
                        sound_write(bus, channel, AccessWidth::Byte, u32::from(status));
                    }
                } else if status & 3 == 2 {
                    let decay = sound_read(bus, channel.wrapping_add(5), AccessWidth::Byte);
                    envelope = envelope.wrapping_mul(decay) >> 8;
                    let sustain = sound_read(bus, channel.wrapping_add(6), AccessWidth::Byte);
                    if envelope <= sustain {
                        envelope = sustain;
                        if sustain == 0 {
                            let echo_volume =
                                sound_read(bus, channel.wrapping_add(0x0C), AccessWidth::Byte);
                            if echo_volume == 0 {
                                sound_write(bus, channel, AccessWidth::Byte, 0);
                                continue;
                            }
                            envelope = echo_volume;
                            status |= 4;
                        } else {
                            status = status.wrapping_sub(1);
                        }
                        sound_write(bus, channel, AccessWidth::Byte, u32::from(status));
                    }
                }
            }
            if status & 3 == 3 {
                envelope = envelope.wrapping_add(sound_read(
                    bus,
                    channel.wrapping_add(4),
                    AccessWidth::Byte,
                ));
                if envelope >= 0xFF {
                    envelope = 0xFF;
                    status = status.wrapping_sub(1);
                    sound_write(bus, channel, AccessWidth::Byte, u32::from(status));
                }
            }
            sound_write(bus, channel.wrapping_add(9), AccessWidth::Byte, envelope);
            let scaled = ((master_volume + 1).wrapping_mul(envelope)) >> 4;
            let right = sound_read(bus, channel.wrapping_add(2), AccessWidth::Byte)
                .wrapping_mul(scaled)
                >> 8;
            let left = sound_read(bus, channel.wrapping_add(3), AccessWidth::Byte)
                .wrapping_mul(scaled)
                >> 8;
            sound_write(bus, channel.wrapping_add(0x0A), AccessWidth::Byte, right);
            sound_write(bus, channel.wrapping_add(0x0B), AccessWidth::Byte, left);

            let mut loop_pointer = 0;
            let mut loop_length = 0;
            if status & 0x10 != 0 {
                let loop_start = sound_read(bus, wave.wrapping_add(8), AccessWidth::Word);
                loop_pointer = wave.wrapping_add(0x10).wrapping_add(loop_start);
                loop_length = sound_read(bus, wave.wrapping_add(0x0C), AccessWidth::Word)
                    .wrapping_sub(loop_start);
            }
            let mut remaining = sound_read(bus, channel.wrapping_add(0x18), AccessWidth::Word);
            let mut source = sound_read(bus, channel.wrapping_add(0x28), AccessWidth::Word);
            let mut phase = sound_read(bus, channel.wrapping_add(0x1C), AccessWidth::Word);
            let frequency = sound_read(bus, channel.wrapping_add(0x20), AccessWidth::Word);
            let fixed_rate = sound_read(bus, channel.wrapping_add(1), AccessWidth::Byte) & 8 != 0;
            for out in 0..samples {
                if remaining == 0 {
                    if loop_length == 0 {
                        sound_write(bus, channel, AccessWidth::Byte, 0);
                        break;
                    }
                    remaining = loop_length;
                    source = loop_pointer;
                }
                let mut sample = signed_sound_byte(sound_read(bus, source, AccessWidth::Byte));
                if !fixed_rate && sample_rate != 0 {
                    let next_sample = signed_sound_byte(sound_read(
                        bus,
                        source.wrapping_add(1),
                        AccessWidth::Byte,
                    ));
                    let delta =
                        i64::from(next_sample - sample) * i64::from(phase) * i64::from(reciprocal);
                    sample += (delta >> 23) as i32;
                }
                let right_sample = sample.wrapping_mul(right as i32) >> 8;
                let left_sample = sample.wrapping_mul(left as i32) >> 8;
                let old_right = sound_read(bus, pcm.wrapping_add(out), AccessWidth::Byte);
                sound_write(
                    bus,
                    pcm.wrapping_add(out),
                    AccessWidth::Byte,
                    u32::from(mixed_sound_byte(old_right, right_sample)),
                );
                let old_left = sound_read(bus, pcm_other.wrapping_add(out), AccessWidth::Byte);
                sound_write(
                    bus,
                    pcm_other.wrapping_add(out),
                    AccessWidth::Byte,
                    u32::from(mixed_sound_byte(old_left, left_sample)),
                );
                if fixed_rate {
                    source = source.wrapping_add(1);
                    remaining = remaining.wrapping_sub(1);
                    if remaining == 0 {
                        if loop_length == 0 {
                            sound_write(bus, channel, AccessWidth::Byte, 0);
                        } else {
                            remaining = loop_length;
                            source = loop_pointer;
                        }
                    }
                } else if sample_rate == 0 {
                    sound_write(bus, channel, AccessWidth::Byte, 0);
                    break;
                } else {
                    phase = phase.wrapping_add(frequency);
                    while phase >= sample_rate {
                        phase = phase.wrapping_sub(sample_rate);
                        source = source.wrapping_add(1);
                        remaining = remaining.wrapping_sub(1);
                        if remaining == 0 {
                            if loop_length == 0 {
                                sound_write(bus, channel, AccessWidth::Byte, 0);
                                break;
                            }
                            remaining = loop_length;
                            source = loop_pointer;
                        }
                    }
                }
            }
            sound_write(bus, channel.wrapping_add(0x18), AccessWidth::Word, remaining);
            sound_write(bus, channel.wrapping_add(0x28), AccessWidth::Word, source);
            sound_write(bus, channel.wrapping_add(0x1C), AccessWidth::Word, phase);
        }
        self.registers[0] = sound_area;
        self.registers[3] = SOUNDAREA_ID;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ZiGBA machine_test: "HLE SoundDriverMain advances an attacking fixed-rate PCM channel".
    #[test]
    fn hle_sound_driver_main_advances_an_attacking_fixed_rate_pcm_channel() {
        let mut bus = Bus::init(&[]);
        let mut cpu = Cpu::init(0x0800_0000, 0x1F, true);
        let area = 0x0200_0100;
        let wave = 0x0200_1000_u32;
        bus.ewram[0x106] = 1;
        bus.ewram[0x107] = 15;
        bus.ewram[0x110..0x114].copy_from_slice(&4_u32.to_le_bytes());
        bus.ewram[0x150] = 0x80;
        bus.ewram[0x151] = 8;
        bus.ewram[0x152..0x155].fill(0xFF);
        bus.ewram[0x174..0x178].copy_from_slice(&wave.to_le_bytes());
        bus.ewram[0x100C..0x1010].copy_from_slice(&4_u32.to_le_bytes());
        bus.ewram[0x1010..0x1014].copy_from_slice(&[1, 0xFE, 3, 0xFC]);

        cpu.mix_hle_sound_driver_main(&mut bus, area, 0);

        assert_eq!(bus.ewram[0x150], 0);
        assert_eq!(bus.ewram[0x159], 0xFF);
        assert_eq!(&bus.ewram[0x15A..0x15C], &[0xFE, 0xFE]);
        assert_eq!(&bus.ewram[0x450..0x454], &[0, 0xFE, 2, 0xFC]);
        assert_eq!(&bus.ewram[0xA80..0xA84], &[0, 0xFE, 2, 0xFC]);
        assert_eq!(u32::from_le_bytes(bus.ewram[0x178..0x17C].try_into().unwrap()), wave + 0x14);
    }

    /// ZiGBA machine_test: "HLE SoundDriverMain linearly interpolates a variable-rate PCM channel".
    #[test]
    fn hle_sound_driver_main_linearly_interpolates_a_variable_rate_pcm_channel() {
        let mut bus = Bus::init(&[]);
        let mut cpu = Cpu::init(0x0800_0000, 0x1F, true);
        let area = 0x0200_0100;
        let wave = 0x0200_1000_u32;
        bus.ewram[0x106] = 1;
        bus.ewram[0x107] = 15;
        bus.ewram[0x110..0x114].copy_from_slice(&2_u32.to_le_bytes());
        bus.ewram[0x114..0x118].copy_from_slice(&4_u32.to_le_bytes());
        bus.ewram[0x118..0x11C].copy_from_slice(&0x0020_0000_u32.to_le_bytes());
        bus.ewram[0x150] = 0x80;
        bus.ewram[0x152..0x155].fill(0xFF);
        bus.ewram[0x170..0x174].copy_from_slice(&2_u32.to_le_bytes());
        bus.ewram[0x174..0x178].copy_from_slice(&wave.to_le_bytes());
        bus.ewram[0x100C..0x1010].copy_from_slice(&2_u32.to_le_bytes());
        bus.ewram[0x1011] = 100;

        cpu.mix_hle_sound_driver_main(&mut bus, area, 0);

        assert_eq!(bus.ewram[0x150], 2);
        assert_eq!(u32::from_le_bytes(bus.ewram[0x168..0x16C].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(bus.ewram[0x16C..0x170].try_into().unwrap()), 0);
        assert_eq!(u32::from_le_bytes(bus.ewram[0x178..0x17C].try_into().unwrap()), wave + 0x11);
        assert_eq!(&bus.ewram[0x450..0x452], &[0, 49]);
        assert_eq!(&bus.ewram[0xA80..0xA82], &[0, 49]);
    }
}
