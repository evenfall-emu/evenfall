//! Pure protocol state for the BIOS MultiBoot data-transfer phase.

use evenfall_state::Codec;

pub const MINIMUM_PAYLOAD_SIZE: u32 = 0x100;
pub const MAXIMUM_PAYLOAD_SIZE: u32 = 0x3ff40;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Codec)]
#[codec(tag_bits = 2)]
#[repr(u8)]
pub enum Mode {
    #[codec(rename = "normal_256_khz")]
    Normal256Khz = 0,
    #[codec(rename = "multiplayer_115_khz")]
    Multiplayer115Khz = 1,
    #[codec(rename = "normal_2_mhz")]
    Normal2Mhz = 2,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    InvalidMode,
    NoClients,
    InvalidClientBits,
    InvalidRange,
    InvalidLength,
}

impl Mode {
    pub fn parse(value: u32) -> Result<Self, Error> {
        match value {
            0 => Ok(Self::Normal256Khz),
            1 => Ok(Self::Multiplayer115Khz),
            2 => Ok(Self::Normal2Mhz),
            _ => Err(Error::InvalidMode),
        }
    }
    pub fn is_multiplayer(self) -> bool {
        self == Self::Multiplayer115Khz
    }
}

#[derive(Clone, Copy, Codec)]
pub struct Parameters {
    pub handshake_data: u8,
    pub client_data: [u8; 3],
    pub palette_data: u8,
    pub client_bit: u8,
    pub source: u32,
    pub end: u32,
}

impl Parameters {
    pub fn validate(self) -> Result<(), Error> {
        if self.client_bit & 0x0e == 0 {
            return Err(Error::NoClients);
        }
        if self.client_bit & !0x0e != 0 {
            return Err(Error::InvalidClientBits);
        }
        if self.end < self.source {
            return Err(Error::InvalidRange);
        }
        let length = self.payload_length();
        if !(MINIMUM_PAYLOAD_SIZE..=MAXIMUM_PAYLOAD_SIZE).contains(&length) || length & 0x0f != 0 {
            return Err(Error::InvalidLength);
        }
        Ok(())
    }
    pub fn payload_length(self) -> u32 {
        self.end - self.source
    }
    pub fn length_command(self) -> u16 {
        ((self.payload_length() + 0xc0) / 4 - 0x34) as u16
    }
}

#[derive(Clone, Copy, Codec)]
pub struct Encoder {
    pub checksum: u32,
    pub key_state: u32,
    pub checksum_polynomial: u32,
    pub key_xor: u32,
}

impl Encoder {
    pub fn init(mode: Mode, palette_data: u8, client_data: [u8; 3]) -> Self {
        let multiplayer = mode.is_multiplayer();
        Self {
            checksum: if multiplayer { 0xfff8 } else { 0xc387 },
            key_state: pack_bytes([palette_data, client_data[0], client_data[1], client_data[2]]),
            checksum_polynomial: if multiplayer { 0xa517 } else { 0xc37b },
            key_xor: if multiplayer { 0x6465_646f } else { 0x4320_2f2f },
        }
    }
    pub fn encode_word(&mut self, plain: u32, file_offset: u32) -> u32 {
        debug_assert!(file_offset >= 0xc0 && file_offset & 3 == 0);
        self.checksum = checksum_word(self.checksum ^ plain, self.checksum_polynomial);
        self.key_state = self.key_state.wrapping_mul(0x6f64_6573).wrapping_add(1);
        plain
            ^ 0_u32.wrapping_sub(0x0200_0000).wrapping_sub(file_offset)
            ^ self.key_state
            ^ self.key_xor
    }
    pub fn finish(&mut self, handshake_data: u8, random_data: [u8; 3]) -> u16 {
        self.checksum = checksum_word(
            self.checksum
                ^ pack_bytes([handshake_data, random_data[0], random_data[1], random_data[2]]),
            self.checksum_polynomial,
        );
        self.checksum as u16
    }
}

fn checksum_word(initial: u32, polynomial: u32) -> u32 {
    let mut checksum = initial;
    for _ in 0..32 {
        let carry = checksum & 1 != 0;
        checksum >>= 1;
        if carry {
            checksum ^= polynomial;
        }
    }
    checksum
}

fn pack_bytes(bytes: [u8; 4]) -> u32 {
    u32::from_le_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Zig: parameter validation and length command follow the BIOS ABI
    #[test]
    fn parameter_validation_and_length_command_follow_the_bios_abi() {
        let mut p = Parameters {
            handshake_data: 0x9d,
            client_data: [0x78, 0x9a, 0xbc],
            palette_data: 0x91,
            client_bit: 0x0e,
            source: 0x0800_00c0,
            end: 0x0800_01c0,
        };
        assert_eq!(p.validate(), Ok(()));
        assert_eq!(p.payload_length(), 0x100);
        assert_eq!(p.length_command(), 0x3c);
        p.client_bit = 0;
        assert_eq!(p.validate(), Err(Error::NoClients));
        p.client_bit = 2;
        p.end -= 1;
        assert_eq!(p.validate(), Err(Error::InvalidLength));
    }

    /// Zig: normal and multiplayer encoders match published algorithm vectors
    #[test]
    fn normal_and_multiplayer_encoders_match_published_algorithm_vectors() {
        for (mode, words, checksum) in [
            (Mode::Normal256Khz, [0xd986_5d4b, 0x34a3_fd3a], 0x7b69),
            (Mode::Multiplayer115Khz, [0xfec3_160b, 0x13e6_b67a], 0xd897),
        ] {
            let mut encoder = Encoder::init(mode, 0x91, [0x12, 0x34, 0x56]);
            assert_eq!(encoder.encode_word(0x0302_0100, 0xc0), words[0]);
            assert_eq!(encoder.encode_word(0x0706_0504, 0xc4), words[1]);
            assert_eq!(encoder.finish(0x9d, [0x78, 0x9a, 0xbc]), checksum);
        }
    }
}
