//! HLE BIOS service state and implementations ported from ZiGBA's `cpu.zig`.

use evenfall_state::Codec;

use crate::cpu::InstructionSet;
use crate::multiboot;

#[derive(Clone, Copy, Debug, Default, Codec)]
pub struct HleIrqFrame {
    pub return_address: u32,
    pub saved_spsr_irq: u32,
    pub saved_r0_r3: [u32; 4],
    pub saved_r12: u32,
}

#[derive(Clone, Copy, Debug, Codec)]
pub struct HleSoundChannelClearContinuation {
    pub sound_area: u32,
    pub identifier: u32,
    pub extension_cursor: u32,
    pub channel: u8,
    pub return_address: u32,
    pub return_cpsr: u32,
    pub return_lr: u32,
    pub saved_r4_r7: [u32; 4],
}

#[derive(Clone, Copy, Debug, Codec)]
pub enum HleSoundDriverMainPhase {
    AfterPlayerCallback,
    AfterMp2kCallback,
    AfterCgbCallback,
}

#[derive(Clone, Copy, Debug, Codec)]
pub struct HleSoundDriverMainContinuation {
    pub sound_area: u32,
    pub scanline_limit: u32,
    pub phase: HleSoundDriverMainPhase,
    pub return_address: u32,
    pub return_cpsr: u32,
    pub return_lr: u32,
    pub saved_r4_r11: [u32; 8],
    pub pending_players: [u32; 16],
    #[codec(bits = 5)]
    pub pending_player_count: u8,
}

#[derive(Clone, Copy, Debug, Codec)]
pub enum HleMultiBootPhase {
    InitialDelay,
    LaunchLength,
    WaitLength,
    LaunchData,
    WaitData,
    LaunchCrcRequest,
    WaitCrcRequest,
    LaunchCrcSignal,
    WaitCrcSignal,
    LaunchCrc,
    WaitCrc,
}

#[derive(Clone, Copy, Debug, Codec)]
pub struct HleMultiBootContinuation {
    pub parameters: multiboot::Parameters,
    pub mode: multiboot::Mode,
    pub encoder: multiboot::Encoder,
    pub phase: HleMultiBootPhase,
    pub source_offset: u32,
    pub encoded_word: u32,
    pub multiplayer_high_half: bool,
    pub random_data: [u8; 3],
    pub final_checksum: u16,
    pub wait_started_at: u64,
    pub instruction_address: u32,
    pub instruction_set: InstructionSet,
}

#[derive(Clone, Copy, Debug, Codec)]
pub struct HleCpuSetWork {
    pub source: u32,
    pub destination: u32,
    pub remaining: u32,
    pub fill_value: u32,
    pub fill: bool,
    pub word: bool,
}

#[derive(Clone, Copy, Debug, Codec)]
pub struct HleCpuFastSetWork {
    pub source: u32,
    pub destination: u32,
    pub remaining_blocks: u32,
    pub last_word: u32,
    pub fill: bool,
}

#[derive(Clone, Copy, Debug, Codec)]
pub struct HleBgAffineSetWork {
    pub source: u32,
    pub destination: u32,
    pub remaining: u32,
}

#[derive(Clone, Copy, Debug, Codec)]
pub struct HleObjAffineSetWork {
    pub source: u32,
    pub destination: u32,
    pub remaining: u32,
    pub stride: u32,
}

#[derive(Clone, Copy, Debug, Codec)]
pub struct HleBitUnpackWork {
    pub source: u32,
    pub destination: u32,
    pub source_length: u32,
    pub bias: u32,
    pub input: u32,
    pub output: u32,
    pub source_width: u8,
    pub destination_width: u8,
    pub input_bits_remaining: u8,
    pub output_bits: u8,
}

#[derive(Clone, Copy, Debug, Codec)]
pub struct HleLz77Work {
    pub source: u32,
    pub destination: u32,
    pub remaining: u32,
    pub pending_halfword: u16,
    pub block_header: u8,
    #[codec(bits = 4)]
    pub blocks_remaining: u8,
    pub vram_output: bool,
}

#[derive(Clone, Copy, Debug, Codec)]
pub struct HleHuffmanWork {
    pub tree_base: u32,
    pub bitstream_address: u32,
    pub destination: u32,
    pub remaining: u32,
    pub node_address: u32,
    pub output: u32,
    pub node: u8,
    #[codec(bits = 6)]
    pub symbol_bits: u8,
    #[codec(bits = 6)]
    pub output_bits: u8,
}

#[derive(Clone, Copy, Debug, Codec)]
pub struct HleRunLengthWork {
    pub source: u32,
    pub destination: u32,
    pub remaining: u32,
    pub padding: u32,
    pub pending_halfword: u16,
    pub vram_output: bool,
}

