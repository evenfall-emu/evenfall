//! ARM code running from IWRAM or EWRAM.
//!
//! A RAM code fetch costs the same whether sequential or not, so every
//! instruction costs its fetch plus its own data and internal cycles, and a
//! branch within the region adds two refill fetches.

use super::mem::{Mem, Running};
use super::{Op, PAGE_SHIFT, fresh_ops, split_memory};
use crate::bus::{AccessWidth, Bus, EWRAM_SIZE, IWRAM_SIZE, read_ram};
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
    /// Single transfers by an immediate offset `x` with writeback, pre- and
    /// post-indexed, in LDR, LDRB, STR, STRB order; never with rd = rn.
    pub const LDR_W: u8 = 208;
    pub const LDR_P: u8 = 212;
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

const fn is_logical(opcode: u32) -> bool {
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
            let mode = u8::from(!load) << 1 | u8::from(byte);
            let down = word & (1 << 23) == 0;
            let immediate = word & (1 << 25) == 0;
            let offset = word & 0xfff;
            let x = if down { offset.wrapping_neg() } else { offset };
            // Pre-indexed with writeback, or post-indexed (not the T forms).
            if immediate && pre == writeback && rn != 15 && rd != 15 && rd != rn {
                return Op { x, ..raw(if pre { k::LDR_W } else { k::LDR_P } + mode) };
            }
            if !pre || writeback || rn == 15 || rd == 15 || (!immediate && word & 15 == 15) {
                return raw(k::SINGLE);
            }
            if !immediate {
                let x = word & 15 | (word >> 5 & 3) << 8 | (word >> 7 & 31) << 16;
                return Op { x: x | u32::from(down) << 24, ..raw(k::LDR_R + mode) };
            }
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
    mem: Mem<'a>,
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
            read_ram(self.mem.ewram, index, AccessWidth::Word)
        } else {
            read_ram(self.mem.iwram, index, AccessWidth::Word)
        }
    }

    /// The region, for the block loop.
    #[inline(always)]
    fn code(&self) -> Code<'_> {
        Code {
            slots: Self::SLOTS as u32,
            fetch: Self::FETCH,
            #[cfg(debug_assertions)]
            memory: if EW { &self.mem.ewram[..] } else { &self.mem.iwram[..] },
            #[cfg(debug_assertions)]
            base: self.base,
            #[cfg(not(debug_assertions))]
            memory: std::marker::PhantomData,
        }
    }

    /// The running code, for store checks.
    #[inline(always)]
    fn running<'o>(&self, ops: &'o [Op], idx: u32) -> Running<'o> {
        Running { ops, region: Self::REGION, thumb: false, pc: self.pc(idx) }
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
    let budget = (limit - start) as i64;
    let memory = bus.engine_memory();
    let slots = (mask as usize + 1) / 4;
    let mut ops = memory.code.take(Ctx::<EW>::REGION, false, || {
        let mut tail = vec![END_OP; slots];
        tail[slots - 1] = STOP_OP;
        fresh_ops(slots, &tail)
    });
    let (mem, _) = split_memory(memory, budget);
    let pipeline = core.pipeline;
    let cpsr = core.cpsr;
    let mut c = Ctx::<EW> { mem, core, base: pc & !mask, load_mark: -1, store_mark: -1, data: 0 };
    let latched = [c.code_word(pc), c.code_word(pc.wrapping_add(4))];
    if pipeline != Some(Pipeline { instructions: latched, instruction_set: InstructionSet::Arm }) {
        bus.code.put(Ctx::<EW>::REGION, false, ops);
        return false;
    }
    let hot = Hot { idx: (pc & mask) >> 2, budget, cpsr };
    let hot = if EW {
        let ops = (&mut *ops).try_into().expect("EWRAM op array");
        run_loop::<EW, { 2 * EWRAM_SIZE / 4 }>(ops, &mut c, hot)
    } else {
        let ops = (&mut *ops).try_into().expect("IWRAM op array");
        run_loop::<EW, { 2 * IWRAM_SIZE / 4 }>(ops, &mut c, hot)
    };
    if hot.budget == budget {
        bus.code.put(Ctx::<EW>::REGION, false, ops);
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
    bus.code.put(Ctx::<EW>::REGION, false, ops);
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

/// A data processing op of kind `$kind`: with the rest of the block it
/// leads in one charge when the budget covers the block, else alone. The
/// first time it runs, it measures its block and dispatches again.
macro_rules! dp {
    ($h:ident, $c:ident, $ops:ident, $regs:ident, $op:ident, $kind:literal) => {{
        if !in_block($kind) || $op.inner == 1 {
            go!($h, $c, dp::<EW, $kind>(&mut $h, $regs, $op))
        } else if $op.inner == 0 {
            measure_block(&mut $ops[..], $c, $h.idx);
        } else if $h.budget >= i64::from($op.inner) * Ctx::<EW>::FETCH {
            $h = alu_block($h, &$ops[..], $regs, $c.code());
        } else {
            go!($h, $c, dp::<EW, $kind>(&mut $h, $regs, $op))
        }
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
        check_op(c, h.idx, op);
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
            208 => go!(h, c, pre_indexed::<EW, LDR>(&mut h, c, ops, regs, op)),
            209 => go!(h, c, pre_indexed::<EW, LDRB>(&mut h, c, ops, regs, op)),
            210 => go!(h, c, pre_indexed::<EW, STR>(&mut h, c, ops, regs, op)),
            211 => go!(h, c, pre_indexed::<EW, STRB>(&mut h, c, ops, regs, op)),
            212 => go!(h, c, post_indexed::<EW, LDR>(&mut h, c, ops, regs, op)),
            213 => go!(h, c, post_indexed::<EW, LDRB>(&mut h, c, ops, regs, op)),
            214 => go!(h, c, post_indexed::<EW, STR>(&mut h, c, ops, regs, op)),
            215 => go!(h, c, post_indexed::<EW, STRB>(&mut h, c, ops, regs, op)),
            32 => dp!(h, c, ops, regs, op, 32),
            33 => dp!(h, c, ops, regs, op, 33),
            34 => dp!(h, c, ops, regs, op, 34),
            35 => dp!(h, c, ops, regs, op, 35),
            36 => dp!(h, c, ops, regs, op, 36),
            37 => dp!(h, c, ops, regs, op, 37),
            38 => dp!(h, c, ops, regs, op, 38),
            39 => dp!(h, c, ops, regs, op, 39),
            44 => dp!(h, c, ops, regs, op, 44),
            45 => dp!(h, c, ops, regs, op, 45),
            46 => dp!(h, c, ops, regs, op, 46),
            47 => dp!(h, c, ops, regs, op, 47),
            48 => dp!(h, c, ops, regs, op, 48),
            49 => dp!(h, c, ops, regs, op, 49),
            50 => dp!(h, c, ops, regs, op, 50),
            51 => dp!(h, c, ops, regs, op, 51),
            52 => dp!(h, c, ops, regs, op, 52),
            53 => dp!(h, c, ops, regs, op, 53),
            54 => dp!(h, c, ops, regs, op, 54),
            55 => dp!(h, c, ops, regs, op, 55),
            60 => dp!(h, c, ops, regs, op, 60),
            61 => dp!(h, c, ops, regs, op, 61),
            62 => dp!(h, c, ops, regs, op, 62),
            63 => dp!(h, c, ops, regs, op, 63),
            64 => dp!(h, c, ops, regs, op, 64),
            65 => dp!(h, c, ops, regs, op, 65),
            66 => dp!(h, c, ops, regs, op, 66),
            67 => dp!(h, c, ops, regs, op, 67),
            68 => dp!(h, c, ops, regs, op, 68),
            69 => dp!(h, c, ops, regs, op, 69),
            70 => dp!(h, c, ops, regs, op, 70),
            71 => dp!(h, c, ops, regs, op, 71),
            76 => dp!(h, c, ops, regs, op, 76),
            77 => dp!(h, c, ops, regs, op, 77),
            78 => dp!(h, c, ops, regs, op, 78),
            79 => dp!(h, c, ops, regs, op, 79),
            80 => dp!(h, c, ops, regs, op, 80),
            81 => dp!(h, c, ops, regs, op, 81),
            82 => dp!(h, c, ops, regs, op, 82),
            83 => dp!(h, c, ops, regs, op, 83),
            84 => dp!(h, c, ops, regs, op, 84),
            85 => dp!(h, c, ops, regs, op, 85),
            86 => dp!(h, c, ops, regs, op, 86),
            87 => dp!(h, c, ops, regs, op, 87),
            92 => dp!(h, c, ops, regs, op, 92),
            93 => dp!(h, c, ops, regs, op, 93),
            94 => dp!(h, c, ops, regs, op, 94),
            95 => dp!(h, c, ops, regs, op, 95),
            96 => dp!(h, c, ops, regs, op, 96),
            97 => dp!(h, c, ops, regs, op, 97),
            98 => dp!(h, c, ops, regs, op, 98),
            99 => dp!(h, c, ops, regs, op, 99),
            100 => dp!(h, c, ops, regs, op, 100),
            101 => dp!(h, c, ops, regs, op, 101),
            102 => dp!(h, c, ops, regs, op, 102),
            103 => dp!(h, c, ops, regs, op, 103),
            108 => dp!(h, c, ops, regs, op, 108),
            109 => dp!(h, c, ops, regs, op, 109),
            110 => dp!(h, c, ops, regs, op, 110),
            111 => dp!(h, c, ops, regs, op, 111),
            112 => dp!(h, c, ops, regs, op, 112),
            113 => dp!(h, c, ops, regs, op, 113),
            114 => dp!(h, c, ops, regs, op, 114),
            115 => dp!(h, c, ops, regs, op, 115),
            116 => dp!(h, c, ops, regs, op, 116),
            117 => dp!(h, c, ops, regs, op, 117),
            118 => dp!(h, c, ops, regs, op, 118),
            119 => dp!(h, c, ops, regs, op, 119),
            120 => dp!(h, c, ops, regs, op, 120),
            121 => dp!(h, c, ops, regs, op, 121),
            122 => dp!(h, c, ops, regs, op, 122),
            123 => dp!(h, c, ops, regs, op, 123),
            124 => dp!(h, c, ops, regs, op, 124),
            125 => dp!(h, c, ops, regs, op, 125),
            126 => dp!(h, c, ops, regs, op, 126),
            127 => dp!(h, c, ops, regs, op, 127),
            128 => dp!(h, c, ops, regs, op, 128),
            129 => dp!(h, c, ops, regs, op, 129),
            136 => dp!(h, c, ops, regs, op, 136),
            137 => dp!(h, c, ops, regs, op, 137),
            140 => dp!(h, c, ops, regs, op, 140),
            141 => dp!(h, c, ops, regs, op, 141),
            142 => dp!(h, c, ops, regs, op, 142),
            143 => dp!(h, c, ops, regs, op, 143),
            144 => dp!(h, c, ops, regs, op, 144),
            145 => dp!(h, c, ops, regs, op, 145),
            146 => dp!(h, c, ops, regs, op, 146),
            147 => dp!(h, c, ops, regs, op, 147),
            148 => dp!(h, c, ops, regs, op, 148),
            149 => dp!(h, c, ops, regs, op, 149),
            150 => dp!(h, c, ops, regs, op, 150),
            151 => dp!(h, c, ops, regs, op, 151),
            152 => dp!(h, c, ops, regs, op, 152),
            153 => dp!(h, c, ops, regs, op, 153),
            154 => dp!(h, c, ops, regs, op, 154),
            155 => dp!(h, c, ops, regs, op, 155),
            156 => dp!(h, c, ops, regs, op, 156),
            157 => dp!(h, c, ops, regs, op, 157),
            158 => dp!(h, c, ops, regs, op, 158),
            159 => dp!(h, c, ops, regs, op, 159),
            160 => dp!(h, c, ops, regs, op, 160),
            161 => dp!(h, c, ops, regs, op, 161),
            162 => dp!(h, c, ops, regs, op, 162),
            163 => dp!(h, c, ops, regs, op, 163),
            164 => dp!(h, c, ops, regs, op, 164),
            165 => dp!(h, c, ops, regs, op, 165),
            166 => dp!(h, c, ops, regs, op, 166),
            167 => dp!(h, c, ops, regs, op, 167),
            168 => dp!(h, c, ops, regs, op, 168),
            169 => dp!(h, c, ops, regs, op, 169),
            170 => dp!(h, c, ops, regs, op, 170),
            171 => dp!(h, c, ops, regs, op, 171),
            172 => dp!(h, c, ops, regs, op, 172),
            173 => dp!(h, c, ops, regs, op, 173),
            174 => dp!(h, c, ops, regs, op, 174),
            175 => dp!(h, c, ops, regs, op, 175),
            176 => dp!(h, c, ops, regs, op, 176),
            177 => dp!(h, c, ops, regs, op, 177),
            178 => dp!(h, c, ops, regs, op, 178),
            179 => dp!(h, c, ops, regs, op, 179),
            180 => dp!(h, c, ops, regs, op, 180),
            181 => dp!(h, c, ops, regs, op, 181),
            182 => dp!(h, c, ops, regs, op, 182),
            183 => dp!(h, c, ops, regs, op, 183),
            184 => dp!(h, c, ops, regs, op, 184),
            185 => dp!(h, c, ops, regs, op, 185),
            186 => dp!(h, c, ops, regs, op, 186),
            187 => dp!(h, c, ops, regs, op, 187),
            188 => dp!(h, c, ops, regs, op, 188),
            189 => dp!(h, c, ops, regs, op, 189),
            190 => dp!(h, c, ops, regs, op, 190),
            191 => dp!(h, c, ops, regs, op, 191),
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
            | 207
            | 216..=239
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
            c.mem.ewram_code[page] = true;
        } else {
            c.mem.iwram_code[page] = true;
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

/// The operand form of a data processing kind.
const fn form(kind: u8) -> u8 {
    [FI, FR, FL, FG, FX, FI, FIC, FR, FG, FX][((kind - k::DP_I) / 16) as usize]
}

/// Whether ops of a kind cost exactly one fetch, whatever their operands,
/// and change only registers and flags: the ops a block may hold, besides
/// conditional ones whose inner kind is such a kind.
const fn in_block(kind: u8) -> bool {
    if !matches!(kind, k::DP_I..k::BCOND) || form(kind) == FX {
        return false;
    }
    let opcode = (kind % 16) as u32;
    // The decoder leaves compares without S, and FIC for arithmetic, out.
    (kind >= k::DPS_I || !matches!(opcode, 8..=11)) && (form(kind) != FIC || is_logical(opcode))
}

/// A data processing op of kind `KIND` (in `DP_I..BCOND`).
#[inline(always)]
fn dp<const EW: bool, const KIND: u8>(h: &mut Hot, regs: &mut Registers, op: &Op) -> bool {
    if !h.charge(Ctx::<EW>::FETCH + i64::from(form(KIND) == FX)) {
        return false;
    }
    dp_body::<KIND>(h, regs, op);
    h.next()
}

/// The effect of a data processing op of kind `KIND` on registers and flags.
#[inline(always)]
fn dp_body<const KIND: u8>(h: &mut Hot, regs: &mut Registers, op: &Op) {
    let carry_in = (h.cpsr >> 29) & 1;
    let rm = regs[op.x as u8 as usize];
    let (rhs, carry) = match form(KIND) {
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
    alu(h, regs, u32::from(KIND % 16), KIND >= k::DPS_I, op.rd, lhs, rhs, carry, carry_in);
}

/// Calls `$m!` with the data processing kinds a block holds.
macro_rules! block_kinds {
    ($m:ident $(, $arg:tt)*) => {
        $m!(
            $($arg),*;
            32 33 34 35 36 37 38 39 44 45 46 47
            48 49 50 51 52 53 54 55 60 61 62 63
            64 65 66 67 68 69 70 71 76 77 78 79
            80 81 82 83 84 85 86 87 92 93 94 95
            112 113 114 115 116 117 118 119 120 121 122 123 124 125 126 127
            128 129 136 137 140 141 142 143
            144 145 146 147 148 149 150 151 152 153 154 155 156 157 158 159
            160 161 162 163 164 165 166 167 168 169 170 171 172 173 174 175
        )
    };
}

/// One op `$op` of a block: its effect on `$h` and `$regs`, or `$stop` if
/// it does not belong to one. A single table dispatch over every kind,
/// conditional ones included.
macro_rules! block_step {
    ($h:ident, $regs:ident, $op:ident, $stop:tt; $($kind:literal)*) => {
        match $op.kind {
            $($kind => dp_body::<$kind>($h, $regs, $op),)*
            240 if !cond_step::<0>($h, $regs, $op) => $stop,
            241 if !cond_step::<1>($h, $regs, $op) => $stop,
            242 if !cond_step::<2>($h, $regs, $op) => $stop,
            243 if !cond_step::<3>($h, $regs, $op) => $stop,
            244 if !cond_step::<4>($h, $regs, $op) => $stop,
            245 if !cond_step::<5>($h, $regs, $op) => $stop,
            246 if !cond_step::<6>($h, $regs, $op) => $stop,
            247 if !cond_step::<7>($h, $regs, $op) => $stop,
            248 if !cond_step::<8>($h, $regs, $op) => $stop,
            249 if !cond_step::<9>($h, $regs, $op) => $stop,
            250 if !cond_step::<10>($h, $regs, $op) => $stop,
            251 if !cond_step::<11>($h, $regs, $op) => $stop,
            252 if !cond_step::<12>($h, $regs, $op) => $stop,
            253 if !cond_step::<13>($h, $regs, $op) => $stop,
            254 if !cond_step::<14>($h, $regs, $op) => $stop,
            k::NOP | 240..=254 => {}
            0..=2
            | 4..=31
            | 40..=43
            | 56..=59
            | 72..=75
            | 88..=91
            | 96..=111
            | 130..=135
            | 138
            | 139
            | 176..=239
            | 255 => $stop,
        }
    };
}

/// Runs the block `ops[h.idx]` leads, whose cost the caller checked the
/// budget covers, up to its end or to the first op that no longer belongs
/// to it (a slot changed since the block was measured). Each op is one
/// table dispatch over every kind, conditional ones included. A B or taken
/// conditional B that ends the block continues here if it lands on a
/// measured block that the budget covers.
#[inline(never)]
fn alu_block(mut h: Hot, ops: &[Op], regs: &mut Registers, code: Code<'_>) -> Hot {
    loop {
        if !run_block(&mut h, ops, regs, code) {
            return h;
        }
        let branch = &ops[h.idx as usize];
        code.check(h.idx, branch);
        let taken = match branch.kind {
            k::B => true,
            k::BCOND..=206 => condition_passes(h.cpsr, u32::from(branch.kind - k::BCOND)),
            _ => false,
        };
        let target = h.idx.wrapping_add(branch.x);
        if !taken || target >= code.slots {
            return h;
        }
        // Only measured block leaders, of kinds below BCOND, have an inner
        // of 2 or more there.
        let lead = &ops[target as usize];
        let cost = (3 + i64::from(lead.inner)) * code.fetch;
        if lead.kind >= k::BCOND || lead.inner < 2 || h.budget < cost {
            return h;
        }
        h.budget -= 3 * code.fetch;
        h.idx = target;
    }
}

/// What the block loop needs to know of the region it runs in: its
/// instruction slots and their fetch cost, and in debug builds the code
/// itself, to check each op against a fresh decode.
#[derive(Clone, Copy)]
struct Code<'a> {
    slots: u32,
    fetch: i64,
    #[cfg(debug_assertions)]
    memory: &'a [u8],
    #[cfg(debug_assertions)]
    base: u32,
    #[cfg(not(debug_assertions))]
    memory: std::marker::PhantomData<&'a [u8]>,
}

impl Code<'_> {
    #[inline(always)]
    fn check(&self, _idx: u32, _op: &Op) {
        #[cfg(debug_assertions)]
        {
            let pc = self.base.wrapping_add(_idx << 2);
            let at = pc as usize & (self.memory.len() - 4);
            let word = u32::from_le_bytes(self.memory[at..at + 4].try_into().expect("a word"));
            check_word(pc, word, _op);
        }
    }
}

/// Runs one block (see [`alu_block`]); false if it stopped before an op
/// that no longer belongs to it.
#[inline(always)]
fn run_block(h: &mut Hot, ops: &[Op], regs: &mut Registers, code: Code<'_>) -> bool {
    let start = h.idx as usize;
    let block = &ops[start..(start + usize::from(ops[start].inner)).min(ops.len())];
    let mut rest = block;
    let first = h.idx;
    let check = |rest: &[Op]| code.check(first + (block.len() - rest.len()) as u32, &rest[0]);
    // Two ops per turn, so that the loop's own work is shared.
    'run: {
        while let [a, b, tail @ ..] = rest {
            check(rest);
            block_kinds!(block_step, h, regs, a, { break 'run });
            check(&rest[1..]);
            block_kinds!(block_step, h, regs, b, {
                rest = &rest[1..];
                break 'run;
            });
            rest = tail;
        }
        if let [a, ..] = rest {
            check(rest);
            block_kinds!(block_step, h, regs, a, { break 'run });
            rest = &rest[1..];
        }
    }
    let ran = block.len() - rest.len();
    h.idx += ran as u32;
    h.budget -= ran as i64 * code.fetch;
    rest.is_empty()
}

/// A conditional op in a block: a failed condition costs the fetch,
/// whatever the op.
#[inline(always)]
fn cond_step<const C: u32>(h: &mut Hot, regs: &mut Registers, op: &Op) -> bool {
    !condition_passes(h.cpsr, C) || block_body(op.inner, h, regs, op)
}

/// The effect of a block op of `kind`: false if no block holds that kind.
/// Out of line: every conditional kind of every copy of the block loop
/// calls this one instance.
#[inline(never)]
fn block_body(kind: u8, h: &mut Hot, regs: &mut Registers, op: &Op) -> bool {
    macro_rules! bodies {
        (; $($kind:literal)*) => {
            match kind {
                $($kind => dp_body::<$kind>(h, regs, op),)*
                _ => return false,
            }
        };
    }
    block_kinds!(bodies);
    true
}

/// Whether a decoded op may sit in a block.
fn joins_block(op: &Op) -> bool {
    match op.kind {
        k::COND..=254 => in_block(op.inner),
        k::NOP => true,
        kind => in_block(kind),
    }
}

/// Records in `ops[idx].inner` how many ops from it form a block, decoding
/// the ones after it that are not decoded yet.
#[cold]
#[inline(never)]
fn measure_block<const EW: bool>(ops: &mut [Op], c: &mut Ctx<'_, EW>, idx: u32) {
    let start = idx as usize;
    let mut end = start + 1;
    while end - start < usize::from(u8::MAX) && end < Ctx::<EW>::SLOTS {
        if ops[end].kind == k::UNDECODED {
            decode_slot(ops, c, end as u32);
        }
        if !joins_block(&ops[end]) {
            break;
        }
        end += 1;
    }
    ops[start].inner = (end - start) as u8;
}

/// Asserts that a cached op is what its instruction decodes to; a block
/// leader carries its block's length besides.
#[cfg(debug_assertions)]
fn check_op<const EW: bool>(c: &Ctx<'_, EW>, idx: u32, op: &Op) {
    let pc = c.pc(idx);
    check_word(pc, c.code_word(pc), op);
}

/// Asserts that a cached op at `pc` is what `word` decodes to.
#[cfg(debug_assertions)]
fn check_word(pc: u32, word: u32, op: &Op) {
    if matches!(op.kind, k::UNDECODED | k::END | k::STOP) {
        return;
    }
    let mut expected = decode(word);
    if matches!(op.kind, k::DP_I..k::BCOND) {
        expected.inner = op.inner;
    }
    debug_assert_eq!(*op, expected, "stale op at {pc:08x}");
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

/// The width and bus-aligned address of a transfer in mode `MODE`.
#[inline(always)]
fn access<const MODE: u8>(address: u32) -> (AccessWidth, u32) {
    match MODE {
        LDR | STR => (AccessWidth::Word, address & !3),
        LDRB | STRB | LDRSB => (AccessWidth::Byte, address),
        _ => (AccessWidth::Half, address & !1),
    }
}

/// The register value a load in mode `MODE` at `address` makes of the
/// bus value.
#[inline(always)]
fn loaded<const MODE: u8>(value: u32, address: u32) -> u32 {
    match MODE {
        LDR => value.rotate_right((address & 3) * 8),
        LDRH => value.rotate_right((address & 1) * 8),
        LDRSB => (value as u8 as i8 as i32) as u32,
        LDRSH if address & 1 != 0 => ((value >> 8) as u8 as i8 as i32) as u32,
        LDRSH => (value as u16 as i16 as i32) as u32,
        _ => value,
    }
}

/// A pre-indexed transfer without writeback at `address`: inline for RAM
/// (a store only to a page that never held code), out of line otherwise.
#[inline(always)]
fn transfer<const EW: bool, const MODE: u8>(
    h: &mut Hot,
    c: &mut Ctx<'_, EW>,
    ops: &[Op],
    regs: &mut Registers,
    op: &Op,
    address: u32,
) -> bool {
    let (width, bus_address) = access::<MODE>(address);
    if matches!(MODE, STR | STRB | STRH) {
        if let Some(cycles) = c.mem.plain_ram_store(bus_address, width) {
            if !h.charge(Ctx::<EW>::FETCH + cycles) {
                return false;
            }
            c.mem.store(bus_address, width, regs[usize::from(op.rd)]);
            c.store_mark = h.budget;
            return h.next();
        }
    } else if let Some((value, cycles)) = c.mem.ram_load(bus_address, width) {
        if !h.charge(Ctx::<EW>::FETCH + cycles + 1) {
            return false;
        }
        regs[usize::from(op.rd)] = loaded::<MODE>(value, address);
        c.data = value;
        c.load_mark = h.budget;
        return h.next();
    }
    let next = transfer_elsewhere::<EW, MODE>(*h, c, ops, regs, op, address);
    if next.idx == u32::MAX {
        return false;
    }
    *h = next;
    true
}

/// A transfer outside plain RAM: to video memory, the Game Pak or pages
/// that have held code.
#[inline(never)]
fn transfer_elsewhere<const EW: bool, const MODE: u8>(
    mut h: Hot,
    c: &mut Ctx<'_, EW>,
    ops: &[Op],
    regs: &mut Registers,
    op: &Op,
    address: u32,
) -> Hot {
    let (width, bus_address) = access::<MODE>(address);
    if matches!(MODE, STR | STRB | STRH) {
        let run = c.running(ops, h.idx);
        let fetch = Ctx::<EW>::FETCH;
        let Some(cost) = c.mem.store_cost(run, bus_address, width, fetch, h.budget) else {
            return Hot::REFUSED;
        };
        if !h.charge(cost) {
            return Hot::REFUSED;
        }
        c.mem.store(bus_address, width, regs[usize::from(op.rd)]);
        c.store_mark = h.budget;
        h.next();
        return h;
    }
    let Some((value, cycles)) = c.mem.load(bus_address, width) else {
        return Hot::REFUSED;
    };
    if !h.charge(Ctx::<EW>::FETCH + cycles + 1) {
        return Hot::REFUSED;
    }
    regs[usize::from(op.rd)] = loaded::<MODE>(value, address);
    c.data = value;
    c.load_mark = h.budget;
    h.next();
    h
}

/// A transfer by an immediate offset that writes the address back.
#[inline(always)]
fn pre_indexed<const EW: bool, const MODE: u8>(
    h: &mut Hot,
    c: &mut Ctx<'_, EW>,
    ops: &[Op],
    regs: &mut Registers,
    op: &Op,
) -> bool {
    let address = immediate(regs, op);
    transfer::<EW, MODE>(h, c, ops, regs, op, address) && {
        regs[usize::from(op.rn)] = address;
        true
    }
}

/// A transfer at the base address, which then moves by the offset.
#[inline(always)]
fn post_indexed<const EW: bool, const MODE: u8>(
    h: &mut Hot,
    c: &mut Ctx<'_, EW>,
    ops: &[Op],
    regs: &mut Registers,
    op: &Op,
) -> bool {
    let base = regs[usize::from(op.rn)];
    transfer::<EW, MODE>(h, c, ops, regs, op, base) && {
        regs[usize::from(op.rn)] = base.wrapping_add(op.x);
        true
    }
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
        let (value, cycles) = c.mem.load(bus_address, width)?;
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
        let run = c.running(ops, h.idx);
        let cost = c.mem.store_cost(run, bus_address, width, Ctx::<EW>::FETCH, h.budget)?;
        if !h.charge(cost) {
            return None;
        }
        c.mem.store(bus_address, width, regs[rd].wrapping_add(if rd == 15 { 4 } else { 0 }));
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
        let (value, cycles) = c.mem.load(bus_address, width)?;
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
        let run = c.running(ops, h.idx);
        let cost = c.mem.store_cost(run, bus_address, width, Ctx::<EW>::FETCH, h.budget)?;
        if !h.charge(cost) {
            return None;
        }
        c.mem.store(bus_address, width, regs[rd]);
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
        && (0..count).any(|beat| {
            c.mem.store_reaches_code(c.running(ops, h.idx), first + 4 * beat, AccessWidth::Word)
        })
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
            let (value, _) = c.mem.load(address, AccessWidth::Word).expect("RAM block");
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
            c.mem.store(address, AccessWidth::Word, value);
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
        k::LDR..=k::STRB | k::SINGLE | k::LDR_R..k::HALF_I | k::LDR_W..=215 => {
            single_op(h, c, ops, regs, word)
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The kinds a block admits are exactly the kinds its loop runs, and
    /// every data processing kind the decoder produces without a register
    /// shift is one of them.
    #[test]
    fn block_kinds_match_the_block_loop_and_the_decoder() {
        for kind in 0..=u8::MAX {
            let mut h = Hot { budget: 0, idx: 0, cpsr: 0 };
            let ran = block_body(kind, &mut h, &mut [0; 256], &Op::default());
            assert_eq!(in_block(kind), ran, "kind {kind}");
        }
        let mut word = 0x2545_f00d_u32;
        for _ in 0..200_000 {
            word ^= word << 13;
            word ^= word >> 17;
            word ^= word << 5;
            let op = decode_unconditional(word & 0x0fff_ffff);
            if matches!(op.kind, k::DP_I..k::BCOND) {
                assert_eq!(in_block(op.kind), form(op.kind) != FX, "{word:08x}");
            }
        }
    }
}
