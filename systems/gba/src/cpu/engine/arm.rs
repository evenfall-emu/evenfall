//! ARM code running from IWRAM or EWRAM.
//!
//! A RAM code fetch costs the same whether sequential or not, so every
//! instruction costs its fetch plus its own data and internal cycles, and a
//! branch within the region adds two refill fetches.

use super::{CodeCache, Op, PAGE_SHIFT, fresh_ops};
use crate::bus::{AccessWidth, Bus, EWRAM_SIZE, IWRAM_SIZE, RomData, read_ram, write_ram};
use evenfall_cpu_arm::{Core, InstructionSet, Pipeline, condition_passes};

/// Op kinds. Data processing kinds are a form's base plus the opcode;
/// conditional kinds are a base plus the condition.
mod k {
    pub const UNDECODED: u8 = 0;
    /// The sentinel after a region's last slot: continue in the next mirror.
    pub const END: u8 = 1;
    /// The interpreter executes this instruction.
    pub const EXIT: u8 = 2;
    /// A condition that never passes (`cond` 15).
    pub const NOP: u8 = 3;
    /// B and BL: `x` is the target's slot distance from this instruction.
    pub const B: u8 = 4;
    pub const BL: u8 = 5;
    /// MRS rd, CPSR.
    pub const MRS: u8 = 6;
    /// Multiplies; `x` is the instruction.
    pub const MUL: u8 = 7;
    pub const MULL: u8 = 8;
    /// Pre-indexed immediate transfers without writeback; `x` is the signed offset.
    pub const LDR: u8 = 9;
    pub const LDRB: u8 = 10;
    pub const STR: u8 = 11;
    pub const STRB: u8 = 12;
    /// Any other single, halfword or block transfer; `x` is the instruction.
    pub const SINGLE: u8 = 13;
    pub const HALF: u8 = 14;
    pub const BLOCK: u8 = 15;
    /// Data processing that reads r15; `x` is the instruction.
    pub const DP_PC: u8 = 16;
    /// The last slot of the sentinel half: a refused instruction jumps here
    /// to leave the loop.
    pub const STOP: u8 = 17;
    /// Pre-indexed transfers without writeback by register offset (`x` =
    /// rm | type << 8 | amount << 16 | down << 24), then halfword and signed
    /// transfers by immediate (`x` = signed offset) and by register (`x` =
    /// rm | down << 8), in [`super::Mode`] order.
    pub const LDR_R: u8 = 18;
    pub const HALF_I: u8 = 22;
    pub const HALF_R: u8 = 26;
    /// Data processing without S, then with S, by operand form.
    pub const DP_I: u8 = 32;
    pub const DP_R: u8 = 48;
    pub const DP_L: u8 = 64;
    pub const DP_G: u8 = 80;
    pub const DP_X: u8 = 96;
    pub const DPS_I: u8 = 112;
    pub const DPS_IC: u8 = 128;
    pub const DPS_R: u8 = 144;
    pub const DPS_G: u8 = 160;
    pub const DPS_X: u8 = 176;
    /// Conditional B, plus the condition.
    pub const BCOND: u8 = 192;
    /// A condition, then `inner`, plus the condition.
    pub const COND: u8 = 240;
}

/// Operand forms: an immediate (`x`), an immediate whose rotation sets the
/// shifter carry, a register (`x` = rm), LSL by 1..31 (`x` = rm | n << 16),
/// another immediate shift (`x` = rm | type << 8 | amount << 16) and a shift
/// by a register (`x` = rm | type << 8 | rs << 16).
const FI: u8 = 0;
const FIC: u8 = 1;
const FR: u8 = 2;
const FL: u8 = 3;
const FG: u8 = 4;
const FX: u8 = 5;

const END_OP: Op = Op { kind: k::END, inner: 0, rd: 0, rn: 0, x: 0 };
const EXIT_OP: Op = Op { kind: k::EXIT, inner: 0, rd: 0, rn: 0, x: 0 };
const STOP_OP: Op = Op { kind: k::STOP, inner: 0, rd: 0, rn: 0, x: 0 };

fn uses_rn(opcode: u32) -> bool {
    opcode != 13 && opcode != 15
}

fn is_logical(opcode: u32) -> bool {
    matches!(opcode, 0 | 1 | 8 | 9 | 12..=15)
}

/// Decodes one ARM instruction for execution from RAM.
pub(super) fn decode(word: u32) -> Op {
    if word & 0x0f00_0000 == 0x0f00_0000 {
        return EXIT_OP; // SWI, whatever its condition, belongs to the ordinary step.
    }
    let op = decode_unconditional(word);
    match word >> 28 {
        14 => op,
        15 => Op { kind: k::NOP, ..EXIT_OP },
        cond if op.kind == k::B => Op { kind: k::BCOND + cond as u8, ..op },
        cond => Op { kind: k::COND + cond as u8, inner: op.kind, ..op },
    }
}

fn decode_unconditional(word: u32) -> Op {
    let rn = (word >> 16) & 15;
    let rd = (word >> 12) & 15;
    let raw = |kind| Op { kind, inner: 0, rd: rd as u8, rn: rn as u8, x: word };
    match (word >> 25) & 7 {
        0 => {
            if word & 0x0fff_fff0 == 0x012f_ff10 || word & 0x0fb0_fff0 == 0x0120_f000 {
                return EXIT_OP; // BX, MSR
            }
            if word & 0x0fbf_0fff == 0x010f_0000 {
                return if rd == 15 || word & (1 << 22) != 0 { EXIT_OP } else { raw(k::MRS) };
            }
            if word & 0x0fb0_0ff0 == 0x0100_0090 {
                return EXIT_OP; // SWP
            }
            let rs = (word >> 8) & 15;
            let rm = word & 15;
            if word & 0x0f80_00f0 == 0x0080_0090 {
                return if rn == 15 || rd == 15 || rs == 15 || rm == 15 || rn == rd {
                    EXIT_OP
                } else {
                    raw(k::MULL)
                };
            }
            if word & 0x0fc0_00f0 == 0x0000_0090 {
                let accumulate = word & (1 << 21) != 0;
                return if rn == 15 || rs == 15 || rm == 15 || (accumulate && rd == 15) {
                    EXIT_OP
                } else {
                    raw(k::MUL)
                };
            }
            if word & 0x0e00_0090 == 0x0000_0090 && (word >> 5) & 3 != 0 {
                let pre = word & (1 << 24) != 0;
                let writeback = word & (1 << 21) != 0;
                let load = word & (1 << 20) != 0;
                let immediate = word & (1 << 22) != 0;
                if (!load && (word >> 5) & 3 != 1)
                    || rd == 15
                    || (rn == 15 && (!pre || writeback))
                    || (!immediate && rm == 15)
                {
                    return EXIT_OP;
                }
                if !pre || writeback || rn == 15 {
                    return raw(k::HALF);
                }
                let down = word & (1 << 23) == 0;
                let mode = if load { (word >> 5) & 3 } else { 0 };
                return if immediate {
                    let offset = ((word >> 8) & 15) << 4 | (word & 15);
                    let x = if down { offset.wrapping_neg() } else { offset };
                    Op { x, ..raw(k::HALF_I + mode as u8) }
                } else {
                    Op { x: rm | u32::from(down) << 8, ..raw(k::HALF_R + mode as u8) }
                };
            }
            decode_data_processing(word)
        }
        1 if word & 0x0fb0_f000 == 0x0320_f000 => EXIT_OP, // MSR immediate
        1 => decode_data_processing(word),
        2 | 3 if word & 0x0e00_0010 == 0x0600_0010 => EXIT_OP, // Undefined
        2 | 3 => {
            let pre = word & (1 << 24) != 0;
            let byte = word & (1 << 22) != 0;
            let writeback = word & (1 << 21) != 0;
            let load = word & (1 << 20) != 0;
            if (rn == 15 && (!pre || writeback)) || (rd == 15 && (byte || load)) {
                return EXIT_OP;
            }
            if !pre
                || writeback
                || rn == 15
                || rd == 15
                || (word & (1 << 25) != 0 && word & 15 == 15)
            {
                return raw(k::SINGLE);
            }
            let mode = u8::from(!load) << 1 | u8::from(byte);
            let down = word & (1 << 23) == 0;
            if word & (1 << 25) != 0 {
                let x = word & 15 | (word >> 5 & 3) << 8 | (word >> 7 & 31) << 16;
                return Op { x: x | u32::from(down) << 24, ..raw(k::LDR_R + mode) };
            }
            let offset = word & 0xfff;
            let x = if down { offset.wrapping_neg() } else { offset };
            Op { x, ..raw(k::LDR + mode) }
        }
        4 => {
            let list = word & 0xffff;
            let load = word & (1 << 20) != 0;
            if list == 0 || rn == 15 || (load && list & 0x8000 != 0) {
                EXIT_OP
            } else {
                raw(k::BLOCK)
            }
        }
        5 => Op {
            kind: if word & (1 << 24) != 0 { k::BL } else { k::B },
            inner: 0,
            rd: 0,
            rn: 0,
            x: ((((word & 0x00ff_ffff) << 8) as i32 >> 8) + 2) as u32,
        },
        _ => EXIT_OP,
    }
}

