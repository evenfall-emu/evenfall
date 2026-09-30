//! The C flag of the ARM7TDMI's flag-setting multiplies.
//!
//! ARM calls C "meaningless" after `MULS`, `MLAS` and the long multiplies,
//! but on the ARM7TDMI it is deterministic: the multiplier sums Booth
//! addends in carry-save form, eight multiplier bits per cycle, and the
//! adder that resolves the two carry-save vectors takes one of them through
//! the barrel shifter, whose carry-out becomes C. This module models that
//! datapath only as far as C needs: the carry vector. The product itself is
//! ordinary arithmetic.
//!
//! The model is zayd's reconstruction from the ARM7TDMI's multiplier
//! patents and fuzzing on hardware ("Solving the Mystery of ARM7TDMI
//! Multiply Carry Flag", <https://bmchtech.github.io/post/multiply/>). The
//! mGBA suite's multiply-long table, recorded on hardware, pins it for
//! `SMULLS` and `UMULLS` (`flag_setting_long_multiplies_match_the_mgba_suite_hardware_table`).

/// The multiplier's operating mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MultiplyKind {
    /// `MUL` and `MLA`, which recode their operands as signed.
    Short,
    /// `UMULL` and `UMLAL`.
    LongUnsigned,
    /// `SMULL` and `SMLAL`.
    LongSigned,
}

const LOW_33: u64 = (1 << 33) - 1;
const LOW_34: u64 = (1 << 34) - 1;

/// C after a flag-setting multiply of `multiplicand` (Rm) by `multiplier`
/// (Rs, the operand Booth-recoded and early-terminated, which also sets the
/// cycle count) plus `accumulator` (Rn, or RdHi:RdLo; zero without one).
pub fn multiply_carry(
    kind: MultiplyKind,
    multiplicand: u32,
    multiplier: u32,
    accumulator: u64,
) -> bool {
    let signed = kind != MultiplyKind::LongUnsigned;
    let widen = |value: u32| {
        if signed { (value as i32 as i64 as u64) & LOW_34 } else { u64::from(value) }
    };
    let multiplicand = widen(multiplicand);
    let mut booth = widen(multiplier);
    // Multiplier bit 0 forms a digit of its own before the first cycle: it
    // seeds the carry vector with the inverted multiplicand, whose +1 the
    // final adder's carry-in supplies. The sum vector starts as the
    // accumulator; the accumulator bits it cannot hold enter two per stage.
    let mut sum = accumulator;
    let mut carry = if multiplier & 1 != 0 { !multiplicand } else { 0 };
    let mut accumulator_high = accumulator >> 34;
    // The bits each cycle settles rotate into a 128-bit shift register.
    let mut settled = u128::from(carry & 1).rotate_right(1);
    sum >>= 1;
    carry >>= 1;
    let mut cycles = 0;
    loop {
        let (cycle_sum, cycle_carry) =
            compress(sum, carry, multiplicand, booth, &mut accumulator_high);
        settled = (settled | u128::from(cycle_carry & 0xff)).rotate_right(8);
        sum = cycle_sum >> 8;
        carry = cycle_carry >> 8;
        booth = arithmetic_shift_33(booth, 8);
        cycles += 1;
        // Early termination: the remaining multiplier bits only extend its
        // sign.
        if booth == 0 || (signed && booth == LOW_33) {
            break;
        }
    }
    settled |= u128::from(carry);
    // The adder realigns the vectors through the barrel shifter; C is the
    // bit the rotation takes out last.
    let realigned = settled.rotate_right([23, 15, 7, 31][cycles - 1]);
    let bit = if kind == MultiplyKind::Short && cycles == 4 { 95 } else { 127 };
    realigned >> bit & 1 != 0
}

/// One multiplier cycle: four Booth addends from the low bits of `booth`
/// through four carry-save stages. Each stage settles its two low sum and
/// carry bits; the vectors it passes on are 33 bits wide, with the sign
/// extension of the carry vector and of the addend, and the next
/// accumulator bits, folded into their top.
fn compress(
    mut sum: u64,
    mut carry: u64,
    multiplicand: u64,
    booth: u64,
    accumulator_high: &mut u64,
) -> (u64, u64) {
    let (mut settled_sum, mut settled_carry) = (0, 0);
    for stage in 0..4 {
        sum &= LOW_33;
        carry &= LOW_33;
        let (addend, negated) = booth_addend(multiplicand, (booth >> (2 * stage)) & 7);
        let low = addend & LOW_33;
        let stage_sum = sum ^ low ^ carry;
        let stage_carry = ((sum & low) | (low & carry) | (carry & sum)) << 1 | u64::from(negated);
        settled_sum |= (stage_sum & 3) << (2 * stage);
        settled_carry |= (stage_carry & 3) << (2 * stage);
        let top = (*accumulator_high & 1)
            + u64::from(carry >> 32 & 1 == 0)
            + u64::from(addend >> 33 & 1 == 0);
        sum = stage_sum >> 2 | top << 31;
        carry = stage_carry >> 2 | u64::from(*accumulator_high >> 1 & 1 == 0) << 32;
        *accumulator_high >>= 2;
    }
    (settled_sum | sum << 8, settled_carry | carry << 8)
}

/// Each three-bit Booth window's digit as a nibble: bit 0 set for a nonzero
/// digit, bit 1 for a doubled one, bit 2 for a negative one. Windows 1 and 2
/// are +1, 3 is +2, 4 is −2, 5 and 6 are −1, 0 and 7 are 0.
const BOOTH_DIGITS: u32 = 0x0557_3110;

/// The 34-bit addend for a three-bit Booth window of the multiplier, and
/// whether it is a negation, whose +1 enters as a carry. It selects without
/// branches: the window varies from stage to stage and call to call.
fn booth_addend(multiplicand: u64, window: u64) -> (u64, bool) {
    let digit = BOOTH_DIGITS >> (4 * window) & 0xf;
    let nonzero = 0_u64.wrapping_sub(u64::from(digit & 1));
    let negated = digit >> 2;
    let addend = (multiplicand << (digit >> 1 & 1)) & nonzero;
    ((addend ^ 0_u64.wrapping_sub(u64::from(negated))) & LOW_34, negated != 0)
}

/// `value` as a 33-bit signed number, shifted right arithmetically.
fn arithmetic_shift_33(value: u64, shift: u32) -> u64 {
    ((((value << 31) as i64) >> 31) >> shift) as u64 & LOW_33
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Booth addend by cases: the reference form of [`booth_addend`],
    /// whose branch-free selection must agree with it.
    fn booth_addend_by_cases(multiplicand: u64, window: u64) -> (u64, bool) {
        let (addend, negated) = match window {
            1 | 2 => (multiplicand, false),
            3 => (multiplicand << 1, false),
            4 => (!(multiplicand << 1), true),
            5 | 6 => (!multiplicand, true),
            _ => (0, false),
        };
        (addend & LOW_34, negated)
    }

    #[test]
    fn branch_free_booth_addends_match_the_digit_cases() {
        let multiplicands = [0, 1, 0x1_5555_5555, 0x2_aaaa_aaaa, LOW_33, LOW_34, 0x3_8000_0001];
        for multiplicand in multiplicands {
            for window in 0..8 {
                assert_eq!(
                    booth_addend(multiplicand, window),
                    booth_addend_by_cases(multiplicand, window),
                    "window {window} of {multiplicand:09x}"
                );
            }
        }
    }
}