#[derive(Clone, Copy, Debug, Codec)]
pub struct HleDifferentialFilterWork {
    pub source: u32,
    pub destination: u32,
    pub remaining: u32,
    pub previous: u16,
    pub pending_halfword: u16,
    pub input_halfword: bool,
    pub output_halfword: bool,
}

#[derive(Clone, Copy, Debug, Codec)]
pub enum HleBulkWork {
    CpuSet(HleCpuSetWork),
    CpuFastSet(HleCpuFastSetWork),
    BgAffineSet(HleBgAffineSetWork),
    ObjAffineSet(HleObjAffineSetWork),
    BitUnpack(HleBitUnpackWork),
    Lz77(HleLz77Work),
    Huffman(HleHuffmanWork),
    RunLength(HleRunLengthWork),
    DifferentialFilter(HleDifferentialFilterWork),
}

#[derive(Clone, Copy, Debug, Codec)]
pub struct HleBulkService {
    pub instruction_address: u32,
    pub instruction: u32,
    pub instruction_set: InstructionSet,
    pub work: HleBulkWork,
}

#[derive(Clone, Copy, Debug, Codec)]
pub enum HleService {
    Multiboot(HleMultiBootContinuation),
    Bulk(HleBulkService),
}

impl HleBulkWork {
    pub fn service(self) -> u8 {
        match self {
            Self::CpuSet(_) => 0x0B,
            Self::CpuFastSet(_) => 0x0C,
            Self::BgAffineSet(_) => 0x0E,
            Self::ObjAffineSet(_) => 0x0F,
            Self::BitUnpack(_) => 0x10,
            Self::Lz77(work) => if work.vram_output { 0x12 } else { 0x11 },
            Self::Huffman(_) => 0x13,
            Self::RunLength(work) => if work.vram_output { 0x15 } else { 0x14 },
            Self::DifferentialFilter(work) => if work.input_halfword { 0x18 } else if work.output_halfword { 0x17 } else { 0x16 },
        }
    }