fn decode_data_processing(word: u32) -> Op {
    let opcode = (word >> 21) & 15;
    let flags = word & (1 << 20) != 0;
    let rn = (word >> 16) & 15;
    let rd = (word >> 12) & 15;
    if rd == 15 || (!flags && (8..=11).contains(&opcode)) {
        return EXIT_OP;
    }
    let pc_operand = Op { kind: k::DP_PC, inner: 0, rd: rd as u8, rn: rn as u8, x: word };
    let reads_pc = rn == 15 && uses_rn(opcode);
    let (form, x) = if word & (1 << 25) != 0 {
        let rotate = ((word >> 8) & 15) * 2;
        let value = (word & 255).rotate_right(rotate);
        if reads_pc {
            return pc_operand;
        }
        (if flags && is_logical(opcode) && rotate != 0 { FIC } else { FI }, value)
    } else {
        let rm = word & 15;
        let shift = (word >> 5) & 3;
        if word & 0x10 != 0 {
            let rs = (word >> 8) & 15;
            if reads_pc || rm == 15 || rs == 15 {
                return pc_operand;
            }
            (FX, rm | shift << 8 | rs << 16)
        } else {
            let amount = (word >> 7) & 31;
            if reads_pc || rm == 15 {
                return pc_operand;
            }
            if shift == 0 && amount == 0 {
                (FR, rm)
            } else if shift == 0 && !flags {
                (FL, rm | amount << 16)
            } else {
                (FG, rm | shift << 8 | amount << 16)
            }
        }
    };
    let base = match (flags, form) {
        (false, FI) => k::DP_I,
        (false, FR) => k::DP_R,
        (false, FL) => k::DP_L,
        (false, FG) => k::DP_G,
        (false, _) => k::DP_X,
        (true, FI) => k::DPS_I,
        (true, FIC) => k::DPS_IC,
        (true, FR) => k::DPS_R,
        (true, FG) => k::DPS_G,
        (true, _) => k::DPS_X,
    };
    Op { kind: base + opcode as u8, inner: 0, rd: rd as u8, rn: rn as u8, x }
}

/// ARM immediate shifts (`LSR #0` means 32 and so on) with their carry out.
#[inline(always)]
fn shift_immediate(value: u32, shift: u32, amount: u32, carry: u32) -> (u32, Option<bool>) {
    let sign = value >> 31 != 0;
    match (shift & 3, amount & 31) {
        (0, 0) => (value, None),
        (0, n) => (value << n, Some(value.wrapping_shr(32 - n) & 1 != 0)),
        (1, 0) => (0, Some(sign)),
        (1, n) => (value >> n, Some((value >> (n - 1)) & 1 != 0)),
        (2, 0) => (if sign { u32::MAX } else { 0 }, Some(sign)),
        (2, n) => (((value as i32) >> n) as u32, Some((value >> (n - 1)) & 1 != 0)),
        (3, 0) => ((carry << 31) | (value >> 1), Some(value & 1 != 0)),
        (_, n) => {
            let rotated = value.rotate_right(n);
            (rotated, Some(rotated >> 31 != 0))
        }
    }
}

/// The register file of a run: r0..r15, indexed by any byte so that decoded
/// register fields need neither masks nor bounds checks.
type Registers = [u32; 256];

/// Memory, caches and rarely changing state of a run of ARM code from IWRAM
/// (`EW` false) or EWRAM (`EW` true).
struct Ctx<'a, const EW: bool> {
    /// The other RAM region's ARM ops, if any were decoded.
    other_ops: Option<&'a [Op]>,
    iwram: &'a mut [u8; IWRAM_SIZE],
    ewram: &'a mut [u8; EWRAM_SIZE],
    iwram_code: &'a mut [bool; IWRAM_SIZE >> PAGE_SHIFT],
    ewram_code: &'a mut [bool; EWRAM_SIZE >> PAGE_SHIFT],
    rom: RomData<'a>,
    /// Banked registers, for user-bank transfers.
    core: &'a mut Core,
    /// The address of `ops[0]` in the current mirror.
    base: u32,
    /// The budget left after the last load and after the last store.
    load_mark: i64,
    store_mark: i64,
    /// The last load's bus value, the open-bus latch if it ends the run.
    data: u32,
}

/// The state every instruction changes, kept small so that it stays in host
/// registers across the dispatch loop.
#[derive(Clone, Copy)]
struct Hot {
    /// Cycles left before the run's limit.
    budget: i64,
    idx: u32,
    cpsr: u32,
}

impl Hot {
    /// What an out-of-line handler returns for a refused instruction: the
    /// state fits two host registers, so no `Option` wraps it.
    const REFUSED: Self = Self { budget: 0, idx: u32::MAX, cpsr: 0 };
}

impl<const EW: bool> Ctx<'_, EW> {
    const REGION: u32 = if EW { 2 } else { 3 };
    const MASK: u32 = (if EW { EWRAM_SIZE } else { IWRAM_SIZE }) as u32 - 1;
    const SLOTS: usize = (Self::MASK as usize + 1) / 4;
    /// The base of the region's last mirror, where no instruction may run:
    /// its last instructions would fetch beyond the region.
    const LAST: u32 = Self::REGION << 24 | (0x00ff_ffff & !Self::MASK);
    const FETCH: i64 = if EW { 6 } else { 1 };

    fn code_word(&self, address: u32) -> u32 {
        let index = (address & Self::MASK) as usize;
        if EW {
            read_ram(self.ewram, index, AccessWidth::Word)
        } else {
            read_ram(self.iwram, index, AccessWidth::Word)
        }
    }

    #[inline(always)]
    fn pc(&self, idx: u32) -> u32 {
        self.base.wrapping_add(idx << 2)
    }
}

/// Runs ARM code from RAM at `core.next_instruction_address`; see
/// [`crate::cpu::Cpu::run_engine`].
pub(super) fn run<const EW: bool>(core: &mut Core, bus: &mut Bus, limit: u64) -> bool {
    let pc = core.next_instruction_address;
    let mask = Ctx::<EW>::MASK;
    if pc & 3 != 0 || pc & !mask == Ctx::<EW>::LAST || !bus.game_pak_code_idle() {
        return false;
    }
    let start = bus.now();
    let memory = bus.engine_memory();
    let CodeCache { iwram_arm, ewram_arm, iwram_code, ewram_code } = memory.code;
    let (ops, other_ops): (&mut [Op], Option<&[Op]>) = if EW {
        (
            &mut ewram_arm.get_or_insert_with(|| fresh_ops(END_OP, STOP_OP))[..],
            iwram_arm.as_deref().map(|o| &o[..]),
        )
    } else {
        (
            &mut iwram_arm.get_or_insert_with(|| fresh_ops(END_OP, STOP_OP))[..],
            ewram_arm.as_deref().map(|o| &o[..]),
        )
    };
    let pipeline = core.pipeline;
    let cpsr = core.cpsr;
    let mut c = Ctx::<EW> {
        other_ops,
        iwram: memory.iwram,
        ewram: memory.ewram,
        iwram_code,
        ewram_code,
        rom: memory.rom,
        core,
        base: pc & !mask,
        load_mark: -1,
        store_mark: -1,
        data: 0,
    };
    let latched = [c.code_word(pc), c.code_word(pc.wrapping_add(4))];
    if pipeline != Some(Pipeline { instructions: latched, instruction_set: InstructionSet::Arm }) {
        return false;
    }
    let budget = (limit - start) as i64;
    let hot = Hot { idx: (pc & mask) >> 2, budget, cpsr };
    let hot = if EW {
        let ops = ops.try_into().expect("EWRAM op array");
        run_loop::<EW, { 2 * EWRAM_SIZE / 4 }>(ops, &mut c, hot)
    } else {
        let ops = ops.try_into().expect("IWRAM op array");
        run_loop::<EW, { 2 * IWRAM_SIZE / 4 }>(ops, &mut c, hot)
    };
    if hot.budget == budget {
        return false;
    }
    let pc = c.pc(hot.idx);
    let pipeline = [c.code_word(pc), c.code_word(pc.wrapping_add(4))];
    let loaded = c.load_mark == hot.budget;
    let data_last = loaded || c.store_mark == hot.budget;
    let open_bus = if loaded { c.data } else { pipeline[1] };
    let core = c.core;
    core.registers[15] = pc.wrapping_add(8);
    core.cpsr = hot.cpsr;
    core.next_instruction_address = pc;
    core.pipeline = Some(Pipeline { instructions: pipeline, instruction_set: InstructionSet::Arm });
    bus.engine_finish(limit - hot.budget as u64, open_bus, data_last);
    true
}

