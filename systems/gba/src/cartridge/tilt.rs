//! Yoshi/Happy Panechu cartridge two-axis tilt peripheral.

use evenfall_state::Codec;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Codec)]
pub struct Sample {
    #[codec(bits = 12)]
    pub x: u16,
    #[codec(bits = 12)]
    pub y: u16,
}

impl Default for Sample {
    fn default() -> Self {
        Self { x: 0x3a0, y: 0x3a0 }
    }
}

#[derive(Clone, Copy, Codec)]
pub struct Tilt {
    pub input: Sample,
    pub latched: Sample,
    pub command_armed: bool,
}

/// The power-on sensor with a centred input. Decoding an `Option<Tilt>` starts
/// from this value and then overwrites every field.
impl Default for Tilt {
    fn default() -> Self {
        Self::init(Sample::default())
    }
}

impl Tilt {
    pub fn init(input: Sample) -> Self {
        Self { input, latched: Sample { x: 0xfff, y: 0xfff }, command_armed: false }
    }
    pub fn set_input(&mut self, input: Sample) {
        self.input = input;
    }
    pub fn accepts_read(offset: u32) -> bool {
        matches!(offset, 0x8200 | 0x8300 | 0x8400 | 0x8500)
    }
    pub fn accepts_write(offset: u32) -> bool {
        matches!(offset, 0x8000 | 0x8100)
    }
    pub fn read(&self, offset: u32) -> u8 {
        match offset {
            0x8200 => self.latched.x as u8,
            0x8300 => ((self.latched.x >> 8) as u8) | 0x80,
            0x8400 => self.latched.y as u8,
            0x8500 => (self.latched.y >> 8) as u8,
            _ => unreachable!(),
        }
    }
    #[inline(never)]
    pub fn write(&mut self, offset: u32, value: u8) {
        match offset {
            0x8000 => {
                if value == 0x55 {
                    self.command_armed = true;
                }
            }
            0x8100 => {
                if value == 0xaa && self.command_armed {
                    self.command_armed = false;
                    self.latched = self.input;
                }
            }
            _ => unreachable!(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Zig bus_test: tilt cartridge latches two-axis samples through byte-wide SRAM registers
    #[test]
    fn tilt_cartridge_latches_two_axis_samples_through_byte_wide_sram_registers() {
        let mut tilt = Tilt::init(Sample { x: 0x392, y: 0x3a0 });
        assert_eq!(tilt.read(0x8200), 0xff);
        assert_eq!(tilt.read(0x8300), 0x8f);
        tilt.write(0x8000, 0x55);
        tilt.write(0x8100, 0xaa);
        assert_eq!(
            [tilt.read(0x8200), tilt.read(0x8300), tilt.read(0x8400), tilt.read(0x8500)],
            [0x92, 0x83, 0xa0, 0x03]
        );
        tilt.set_input(Sample { x: 0x477, y: 0x2c3 });
        assert_eq!(tilt.read(0x8200), 0x92);
        tilt.write(0x8100, 0xaa);
        assert_eq!(tilt.read(0x8200), 0x92);
        tilt.write(0x8000, 0x55);
        tilt.write(0x8100, 0xaa);
        assert_eq!(
            [tilt.read(0x8200), tilt.read(0x8300), tilt.read(0x8400), tilt.read(0x8500)],
            [0x77, 0x84, 0xc3, 0x02]
        );
        assert!(!Tilt::accepts_read(0x8600));
    }
}