    pub fn valid(self) -> bool {
        match self {
            Self::CpuSet(work) => work.remaining <= 0x001F_FFFF &&
                (work.source | work.destination) & if work.word { 3 } else { 1 } == 0,
            Self::CpuFastSet(work) => work.remaining_blocks <= 0x0004_0000 &&
                (work.source | work.destination) & 3 == 0,
            Self::BgAffineSet(work) => bg_affine_set_mmio_fault(work.source, work.destination, work.remaining).is_none(),
            Self::ObjAffineSet(_) => true,
            Self::BitUnpack(work) => matches!(work.source_width, 1 | 2 | 4 | 8) &&
                matches!(work.destination_width, 1 | 2 | 4 | 8 | 16 | 32) &&
                work.input_bits_remaining <= 8 &&
                work.input_bits_remaining % work.source_width == 0 &&
                work.output_bits < 32 &&
                work.output_bits % work.destination_width == 0,
            Self::Lz77(work) => work.blocks_remaining <= 8,
            Self::Huffman(work) => matches!(work.symbol_bits, 4 | 8) &&
                work.output_bits < 32 && work.output_bits % work.symbol_bits == 0,
            Self::RunLength(work) => work.padding <= 3,
            Self::DifferentialFilter(work) => work.output_halfword || !work.input_halfword,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HleArcTanResult {
    angle: i16,
    a: i32,
    b: i32,
    cycles: u64,
}

fn multiply_iterations(multiplier: u32, signed: bool) -> u64 {
    let sign = ((multiplier as i32) >> 31) as u32;
    let magnitude = if signed { multiplier ^ sign } else { multiplier };
    1 + u64::from(magnitude > 0xFF)
        + u64::from(magnitude > 0xFFFF)
        + u64::from(magnitude > 0xFF_FFFF)
}

fn hle_arc_tan(input: i32) -> HleArcTanResult {
    let mut cycles = 37;
    let square = input.wrapping_mul(input);
    cycles += multiply_iterations(square as u32, true);
    let a = (square >> 14).wrapping_neg();
    let mut product = 0xA9_i32.wrapping_mul(a);
    cycles += multiply_iterations(product as u32, true);
    let mut b = (product >> 14).wrapping_add(0x390);
    for constant in [0x91C, 0xFB6, 0x16AA, 0x2081, 0x3651, 0xA2F9] {
        product = b.wrapping_mul(a);
        cycles += multiply_iterations(product as u32, true);
        b = (product >> 14).wrapping_add(constant);
    }
    let angle = (input.wrapping_mul(b) >> 16) as i16;
    HleArcTanResult { angle, a, b, cycles }
}

fn scaled_arc_tan_ratio(numerator: i32, denominator: i32) -> i32 {
    let shifted = ((numerator as u32) << 14) as i32;
    if shifted == i32::MIN && denominator == -1 {
        i32::MIN
    } else {
        shifted / denominator
    }
}

fn hle_arc_tan2(x: i32, y: i32) -> (u16, Option<HleArcTanResult>) {
    if y == 0 {
        return (if x >= 0 { 0 } else { 0x8000 }, None);
    }
    if x == 0 {
        return (if y >= 0 { 0x4000 } else { 0xC000 }, None);
    }
    let negative_x = x.wrapping_neg();
    let negative_y = y.wrapping_neg();
    let (input, transform) = if y >= 0 {
        if x >= 0 && x >= y {
            (scaled_arc_tan_ratio(y, x), 0)
        } else if x < 0 && negative_x >= y {
            (scaled_arc_tan_ratio(y, x), 1)
        } else {
            (scaled_arc_tan_ratio(x, y), 2)
        }
    } else if x <= 0 && negative_x > negative_y {
        (scaled_arc_tan_ratio(y, x), 1)
    } else if x > 0 && x >= negative_y {
        (scaled_arc_tan_ratio(y, x), 0)
    } else {
        (scaled_arc_tan_ratio(x, y), 3)
    };
    let result = hle_arc_tan(input);
    let base = result.angle as u16;
    let angle = match transform {
        0 => base,
        1 => base.wrapping_add(0x8000),
        2 => 0x4000_u16.wrapping_sub(base),
        _ => 0xC000_u16.wrapping_sub(base),
    };
    (angle, Some(result))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ZiGBA: "mGBA BIOS math table ArcTan cases preserve fixed-point BIOS intermediates".
    #[test]
    fn mgba_bios_math_table_arc_tan_cases_preserve_fixed_point_bios_intermediates() {
        let cases = [
            (0x0000_0000_u32, 0x0000_0000, 0, 0xA2F9),
            (0x0000_3FFF, 0x0000_1FFF, 0xFFFF_C002, 0x8001),
            (0x0000_4000, 0x0000_2000, 0xFFFF_C000, 0x8000),
            (0x0000_4001, 0x0000_1FFF, 0xFFFF_BFFE, 0x7FFE),
            (0x0000_8000, 0x0000_16A2, 0xFFFF_0000, 0x0002_2D45),
            (0x0000_FFFF, 0xFFFF_A2FE, 8, 0xA2FF),
            (0xFFFF_0000, 0x0000_5D07, 0, 0xA2F9),
            (0xFFFF_C000, 0xFFFF_E000, 0xFFFF_C000, 0x8000),
            (0xFFFF_FFFF, 0xFFFF_FFFF, 0, 0xA2F9),
        ];
        for (input, angle, a, b) in cases {
            let result = hle_arc_tan(input as i32);
            assert_eq!((result.angle as i32 as u32, result.a as u32, result.b as u32), (angle, a, b), "input={input:08x}");
        }
        assert_eq!(hle_arc_tan(0).cycles, 45);
    }

    /// ZiGBA: "mGBA BIOS math table ArcTan2 covers axes and every quadrant".
    #[test]
    fn mgba_bios_math_table_arc_tan2_covers_axes_and_every_quadrant() {
        let cases = [
            (0_u32, 0_u32, 0_u16, None),
            (1, 0, 0, None),
            (0xFFFF_FFFF, 0, 0x8000, None),
            (0, 1, 0x4000, None),
            (0, 0xFFFF_0000, 0xC000, None),
            (1, 1, 0x2000, Some(0xFFFF_C000)),
            (0xFFFF_FFFF, 1, 0x6000, Some(0xFFFF_C000)),
            (0xFFFF_0000, 0xFFFF_0000, 0xA000, Some(0xFFFF_C000)),
            (0x0000_FFFF, 0xFFFF_0000, 0xE000, Some(0xFFFF_C002)),
            (0x0000_8000, 0x0000_4000, 0x12E4, Some(0xFFFF_F000)),
            (0xFFFF_4000, 0x0000_8000, 0x680B, Some(0xFFFF_E390)),
        ];
        for (x, y, angle, a) in cases {
            let (actual, intermediate) = hle_arc_tan2(x as i32, y as i32);
            assert_eq!(actual, angle, "x={x:08x} y={y:08x}");
            assert_eq!(intermediate.map(|r| r.a as u32), a, "x={x:08x} y={y:08x}");
        }
    }
}