/// Runs an inline handler; a refused instruction ends the run.
macro_rules! go {
    ($h:ident, $c:ident, $run:expr) => {
        if !$run {
            return stopped($h);
        }
    };
}

/// Runs an out-of-line handler, which takes and returns the hot state.
macro_rules! call {
    ($h:ident, $c:ident, $run:expr) => {{
        let next = $run;
        if next.idx == u32::MAX {
            return stopped($h);
        }
        $h = next;
    }};
}

/// A conditional wrapper: run the instruction through the generic handlers
/// if condition `$cond` passes, otherwise charge the fetch and move on.
macro_rules! cond {
    ($h:ident, $c:ident, $ops:ident, $regs:ident, $op:ident, $cond:literal) => {{
        // Opaque flags keep the compiler from hoisting condition checks
        // into every instruction's dispatch.
        if condition_passes(std::hint::black_box($h.cpsr), $cond) {
            call!($h, $c, conditional($h, $c, $ops, $regs, $op))
        } else {
            go!($h, $c, $h.skip::<EW>())
        }
    }};
}

/// A conditional B.
macro_rules! bcond {
    ($h:ident, $c:ident, $regs:ident, $op:ident, $cond:literal) => {{
        if condition_passes(std::hint::black_box($h.cpsr), $cond) {
            go!($h, $c, branch(&mut $h, $c, $regs, $op, false))
        } else {
            go!($h, $c, $h.skip::<EW>())
        }
    }};
}

/// Transfer modes of the fast single and halfword kinds.
const LDR: u8 = 0;
const LDRB: u8 = 1;
const STR: u8 = 2;
const STRB: u8 = 3;
const STRH: u8 = 4;
const LDRH: u8 = 5;
const LDRSB: u8 = 6;
const LDRSH: u8 = 7;

/// Runs ops from `state` until one is refused; returns the state before it.
/// The register file lives in this frame, so that register operands are
/// addressed from the stack pointer rather than from a host register.
#[inline(never)]
fn run_loop<const EW: bool, const N: usize>(
    ops: &mut [Op; N],
    c: &mut Ctx<'_, EW>,
    state: Hot,
) -> Hot {
    let mut regs: Registers = [0; 256];
    regs[..16].copy_from_slice(&c.core.registers);
    let h = run_ops(ops, c, &mut regs, state);
    c.core.registers.copy_from_slice(&regs[..16]);
    h
}

