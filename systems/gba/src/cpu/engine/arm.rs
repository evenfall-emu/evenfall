//! ARM code running from IWRAM or EWRAM.
//!
//! A RAM code fetch costs the same whether sequential or not, so every
//! instruction costs its fetch plus its own data and internal cycles, and a
//! branch within the region adds two refill fetches.

use super::mem::{Mem, Running};
use super::{CodeCache, Load, Mmio, Op, PAGE_SHIFT, Reg, fresh_ops, split_memory};
use crate::bus::{AccessWidth, BIOS_SIZE, Bus, EWRAM_SIZE, IWRAM_SIZE, PowerState, read_ram};
use evenfall_cpu_arm::{
    Core, InstructionSet, MultiplyKind, Pipeline, condition_passes, multiply_carry,
};

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
    /// Branches to a computed address in ARM code: BX (`rn` = rm), data
    /// processing that writes r15 without S, LDR of r15 and LDM of r15
    /// without S; `x` is the instruction.
    pub const BX: u8 = 216;
    pub const DP_JUMP: u8 = 217;
    pub const LDR_JUMP: u8 = 218;
    pub const LDM_JUMP: u8 = 219;
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

const END_OP: Op = Op { kind: k::END, inner: 0, rd: Reg::R0, rn: Reg::R0, x: 0 };
const EXIT_OP: Op = Op { kind: k::EXIT, inner: 0, rd: Reg::R0, rn: Reg::R0, x: 0 };
const STOP_OP: Op = Op { kind: k::STOP, inner: 0, rd: Reg::R0, rn: Reg::R0, x: 0 };

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

/// BIOS instruction slots.
const BIOS_SLOTS: u32 = (BIOS_SIZE / 4) as u32;

/// A load from `image`, the BIOS; see [`Ctx::load_any`].
#[inline(never)]
fn load_bios(image: &[u8], address: u32, width: AccessWidth) -> Option<(u32, i64)> {
    let index = address as usize;
    let bytes = image.get(index..index + width.bytes())?;
    let value = bytes.iter().rev().fold(0, |value, &byte| value << 8 | u32::from(byte));
    Some((value, 1))
}

/// Decodes the BIOS instruction in slot `idx`: as from RAM, except that the
/// interpreter runs the last two slots, whose prefetch leaves the BIOS, and
/// branches whose refill would.
fn decode_bios(word: u32, idx: u32) -> Op {
    if idx >= BIOS_SLOTS - 2 {
        return EXIT_OP;
    }
    let op = decode(word);
    let branch = match op.kind {
        k::B | k::BL | k::BCOND..=206 => true,
        k::COND..=254 => matches!(op.inner, k::B | k::BL),
        _ => false,
    };
    if branch && idx.wrapping_add(op.x) >= BIOS_SLOTS - 1 {
        return EXIT_OP;
    }
    op
}

