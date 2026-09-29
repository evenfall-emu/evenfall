//! HLE BIOS services ported from ZiGBA's `cpu.zig`.

mod bulk;
mod callback;
mod dispatch;
mod multiboot;
mod sound;
mod sound_engine;
mod sound_events;
mod sound_main;
mod sound_mix;
mod sound_note;
mod sound_player;
mod utility;

use crate::bus::AccessWidth;
use crate::cpu::{Cpu, FaultReason, HleMemoryOperation};

fn invalid_hle_mmio_word_access(
    service: u8,
    operation: HleMemoryOperation,
    address: u32,
) -> Option<FaultReason> {
    if address >> 24 != 4 || address & 3 == 0 {
        return None;
    }
    Some(FaultReason::InvalidHleMemoryAccess {
        service,
        operation,
        address,
        width: AccessWidth::Word,
    })
}

fn first_unaligned_mmio_record(base: u32, stride: u32, count: u32) -> Option<u32> {
    debug_assert!(stride != 0 && stride.is_multiple_of(4) && stride <= 0x0100_0000);
    if base & 3 == 0 {
        return None;
    }
    let record = if base >> 24 == 0x04 {
        0_u64
    } else {
        u64::from(0x0400_0000_u32.wrapping_sub(base)).div_ceil(u64::from(stride))
    };
    (record < u64::from(count)).then_some(record as u32)
}

/// Returns the first misaligned I/O word access (address, whether it writes).
/// A record visits source, source + 4, destination + 8, destination + 12.
fn bg_affine_set_mmio_fault(source: u32, destination: u32, count: u32) -> Option<(u32, bool)> {
    let streams = [
        (source, 20_u32, false),
        (source.wrapping_add(4), 20, false),
        (destination.wrapping_add(8), 16, true),
        (destination.wrapping_add(12), 16, true),
    ];
    let mut first: Option<(u32, usize)> = None;
    for (index, &(base, stride, _)) in streams.iter().enumerate() {
        if let Some(record) = first_unaligned_mmio_record(base, stride, count)
            && first.is_none_or(|(earlier, _)| record < earlier)
        {
            first = Some((record, index));
        }
    }
    let (record, index) = first?;
    let (base, stride, write) = streams[index];
    Some((base.wrapping_add(stride.wrapping_mul(record)), write))
}

