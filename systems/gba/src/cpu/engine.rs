//! Self-decoding IWRAM ARM slots. The raw pipeline word is the cache key:
//! writes behind an already-fetched pipeline instruction cannot change it.

use evenfall_cpu_arm::Core;

#[derive(Clone, Copy, Default)]
pub(crate) struct ArmOp {
    word: u32,
    kind: u8,
    rn: u8,
    rd: u8,
    rm: u8,
    operand: u32,
}

impl ArmOp {
    pub(crate) fn get(slot: &mut Self, word: u32) -> Self {
        if slot.word != word || slot.kind == 0 {
            *slot = Self::decode(word);
        }
        *slot
    }

    fn decode(word: u32) -> Self {
        let mut op = Self { word, ..Self::default() };
        let opcode = (word >> 21) & 15;
        if (word >> 26) & 3 != 0
            || word & (1 << 20) != 0
            || (8..=11).contains(&opcode)
            || (word >> 12) & 15 == 15
        {
            return op;
        }
        let immediate = word & (1 << 25) != 0;
        if !immediate && word & 0x70 != 0 {
            return op;
        }
        op.kind = opcode as u8 + 1;
        op.rn = ((word >> 16) & 15) as u8;
        op.rd = ((word >> 12) & 15) as u8;
        op.rm = (word & 15) as u8;
        op.operand = if immediate {
            (word & 255).rotate_right(((word >> 8) & 15) * 2)
        } else {
            (word >> 7) & 31
        };
        op
    }

    #[inline]
    pub(crate) fn admitted(self) -> bool {
        self.kind != 0
    }

    #[inline]
    pub(crate) fn execute(self, core: &mut Core, pc8: u32) {
        let rhs = if self.word & (1 << 25) != 0 {
            self.operand
        } else if self.rm == 15 {
            pc8 << self.operand
        } else {
            core.registers[self.rm as usize] << self.operand
        };
        let lhs = if self.rn == 15 { pc8 } else { core.registers[self.rn as usize] };
        let carry = u32::from(core.cpsr & (1 << 29) != 0);
        let result = match self.kind {
            1 => lhs & rhs,
            2 => lhs ^ rhs,
            3 => lhs.wrapping_sub(rhs),
            4 => rhs.wrapping_sub(lhs),
            5 => lhs.wrapping_add(rhs),
            6 => lhs.wrapping_add(rhs).wrapping_add(carry),
            7 => lhs.wrapping_sub(rhs).wrapping_sub(1 - carry),
            8 => rhs.wrapping_sub(lhs).wrapping_sub(1 - carry),
            13 => lhs | rhs,
            14 => rhs,
            15 => lhs & !rhs,
            16 => !rhs,
            _ => unreachable!("decoded ALU operation"),
        };
        core.registers[self.rd as usize] = result;
    }
}