fn decode_unconditional(word: u32) -> Op {
    let rn = (word >> 16) & 15;
    let rd = (word >> 12) & 15;
    let raw = |kind| Op { kind, inner: 0, rd: Reg::new(rd), rn: Reg::new(rn), x: word };
    match (word >> 25) & 7 {
        0 => {
            if word & 0x0fff_fff0 == 0x012f_ff10 {
                return Op { rn: Reg::new(word), ..raw(k::BX) };
            }
            if word & 0x0fb0_fff0 == 0x0120_f000 {
                return EXIT_OP; // MSR
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
            if (rn == 15 && (!pre || writeback)) || (rd == 15 && byte) {
                return EXIT_OP;
            }
            if rd == 15 && load {
                return raw(k::LDR_JUMP);
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
            let user_bank = word & (1 << 22) != 0;
            if list == 0 || rn == 15 || (load && list & 0x8000 != 0 && user_bank) {
                EXIT_OP
            } else if load && list & 0x8000 != 0 {
                raw(k::LDM_JUMP)
            } else {
                raw(k::BLOCK)
            }
        }
        5 => Op {
            kind: if word & (1 << 24) != 0 { k::BL } else { k::B },
            inner: 0,
            rd: Reg::R0,
            rn: Reg::R0,
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
    if !flags && (8..=11).contains(&opcode) {
        return EXIT_OP;
    }
    if rd == 15 {
        // With S, an exception return: the interpreter's.
        return if flags {
            EXIT_OP
        } else {
            Op { kind: k::DP_JUMP, inner: 0, rd: Reg::R15, rn: Reg::R0, x: word }
        };
    }
    let pc_operand = Op { kind: k::DP_PC, inner: 0, rd: Reg::new(rd), rn: Reg::new(rn), x: word };
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
    Op { kind: base + opcode as u8, inner: 0, rd: Reg::new(rd), rn: Reg::new(rn), x }
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

/// The register file of a run: r0..r15 alone. [`Reg`] operands index it
/// with no bounds check, and the register numbers packed in `x` are
/// masked. The run loop is entered again after every MMIO access, so a
/// file padded for byte indices would be cleared that often (Thumb's is).
type Registers = [u32; 16];

/// Memory, caches and rarely changing state of a run of ARM code from IWRAM
/// (`EW` false) or EWRAM (`EW` true).
struct Ctx<'a, const EW: bool> {
    mem: Mem<'a>,
    /// Banked registers, for user-bank transfers.
    core: &'a mut Core,
    /// The address of `ops[0]` in the current mirror.
    base: u32,
    /// The budget left after the last load, store and MMIO access.
    load_mark: i64,
    store_mark: i64,
    mmio_mark: i64,
    /// The last load's bus value, the open-bus latch if it ends the run.
    data: u32,
    /// The MMIO transfer the run stopped at, for the driver ([`Mmio`]).
    mmio: Option<Mmio>,
    /// Whether the run executes BIOS code, in the IWRAM loop (`EW` false),
    /// and whether it may branch there from IWRAM.
    bios: bool,
    bios_ok: bool,
    /// The BIOS image in a run of BIOS code, otherwise empty.
    image: &'a [u8],
    /// A branch between the BIOS and IWRAM the run stopped after, with the
    /// state after it, for the driver to switch op arrays.
    switch: Option<(u32, Hot)>,
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

    #[inline(always)]
    fn code_word(&self, address: u32) -> u32 {
        let index = (address & Self::MASK) as usize;
        if EW {
            read_ram(self.mem.ewram, index, AccessWidth::Word)
        } else if self.bios {
            let index = address as usize & (BIOS_SIZE - 4);
            u32::from_le_bytes(self.image[index..index + 4].try_into().expect("a BIOS word"))
        } else {
            read_ram(self.mem.iwram, index, AccessWidth::Word)
        }
    }

    /// [`Mem::load`], and in a run of BIOS code BIOS loads, as
    /// [`Bus::read`] makes them with an image: the image's bytes, with no
    /// read protection, in a cycle. For out-of-line transfers.
    #[inline(always)]
    fn load_any(&self, address: u32, width: AccessWidth) -> Option<(u32, i64)> {
        if address >> 24 == 0 {
            return load_bios(self.image, address, width);
        }
        self.mem.load(address, width)
    }

    /// The region the run executes from.
    #[inline(always)]
    fn region(&self) -> u32 {
        if !EW && self.bios { 0 } else { Self::REGION }
    }

    /// The region, for the block loop.
    #[inline(always)]
    fn code(&self) -> Code<'_> {
        Code {
            slots: Self::SLOTS as u32,
            fetch: Self::FETCH,
            #[cfg(debug_assertions)]
            memory: if EW {
                &self.mem.ewram[..]
            } else if self.bios {
                self.image
            } else {
                &self.mem.iwram[..]
            },
            #[cfg(debug_assertions)]
            base: self.base,
            #[cfg(debug_assertions)]
            bios: self.bios,
            #[cfg(not(debug_assertions))]
            memory: std::marker::PhantomData,
        }
    }

    /// The running code, for store checks.
    #[inline(always)]
    fn running<'o>(&self, ops: &'o [Op], idx: u32) -> Running<'o> {
        Running { ops, region: self.region(), thumb: false, pc: self.pc(idx) }
    }

    #[inline(always)]
    fn pc(&self, idx: u32) -> u32 {
        self.base.wrapping_add(idx << 2)
    }
}

/// Runs ARM code from RAM at `core.next_instruction_address`; see
/// [`crate::cpu::Cpu::run_engine`]. With `bios_ok`, a run may branch into
/// the BIOS and back.
#[inline(never)]
pub(super) fn run<const EW: bool>(
    core: &mut Core,
    bus: &mut Bus,
    limit: u64,
    bios_ok: bool,
) -> bool {
    let pc = core.next_instruction_address;
    let mask = Ctx::<EW>::MASK;
    if pc & 3 != 0 || pc & !mask == Ctx::<EW>::LAST || !bus.game_pak_code_idle() {
        return false;
    }
    // `[EWRAM, IWRAM]` by `[ARM, Thumb]`, as `CodeCache::take` indexes.
    let region = (Ctx::<EW>::REGION & 1) as usize;
    let cached = bus.code.ram[region][0].as_ref().map(|ops| ops[((pc & mask) >> 2) as usize]);
    if cached.is_some_and(|op| refused_at_once(&op, &core.registers)) {
        return false;
    }
    drive::<EW>(core, bus, limit, pc & !mask, false, bios_ok && bus.bios.len() == BIOS_SIZE)
}

/// Runs BIOS code at `core.next_instruction_address` in the IWRAM loop,
/// from ops that stay decoded: the image never changes. Out of line, so
/// that the RAM entries keep their code.
#[cold]
#[inline(never)]
pub(super) fn run_bios(core: &mut Core, bus: &mut Bus, limit: u64) -> bool {
    let pc = core.next_instruction_address;
    if pc & 3 != 0
        || pc >= (BIOS_SLOTS - 2) * 4
        || bus.bios.len() != BIOS_SIZE
        || !bus.game_pak_code_idle()
    {
        return false;
    }
    let cached = bus.code.bios_arm.as_ref().map(|ops| ops[(pc >> 2) as usize]);
    if cached.is_some_and(|op| refused_at_once(&op, &core.registers)) {
        return false;
    }
    drive::<false>(core, bus, limit, 0, true, true)
}

/// The op array of a region, allocated the first time: RAM arrays end with
/// sentinels for the next mirror, and the BIOS array with refusals.
#[inline(always)]
fn take_ops<const EW: bool>(code: &mut CodeCache, bios: bool) -> Box<[Op]> {
    if bios {
        return code.bios_arm.take().unwrap_or_else(|| {
            let decoded = BIOS_SLOTS as usize - 2;
            fresh_ops(decoded, &vec![EXIT_OP; 2 * IWRAM_SIZE / 4 - decoded])
        });
    }
    let slots = Ctx::<EW>::SLOTS;
    code.take(Ctx::<EW>::REGION, false, || {
        let mut tail = vec![END_OP; slots];
        tail[slots - 1] = STOP_OP;
        fresh_ops(slots, &tail)
    })
}

fn put_ops<const EW: bool>(code: &mut CodeCache, bios: bool, ops: Box<[Op]>) {
    if bios {
        code.bios_arm = Some(ops);
    } else {
        code.put(Ctx::<EW>::REGION, false, ops);
    }
}

/// Runs ops from `core.next_instruction_address`, in the mirror at `base`
/// of RAM or in the BIOS, serving the MMIO transfers runs stop at and
/// switching op arrays between the BIOS and IWRAM; returns whether any
/// instruction ran.
#[inline(always)]
fn drive<const EW: bool>(
    core: &mut Core,
    bus: &mut Bus,
    limit: u64,
    base: u32,
    bios: bool,
    bios_ok: bool,
) -> bool {
    let pc = core.next_instruction_address;
    let start = bus.now();
    let budget = (limit - start) as i64;
    let mut ops = take_ops::<EW>(&mut bus.code, bios);
    let mut saved =
        Saved { base, load_mark: -1, store_mark: -1, mmio_mark: -1, data: 0, bios, bios_ok };
    let mut resume = Resume::Start;
    let exit = 'pass: loop {
        let left = limit as i64 - bus.now() as i64;
        let memory = bus.engine_memory(saved.bios);
        let image = memory.bios;
        let (mem, _) = split_memory(memory, left);
        let mut c = saved.ctx::<EW>(mem, image, core);
        let mut hot = match resume {
            Resume::Start => {
                let latched = [c.code_word(pc), c.code_word(pc.wrapping_add(4))];
                let arm = Pipeline { instructions: latched, instruction_set: InstructionSet::Arm };
                if c.core.pipeline != Some(arm) {
                    break None;
                }
                Hot { idx: pc.wrapping_sub(base) >> 2, budget, cpsr: c.core.cpsr }
            }
            Resume::On(hot) => hot,
            Resume::Out(hot) => break finish(c, hot, budget),
        };
        // Passes on this view of memory, through accesses to the registers
        // a run serves itself, which need no other.
        let (hot, access) = loop {
            hot = if EW {
                let ops = (&mut *ops).try_into().expect("EWRAM op array");
                run_loop::<EW, { 2 * EWRAM_SIZE / 4 }>(ops, &mut c, hot)
            } else {
                let ops = (&mut *ops).try_into().expect("IWRAM op array");
                run_loop::<EW, { 2 * IWRAM_SIZE / 4 }>(ops, &mut c, hot)
            };
            let Some(access) = c.mmio.take() else {
                break (hot, None);
            };
            match serve_local::<EW>(&mut c, hot, &access) {
                Some(Resume::On(next)) => hot = next,
                Some(Resume::Out(next)) => break 'pass finish(c, next, budget),
                Some(Resume::Start) => unreachable!("an access leaves a run going or out"),
                None => break (hot, Some(access)),
            }
        };
        if let Some((target, hot)) = c.switch.take() {
            #[cfg(test)]
            super::SWITCHES.with(|count| count.set(count.get() + 1));
            // Leaving the BIOS, the branch fetched the opcode two slots on.
            let latch = c.bios.then(|| c.code_word(c.pc(hot.idx).wrapping_add(8)));
            saved = c.save();
            if let Some(latch) = latch {
                bus.bios_latch = latch;
            }
            put_ops::<EW>(&mut bus.code, saved.bios, ops);
            saved.bios = target >> 24 == 0;
            saved.base = if saved.bios { 0 } else { target & !Ctx::<EW>::MASK };
            ops = take_ops::<EW>(&mut bus.code, saved.bios);
            resume = Resume::On(Hot { idx: target.wrapping_sub(saved.base) >> 2, ..hot });
            continue;
        }
        let Some(access) = access else {
            break finish(c, hot, budget);
        };
        saved = c.save();
        resume = serve::<EW>(core, bus, limit, hot, access, &mut saved);
    };
    put_ops::<EW>(&mut bus.code, saved.bios, ops);
    let Some((cycle, open_bus, data_last)) = exit else {
        return false;
    };
    bus.engine_finish(start + cycle, open_bus, data_last);
    // A run that ends in the BIOS last fetched the pipeline's second opcode.
    bus.bios_code_active = saved.bios;
    if saved.bios {
        bus.bios_latch = core.pipeline.expect("a finished run's pipeline").instructions[1];
    }
    true
}