#[inline(always)]
fn run_ops<const EW: bool, const N: usize>(
    ops: &mut [Op; N],
    c: &mut Ctx<'_, EW>,
    regs: &mut Registers,
    state: Hot,
) -> Hot {
    let mut h = state;
    loop {
        // The index never passes the first sentinel; the mask only lets
        // the compiler drop the bounds check.
        let op = &ops[h.idx as usize & (N - 1)];
        #[cfg(debug_assertions)]
        if !matches!(op.kind, k::UNDECODED | k::END | k::STOP) {
            let pc = c.pc(h.idx);
            debug_assert_eq!(*op, decode(c.code_word(pc)), "stale op at {pc:08x}");
        }
        match op.kind {
            k::UNDECODED => decode_slot(&mut ops[..], c, h.idx),
            k::END => go!(h, c, end(&mut h, c)),
            k::STOP => return stopped(h),
            k::NOP => go!(h, c, h.skip::<EW>()),
            k::B => go!(h, c, branch(&mut h, c, regs, op, false)),
            k::BL => go!(h, c, branch(&mut h, c, regs, op, true)),
            k::MRS => go!(h, c, mrs::<EW>(&mut h, regs, op)),
            k::MUL => go!(h, c, multiply::<EW>(&mut h, regs, op.x)),
            k::MULL => call!(h, c, multiply_long::<EW>(h, regs, op.x)),
            k::LDR => {
                go!(h, c, transfer::<EW, LDR>(&mut h, c, ops, regs, op, immediate(regs, op)))
            }
            k::LDRB => {
                go!(h, c, transfer::<EW, LDRB>(&mut h, c, ops, regs, op, immediate(regs, op)))
            }
            k::STR => {
                go!(h, c, transfer::<EW, STR>(&mut h, c, ops, regs, op, immediate(regs, op)))
            }
            k::STRB => {
                go!(h, c, transfer::<EW, STRB>(&mut h, c, ops, regs, op, immediate(regs, op)))
            }
            k::SINGLE => call!(h, c, single(h, c, ops, regs, op.x)),
            k::HALF => call!(h, c, half(h, c, ops, regs, op.x)),
            k::BLOCK => call!(h, c, block(h, c, ops, regs, op.x)),
            k::DP_PC => call!(h, c, dp_pc(h, c, regs, op.x)),
            18 => {
                let address = shifted(h.cpsr, regs, op);
                go!(h, c, transfer::<EW, LDR>(&mut h, c, ops, regs, op, address))
            }
            19 => {
                let address = shifted(h.cpsr, regs, op);
                go!(h, c, transfer::<EW, LDRB>(&mut h, c, ops, regs, op, address))
            }
            20 => {
                let address = shifted(h.cpsr, regs, op);
                go!(h, c, transfer::<EW, STR>(&mut h, c, ops, regs, op, address))
            }
            21 => {
                let address = shifted(h.cpsr, regs, op);
                go!(h, c, transfer::<EW, STRB>(&mut h, c, ops, regs, op, address))
            }
            22 => {
                go!(h, c, transfer::<EW, STRH>(&mut h, c, ops, regs, op, immediate(regs, op)))
            }
            23 => {
                go!(h, c, transfer::<EW, LDRH>(&mut h, c, ops, regs, op, immediate(regs, op)))
            }
            24 => {
                go!(h, c, transfer::<EW, LDRSB>(&mut h, c, ops, regs, op, immediate(regs, op)))
            }
            25 => {
                go!(h, c, transfer::<EW, LDRSH>(&mut h, c, ops, regs, op, immediate(regs, op)))
            }
            26 => go!(h, c, transfer::<EW, STRH>(&mut h, c, ops, regs, op, indexed(regs, op))),
            27 => go!(h, c, transfer::<EW, LDRH>(&mut h, c, ops, regs, op, indexed(regs, op))),
            28 => go!(h, c, transfer::<EW, LDRSB>(&mut h, c, ops, regs, op, indexed(regs, op))),
            29 => go!(h, c, transfer::<EW, LDRSH>(&mut h, c, ops, regs, op, indexed(regs, op))),
            32 => go!(h, c, dp::<EW, 0, false, FI>(&mut h, regs, op)),
            33 => go!(h, c, dp::<EW, 1, false, FI>(&mut h, regs, op)),
            34 => go!(h, c, dp::<EW, 2, false, FI>(&mut h, regs, op)),
            35 => go!(h, c, dp::<EW, 3, false, FI>(&mut h, regs, op)),
            36 => go!(h, c, dp::<EW, 4, false, FI>(&mut h, regs, op)),
            37 => go!(h, c, dp::<EW, 5, false, FI>(&mut h, regs, op)),
            38 => go!(h, c, dp::<EW, 6, false, FI>(&mut h, regs, op)),
            39 => go!(h, c, dp::<EW, 7, false, FI>(&mut h, regs, op)),
            44 => go!(h, c, dp::<EW, 12, false, FI>(&mut h, regs, op)),
            45 => go!(h, c, dp::<EW, 13, false, FI>(&mut h, regs, op)),
            46 => go!(h, c, dp::<EW, 14, false, FI>(&mut h, regs, op)),
            47 => go!(h, c, dp::<EW, 15, false, FI>(&mut h, regs, op)),
            48 => go!(h, c, dp::<EW, 0, false, FR>(&mut h, regs, op)),
            49 => go!(h, c, dp::<EW, 1, false, FR>(&mut h, regs, op)),
            50 => go!(h, c, dp::<EW, 2, false, FR>(&mut h, regs, op)),
            51 => go!(h, c, dp::<EW, 3, false, FR>(&mut h, regs, op)),
            52 => go!(h, c, dp::<EW, 4, false, FR>(&mut h, regs, op)),
            53 => go!(h, c, dp::<EW, 5, false, FR>(&mut h, regs, op)),
            54 => go!(h, c, dp::<EW, 6, false, FR>(&mut h, regs, op)),
            55 => go!(h, c, dp::<EW, 7, false, FR>(&mut h, regs, op)),
            60 => go!(h, c, dp::<EW, 12, false, FR>(&mut h, regs, op)),
            61 => go!(h, c, dp::<EW, 13, false, FR>(&mut h, regs, op)),
            62 => go!(h, c, dp::<EW, 14, false, FR>(&mut h, regs, op)),
            63 => go!(h, c, dp::<EW, 15, false, FR>(&mut h, regs, op)),
            64 => go!(h, c, dp::<EW, 0, false, FL>(&mut h, regs, op)),
            65 => go!(h, c, dp::<EW, 1, false, FL>(&mut h, regs, op)),
            66 => go!(h, c, dp::<EW, 2, false, FL>(&mut h, regs, op)),
            67 => go!(h, c, dp::<EW, 3, false, FL>(&mut h, regs, op)),
            68 => go!(h, c, dp::<EW, 4, false, FL>(&mut h, regs, op)),
            69 => go!(h, c, dp::<EW, 5, false, FL>(&mut h, regs, op)),
            70 => go!(h, c, dp::<EW, 6, false, FL>(&mut h, regs, op)),
            71 => go!(h, c, dp::<EW, 7, false, FL>(&mut h, regs, op)),
            76 => go!(h, c, dp::<EW, 12, false, FL>(&mut h, regs, op)),
            77 => go!(h, c, dp::<EW, 13, false, FL>(&mut h, regs, op)),
            78 => go!(h, c, dp::<EW, 14, false, FL>(&mut h, regs, op)),
            79 => go!(h, c, dp::<EW, 15, false, FL>(&mut h, regs, op)),
            80 => go!(h, c, dp::<EW, 0, false, FG>(&mut h, regs, op)),
            81 => go!(h, c, dp::<EW, 1, false, FG>(&mut h, regs, op)),
            82 => go!(h, c, dp::<EW, 2, false, FG>(&mut h, regs, op)),
            83 => go!(h, c, dp::<EW, 3, false, FG>(&mut h, regs, op)),
            84 => go!(h, c, dp::<EW, 4, false, FG>(&mut h, regs, op)),
            85 => go!(h, c, dp::<EW, 5, false, FG>(&mut h, regs, op)),
            86 => go!(h, c, dp::<EW, 6, false, FG>(&mut h, regs, op)),
            87 => go!(h, c, dp::<EW, 7, false, FG>(&mut h, regs, op)),
            92 => go!(h, c, dp::<EW, 12, false, FG>(&mut h, regs, op)),
            93 => go!(h, c, dp::<EW, 13, false, FG>(&mut h, regs, op)),
            94 => go!(h, c, dp::<EW, 14, false, FG>(&mut h, regs, op)),
            95 => go!(h, c, dp::<EW, 15, false, FG>(&mut h, regs, op)),
            96 => go!(h, c, dp::<EW, 0, false, FX>(&mut h, regs, op)),
            97 => go!(h, c, dp::<EW, 1, false, FX>(&mut h, regs, op)),
            98 => go!(h, c, dp::<EW, 2, false, FX>(&mut h, regs, op)),
            99 => go!(h, c, dp::<EW, 3, false, FX>(&mut h, regs, op)),
            100 => go!(h, c, dp::<EW, 4, false, FX>(&mut h, regs, op)),
            101 => go!(h, c, dp::<EW, 5, false, FX>(&mut h, regs, op)),
            102 => go!(h, c, dp::<EW, 6, false, FX>(&mut h, regs, op)),
            103 => go!(h, c, dp::<EW, 7, false, FX>(&mut h, regs, op)),
            108 => go!(h, c, dp::<EW, 12, false, FX>(&mut h, regs, op)),
            109 => go!(h, c, dp::<EW, 13, false, FX>(&mut h, regs, op)),
            110 => go!(h, c, dp::<EW, 14, false, FX>(&mut h, regs, op)),
            111 => go!(h, c, dp::<EW, 15, false, FX>(&mut h, regs, op)),
            112 => go!(h, c, dp::<EW, 0, true, FI>(&mut h, regs, op)),
            113 => go!(h, c, dp::<EW, 1, true, FI>(&mut h, regs, op)),
            114 => go!(h, c, dp::<EW, 2, true, FI>(&mut h, regs, op)),
            115 => go!(h, c, dp::<EW, 3, true, FI>(&mut h, regs, op)),
            116 => go!(h, c, dp::<EW, 4, true, FI>(&mut h, regs, op)),
            117 => go!(h, c, dp::<EW, 5, true, FI>(&mut h, regs, op)),
            118 => go!(h, c, dp::<EW, 6, true, FI>(&mut h, regs, op)),
            119 => go!(h, c, dp::<EW, 7, true, FI>(&mut h, regs, op)),
            120 => go!(h, c, dp::<EW, 8, true, FI>(&mut h, regs, op)),
            121 => go!(h, c, dp::<EW, 9, true, FI>(&mut h, regs, op)),
            122 => go!(h, c, dp::<EW, 10, true, FI>(&mut h, regs, op)),
            123 => go!(h, c, dp::<EW, 11, true, FI>(&mut h, regs, op)),
            124 => go!(h, c, dp::<EW, 12, true, FI>(&mut h, regs, op)),
            125 => go!(h, c, dp::<EW, 13, true, FI>(&mut h, regs, op)),
            126 => go!(h, c, dp::<EW, 14, true, FI>(&mut h, regs, op)),
            127 => go!(h, c, dp::<EW, 15, true, FI>(&mut h, regs, op)),
            128 => go!(h, c, dp::<EW, 0, true, FIC>(&mut h, regs, op)),
            129 => go!(h, c, dp::<EW, 1, true, FIC>(&mut h, regs, op)),
            136 => go!(h, c, dp::<EW, 8, true, FIC>(&mut h, regs, op)),
            137 => go!(h, c, dp::<EW, 9, true, FIC>(&mut h, regs, op)),
            140 => go!(h, c, dp::<EW, 12, true, FIC>(&mut h, regs, op)),
            141 => go!(h, c, dp::<EW, 13, true, FIC>(&mut h, regs, op)),
            142 => go!(h, c, dp::<EW, 14, true, FIC>(&mut h, regs, op)),
            143 => go!(h, c, dp::<EW, 15, true, FIC>(&mut h, regs, op)),
            144 => go!(h, c, dp::<EW, 0, true, FR>(&mut h, regs, op)),
            145 => go!(h, c, dp::<EW, 1, true, FR>(&mut h, regs, op)),
            146 => go!(h, c, dp::<EW, 2, true, FR>(&mut h, regs, op)),
            147 => go!(h, c, dp::<EW, 3, true, FR>(&mut h, regs, op)),
            148 => go!(h, c, dp::<EW, 4, true, FR>(&mut h, regs, op)),
            149 => go!(h, c, dp::<EW, 5, true, FR>(&mut h, regs, op)),
            150 => go!(h, c, dp::<EW, 6, true, FR>(&mut h, regs, op)),
            151 => go!(h, c, dp::<EW, 7, true, FR>(&mut h, regs, op)),
            152 => go!(h, c, dp::<EW, 8, true, FR>(&mut h, regs, op)),
            153 => go!(h, c, dp::<EW, 9, true, FR>(&mut h, regs, op)),
            154 => go!(h, c, dp::<EW, 10, true, FR>(&mut h, regs, op)),
            155 => go!(h, c, dp::<EW, 11, true, FR>(&mut h, regs, op)),
            156 => go!(h, c, dp::<EW, 12, true, FR>(&mut h, regs, op)),
            157 => go!(h, c, dp::<EW, 13, true, FR>(&mut h, regs, op)),
            158 => go!(h, c, dp::<EW, 14, true, FR>(&mut h, regs, op)),
            159 => go!(h, c, dp::<EW, 15, true, FR>(&mut h, regs, op)),
            160 => go!(h, c, dp::<EW, 0, true, FG>(&mut h, regs, op)),
            161 => go!(h, c, dp::<EW, 1, true, FG>(&mut h, regs, op)),
            162 => go!(h, c, dp::<EW, 2, true, FG>(&mut h, regs, op)),
            163 => go!(h, c, dp::<EW, 3, true, FG>(&mut h, regs, op)),
            164 => go!(h, c, dp::<EW, 4, true, FG>(&mut h, regs, op)),
            165 => go!(h, c, dp::<EW, 5, true, FG>(&mut h, regs, op)),
            166 => go!(h, c, dp::<EW, 6, true, FG>(&mut h, regs, op)),
            167 => go!(h, c, dp::<EW, 7, true, FG>(&mut h, regs, op)),
            168 => go!(h, c, dp::<EW, 8, true, FG>(&mut h, regs, op)),
            169 => go!(h, c, dp::<EW, 9, true, FG>(&mut h, regs, op)),
            170 => go!(h, c, dp::<EW, 10, true, FG>(&mut h, regs, op)),
            171 => go!(h, c, dp::<EW, 11, true, FG>(&mut h, regs, op)),
            172 => go!(h, c, dp::<EW, 12, true, FG>(&mut h, regs, op)),
            173 => go!(h, c, dp::<EW, 13, true, FG>(&mut h, regs, op)),
            174 => go!(h, c, dp::<EW, 14, true, FG>(&mut h, regs, op)),
            175 => go!(h, c, dp::<EW, 15, true, FG>(&mut h, regs, op)),
            176 => go!(h, c, dp::<EW, 0, true, FX>(&mut h, regs, op)),
            177 => go!(h, c, dp::<EW, 1, true, FX>(&mut h, regs, op)),
            178 => go!(h, c, dp::<EW, 2, true, FX>(&mut h, regs, op)),
            179 => go!(h, c, dp::<EW, 3, true, FX>(&mut h, regs, op)),
            180 => go!(h, c, dp::<EW, 4, true, FX>(&mut h, regs, op)),
            181 => go!(h, c, dp::<EW, 5, true, FX>(&mut h, regs, op)),
            182 => go!(h, c, dp::<EW, 6, true, FX>(&mut h, regs, op)),
            183 => go!(h, c, dp::<EW, 7, true, FX>(&mut h, regs, op)),
            184 => go!(h, c, dp::<EW, 8, true, FX>(&mut h, regs, op)),
            185 => go!(h, c, dp::<EW, 9, true, FX>(&mut h, regs, op)),
            186 => go!(h, c, dp::<EW, 10, true, FX>(&mut h, regs, op)),
            187 => go!(h, c, dp::<EW, 11, true, FX>(&mut h, regs, op)),
            188 => go!(h, c, dp::<EW, 12, true, FX>(&mut h, regs, op)),
            189 => go!(h, c, dp::<EW, 13, true, FX>(&mut h, regs, op)),
            190 => go!(h, c, dp::<EW, 14, true, FX>(&mut h, regs, op)),
            191 => go!(h, c, dp::<EW, 15, true, FX>(&mut h, regs, op)),
            192 => bcond!(h, c, regs, op, 0),
            193 => bcond!(h, c, regs, op, 1),
            194 => bcond!(h, c, regs, op, 2),
            195 => bcond!(h, c, regs, op, 3),
            196 => bcond!(h, c, regs, op, 4),
            197 => bcond!(h, c, regs, op, 5),
            198 => bcond!(h, c, regs, op, 6),
            199 => bcond!(h, c, regs, op, 7),
            200 => bcond!(h, c, regs, op, 8),
            201 => bcond!(h, c, regs, op, 9),
            202 => bcond!(h, c, regs, op, 10),
            203 => bcond!(h, c, regs, op, 11),
            204 => bcond!(h, c, regs, op, 12),
            205 => bcond!(h, c, regs, op, 13),
            206 => bcond!(h, c, regs, op, 14),
            240 => cond!(h, c, ops, regs, op, 0),
            241 => cond!(h, c, ops, regs, op, 1),
            242 => cond!(h, c, ops, regs, op, 2),
            243 => cond!(h, c, ops, regs, op, 3),
            244 => cond!(h, c, ops, regs, op, 4),
            245 => cond!(h, c, ops, regs, op, 5),
            246 => cond!(h, c, ops, regs, op, 6),
            247 => cond!(h, c, ops, regs, op, 7),
            248 => cond!(h, c, ops, regs, op, 8),
            249 => cond!(h, c, ops, regs, op, 9),
            250 => cond!(h, c, ops, regs, op, 10),
            251 => cond!(h, c, ops, regs, op, 11),
            252 => cond!(h, c, ops, regs, op, 12),
            253 => cond!(h, c, ops, regs, op, 13),
            254 => cond!(h, c, ops, regs, op, 14),
            k::EXIT
            | 30
            | 31
            | 40..=43
            | 56..=59
            | 72..=75
            | 88..=91
            | 104..=107
            | 130..=135
            | 138
            | 139
            | 207..=239
            | 255 => return stopped(h),
        }
    }
}