impl Cpu {
    /// Rejects direct guest word pointers before any register or bus changes.
    pub fn preflight_hle_software_interrupt(&self, service: u8) -> Option<FaultReason> {
        let r = &self.registers;
        match service {
            0x0E => bg_affine_set_mmio_fault(r[0], r[1], r[2]).and_then(|(address, write)| {
                invalid_hle_mmio_word_access(
                    service,
                    if write { HleMemoryOperation::Write } else { HleMemoryOperation::Read },
                    address,
                )
            }),
            0x10 => invalid_hle_mmio_word_access(
                service,
                HleMemoryOperation::Read,
                r[2].wrapping_add(4),
            ),
            0x11 | 0x12 => invalid_hle_mmio_word_access(service, HleMemoryOperation::Read, r[0]),
            0x1A => invalid_hle_mmio_word_access(service, HleMemoryOperation::Write, r[13])
                .or_else(|| {
                    invalid_hle_mmio_word_access(
                        service,
                        HleMemoryOperation::Write,
                        r[0].wrapping_add(0x10),
                    )
                }),
            0x1F => invalid_hle_mmio_word_access(
                service,
                HleMemoryOperation::Read,
                r[0].wrapping_add(4),
            ),
            0x20 if (r[2] as i32) > 0 => {
                invalid_hle_mmio_word_access(service, HleMemoryOperation::Write, r[0])
            }
            0x21..=0x24 => invalid_hle_mmio_word_access(
                service,
                HleMemoryOperation::Read,
                r[0].wrapping_add(0x34),
            ),
            0x25 if r[1] <= 2 => invalid_hle_mmio_word_access(
                service,
                HleMemoryOperation::Read,
                r[0].wrapping_add(0x20),
            )
            .or_else(|| {
                invalid_hle_mmio_word_access(
                    service,
                    HleMemoryOperation::Read,
                    r[0].wrapping_add(0x24),
                )
            }),
            0x2A => invalid_hle_mmio_word_access(service, HleMemoryOperation::Write, r[0]),
            _ => None,
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
    if shifted == i32::MIN && denominator == -1 { i32::MIN } else { shifted / denominator }
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

impl Cpu {
    /// CPU half of ZiGBA's SoftReset and HardReset, after the bus-side reset.
    pub fn reset_hle_cpu(&mut self, entry_address: u32) {
        self.switch_banked_mode(0x1F);
        self.registers[..13].fill(0);
        self.user_r8_12.fill(0);
        self.user_r13 = 0x0300_7F00;
        self.user_r14 = entry_address;
        self.irq_r13 = 0x0300_7FA0;
        self.irq_r14 = 0;
        self.spsr_irq = 0;
        self.supervisor_r13 = 0x0300_7FE0;
        self.supervisor_r14 = 0;
        self.spsr_supervisor = 0;
        self.registers[13] = self.user_r13;
        self.registers[14] = self.user_r14;
        self.cpsr = (self.cpsr & !0x3F) | 0x1F;
        self.next_instruction_address = entry_address;
        self.registers[15] = entry_address.wrapping_add(8);
        self.hle_irq_depth = 0;
        self.hle_interrupt_wait_mask = None;
        self.hle_sound_channel_clear = None;
        self.hle_sound_driver_main = None;
        self.hle_service = None;
    }

    /// ZiGBA `executeHleDivision`: returns the bus stall to charge after the register writes.
    pub fn execute_hle_division(&mut self, swap_operands: bool) -> u64 {
        let numerator_bits = self.registers[usize::from(swap_operands)];
        let denominator_bits = self.registers[usize::from(!swap_operands)];
        let numerator = numerator_bits as i32;
        let denominator = denominator_bits as i32;
        let (quotient, remainder) = if denominator == 0 {
            (if numerator < 0 { -1 } else { 1 }, numerator)
        } else if numerator == i32::MIN && denominator == -1 {
            (i32::MIN, 0)
        } else {
            (numerator / denominator, numerator % denominator)
        };
        self.registers[0] = quotient as u32;
        self.registers[1] = remainder as u32;
        self.registers[3] =
            if quotient < 0 { (quotient as u32).wrapping_neg() } else { quotient as u32 };

        let difference =
            denominator_bits.leading_zeros() as i8 - numerator_bits.leading_zeros() as i8;
        11 + 13 * u64::from(difference.max(1) as u8)
    }

    /// ZiGBA `executeHleArcTan`: register writes precede the cycle charge.
    pub fn execute_hle_arc_tan(&mut self) -> u64 {
        let result = hle_arc_tan(self.registers[0] as i32);
        self.registers[0] = i32::from(result.angle) as u32;
        self.registers[1] = result.a as u32;
        self.registers[3] = result.b as u32;
        result.cycles
    }

    /// ZiGBA `executeHleArcTan2`: axes leave r1 intact; r3 becomes 0x170.
    pub fn execute_hle_arc_tan2(&mut self) -> u64 {
        let (angle, result) = hle_arc_tan2(self.registers[0] as i32, self.registers[1] as i32);
        self.registers[0] = u32::from(angle);
        self.registers[3] = 0x170;
        if let Some(result) = result {
            self.registers[1] = result.a as u32;
            result.cycles
        } else {
            11
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference_bg_affine_set_mmio_fault(
        mut source: u32,
        mut destination: u32,
        count: u32,
    ) -> Option<(u32, bool)> {
        for _ in 0..count {
            for (address, write) in [
                (source, false),
                (source.wrapping_add(4), false),
                (destination.wrapping_add(8), true),
                (destination.wrapping_add(12), true),
            ] {
                if address >> 24 == 4 && address & 3 != 0 {
                    return Some((address, write));
                }
            }
            source = source.wrapping_add(20);
            destination = destination.wrapping_add(16);
        }
        None
    }

    /// ZiGBA: "BgAffineSet analytic preflight matches the record-by-record loop".
    #[test]
    fn bg_affine_set_analytic_preflight_matches_the_record_by_record_loop() {
        for (source, destination, count) in [
            (0x0400_0000, 0x0400_0000, 32),
            (0x0400_0001, 0x0400_0002, 0),
            (0x0400_0002, 0x0200_0000, 1),
            (0x04FF_FFFF, 0x0200_0000, 1),
            (0x0500_0001, 0x0200_0000, 1),
            (0x03FF_FFFD, 0x0200_0000, 1),
            (0x03FF_FFE9, 0x0200_0000, 2),
            (0x03FF_FFED, 0x0200_0000, 1),
            (0x03FF_FFED, 0x0200_0000, 2),
            (0x0200_0000, 0x03FF_FFFA, 1),
            (0x0200_0000, 0x03FF_FFF2, 1),
            (0x0200_0000, 0x03FF_FFE6, 3),
            (0x03FF_FFED, 0x03FF_FFE6, 4),
            (0x03FF_FFC5, 0x03FF_FFF2, 4),
            (0xFFFF_FFF1, 0x0200_0000, 32),
            (0x0200_0000, 0xFFFF_FFF2, 32),
        ] {
            assert_eq!(
                bg_affine_set_mmio_fault(source, destination, count),
                reference_bg_affine_set_mmio_fault(source, destination, count)
            );
        }
        assert_eq!(bg_affine_set_mmio_fault(0x0400_0000, 0x0400_0000, u32::MAX), None);
        for (source, destination, index, fault) in [
            (0xFFFF_FFF1, 0x0200_0000, 3_355_444, (0x0400_0001, false)),
            (0x0200_0000, 0xFFFF_FFF2, 4_194_305, (0x0400_000A, true)),
            (0x0500_0002, 0x0200_0000, 213_909_504, (0x0400_0002, false)),
        ] {
            assert_eq!(bg_affine_set_mmio_fault(source, destination, index), None);
            assert_eq!(bg_affine_set_mmio_fault(source, destination, index + 1), Some(fault));
            assert_eq!(bg_affine_set_mmio_fault(source, destination, u32::MAX), Some(fault));
        }
    }

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
            let mut cpu = Cpu::init(0x0800_0000, 0x1F, true);
            cpu.registers[0] = input;
            cpu.registers[2] = 0xEF09_0000;
            let cycles = cpu.execute_hle_arc_tan();
            assert_eq!(
                (cpu.registers[0], cpu.registers[1], cpu.registers[2], cpu.registers[3]),
                (angle, a, 0xEF09_0000, b),
                "input={input:08x}"
            );
            if input == 0 {
                assert_eq!(cycles, 45);
            }
        }
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
            let mut cpu = Cpu::init(0x0800_0000, 0x1F, true);
            cpu.registers[0] = x;
            cpu.registers[1] = y;
            cpu.registers[2] = 0xEF0A_0000;
            let cycles = cpu.execute_hle_arc_tan2();
            assert_eq!(
                (cpu.registers[0], cpu.registers[1], cpu.registers[2], cpu.registers[3]),
                (u32::from(angle), a.unwrap_or(y), 0xEF0A_0000, 0x170),
                "x={x:08x} y={y:08x}"
            );
            if x == 0 || y == 0 {
                assert_eq!(cycles, 11);
            }
        }
    }

    /// ZiGBA machine_test: "HLE Div and DivArm return quotient remainder and absolute quotient including BIOS edges".
    #[test]
    fn hle_div_and_div_arm_return_quotient_remainder_and_absolute_quotient_including_bios_edges() {
        for (swap, r0, r1, quotient, remainder, absolute) in [
            (false, 100_u32, 7_u32, 14_u32, 2_u32, 14_u32),
            (true, 7, 100, 14, 2, 14),
            (false, (-100_i32) as u32, 7, (-14_i32) as u32, (-2_i32) as u32, 14),
            (false, 0x8000_0000, u32::MAX, 0x8000_0000, 0, 0x8000_0000),
            (false, 0, 0, 1, 0, 1),
        ] {
            let mut cpu = Cpu::init(0x0800_0000, 0x1F, true);
            cpu.registers[0] = r0;
            cpu.registers[1] = r1;
            let cycles = cpu.execute_hle_division(swap);
            assert_eq!(
                (cpu.registers[0], cpu.registers[1], cpu.registers[3]),
                (quotient, remainder, absolute)
            );
            assert!(cycles >= 24);
        }
    }

    /// ZiGBA machine_test: "HLE SoftReset branches according to the boot flag and clears the IRQ stack".
    #[test]
    fn hle_soft_reset_restores_cpu_banks_and_clears_continuations() {
        let mut cpu = Cpu::init(0x0800_0000, 0x92, true);
        cpu.registers[0] = 12;
        cpu.registers[4] = 43;
        cpu.registers[14] = 0xDEAD_BEEF;
        cpu.hle_irq_depth = 1;
        cpu.hle_interrupt_wait_mask = Some(4);
        cpu.reset_hle_cpu(0x0200_0000);
        assert_eq!(cpu.registers[..13], [0; 13]);
        assert_eq!(
            (cpu.registers[13], cpu.registers[14], cpu.registers[15]),
            (0x0300_7F00, 0x0200_0000, 0x0200_0008)
        );
        assert_eq!(cpu.cpsr & 0x3F, 0x1F);
        assert_eq!((cpu.irq_r13, cpu.supervisor_r13), (0x0300_7FA0, 0x0300_7FE0));
        assert_eq!(cpu.hle_irq_depth, 0);
        assert_eq!(cpu.hle_interrupt_wait_mask, None);
    }
}