/// What a run keeps of its [`Ctx`] while the driver has the bus back.
#[derive(Clone, Copy)]
struct Saved {
    base: u32,
    load_mark: i64,
    store_mark: i64,
    mmio_mark: i64,
    data: u32,
    bios: bool,
    bios_ok: bool,
}

impl Saved {
    fn ctx<'a, const EW: bool>(
        self,
        mem: Mem<'a>,
        image: &'a [u8],
        core: &'a mut Core,
    ) -> Ctx<'a, EW> {
        let Self { base, load_mark, store_mark, mmio_mark, data, bios, bios_ok } = self;
        Ctx {
            mem,
            image,
            core,
            base,
            load_mark,
            store_mark,
            mmio_mark,
            data,
            mmio: None,
            bios,
            bios_ok,
            switch: None,
        }
    }
}

impl<const EW: bool> Ctx<'_, EW> {
    fn save(&self) -> Saved {
        Saved {
            base: self.base,
            load_mark: self.load_mark,
            store_mark: self.store_mark,
            mmio_mark: self.mmio_mark,
            data: self.data,
            bios: self.bios,
            bios_ok: self.bios_ok,
        }
    }
}

/// Where a run goes after its driver had the bus: on from a state, or
/// out after it.
#[derive(Clone, Copy)]
enum Resume {
    Start,
    On(Hot),
    Out(Hot),
}