#[cold]
#[inline(never)]
fn decode_slot<const EW: bool>(ops: &mut [Op], c: &mut Ctx<'_, EW>, idx: u32) {
    let offset = idx << 2;
    let op = decode(c.code_word(offset));
    debug_assert_ne!(op.kind, k::UNDECODED);
    ops[idx as usize] = op;
    // The page of the instruction and of its prefetch window hold code.
    for offset in [offset, (offset + 8) & Ctx::<EW>::MASK] {
        let page = (offset >> PAGE_SHIFT) as usize;
        if EW {
            c.ewram_code[page] = true;
        } else {
            c.iwram_code[page] = true;
        }
    }
}

impl Hot {
    /// Charges `cycles` if they fit in the run's budget.
    #[inline(always)]
    fn charge(&mut self, cycles: i64) -> bool {
        self.budget -= cycles;
        if self.budget < 0 {
            self.budget = refund(self.budget, cycles);
            return false;
        }
        true
    }

    #[inline(always)]
    fn next(&mut self) -> bool {
        self.idx += 1;
        true
    }

    /// An instruction whose condition failed: only its fetch.
    #[inline(always)]
    fn skip<const EW: bool>(&mut self) -> bool {
        self.charge(Ctx::<EW>::FETCH) && self.next()
    }
}

/// Past the last slot: the same ops continue in the next mirror.
#[inline(always)]
fn end<const EW: bool>(h: &mut Hot, c: &mut Ctx<'_, EW>) -> bool {
    let next = c.base.wrapping_add(Ctx::<EW>::MASK + 1);
    if next == Ctx::<EW>::LAST {
        return false;
    }
    c.base = next;
    h.idx = 0;
    true
}

#[inline(always)]
fn branch<const EW: bool>(
    h: &mut Hot,
    c: &mut Ctx<'_, EW>,
    regs: &mut Registers,
    op: &Op,
    link: bool,
) -> bool {
    let target = h.idx.wrapping_add(op.x);
    if target as usize >= Ctx::<EW>::SLOTS {
        let next = far_branch(*h, c, regs, op.x, link);
        if next.idx == u32::MAX {
            return false;
        }
        *h = next;
        return true;
    }
    if !h.charge(3 * Ctx::<EW>::FETCH) {
        return false;
    }
    if link {
        regs[14] = c.pc(h.idx).wrapping_add(4);
    }
    h.idx = target;
    true
}

/// A branch into another mirror of the region, if it stays in the region.
#[cold]
#[inline(never)]
fn far_branch<const EW: bool>(
    mut h: Hot,
    c: &mut Ctx<'_, EW>,
    regs: &mut Registers,
    slots: u32,
    link: bool,
) -> Hot {
    let pc = c.pc(h.idx);
    let target = pc.wrapping_add(slots << 2);
    if target >> 24 != Ctx::<EW>::REGION
        || target & !Ctx::<EW>::MASK == Ctx::<EW>::LAST
        || !h.charge(3 * Ctx::<EW>::FETCH)
    {
        return Hot::REFUSED;
    }
    if link {
        regs[14] = pc.wrapping_add(4);
    }
    c.base = target & !Ctx::<EW>::MASK;
    h.idx = (target & Ctx::<EW>::MASK) >> 2;
    h
}

#[inline(always)]
fn mrs<const EW: bool>(h: &mut Hot, regs: &mut Registers, op: &Op) -> bool {
    if !h.charge(Ctx::<EW>::FETCH) {
        return false;
    }
    regs[usize::from(op.rd)] = h.cpsr;
    h.next()
}

#[inline(always)]
fn dp<const EW: bool, const OP: u32, const S: bool, const FORM: u8>(
    h: &mut Hot,
    regs: &mut Registers,
    op: &Op,
) -> bool {
    if !h.charge(Ctx::<EW>::FETCH + i64::from(FORM == FX)) {
        return false;
    }
    let carry_in = (h.cpsr >> 29) & 1;
    let rm = regs[op.x as u8 as usize];
    let (rhs, carry) = match FORM {
        FI => (op.x, None),
        FIC => (op.x, Some(op.x >> 31 != 0)),
        FR => (rm, None),
        FL => {
            let amount = (op.x >> 16) & 31;
            (rm << amount, Some(rm.wrapping_shr(32 - amount) & 1 != 0))
        }
        FG => shift_immediate(rm, op.x >> 8, op.x >> 16, carry_in),
        _ => {
            let amount = regs[(op.x >> 16) as u8 as usize] as u8;
            let shifted = Core::decode_register_shift(rm, (op.x >> 8) & 3, amount);
            (shifted.value, shifted.carry)
        }
    };
    let lhs = regs[usize::from(op.rn)];
    alu(h, regs, OP, S, op.rd, lhs, rhs, carry, carry_in);
    h.next()
}

/// Data processing with r15 among its operands, which reads as the
/// instruction address plus 8 (plus 12 when shifted by a register).
#[inline(always)]
fn dp_pc_op<const EW: bool>(
    mut h: Hot,
    c: &mut Ctx<'_, EW>,
    regs: &mut Registers,
    word: u32,
) -> Option<Hot> {
    let register_shift = word & (1 << 25) == 0 && word & 0x10 != 0;
    if !h.charge(Ctx::<EW>::FETCH + i64::from(register_shift)) {
        return None;
    }
    regs[15] = c.pc(h.idx).wrapping_add(8);
    let carry_in = (h.cpsr >> 29) & 1;
    let (rhs, carry) = if word & (1 << 25) != 0 {
        let rotate = ((word >> 8) & 15) * 2;
        let value = (word & 255).rotate_right(rotate);
        (value, (rotate != 0).then_some(value >> 31 != 0))
    } else if register_shift {
        let rm = (word & 15) as usize;
        let source = regs[rm].wrapping_add(if rm == 15 { 4 } else { 0 });
        let amount = regs[((word >> 8) & 15) as usize] as u8;
        let shifted = Core::decode_register_shift(source, (word >> 5) & 3, amount);
        (shifted.value, shifted.carry)
    } else {
        shift_immediate(regs[(word & 15) as usize], word >> 5, word >> 7, carry_in)
    };
    let rn = ((word >> 16) & 15) as usize;
    let lhs = regs[rn].wrapping_add(if rn == 15 && register_shift { 4 } else { 0 });
    let flags = word & (1 << 20) != 0;
    let rd = ((word >> 12) & 15) as u8;
    alu(&mut h, regs, (word >> 21) & 15, flags, rd, lhs, rhs, carry, carry_in);
    h.next();
    Some(h)
}

#[inline(always)]
fn alu(
    h: &mut Hot,
    regs: &mut Registers,
    opcode: u32,
    flags: bool,
    rd: u8,
    lhs: u32,
    rhs: u32,
    shifter_carry: Option<bool>,
    carry_in: u32,
) {
    let result = match opcode {
        0 | 8 => lhs & rhs,
        1 | 9 => lhs ^ rhs,
        2 | 10 => lhs.wrapping_sub(rhs),
        3 => rhs.wrapping_sub(lhs),
        4 | 11 => lhs.wrapping_add(rhs),
        5 => lhs.wrapping_add(rhs).wrapping_add(carry_in),
        6 => lhs.wrapping_sub(rhs).wrapping_sub(1 - carry_in),
        7 => rhs.wrapping_sub(lhs).wrapping_sub(1 - carry_in),
        12 => lhs | rhs,
        13 => rhs,
        14 => lhs & !rhs,
        _ => !rhs,
    };
    if !(8..=11).contains(&opcode) {
        regs[usize::from(rd)] = result;
    }
    if !flags {
        return;
    }
    let nz = (result & 0x8000_0000) | (u32::from(result == 0) << 30);
    if is_logical(opcode) {
        h.cpsr = (h.cpsr & 0x3fff_ffff) | nz;
        if let Some(carry) = shifter_carry {
            h.cpsr = (h.cpsr & !(1 << 29)) | (u32::from(carry) << 29);
        }
        return;
    }
    let (carry, overflow) = match opcode {
        2 | 10 => (lhs >= rhs, (lhs ^ rhs) & (lhs ^ result)),
        3 => (rhs >= lhs, (rhs ^ lhs) & (rhs ^ result)),
        4 | 11 => (result < lhs, !(lhs ^ rhs) & (lhs ^ result)),
        5 => (
            u64::from(lhs) + u64::from(rhs) + u64::from(carry_in) > u64::from(u32::MAX),
            !(lhs ^ rhs) & (lhs ^ result),
        ),
        6 => (
            u64::from(lhs) >= u64::from(rhs) + u64::from(1 - carry_in),
            (lhs ^ rhs) & (lhs ^ result),
        ),
        _ => (
            u64::from(rhs) >= u64::from(lhs) + u64::from(1 - carry_in),
            (rhs ^ lhs) & (rhs ^ result),
        ),
    };
    h.cpsr = (h.cpsr & 0x0fff_ffff) | nz | (u32::from(carry) << 29) | ((overflow >> 31) << 28);
}

#[inline(always)]
fn multiply<const EW: bool>(h: &mut Hot, regs: &mut Registers, word: u32) -> bool {
    let multiplier = regs[((word >> 8) & 15) as usize];
    let accumulate = word & (1 << 21) != 0;
    let iterations = Core::multiply_iterations(multiplier, true) as i64;
    if !h.charge(Ctx::<EW>::FETCH + iterations + i64::from(accumulate)) {
        return false;
    }
    let mut result = regs[(word & 15) as usize].wrapping_mul(multiplier);
    if accumulate {
        result = result.wrapping_add(regs[((word >> 12) & 15) as usize]);
    }
    regs[((word >> 16) & 15) as usize] = result;
    if word & (1 << 20) != 0 {
        h.cpsr = (h.cpsr & 0x3fff_ffff) | (result & 0x8000_0000) | (u32::from(result == 0) << 30);
    }
    h.next()
}

#[inline(always)]
fn multiply_long_op<const EW: bool>(mut h: Hot, regs: &mut Registers, word: u32) -> Option<Hot> {
    let multiplier = regs[((word >> 8) & 15) as usize];
    let signed = word & (1 << 22) != 0;
    let accumulate = word & (1 << 21) != 0;
    let iterations = Core::multiply_iterations(multiplier, signed) as i64;
    if !h.charge(Ctx::<EW>::FETCH + iterations + 1 + i64::from(accumulate)) {
        return None;
    }
    let (hi, lo) = (((word >> 16) & 15) as usize, ((word >> 12) & 15) as usize);
    let multiplicand = regs[(word & 15) as usize];
    let mut product = if signed {
        (i64::from(multiplicand as i32) * i64::from(multiplier as i32)) as u64
    } else {
        u64::from(multiplicand) * u64::from(multiplier)
    };
    if accumulate {
        product = product.wrapping_add(u64::from(regs[hi]) << 32 | u64::from(regs[lo]));
    }
    regs[lo] = product as u32;
    regs[hi] = (product >> 32) as u32;
    if word & (1 << 20) != 0 {
        h.cpsr = (h.cpsr & 0x3fff_ffff)
            | ((product >> 32) as u32 & 0x8000_0000)
            | (u32::from(product == 0) << 30);
    }
    h.next();
    Some(h)
}