/// Completes the transfer to MMIO that stopped a run at `h` (see [`Mmio`]):
/// charges it as the interpreter does, makes the access on the bus and
/// writes the registers. Returns how the run goes on; the interpreter
/// takes the transfer if the bus does not let the run make it or its
/// cycles pass the limit.
fn serve<const EW: bool>(
    core: &mut Core,
    bus: &mut Bus,
    limit: u64,
    h: Hot,
    access: Mmio,
    saved: &mut Saved,
) -> Resume {
    let store = access.store.is_some();
    let fetch = Ctx::<EW>::FETCH;
    let mut next = h;
    if !access.allowed(bus) || !next.charge(fetch + if store { 1 } else { 2 }) {
        return Resume::Out(h);
    }
    let value = access.perform(bus, &mut core.registers, limit - (h.budget - fetch) as u64);
    next.next();
    saved.mmio_mark = next.budget;
    if !store {
        saved.data = value;
        saved.load_mark = next.budget;
    }
    if bus.engine_may_go_on(limit, core.cpsr & (1 << 7) != 0) {
        Resume::On(next)
    } else {
        Resume::Out(next)
    }
}

/// Completes `access` on the run's own view of the registers it reads and
/// writes itself if it touches only those ([`Mmio::local`]), charged as
/// [`serve`] charges it; None if it touches others. The run stops after a
/// store that halts, stops or raises an IRQ the CPU takes.
#[inline(always)]
fn serve_local<const EW: bool>(c: &mut Ctx<'_, EW>, h: Hot, access: &Mmio) -> Option<Resume> {
    if access.local() { complete_local::<EW>(c, h, access) } else { None }
}

/// The part of [`serve_local`] past the test of the registers.
#[inline(never)]
fn complete_local<const EW: bool>(c: &mut Ctx<'_, EW>, h: Hot, access: &Mmio) -> Option<Resume> {
    let store = access.store.is_some();
    // A store's data cycle ends one cycle after the fetch, as in [`serve`].
    if !access.local_at(&c.mem, h.budget - Ctx::<EW>::FETCH - 1) {
        return None;
    }
    let mut next = h;
    if !next.charge(Ctx::<EW>::FETCH + if store { 1 } else { 2 }) {
        return Some(Resume::Out(h));
    }
    // The data cycle follows the fetch, as in [`serve`].
    let elapsed = (c.mem.start - (h.budget - Ctx::<EW>::FETCH)) as u64;
    let value = access.perform_local(&mut c.mem, &mut c.core.registers, elapsed);
    next.next();
    c.mmio_mark = next.budget;
    if !store {
        c.data = value;
        c.load_mark = next.budget;
    }
    let irq = &c.mem.registers.irq;
    let taken = irq.irq_line && h.cpsr & (1 << 7) == 0;
    Some(if irq.power_state == PowerState::Running && !taken {
        Resume::On(next)
    } else {
        Resume::Out(next)
    })
}