/// The address of a fast transfer with an immediate offset.
#[inline(always)]
fn immediate(regs: &Registers, op: &Op) -> u32 {
    regs[usize::from(op.rn)].wrapping_add(op.x)
}

/// The address of a fast single transfer with a shifted register offset.
#[inline(always)]
fn shifted(cpsr: u32, regs: &Registers, op: &Op) -> u32 {
    let carry_in = (cpsr >> 29) & 1;
    let offset = shift_immediate(regs[op.x as u8 as usize], op.x >> 8, op.x >> 16, carry_in).0;
    let base = regs[usize::from(op.rn)];
    if op.x >> 24 != 0 { base.wrapping_sub(offset) } else { base.wrapping_add(offset) }
}

/// The address of a fast halfword transfer with a register offset.
#[inline(always)]
fn indexed(regs: &Registers, op: &Op) -> u32 {
    let offset = regs[op.x as u8 as usize];
    let base = regs[usize::from(op.rn)];
    if op.x >> 8 != 0 { base.wrapping_sub(offset) } else { base.wrapping_add(offset) }
}

/// A pre-indexed transfer without writeback at `address`.
#[inline(always)]
fn transfer<const EW: bool, const MODE: u8>(
    h: &mut Hot,
    c: &mut Ctx<'_, EW>,
    ops: &[Op],
    regs: &mut Registers,
    op: &Op,
    address: u32,
) -> bool {
    let (width, bus_address) = match MODE {
        LDR | STR => (AccessWidth::Word, address & !3),
        LDRB | STRB | LDRSB => (AccessWidth::Byte, address),
        _ => (AccessWidth::Half, address & !1),
    };
    if matches!(MODE, STR | STRB | STRH) {
        let Some(cycles) = store_cycles(c, ops, c.pc(h.idx), bus_address, width) else {
            return false;
        };
        if !h.charge(Ctx::<EW>::FETCH + cycles) {
            return false;
        }
        store(c, bus_address, width, regs[usize::from(op.rd)]);
        c.store_mark = h.budget;
        return h.next();
    }
    let Some((value, cycles)) = load(c, bus_address, width) else {
        return false;
    };
    if !h.charge(Ctx::<EW>::FETCH + cycles + 1) {
        return false;
    }
    regs[usize::from(op.rd)] = match MODE {
        LDR => value.rotate_right((address & 3) * 8),
        LDRH => value.rotate_right((address & 1) * 8),
        LDRSB => (value as u8 as i8 as i32) as u32,
        LDRSH if address & 1 != 0 => ((value >> 8) as u8 as i8 as i32) as u32,
        LDRSH => (value as u16 as i16 as i32) as u32,
        _ => value,
    };
    c.data = value;
    c.load_mark = h.budget;
    h.next()
}

/// LDR/STR/LDRB/STRB with any addressing mode.
#[inline(always)]
fn single_op<const EW: bool>(
    mut h: Hot,
    c: &mut Ctx<'_, EW>,
    ops: &[Op],
    regs: &mut Registers,
    word: u32,
) -> Option<Hot> {
    regs[15] = c.pc(h.idx).wrapping_add(8);
    let pre = word & (1 << 24) != 0;
    let byte = word & (1 << 22) != 0;
    let writeback = !pre || word & (1 << 21) != 0;
    let (rn, rd) = (((word >> 16) & 15) as usize, ((word >> 12) & 15) as usize);
    let base = regs[rn];
    let offset = if word & (1 << 25) != 0 {
        let carry_in = (h.cpsr >> 29) & 1;
        shift_immediate(regs[(word & 15) as usize], word >> 5, word >> 7, carry_in).0
    } else {
        word & 0xfff
    };
    let adjusted =
        if word & (1 << 23) != 0 { base.wrapping_add(offset) } else { base.wrapping_sub(offset) };
    let address = if pre { adjusted } else { base };
    let (width, bus_address) =
        if byte { (AccessWidth::Byte, address) } else { (AccessWidth::Word, address & !3) };
    if word & (1 << 20) != 0 {
        let (value, cycles) = load(c, bus_address, width)?;
        if !h.charge(Ctx::<EW>::FETCH + cycles + 1) {
            return None;
        }
        regs[rd] = if byte { value } else { value.rotate_right((address & 3) * 8) };
        if writeback && rn != rd {
            regs[rn] = adjusted;
        }
        c.data = value;
        c.load_mark = h.budget;
    } else {
        let cycles = store_cycles(c, ops, c.pc(h.idx), bus_address, width)?;
        if !h.charge(Ctx::<EW>::FETCH + cycles) {
            return None;
        }
        store(c, bus_address, width, regs[rd].wrapping_add(if rd == 15 { 4 } else { 0 }));
        if writeback {
            regs[rn] = adjusted;
        }
        c.store_mark = h.budget;
    }
    h.next();
    Some(h)
}

/// LDRH/STRH/LDRSB/LDRSH with any addressing mode.
#[inline(always)]
fn half_op<const EW: bool>(
    mut h: Hot,
    c: &mut Ctx<'_, EW>,
    ops: &[Op],
    regs: &mut Registers,
    word: u32,
) -> Option<Hot> {
    regs[15] = c.pc(h.idx).wrapping_add(8);
    let pre = word & (1 << 24) != 0;
    let writeback = !pre || word & (1 << 21) != 0;
    let (rn, rd) = (((word >> 16) & 15) as usize, ((word >> 12) & 15) as usize);
    let base = regs[rn];
    let offset = if word & (1 << 22) != 0 {
        ((word >> 8) & 15) << 4 | (word & 15)
    } else {
        regs[(word & 15) as usize]
    };
    let adjusted =
        if word & (1 << 23) != 0 { base.wrapping_add(offset) } else { base.wrapping_sub(offset) };
    let address = if pre { adjusted } else { base };
    let operation = (word >> 5) & 3;
    let (width, bus_address) = if operation == 2 {
        (AccessWidth::Byte, address)
    } else {
        (AccessWidth::Half, address & !1)
    };
    if word & (1 << 20) != 0 {
        let (value, cycles) = load(c, bus_address, width)?;
        if !h.charge(Ctx::<EW>::FETCH + cycles + 1) {
            return None;
        }
        regs[rd] = match operation {
            1 => value.rotate_right((address & 1) * 8),
            2 => (value as u8 as i8 as i32) as u32,
            _ if address & 1 != 0 => ((value >> 8) as u8 as i8 as i32) as u32,
            _ => (value as u16 as i16 as i32) as u32,
        };
        if writeback && rn != rd {
            regs[rn] = adjusted;
        }
        c.data = value;
        c.load_mark = h.budget;
    } else {
        let cycles = store_cycles(c, ops, c.pc(h.idx), bus_address, width)?;
        if !h.charge(Ctx::<EW>::FETCH + cycles) {
            return None;
        }
        store(c, bus_address, width, regs[rd]);
        if writeback {
            regs[rn] = adjusted;
        }
        c.store_mark = h.budget;
    }
    h.next();
    Some(h)
}