/// Publishes a run's CPU state; returns the cycles it took, the open-bus
/// latch and whether its last instruction ended with a data access that
/// makes the next fetch nonsequential, or None if no instruction ran.
#[inline(never)]
fn finish<const EW: bool>(c: Ctx<'_, EW>, hot: Hot, budget: i64) -> Option<(u64, u32, bool)> {
    if hot.budget == budget {
        return None;
    }
    let pc = c.pc(hot.idx);
    let pipeline = [c.code_word(pc), c.code_word(pc.wrapping_add(4))];
    let loaded = c.load_mark == hot.budget;
    // An MMIO access leaves the next fetch sequential.
    let mmio = c.mmio_mark == hot.budget;
    let data_last = (loaded && !mmio) || c.store_mark == hot.budget;
    let open_bus = if loaded { c.data } else { pipeline[1] };
    let core = c.core;
    core.registers[15] = pc.wrapping_add(8);
    core.cpsr = hot.cpsr;
    core.next_instruction_address = pc;
    core.pipeline = Some(Pipeline { instructions: pipeline, instruction_set: InstructionSet::Arm });
    Some(((budget - hot.budget) as u64, open_bus, data_last))
}

/// Whether a run would refuse `op` as its first instruction with these
/// registers, in the frequent cases that cost less to see than a run's
/// setup: an instruction the engine never runs, and a block transfer whose
/// base points at MMIO. Refusing is always safe; this only has to be cheap
/// and usually right.
#[inline(always)]
fn refused_at_once(op: &Op, registers: &[u32; 16]) -> bool {
    match op.kind {
        k::EXIT => true,
        k::BLOCK => registers[usize::from(op.rn)] >> 24 == 4,
        _ => false,
    }
}

/// Runs an inline handler; a refused instruction ends the run.
macro_rules! go {
    ($h:ident, $c:ident, $run:expr) => {
        if !$run {
            return stopped($h.budget, $h.idx, $h.cpsr);
        }
    };
}