/// LDM/STM within one RAM region, without r15 loads.
#[inline(always)]
fn block_op<const EW: bool>(
    mut h: Hot,
    c: &mut Ctx<'_, EW>,
    ops: &[Op],
    regs: &mut Registers,
    word: u32,
) -> Option<Hot> {
    let pc = c.pc(h.idx);
    regs[15] = pc.wrapping_add(8);
    let list = word & 0xffff;
    let count = list.count_ones();
    let rn = (word >> 16) & 15;
    let load_list = word & (1 << 20) != 0;
    let writeback = word & (1 << 21) != 0;
    let user_bank = word & (1 << 22) != 0;
    let pre = word & (1 << 24) != 0;
    let base = regs[rn as usize];
    let (first, final_base) = if word & (1 << 23) != 0 {
        (base.wrapping_add(if pre { 4 } else { 0 }), base.wrapping_add(count * 4))
    } else {
        let final_base = base.wrapping_sub(count * 4);
        (final_base.wrapping_add(if pre { 0 } else { 4 }), final_base)
    };
    let first = first & !3;
    let last = first.wrapping_add((count - 1) * 4);
    let per_word = match (first >> 24, last >> 24) {
        (3, 3) => 1,
        (2, 2) => 6,
        _ => return None,
    };
    if !load_list
        && (0..count)
            .any(|beat| store_cycles(c, ops, pc, first + 4 * beat, AccessWidth::Word).is_none())
    {
        return None;
    }
    if !h.charge(Ctx::<EW>::FETCH + i64::from(count) * per_word + i64::from(load_list)) {
        return None;
    }
    if user_bank {
        c.core.registers.copy_from_slice(&regs[..16]);
    }
    let mut address = first;
    let mut remaining = list;
    let mut index = 0;
    while remaining != 0 {
        let register = remaining.trailing_zeros() as usize;
        remaining &= remaining - 1;
        if load_list {
            let (value, _) = load(c, address, AccessWidth::Word).expect("RAM block");
            if user_bank {
                c.core.write_user_bank_register(register, value);
            } else {
                regs[register] = value;
            }
            c.data = value;
        } else {
            let value = if register == 15 {
                regs[15].wrapping_add(4)
            } else if writeback && register as u32 == rn && index != 0 {
                final_base
            } else if user_bank {
                c.core.read_user_bank_register(register)
            } else {
                regs[register]
            };
            store(c, address, AccessWidth::Word, value);
        }
        address = address.wrapping_add(4);
        index += 1;
    }
    if user_bank {
        regs[..16].copy_from_slice(&c.core.registers);
    }
    if writeback && !(load_list && !user_bank && list & (1 << rn) != 0) {
        regs[rn as usize] = final_base;
    }
    if load_list {
        c.load_mark = h.budget;
    } else {
        c.store_mark = h.budget;
    }
    h.next();
    Some(h)
}

/// A conditional instruction whose condition passed, through the generic
/// handlers.
#[inline(always)]
fn conditional_op<const EW: bool>(
    mut h: Hot,
    c: &mut Ctx<'_, EW>,
    ops: &[Op],
    regs: &mut Registers,
    op: &Op,
) -> Option<Hot> {
    let word = c.code_word(c.pc(h.idx));
    match op.inner {
        k::B | k::BL => branch(&mut h, c, regs, op, op.inner == k::BL).then_some(h),
        k::MRS => mrs::<EW>(&mut h, regs, op).then_some(h),
        k::MUL => multiply::<EW>(&mut h, regs, word).then_some(h),
        k::MULL => multiply_long_op::<EW>(h, regs, word),
        k::LDR..=k::STRB | k::SINGLE | k::LDR_R..k::HALF_I => single_op(h, c, ops, regs, word),
        k::HALF | k::HALF_I..=29 => half_op(h, c, ops, regs, word),
        k::BLOCK => block_op(h, c, ops, regs, word),
        k::DP_PC | k::DP_I..k::BCOND => dp_pc_op(h, c, regs, word),
        _ => None,
    }
}

/// Out-of-line entry points for handlers too large or too rare to inline
/// into the dispatch loop. They take and return the hot state by value.
#[inline(never)]
fn dp_pc<const EW: bool>(h: Hot, c: &mut Ctx<'_, EW>, regs: &mut Registers, word: u32) -> Hot {
    dp_pc_op(h, c, regs, word).unwrap_or(Hot::REFUSED)
}

#[inline(never)]
fn multiply_long<const EW: bool>(h: Hot, regs: &mut Registers, word: u32) -> Hot {
    multiply_long_op::<EW>(h, regs, word).unwrap_or(Hot::REFUSED)
}

#[inline(never)]
fn single<const EW: bool>(
    h: Hot,
    c: &mut Ctx<'_, EW>,
    ops: &[Op],
    regs: &mut Registers,
    word: u32,
) -> Hot {
    single_op(h, c, ops, regs, word).unwrap_or(Hot::REFUSED)
}

#[inline(never)]
fn half<const EW: bool>(
    h: Hot,
    c: &mut Ctx<'_, EW>,
    ops: &[Op],
    regs: &mut Registers,
    word: u32,
) -> Hot {
    half_op(h, c, ops, regs, word).unwrap_or(Hot::REFUSED)
}

#[inline(never)]
fn block<const EW: bool>(
    h: Hot,
    c: &mut Ctx<'_, EW>,
    ops: &[Op],
    regs: &mut Registers,
    word: u32,
) -> Hot {
    block_op(h, c, ops, regs, word).unwrap_or(Hot::REFUSED)
}

#[inline(never)]
fn conditional<const EW: bool>(
    h: Hot,
    c: &mut Ctx<'_, EW>,
    ops: &[Op],
    regs: &mut Registers,
    op: &Op,
) -> Hot {
    conditional_op(h, c, ops, regs, op).unwrap_or(Hot::REFUSED)
}

/// The state a run stops with. Out of line, so that the compiler keeps the
/// many exits of the dispatch loop apart from its hot path.
#[cold]
#[inline(never)]
fn stopped(h: Hot) -> Hot {
    std::hint::black_box(h)
}

/// Returns an overdrawn budget to where it was, off the hot path.
#[cold]
#[inline(never)]
fn refund(budget: i64, cycles: i64) -> i64 {
    budget + cycles
}

/// A data load at a bus-aligned address: its bus value and data cycles, or
/// None when the ordinary bus must perform it.
#[inline(always)]
fn load<const EW: bool>(c: &Ctx<'_, EW>, address: u32, width: AccessWidth) -> Option<(u32, i64)> {
    match address >> 24 {
        3 => Some((read_ram(c.iwram, (address as usize) & (IWRAM_SIZE - 1), width), 1)),
        2 => Some((
            read_ram(c.ewram, (address as usize) & (EWRAM_SIZE - 1), width),
            if width == AccessWidth::Word { 6 } else { 3 },
        )),
        _ => c.rom.load(address, width).map(|(value, cycles)| (value, cycles as i64)),
    }
}

/// The data cycles of a store at a bus-aligned address that the engine
/// may perform: to RAM, and not to a decoded instruction or to one of the
/// two instructions prefetched after the one at `pc`.
#[inline(always)]
fn store_cycles<const EW: bool>(
    c: &Ctx<'_, EW>,
    ops: &[Op],
    pc: u32,
    address: u32,
    width: AccessWidth,
) -> Option<i64> {
    let (cycles, code) = match address >> 24 {
        3 => (1, c.iwram_code[((address as usize) & (IWRAM_SIZE - 1)) >> PAGE_SHIFT]),
        2 => (
            if width == AccessWidth::Word { 6 } else { 3 },
            c.ewram_code[((address as usize) & (EWRAM_SIZE - 1)) >> PAGE_SHIFT],
        ),
        _ => return None,
    };
    (!code || !store_hits_code(c, ops, pc, address)).then_some(cycles)
}

/// Whether a store to the RAM word holding `address`, in a page that has
/// held code, reaches a decoded op or the prefetch window.
#[cold]
#[inline(never)]
fn store_hits_code<const EW: bool>(c: &Ctx<'_, EW>, ops: &[Op], pc: u32, address: u32) -> bool {
    let region = address >> 24;
    let mask = if region == 2 { EWRAM_SIZE - 1 } else { IWRAM_SIZE - 1 } as u32;
    let word = address & mask & !3;
    let ops = if region == Ctx::<EW>::REGION {
        let window = pc.wrapping_add(4) & Ctx::<EW>::MASK;
        if word.wrapping_sub(window) & Ctx::<EW>::MASK < 8 {
            return true;
        }
        Some(ops)
    } else {
        c.other_ops
    };
    ops.is_some_and(|ops| ops[(word >> 2) as usize].kind != k::UNDECODED)
}

#[inline(always)]
fn store<const EW: bool>(c: &mut Ctx<'_, EW>, address: u32, width: AccessWidth, value: u32) {
    if address >> 24 == 3 {
        write_ram(c.iwram, (address as usize) & (IWRAM_SIZE - 1), width, value);
    } else {
        write_ram(c.ewram, (address as usize) & (EWRAM_SIZE - 1), width, value);
    }
}