/// Runs an out-of-line handler, which takes and returns the hot state.
macro_rules! call {
    ($h:ident, $c:ident, $run:expr) => {{
        let next = $run;
        if next.idx == u32::MAX {
            return stopped($h.budget, $h.idx, $h.cpsr);
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
    let mut regs: Registers = c.core.registers;
    let h = run_ops(ops, c, &mut regs, state);
    c.core.registers = regs;
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
            k::STOP => return stopped(h.budget, h.idx, h.cpsr),
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
            k::BX => call!(h, c, bx(h, c, regs, op.x)),
            k::DP_JUMP => call!(h, c, dp_jump(h, c, regs, op.x)),
            k::LDR_JUMP => call!(h, c, ldr_jump(h, c, regs, op.x)),
            k::LDM_JUMP => call!(h, c, ldm_jump(h, c, regs, op.x)),
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
            | 220..=239
            | 255 => return stopped(h.budget, h.idx, h.cpsr),
        }
    }
}

#[cold]
#[inline(never)]
fn decode_slot<const EW: bool>(ops: &mut [Op], c: &mut Ctx<'_, EW>, idx: u32) {
    let offset = idx << 2;
    if c.bios {
        ops[idx as usize] = decode_bios(c.code_word(offset), idx);
        return;
    }
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
    let rm = regs[(op.x & 15) as usize];
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
            let amount = regs[((op.x >> 16) & 15) as usize] as u8;
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
    #[cfg(debug_assertions)]
    bios: bool,
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
            check_word(pc, word, _op, self.bios);
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
#[inline(never)]
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
    check_word(pc, c.code_word(pc), op, c.bios);
}

/// Asserts that a cached op at `pc` is what `word` decodes to.
#[cfg(debug_assertions)]
fn check_word(pc: u32, word: u32, op: &Op, bios: bool) {
    if matches!(op.kind, k::UNDECODED | k::END | k::STOP) {
        return;
    }
    let mut expected = if bios { decode_bios(word, pc >> 2) } else { decode(word) };
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
    let rd = Reg::new(word >> 12);
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
    rd: Reg,
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
    let multiplicand = regs[(word & 15) as usize];
    let accumulator = if accumulate { regs[((word >> 12) & 15) as usize] } else { 0 };
    let result = multiplicand.wrapping_mul(multiplier).wrapping_add(accumulator);
    regs[((word >> 16) & 15) as usize] = result;
    if word & (1 << 20) != 0 {
        let carry =
            multiply_carry(MultiplyKind::Short, multiplicand, multiplier, u64::from(accumulator));
        h.cpsr = (h.cpsr & 0x1fff_ffff)
            | (result & 0x8000_0000)
            | (u32::from(result == 0) << 30)
            | (u32::from(carry) << 29);
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
    let accumulator = if accumulate { u64::from(regs[hi]) << 32 | u64::from(regs[lo]) } else { 0 };
    let product = if signed {
        (i64::from(multiplicand as i32) * i64::from(multiplier as i32)) as u64
    } else {
        u64::from(multiplicand) * u64::from(multiplier)
    }
    .wrapping_add(accumulator);
    regs[lo] = product as u32;
    regs[hi] = (product >> 32) as u32;
    if word & (1 << 20) != 0 {
        let kind = if signed { MultiplyKind::LongSigned } else { MultiplyKind::LongUnsigned };
        let carry = multiply_carry(kind, multiplicand, multiplier, accumulator);
        h.cpsr = (h.cpsr & 0x1fff_ffff)
            | ((product >> 32) as u32 & 0x8000_0000)
            | (u32::from(product == 0) << 30)
            | (u32::from(carry) << 29);
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
    let offset = shift_immediate(regs[(op.x & 15) as usize], op.x >> 8, op.x >> 16, carry_in).0;
    let base = regs[usize::from(op.rn)];
    if op.x >> 24 != 0 { base.wrapping_sub(offset) } else { base.wrapping_add(offset) }
}

/// The address of a fast halfword transfer with a register offset.
#[inline(always)]
fn indexed(regs: &Registers, op: &Op) -> u32 {
    let offset = regs[(op.x & 15) as usize];
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
            let store = Some((regs[usize::from(op.rd)], width));
            return to_bus(c, Mmio { address, store, ..Mmio::default() });
        };
        if !h.charge(cost) {
            return Hot::REFUSED;
        }
        c.mem.store(bus_address, width, regs[usize::from(op.rd)]);
        c.store_mark = h.budget;
        h.next();
        return h;
    }
    let Some((value, cycles)) = c.load_any(bus_address, width) else {
        let load = match MODE {
            LDR => Load::Word,
            LDRB => Load::Byte,
            LDRH => Load::Half,
            LDRSB => Load::SignedByte,
            _ => Load::SignedHalf,
        };
        return to_bus(c, Mmio { address, load: Some((op.rd, load)), ..Mmio::default() });
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
    if !transfer::<EW, MODE>(h, c, ops, regs, op, address) {
        return with_writeback(c, op.rn, address);
    }
    regs[usize::from(op.rn)] = address;
    true
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
    if !transfer::<EW, MODE>(h, c, ops, regs, op, base) {
        return with_writeback(c, op.rn, base.wrapping_add(op.x));
    }
    regs[usize::from(op.rn)] = base.wrapping_add(op.x);
    true
}

/// A transfer the run cannot make: to MMIO, for the driver to complete,
/// otherwise for the interpreter.
#[cold]
#[inline(never)]
fn to_bus<const EW: bool>(c: &mut Ctx<'_, EW>, access: Mmio) -> Hot {
    if access.address >> 24 == 4 {
        c.mmio = Some(access);
    }
    Hot::REFUSED
}

/// Adds the base register write of a writeback transfer to the MMIO access
/// it left to the driver, if any; the run stops either way.
#[cold]
#[inline(never)]
fn with_writeback<const EW: bool>(c: &mut Ctx<'_, EW>, rn: Reg, base: u32) -> bool {
    if let Some(access) = &mut c.mmio {
        access.writeback = Some((rn, base));
    }
    false
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
        let Some((value, cycles)) = c.load_any(bus_address, width) else {
            let load = Some((Reg::new(rd as u32), if byte { Load::Byte } else { Load::Word }));
            let writeback = (writeback && rn != rd).then_some((Reg::new(rn as u32), adjusted));
            to_bus(c, Mmio { address, load, writeback, ..Mmio::default() });
            return None;
        };
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
        let value = regs[rd].wrapping_add(if rd == 15 { 4 } else { 0 });
        let Some(cost) = c.mem.store_cost(run, bus_address, width, Ctx::<EW>::FETCH, h.budget)
        else {
            let writeback = writeback.then_some((Reg::new(rn as u32), adjusted));
            to_bus(c, Mmio { address, store: Some((value, width)), writeback, load: None });
            return None;
        };
        if !h.charge(cost) {
            return None;
        }
        c.mem.store(bus_address, width, value);
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
        let Some((value, cycles)) = c.load_any(bus_address, width) else {
            let load = [Load::Half, Load::Half, Load::SignedByte, Load::SignedHalf];
            let load = Some((Reg::new(rd as u32), load[operation as usize]));
            let writeback = (writeback && rn != rd).then_some((Reg::new(rn as u32), adjusted));
            to_bus(c, Mmio { address, load, writeback, ..Mmio::default() });
            return None;
        };
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
        let Some(cost) = c.mem.store_cost(run, bus_address, width, Ctx::<EW>::FETCH, h.budget)
        else {
            let writeback = writeback.then_some((Reg::new(rn as u32), adjusted));
            to_bus(c, Mmio { address, store: Some((regs[rd], width)), writeback, load: None });
            return None;
        };
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
    // A block spans at most two code pages: most stores reach neither.
    if !load_list
        && (c.mem.holds_code(first) || c.mem.holds_code(last))
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
        user_bank_block(c, regs, word, first, final_base);
    } else {
        let ram: &mut [u8] =
            if first >> 24 == 3 { &mut c.mem.iwram[..] } else { &mut c.mem.ewram[..] };
        let mask = ram.len() - 4;
        let mut address = first as usize;
        let mut remaining = list;
        let mut index = 0;
        while remaining != 0 {
            let register = remaining.trailing_zeros() as usize;
            remaining &= remaining - 1;
            let at = address & mask;
            let slot: &mut [u8; 4] = (&mut ram[at..at + 4]).try_into().expect("a word");
            if load_list {
                let value = u32::from_le_bytes(*slot);
                regs[register & 15] = value;
                c.data = value;
            } else {
                let value = if register == 15 {
                    regs[15].wrapping_add(4)
                } else if writeback && register as u32 == rn && index != 0 {
                    final_base
                } else {
                    regs[register & 15]
                };
                *slot = value.to_le_bytes();
            }
            address = address.wrapping_add(4);
            index += 1;
        }
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

/// The transfers of an LDM or STM with S that moves user-bank registers,
/// from `first` in one RAM region.
#[cold]
#[inline(never)]
fn user_bank_block<const EW: bool>(
    c: &mut Ctx<'_, EW>,
    regs: &mut Registers,
    word: u32,
    first: u32,
    final_base: u32,
) {
    let list = word & 0xffff;
    let rn = (word >> 16) & 15;
    let load_list = word & (1 << 20) != 0;
    let writeback = word & (1 << 21) != 0;
    c.core.registers.copy_from_slice(&regs[..16]);
    let mut address = first;
    let mut remaining = list;
    let mut index = 0;
    while remaining != 0 {
        let register = remaining.trailing_zeros() as usize;
        remaining &= remaining - 1;
        if load_list {
            let (value, _) = c.mem.load(address, AccessWidth::Word).expect("RAM block");
            c.core.write_user_bank_register(register, value);
            c.data = value;
        } else {
            let value = if register == 15 {
                regs[15].wrapping_add(4)
            } else if writeback && register as u32 == rn && index != 0 {
                final_base
            } else {
                c.core.read_user_bank_register(register)
            };
            c.mem.store(address, AccessWidth::Word, value);
        }
        address = address.wrapping_add(4);
        index += 1;
    }
    regs[..16].copy_from_slice(&c.core.registers);
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
        k::BX => bx_op(h, c, regs, word),
        k::DP_JUMP => dp_jump_op(h, c, regs, word),
        k::LDR_JUMP => ldr_jump_op(h, c, regs, word),
        k::LDM_JUMP => ldm_jump_op(h, c, regs, word),
        _ => None,
    }
}

/// Where an indirect branch to ARM code lands.
enum Landing {
    /// A slot of the running mirror.
    Here(u32),
    /// Another mirror of the same RAM region, at this base and slot.
    Mirror(u32, u32),
    /// The BIOS from IWRAM or IWRAM from the BIOS, which share the IWRAM
    /// loop: the driver switches op arrays ([`Ctx::switch`]).
    Switch,
}

/// Where a branch to word-aligned ARM `target` lands, if the run may follow
/// it: into its own region, or between the BIOS and IWRAM; not where the
/// refill's two fetches would leave the region.
#[inline(always)]
fn landing<const EW: bool>(c: &Ctx<'_, EW>, target: u32) -> Option<Landing> {
    let bios_code = target >> 24 == 0 && target < (BIOS_SLOTS - 2) * 4;
    if !EW && c.bios {
        return if bios_code {
            Some(Landing::Here(target >> 2))
        } else if target >> 24 == 3 && target & !Ctx::<EW>::MASK != Ctx::<EW>::LAST {
            Some(Landing::Switch)
        } else {
            None
        };
    }
    if target >> 24 == Ctx::<EW>::REGION && target & !Ctx::<EW>::MASK != Ctx::<EW>::LAST {
        let idx = (target & Ctx::<EW>::MASK) >> 2;
        let base = target & !Ctx::<EW>::MASK;
        return Some(if base == c.base { Landing::Here(idx) } else { Landing::Mirror(base, idx) });
    }
    (!EW && bios_code && c.bios_ok).then_some(Landing::Switch)
}

/// Completes an indirect branch whose instruction cost `cost` before the
/// refill, after `landing` admitted `target` and `h` was charged: the next
/// slot, or for a switch the state the driver resumes from.
#[inline(always)]
fn land<const EW: bool>(mut h: Hot, c: &mut Ctx<'_, EW>, target: u32, landing: Landing) -> Hot {
    match landing {
        Landing::Here(idx) => h.idx = idx,
        Landing::Mirror(base, idx) => {
            c.base = base;
            h.idx = idx;
        }
        Landing::Switch => {
            c.switch = Some((target, h));
            return Hot::REFUSED;
        }
    }
    h
}

/// BX to ARM code; BX to Thumb is the interpreter's.
#[inline(always)]
fn bx_op<const EW: bool>(
    mut h: Hot,
    c: &mut Ctx<'_, EW>,
    regs: &mut Registers,
    word: u32,
) -> Option<Hot> {
    regs[15] = c.pc(h.idx).wrapping_add(8);
    let target = regs[(word & 15) as usize];
    if target & 1 != 0 {
        return None;
    }
    let target = target & !3;
    let landing = landing(c, target)?;
    if !h.charge(3 * Ctx::<EW>::FETCH) {
        return None;
    }
    Some(land(h, c, target, landing))
}

/// Data processing that writes r15 without S (MOV pc, lr and jump tables).
#[inline(always)]
fn dp_jump_op<const EW: bool>(
    mut h: Hot,
    c: &mut Ctx<'_, EW>,
    regs: &mut Registers,
    word: u32,
) -> Option<Hot> {
    let register_shift = word & (1 << 25) == 0 && word & 0x10 != 0;
    // Evaluate into r15 from a scratch state (without S, no flag changes),
    // then leave r15 as the instruction reads it.
    dp_pc_op(Hot { budget: i64::MAX, ..h }, c, regs, word)?;
    let target = regs[15] & !3;
    regs[15] = c.pc(h.idx).wrapping_add(8);
    let landing = landing(c, target)?;
    if !h.charge(3 * Ctx::<EW>::FETCH + i64::from(register_shift)) {
        return None;
    }
    Some(land(h, c, target, landing))
}

/// LDR of r15 from memory the run reads (not MMIO).
#[inline(always)]
fn ldr_jump_op<const EW: bool>(
    mut h: Hot,
    c: &mut Ctx<'_, EW>,
    regs: &mut Registers,
    word: u32,
) -> Option<Hot> {
    regs[15] = c.pc(h.idx).wrapping_add(8);
    let pre = word & (1 << 24) != 0;
    let writeback = !pre || word & (1 << 21) != 0;
    let rn = ((word >> 16) & 15) as usize;
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
    let (value, cycles) = c.load_any(address & !3, AccessWidth::Word)?;
    let target = value.rotate_right((address & 3) * 8) & !3;
    let landing = landing(c, target)?;
    if !h.charge(Ctx::<EW>::FETCH + cycles + 1 + 2 * Ctx::<EW>::FETCH) {
        return None;
    }
    if writeback {
        regs[rn] = adjusted;
    }
    Some(land(h, c, target, landing))
}

/// LDM of r15 without S, within one RAM region.
#[inline(always)]
fn ldm_jump_op<const EW: bool>(
    mut h: Hot,
    c: &mut Ctx<'_, EW>,
    regs: &mut Registers,
    word: u32,
) -> Option<Hot> {
    let list = word & 0xffff;
    let count = list.count_ones();
    let rn = ((word >> 16) & 15) as usize;
    let writeback = word & (1 << 21) != 0;
    let pre = word & (1 << 24) != 0;
    let base = regs[rn];
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
    let (value, _) = c.mem.load(last, AccessWidth::Word)?;
    let target = value & !3;
    let landing = landing(c, target)?;
    if !h.charge(Ctx::<EW>::FETCH + i64::from(count) * per_word + 1 + 2 * Ctx::<EW>::FETCH) {
        return None;
    }
    let mut address = first;
    let mut remaining = list & 0x7fff;
    while remaining != 0 {
        let register = remaining.trailing_zeros() as usize;
        remaining &= remaining - 1;
        regs[register & 15] = c.mem.load(address, AccessWidth::Word).expect("RAM block").0;
        address = address.wrapping_add(4);
    }
    if writeback && list & (1 << rn) == 0 {
        regs[rn] = final_base;
    }
    Some(land(h, c, target, landing))
}

/// Out-of-line entry points for handlers too large or too rare to inline
/// into the dispatch loop. They take and return the hot state by value.
#[inline(never)]
fn dp_pc<const EW: bool>(h: Hot, c: &mut Ctx<'_, EW>, regs: &mut Registers, word: u32) -> Hot {
    dp_pc_op(h, c, regs, word).unwrap_or(Hot::REFUSED)
}

#[inline(never)]
fn bx<const EW: bool>(h: Hot, c: &mut Ctx<'_, EW>, regs: &mut Registers, word: u32) -> Hot {
    bx_op(h, c, regs, word).unwrap_or(Hot::REFUSED)
}

#[inline(never)]
fn dp_jump<const EW: bool>(h: Hot, c: &mut Ctx<'_, EW>, regs: &mut Registers, word: u32) -> Hot {
    dp_jump_op(h, c, regs, word).unwrap_or(Hot::REFUSED)
}

#[inline(never)]
fn ldr_jump<const EW: bool>(h: Hot, c: &mut Ctx<'_, EW>, regs: &mut Registers, word: u32) -> Hot {
    ldr_jump_op(h, c, regs, word).unwrap_or(Hot::REFUSED)
}

#[inline(never)]
fn ldm_jump<const EW: bool>(h: Hot, c: &mut Ctx<'_, EW>, regs: &mut Registers, word: u32) -> Hot {
    ldm_jump_op(h, c, regs, word).unwrap_or(Hot::REFUSED)
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
/// many exits of the dispatch loop apart from its hot path. Its fields come
/// in registers: a state stored field by field and passed by reference
/// would be read back whole, which stalls store forwarding.
#[cold]
#[inline(never)]
fn stopped(budget: i64, idx: u32, cpsr: u32) -> Hot {
    Hot { budget, idx, cpsr }
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
            let ran = block_body(kind, &mut h, &mut [0; 16], &Op::default());
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
