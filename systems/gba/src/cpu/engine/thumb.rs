//! Thumb code running from IWRAM, EWRAM or Game Pak ROM.
//!
//! Flags are lazy: N and Z come from a sign-extended result, C from a bit
//! and V from bit 31 of an overflow word, so a flag-setting instruction
//! stores results instead of assembling the CPSR.
//!
//! RAM fetches cost the same whether sequential or not. A ROM fetch without
//! prefetch is sequential unless the instruction before it accessed data or
//! spent internal cycles, so such a run carries the cost of its next fetch.
//! With the prefetch buffer enabled, a run follows the Thumb ROM kernel's
//! closed-form schedule of the buffer (see [`PrefetchClock`]), expressed in
//! the run's budget.

use super::mem::{Mem, Running};
use super::{Op, PAGE_SHIFT, ROM_PAGE_SLOTS, fresh_ops, split_memory};
use crate::bus::{
    AccessKind, AccessWidth, Bus, EWRAM_SIZE, IWRAM_SIZE, PrefetchClock, RomData, read_ram,
};
use evenfall_cpu_arm::{Core, InstructionSet, Pipeline};

/// Op kinds; ALU, register-offset transfer and conditional branch kinds are
/// a base plus the opcode or condition.
mod k {
    pub const UNDECODED: u8 = 0;
    /// The interpreter executes this instruction.
    pub const EXIT: u8 = 1;
    /// Shifts by an immediate: MOVS (LSL #0), then LSL, LSR and ASR with
    /// `x` = 1..32.
    pub const MOVS: u8 = 2;
    pub const LSL: u8 = 3;
    pub const LSR: u8 = 4;
    pub const ASR: u8 = 5;
    /// ADD/SUB with a register (`x` = rm) or a 3-bit immediate (`x`).
    pub const ADD_R: u8 = 6;
    pub const SUB_R: u8 = 7;
    pub const ADD_3: u8 = 8;
    pub const SUB_3: u8 = 9;
    /// MOV/CMP/ADD/SUB with an 8-bit immediate `x`.
    pub const MOV_I: u8 = 10;
    pub const CMP_I: u8 = 11;
    pub const ADD_I: u8 = 12;
    pub const SUB_I: u8 = 13;
    /// High-register ADD, CMP and MOV that do not write r15.
    pub const ADD_H: u8 = 14;
    pub const CMP_H: u8 = 15;
    pub const MOV_H: u8 = 16;
    /// LDR rd, [pc, #x].
    pub const LDR_PC: u8 = 17;
    /// ADD rd, pc/sp, #x and ADD sp, #x (signed).
    pub const ADD_PC: u8 = 18;
    pub const ADD_SP: u8 = 19;
    pub const ADJ_SP: u8 = 20;
    /// PUSH (`x` = list | lr << 8), POP, POP with pc, STMIA and LDMIA.
    pub const PUSH: u8 = 21;
    pub const POP: u8 = 22;
    pub const POP_PC: u8 = 23;
    pub const STMIA: u8 = 24;
    pub const LDMIA: u8 = 25;
    /// B (`x` = the target's slot distance), BL's halves (`x` = the offset).
    pub const B: u8 = 26;
    pub const BL_HI: u8 = 27;
    pub const BL_LO: u8 = 28;
    /// Branches through a high register: ADD pc, MOV pc, and BX to Thumb.
    pub const ADD_PC_H: u8 = 29;
    pub const MOV_PC_H: u8 = 30;
    pub const BX: u8 = 31;
    /// Format 4 ALU operations, plus the opcode.
    pub const ALU: u8 = 32;
    /// Register-offset transfers, plus the opcode (STR, STRH, STRB, LDRSB,
    /// LDR, LDRH, LDRB, LDRSH); `x` = rm.
    pub const REG: u8 = 48;
    /// Immediate-offset transfers: STR, LDR, STRH, LDRH, STRB, LDRB (the
    /// SP-relative ones too); `x` = offset.
    pub const IMM: u8 = 56;
    /// Conditional B, plus the condition.
    pub const BCOND: u8 = 64;
}

/// Transfer modes, in the order of the register-offset opcodes.
const STR: u8 = 0;
const STRH: u8 = 1;
const STRB: u8 = 2;
const LDRSB: u8 = 3;
const LDR: u8 = 4;
const LDRH: u8 = 5;
const LDRB: u8 = 6;
const LDRSH: u8 = 7;

const EXIT_OP: Op = Op { kind: k::EXIT, inner: 0, rd: 0, rn: 0, x: 0 };

/// Decodes one Thumb instruction.
pub(super) fn decode(i: u16) -> Op {
    let op =
        |kind: u8, rd: u16, rn: u16, x: u32| Op { kind, inner: 0, rd: rd as u8, rn: rn as u8, x };
    let (low, mid) = (i & 7, (i >> 3) & 7);
    let high = (i >> 8) & 7;
    let imm5 = u32::from((i >> 6) & 31);
    let imm8 = u32::from(i & 255);
    match i >> 8 {
        0x00..=0x07 if imm5 == 0 => op(k::MOVS, low, mid, 0),
        0x00..=0x07 => op(k::LSL, low, mid, imm5),
        0x08..=0x0f => op(k::LSR, low, mid, if imm5 == 0 { 32 } else { imm5 }),
        0x10..=0x17 => op(k::ASR, low, mid, if imm5 == 0 { 32 } else { imm5 }),
        0x18..=0x1f => {
            let kind = [k::ADD_R, k::SUB_R, k::ADD_3, k::SUB_3][usize::from((i >> 9) & 3)];
            op(kind, low, mid, u32::from((i >> 6) & 7))
        }
        0x20..=0x3f => {
            let kind = [k::MOV_I, k::CMP_I, k::ADD_I, k::SUB_I][usize::from((i >> 11) & 3)];
            op(kind, high, high, imm8)
        }
        0x40..=0x43 => op(k::ALU + ((i >> 6) & 15) as u8, low, mid, 0),
        0x44..=0x47 => {
            let rd = (i & 7) | ((i >> 4) & 8);
            let rs = (i >> 3) & 15;
            match (i >> 8) & 3 {
                0 if rd != 15 => op(k::ADD_H, rd, rs, 0),
                0 => op(k::ADD_PC_H, 0, rs, 0),
                1 => op(k::CMP_H, rd, rs, 0),
                2 if rd != 15 => op(k::MOV_H, rd, rs, 0),
                2 => op(k::MOV_PC_H, 0, rs, 0),
                _ => op(k::BX, 0, rs, 0),
            }
        }
        0x48..=0x4f => op(k::LDR_PC, high, 0, imm8 << 2),
        0x50..=0x5f => op(k::REG + ((i >> 9) & 7) as u8, low, mid, u32::from((i >> 6) & 7)),
        0x60..=0x6f => op(k::IMM + mode(i, STR, LDR), low, mid, imm5 << 2),
        0x70..=0x7f => op(k::IMM + mode(i, STRB, LDRB), low, mid, imm5),
        0x80..=0x8f => op(k::IMM + mode(i, STRH, LDRH), low, mid, imm5 << 1),
        0x90..=0x9f => op(k::IMM + mode(i, STR, LDR), high, 13, imm8 << 2),
        0xa0..=0xa7 => op(k::ADD_PC, high, 0, imm8 << 2),
        0xa8..=0xaf => op(k::ADD_SP, high, 0, imm8 << 2),
        0xb0 => {
            let offset = u32::from(i & 127) << 2;
            op(k::ADJ_SP, 0, 0, if i & 0x80 != 0 { offset.wrapping_neg() } else { offset })
        }
        0xb4 | 0xb5 if i & 0x1ff != 0 => op(k::PUSH, 0, 0, u32::from(i & 0x1ff)),
        0xbc if imm8 != 0 => op(k::POP, 0, 0, imm8),
        0xbd => op(k::POP_PC, 0, 0, imm8),
        0xc0..=0xc7 if imm8 != 0 => op(k::STMIA, 0, high, imm8),
        0xc8..=0xcf if imm8 != 0 => op(k::LDMIA, 0, high, imm8),
        0xd0..=0xdd => op(k::BCOND + ((i >> 8) & 15) as u8, 0, 0, (i as i8 as i32 + 2) as u32),
        0xe0..=0xe7 => op(k::B, 0, 0, ((((i as u32) << 21) as i32 >> 21) + 2) as u32),
        0xf0..=0xf7 => op(k::BL_HI, 0, 0, (((i as u32) << 21) as i32 >> 9) as u32),
        0xf8..=0xff => op(k::BL_LO, 0, 0, u32::from(i & 0x7ff) << 1),
        _ => EXIT_OP,
    }
}

/// The immediate-transfer kind offset of a store or a load.
fn mode(i: u16, store: u8, load: u8) -> u8 {
    let mode = if i & (1 << 11) != 0 { load } else { store };
    [0, 4, 1, 5, 2, 6, 3, 7].iter().position(|&m| m == mode).expect("transfer mode") as u8
}

/// How an instruction's fetches are priced.
trait Timing: Copy {
    /// The fetch of the instruction about to run.
    fn fetch(self, h: &Hot) -> i64;
    /// Advances fetch state after an instruction that ran from budget
    /// `before`; `breaking` if it accessed data or spent internal cycles.
    fn after(self, h: &mut Hot, before: i64, breaking: bool);
    /// A branch's two refill fetches at `target`, from the state after the
    /// branch instruction; `head` is the address the buffer was fetching.
    fn refill(self, h: &mut Hot, target: u32, head: u32) -> i64;
    /// Extra cycles of a ROM data load by an instruction that runs from
    /// budget `before`, or None if the engine must leave it.
    fn rom_load(
        self,
        h: &Hot,
        before: i64,
        rom: &RomData<'_>,
        address: u32,
        width: AccessWidth,
    ) -> Option<i64>;
    /// A ROM data load stopped the buffer.
    fn discard(self, h: &mut Hot, c: &mut Ctx<'_>);
}

#[derive(Clone, Copy)]
struct Ram<const EW: bool>;

impl<const EW: bool> Timing for Ram<EW> {
    #[inline(always)]
    fn fetch(self, _h: &Hot) -> i64 {
        if EW { 3 } else { 1 }
    }
    #[inline(always)]
    fn after(self, _h: &mut Hot, _before: i64, _breaking: bool) {}
    #[inline(always)]
    fn refill(self, h: &mut Hot, _target: u32, _head: u32) -> i64 {
        2 * self.fetch(h)
    }
    #[inline(always)]
    fn rom_load(self, _: &Hot, _: i64, _: &RomData<'_>, _: u32, _: AccessWidth) -> Option<i64> {
        Some(0)
    }
    #[inline(always)]
    fn discard(self, _h: &mut Hot, _c: &mut Ctx<'_>) {}
}

/// Game Pak ROM with the prefetch buffer disabled: every fetch is a plain
/// bus access, sequential unless the previous instruction broke the burst.
#[derive(Clone, Copy)]
struct Rom {
    n: i64,
    s: i64,
}

impl Timing for Rom {
    #[inline(always)]
    fn fetch(self, h: &Hot) -> i64 {
        h.fetch
    }
    #[inline(always)]
    fn after(self, h: &mut Hot, _before: i64, breaking: bool) {
        h.fetch = if breaking { self.n } else { self.s };
    }
    #[inline(always)]
    fn refill(self, h: &mut Hot, target: u32, _head: u32) -> i64 {
        h.fetch = self.s;
        self.n + if (target + 2) & 0x1ffff == 0 { self.n } else { self.s }
    }
    #[inline(always)]
    fn rom_load(self, _: &Hot, _: i64, _: &RomData<'_>, _: u32, _: AccessWidth) -> Option<i64> {
        Some(0)
    }
    #[inline(always)]
    fn discard(self, _h: &mut Hot, _c: &mut Ctx<'_>) {}
}

/// Game Pak ROM with the prefetch buffer enabled. In budget terms, the
/// buffer head is ready when the budget reaches `h.ready`; each fetch waits
/// for it and the buffer runs one `duty` ahead, at most `fill` early.
#[derive(Clone, Copy)]
struct Prefetch {
    duty: i64,
    miss: i64,
    fill: i64,
}

impl Timing for Prefetch {
    #[inline(always)]
    fn fetch(self, h: &Hot) -> i64 {
        (h.budget - h.ready).max(1)
    }
    #[inline(always)]
    fn after(self, h: &mut Hot, before: i64, _breaking: bool) {
        h.ready = h.ready.min(before + self.fill) - self.duty;
    }
    #[inline(always)]
    fn refill(self, h: &mut Hot, target: u32, head: u32) -> i64 {
        let clock = h.budget;
        let offset = target.wrapping_sub(head);
        if offset != 0 {
            let lands = h.ready.min(clock + self.fill) - i64::from(offset / 2) * self.duty;
            h.ready = if offset < 16 && lands < clock && lands + self.duty >= clock {
                lands
            } else {
                clock - self.miss
            };
        }
        let mut budget = clock;
        for _ in 0..2 {
            let before = budget;
            budget = (before - 1).min(h.ready);
            h.ready = h.ready.min(before + self.fill) - self.duty;
        }
        clock - budget
    }
    #[inline(always)]
    fn rom_load(
        self,
        h: &Hot,
        before: i64,
        rom: &RomData<'_>,
        address: u32,
        width: AccessWidth,
    ) -> Option<i64> {
        if address >= 0x0d00_0000 || !rom.plain(address, width.bytes() as u32) {
            return None;
        }
        // A data access that meets a buffer fetch in flight waits a cycle.
        let fetched = before - (before - h.ready).max(1);
        let ready = h.ready.min(before + self.fill) - self.duty;
        let next = fetched - 1;
        let ahead = ready - next;
        Some(i64::from(ahead >= 0 && ahead <= self.fill && ahead % self.duty == 0))
    }
    #[inline(always)]
    fn discard(self, h: &mut Hot, c: &mut Ctx<'_>) {
        // The load's internal cycle ends as the buffer restarts.
        h.ready = h.budget - self.miss;
        c.discard_mark = h.budget;
    }
}

/// The register file of a run, indexed by any byte.
type Registers = [u32; 256];

/// Where the code of a run lives.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    Iwram,
    Ewram,
    Rom,
}

impl Place {
    fn region(self) -> u32 {
        match self {
            Self::Ewram => 2,
            Self::Iwram => 3,
            Self::Rom => 8,
        }
    }
}

/// Memory, caches and rarely changing state of a run.
struct Ctx<'a> {
    mem: Mem<'a>,
    place: Place,
    /// The address of the page's first slot.
    base: u32,
    /// The budget left after the last load, store, internal cycle and
    /// buffer discard.
    load_mark: i64,
    store_mark: i64,
    internal_mark: i64,
    discard_mark: i64,
    /// The last load's bus value, the open-bus latch if it ends the run.
    data: u32,
    /// Whether every slot of the page and the halfwords after them are
    /// plain code, so that a branch within it needs no target check.
    clean: bool,
}

impl Ctx<'_> {
    #[inline(always)]
    fn pc(&self, idx: u32) -> u32 {
        self.base.wrapping_add(idx << 1)
    }

    /// The halfword the interpreter would have fetched at `address`.
    fn half(&self, address: u32) -> u32 {
        match self.place {
            Place::Iwram => {
                read_ram(self.mem.iwram, address as usize & (IWRAM_SIZE - 1), AccessWidth::Half)
            }
            Place::Ewram => {
                read_ram(self.mem.ewram, address as usize & (EWRAM_SIZE - 1), AccessWidth::Half)
            }
            Place::Rom => self.mem.rom.half(address),
        }
    }

    /// The instruction at `address` if the engine may run it.
    fn code(&self, address: u32) -> Option<u16> {
        match self.place {
            Place::Rom => self.mem.rom.thumb_code(address),
            _ => Some(self.half(address) as u16),
        }
    }
}

/// The per-instruction state, kept in host registers.
#[derive(Clone, Copy)]
struct Hot {
    /// Cycles left before the run's limit.
    budget: i64,
    idx: u32,
    /// C, as 0 or 1.
    carry: u32,
    /// N and Z: the last result sign-extended, or `N_AND_Z`.
    nz: i64,
    /// V, in bit 31.
    overflow: u32,
    /// The cost of the next fetch, without prefetch.
    fetch: i64,
    /// The budget at which the prefetch buffer's head is ready.
    ready: i64,
}

/// N and Z both set, which no result produces.
const N_AND_Z: i64 = 0xffff_ffff_0000_0000_u64 as i64;

impl Hot {
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
    fn set_nz(&mut self, result: u32) {
        self.nz = i64::from(result as i32);
    }

    #[inline(always)]
    fn set_add(&mut self, lhs: u32, rhs: u32, result: u32) {
        self.set_nz(result);
        self.carry = u32::from(result < lhs);
        self.overflow = !(lhs ^ rhs) & (lhs ^ result);
    }

    #[inline(always)]
    fn set_sub(&mut self, lhs: u32, rhs: u32, result: u32) {
        self.set_nz(result);
        self.carry = u32::from(lhs >= rhs);
        self.overflow = (lhs ^ rhs) & (lhs ^ result);
    }

    #[inline(always)]
    fn set_shift(&mut self, result: u32, carry: Option<bool>) {
        self.set_nz(result);
        if let Some(carry) = carry {
            self.carry = u32::from(carry);
        }
    }

    #[inline(always)]
    fn condition<const C: u8>(&self) -> bool {
        let (n, z) = (self.nz < 0, self.nz as u32 == 0);
        let (c, v) = (self.carry != 0, (self.overflow as i32) < 0);
        match C {
            0 => z,
            1 => !z,
            2 => c,
            3 => !c,
            4 => n,
            5 => !n,
            6 => v,
            7 => !v,
            8 => c && !z,
            9 => !c || z,
            10 => n == v,
            11 => n != v,
            12 => !z && n == v,
            _ => z || n != v,
        }
    }

    fn from_cpsr(budget: i64, idx: u32, cpsr: u32) -> Self {
        let nz = match (cpsr >> 30) & 3 {
            0 => 1,
            1 => 0,
            2 => -1,
            _ => N_AND_Z,
        };
        Self {
            budget,
            idx,
            carry: (cpsr >> 29) & 1,
            nz,
            overflow: (cpsr & (1 << 28)) << 3,
            fetch: 0,
            ready: 0,
        }
    }

    fn cpsr(&self, cpsr: u32) -> u32 {
        let n = u32::from(self.nz < 0);
        let z = u32::from(self.nz as u32 == 0);
        (cpsr & 0x0fff_ffff) | n << 31 | z << 30 | self.carry << 29 | (self.overflow >> 31) << 28
    }
}

/// Returns an overdrawn budget to where it was, off the hot path.
#[cold]
#[inline(never)]
fn refund(budget: i64, cycles: i64) -> i64 {
    budget + cycles
}

/// How a run of one page ended.
enum Stop {
    /// Before an instruction the engine does not run.
    Refused,
    /// After a branch to another page.
    Far(u32),
}

/// Runs Thumb code at `core.next_instruction_address`; see
/// [`crate::cpu::Cpu::run_engine`].
pub(super) fn run(core: &mut Core, bus: &mut Bus, limit: u64) -> bool {
    let pc = core.next_instruction_address;
    match pc >> 24 {
        2 => run_ram::<true>(core, bus, limit),
        3 => run_ram::<false>(core, bus, limit),
        8..=13 => run_rom(core, bus, limit),
        _ => false,
    }
}

fn run_ram<const EW: bool>(core: &mut Core, bus: &mut Bus, limit: u64) -> bool {
    let pc = core.next_instruction_address;
    let (region, size) = if EW { (2, EWRAM_SIZE) } else { (3, IWRAM_SIZE) };
    let mask = size as u32 - 1;
    let last_mirror = region << 24 | (0x00ff_ffff & !mask);
    if pc & 1 != 0 || pc & !mask == last_mirror || !bus.game_pak_code_idle() {
        return false;
    }
    let start = bus.now();
    let budget = (limit - start) as i64;
    let memory = bus.engine_memory();
    let mut ops = memory.code.take(region, true, || fresh_ops(size / 2, &[]));
    let (mem, _) = split_memory(memory, budget);
    let mut c = Ctx {
        mem,
        place: if EW { Place::Ewram } else { Place::Iwram },
        base: pc & !mask,
        load_mark: -1,
        store_mark: -1,
        internal_mark: -1,
        discard_mark: -1,
        data: 0,
        clean: true,
    };
    let mut exit = None;
    if let Some(mut hot) = enter(core, &c, pc, budget) {
        loop {
            let (next, stop) = if EW {
                let ops = (&mut *ops).try_into().expect("EWRAM Thumb ops");
                run_loop::<{ EWRAM_SIZE / 2 }, _>(ops, &mut c, core, Ram::<EW>, hot)
            } else {
                let ops = (&mut *ops).try_into().expect("IWRAM Thumb ops");
                run_loop::<{ IWRAM_SIZE / 2 }, _>(ops, &mut c, core, Ram::<EW>, hot)
            };
            hot = next;
            match stop {
                Stop::Refused => break,
                // A branch into another mirror: same ops, new base.
                Stop::Far(target) => {
                    c.base = target & !mask;
                    hot.idx = (target & mask) >> 1;
                }
            }
        }
        exit = leave(core, &c, hot, start, budget);
    }
    bus.code.put(region, true, ops);
    let Some(exit) = exit else {
        return false;
    };
    bus.engine_finish(exit.cycle, exit.open_bus, exit.data_last);
    true
}

fn run_rom(core: &mut Core, bus: &mut Bus, limit: u64) -> bool {
    let pc = core.next_instruction_address;
    if pc & 1 != 0 {
        return false;
    }
    let start = bus.now();
    let budget = (limit - start) as i64;
    let first_fetch_n =
        bus.fetch_kind == AccessKind::Nonsequential || bus.prefetch_disabled_bug_pending;
    let prefetch = if bus.waitcnt & (1 << 14) != 0 {
        let Some(clock) = bus.thumb_prefetch_entry(pc) else {
            return false;
        };
        Some(clock)
    } else if bus.game_pak_prefetch.active || bus.prefetch_buffer_was_disabled {
        return false;
    } else {
        None
    };
    let memory = bus.engine_memory();
    let (n, s) = memory.rom.window_cycles(pc);
    let (mem, pages) = split_memory(memory, budget);
    if pages.is_empty() {
        pages.resize_with(mem.rom.len().div_ceil(PAGE_BYTES as usize).max(1), || None);
    }
    let Some(slot) = pages.get_mut(page_of(pc)) else {
        return false;
    };
    let ops = slot.take().unwrap_or_else(rom_page);
    let mut c = Ctx {
        mem,
        place: Place::Rom,
        base: pc & !(PAGE_BYTES - 1),
        load_mark: -1,
        store_mark: -1,
        internal_mark: -1,
        discard_mark: -1,
        data: 0,
        clean: false,
    };
    c.clean = clean_page(&c);
    let (exit, ready) = match (enter(core, &c, pc, budget), prefetch) {
        (None, _) => {
            pages[page_of(pc)] = Some(ops);
            return false;
        }
        (Some(mut hot), None) => {
            hot.fetch = (if first_fetch_n { n } else { s }) as i64;
            let timing = Rom { n: n as i64, s: s as i64 };
            rom_runs(core, &mut c, pages, ops, hot, timing, start, budget)
        }
        (Some(mut hot), Some(clock)) => {
            hot.ready = limit as i64 - clock.head_ready as i64;
            // A buffer restart after a data load is a nonsequential access.
            let timing =
                Prefetch { duty: clock.duty as i64, miss: n as i64, fill: clock.fill as i64 };
            rom_runs(core, &mut c, pages, ops, hot, timing, start, budget)
        }
    };
    let Some(exit) = exit else {
        return false;
    };
    let pc = core.next_instruction_address;
    match prefetch {
        None => {
            bus.engine_finish(exit.cycle, exit.open_bus, exit.data_last);
            bus.prefetch_disabled_bug_pending = exit.internal_last;
        }
        Some(clock) => {
            let clock = PrefetchClock { head_ready: (limit as i64 - ready) as u64, ..clock };
            if exit.load_last {
                bus.open_bus_value = exit.open_bus;
            }
            bus.thumb_prefetch_finish(
                pc,
                exit.cycle,
                &clock,
                exit.discarded,
                exit.data_last,
                exit.load_last,
                exit.open_bus,
            );
        }
    }
    true
}

/// Runs ROM pages from `hot` until a run stops, switching pages on far
/// branches; returns what the run leaves and the final buffer readiness.
#[allow(clippy::too_many_arguments)]
fn rom_runs<T: Timing>(
    core: &mut Core,
    c: &mut Ctx<'_>,
    pages: &mut [Option<Box<[Op]>>],
    mut ops: Box<[Op]>,
    mut hot: Hot,
    timing: T,
    start: u64,
    budget: i64,
) -> (Option<Exit>, i64) {
    let mut page = page_of(c.base);
    loop {
        let slots = (&mut *ops).try_into().expect("ROM Thumb page");
        let (next, stop) = run_loop::<ROM_PAGE_SLOTS, _>(slots, c, core, timing, hot);
        hot = next;
        let Stop::Far(target) = stop else {
            break;
        };
        let next_page = pages[page_of(target)].take().unwrap_or_else(rom_page);
        pages[page] = Some(std::mem::replace(&mut ops, next_page));
        page = page_of(target);
        c.base = target & !(PAGE_BYTES - 1);
        c.clean = clean_page(c);
        hot.idx = (target & (PAGE_BYTES - 1)) >> 1;
    }
    pages[page] = Some(ops);
    (leave(core, c, hot, start, budget), hot.ready)
}

/// Bytes of Game Pak ROM per page of Thumb ops.
const PAGE_BYTES: u32 = 2 * ROM_PAGE_SLOTS as u32;

fn page_of(address: u32) -> usize {
    (((address - 0x0800_0000) & 0x01ff_ffff) / PAGE_BYTES) as usize
}

fn rom_page() -> Box<[Op]> {
    fresh_ops(ROM_PAGE_SLOTS, &[])
}

/// Whether the ROM page at `c.base` and the halfwords just after it are
/// plain image bytes, so that any branch within it may run.
fn clean_page(c: &Ctx<'_>) -> bool {
    // The last page of a 128 KiB block holds code the buffer cannot run.
    (c.base + PAGE_BYTES) & 0x1ffff != 0 && c.mem.rom.plain(c.base, PAGE_BYTES + 4)
}

/// The state of a run at its first instruction, if the pipeline holds what
/// memory holds there and the engine may run from `pc`.
fn enter(core: &Core, c: &Ctx<'_>, pc: u32, budget: i64) -> Option<Hot> {
    c.code(pc)?;
    let latched = [c.half(pc), c.half(pc.wrapping_add(2))];
    let thumb = Pipeline { instructions: latched, instruction_set: InstructionSet::Thumb };
    (core.pipeline == Some(thumb))
        .then(|| Hot::from_cpsr(budget, pc.wrapping_sub(c.base) >> 1, core.cpsr))
}

/// What a run leaves on the bus.
struct Exit {
    cycle: u64,
    open_bus: u32,
    data_last: bool,
    load_last: bool,
    internal_last: bool,
    discarded: bool,
}

/// Publishes the CPU state after a run; None if no instruction ran.
fn leave(core: &mut Core, c: &Ctx<'_>, hot: Hot, start: u64, budget: i64) -> Option<Exit> {
    let spent = budget - hot.budget;
    if spent == 0 {
        return None;
    }
    let pc = c.pc(hot.idx);
    let pipeline = [c.half(pc), c.half(pc.wrapping_add(2))];
    let loaded = c.load_mark == hot.budget;
    core.cpsr = hot.cpsr(core.cpsr);
    core.next_instruction_address = pc;
    core.registers[15] = pc.wrapping_add(4);
    core.pipeline =
        Some(Pipeline { instructions: pipeline, instruction_set: InstructionSet::Thumb });
    Some(Exit {
        cycle: start + spent as u64,
        open_bus: if loaded { c.data } else { pipeline[1] },
        data_last: loaded || c.store_mark == hot.budget,
        load_last: loaded,
        internal_last: c.internal_mark == hot.budget,
        discarded: c.discard_mark == hot.budget,
    })
}

/// Runs one page's ops from `state` until one is refused or a branch
/// leaves the page. The register file lives in this frame.
#[inline(never)]
fn run_loop<const N: usize, T: Timing>(
    ops: &mut [Op; N],
    c: &mut Ctx<'_>,
    core: &mut Core,
    t: T,
    state: Hot,
) -> (Hot, Stop) {
    let mut regs: Registers = [0; 256];
    regs[..16].copy_from_slice(&core.registers);
    let result = run_ops(ops, c, &mut regs, t, state);
    core.registers.copy_from_slice(&regs[..16]);
    result
}

/// Runs a handler: `Some(stop)` ends the run.
macro_rules! go {
    ($h:ident, $run:expr) => {
        match $run {
            Flow::Next => {}
            Flow::Refused => return ($h, Stop::Refused),
            Flow::Far(target) => return ($h, Stop::Far(target)),
        }
    };
}

/// What an instruction did to the flow of a run.
enum Flow {
    Next,
    Refused,
    Far(u32),
}

impl From<bool> for Flow {
    #[inline(always)]
    fn from(ran: bool) -> Self {
        if ran { Self::Next } else { Self::Refused }
    }
}

#[inline(always)]
fn run_ops<const N: usize, T: Timing>(
    ops: &mut [Op; N],
    c: &mut Ctx<'_>,
    r: &mut Registers,
    t: T,
    state: Hot,
) -> (Hot, Stop) {
    let mut h = state;
    loop {
        let op = &ops[h.idx as usize & (N - 1)];
        #[cfg(debug_assertions)]
        if op.kind != k::UNDECODED {
            let pc = c.pc(h.idx);
            let expected =
                if h.idx as usize == N - 1 { Some(EXIT_OP) } else { c.code(pc).map(decode) };
            debug_assert_eq!(*op, expected.unwrap_or(EXIT_OP), "stale Thumb op at {pc:08x}");
        }
        match op.kind {
            k::UNDECODED => decode_slot(&mut ops[..], c, h.idx),
            k::MOVS => go!(
                h,
                alu(&mut h, t, |h| {
                    let value = r[usize::from(op.rn)];
                    r[usize::from(op.rd)] = value;
                    h.set_nz(value);
                })
            ),
            k::LSL => go!(
                h,
                alu(&mut h, t, |h| {
                    let value = r[usize::from(op.rn)];
                    let n = op.x & 31;
                    let result = value << n;
                    r[usize::from(op.rd)] = result;
                    h.set_shift(result, Some(value.wrapping_shr(32 - n) & 1 != 0));
                })
            ),
            k::LSR => go!(
                h,
                alu(&mut h, t, |h| {
                    let value = r[usize::from(op.rn)];
                    let n = op.x;
                    let result = if n == 32 { 0 } else { value >> (n & 31) };
                    r[usize::from(op.rd)] = result;
                    h.set_shift(result, Some((value >> ((n - 1) & 31)) & 1 != 0));
                })
            ),
            k::ASR => go!(
                h,
                alu(&mut h, t, |h| {
                    let value = r[usize::from(op.rn)];
                    let n = op.x;
                    let result = ((value as i32) >> (n.min(31))) as u32;
                    r[usize::from(op.rd)] = result;
                    h.set_shift(result, Some((value >> ((n - 1) & 31)) & 1 != 0));
                })
            ),
            k::ADD_R => go!(
                h,
                alu(&mut h, t, |h| {
                    let (a, b) = (r[usize::from(op.rn)], r[op.x as u8 as usize]);
                    let result = a.wrapping_add(b);
                    r[usize::from(op.rd)] = result;
                    h.set_add(a, b, result);
                })
            ),
            k::SUB_R => go!(
                h,
                alu(&mut h, t, |h| {
                    let (a, b) = (r[usize::from(op.rn)], r[op.x as u8 as usize]);
                    let result = a.wrapping_sub(b);
                    r[usize::from(op.rd)] = result;
                    h.set_sub(a, b, result);
                })
            ),
            k::ADD_3 | k::ADD_I => go!(
                h,
                alu(&mut h, t, |h| {
                    let (a, b) = (r[usize::from(op.rn)], op.x);
                    let result = a.wrapping_add(b);
                    r[usize::from(op.rd)] = result;
                    h.set_add(a, b, result);
                })
            ),
            k::SUB_3 | k::SUB_I => go!(
                h,
                alu(&mut h, t, |h| {
                    let (a, b) = (r[usize::from(op.rn)], op.x);
                    let result = a.wrapping_sub(b);
                    r[usize::from(op.rd)] = result;
                    h.set_sub(a, b, result);
                })
            ),
            k::MOV_I => go!(
                h,
                alu(&mut h, t, |h| {
                    r[usize::from(op.rd)] = op.x;
                    h.set_nz(op.x);
                })
            ),
            k::CMP_I => go!(
                h,
                alu(&mut h, t, |h| {
                    let (a, b) = (r[usize::from(op.rd)], op.x);
                    h.set_sub(a, b, a.wrapping_sub(b));
                })
            ),
            k::ADD_H => go!(
                h,
                alu(&mut h, t, |h| {
                    r[15] = c.pc(h.idx).wrapping_add(4);
                    r[usize::from(op.rd)] =
                        r[usize::from(op.rd)].wrapping_add(r[usize::from(op.rn)]);
                })
            ),
            k::CMP_H => go!(
                h,
                alu(&mut h, t, |h| {
                    r[15] = c.pc(h.idx).wrapping_add(4);
                    let (a, b) = (r[usize::from(op.rd)], r[usize::from(op.rn)]);
                    h.set_sub(a, b, a.wrapping_sub(b));
                })
            ),
            k::MOV_H => go!(
                h,
                alu(&mut h, t, |h| {
                    r[15] = c.pc(h.idx).wrapping_add(4);
                    r[usize::from(op.rd)] = r[usize::from(op.rn)];
                })
            ),
            k::ADD_PC => go!(
                h,
                alu(&mut h, t, |h| {
                    r[usize::from(op.rd)] = (c.pc(h.idx).wrapping_add(4) & !3).wrapping_add(op.x);
                })
            ),
            k::ADD_SP => go!(
                h,
                alu(&mut h, t, |_| {
                    r[usize::from(op.rd)] = r[13].wrapping_add(op.x);
                })
            ),
            k::ADJ_SP => go!(
                h,
                alu(&mut h, t, |_| {
                    r[13] = r[13].wrapping_add(op.x);
                })
            ),
            k::LDR_PC => {
                let address = (c.pc(h.idx).wrapping_add(4) & !3).wrapping_add(op.x);
                go!(h, transfer::<N, T, LDR>(&mut h, c, ops, r, t, op, address))
            }
            k::PUSH => go!(h, push::<N, T>(&mut h, c, ops, r, t, op.x)),
            k::POP => go!(h, pop::<N, T>(&mut h, c, r, t, op.x, false)),
            k::POP_PC => go!(h, pop::<N, T>(&mut h, c, r, t, op.x, true)),
            k::STMIA => go!(h, multiple::<N, T>(&mut h, c, ops, r, t, op, false)),
            k::LDMIA => go!(h, multiple::<N, T>(&mut h, c, ops, r, t, op, true)),
            k::B => {
                let slot = h.idx.wrapping_add(op.x);
                go!(h, jump::<N, T>(&mut h, c, t, slot, 0))
            }
            k::BL_HI => go!(
                h,
                alu(&mut h, t, |h| {
                    r[14] = c.pc(h.idx).wrapping_add(4).wrapping_add(op.x);
                })
            ),
            k::BL_LO => {
                let pc = c.pc(h.idx);
                let slot = slot_of(c, r[14].wrapping_add(op.x) & !1);
                let flow = jump::<N, T>(&mut h, c, t, slot, 0);
                if !matches!(flow, Flow::Refused) {
                    r[14] = pc.wrapping_add(2) | 1;
                }
                go!(h, flow)
            }
            k::ADD_PC_H | k::MOV_PC_H | k::BX => {
                let pc4 = c.pc(h.idx).wrapping_add(4);
                r[15] = pc4;
                let source = r[usize::from(op.rn)];
                let target = match op.kind {
                    k::ADD_PC_H => pc4.wrapping_add(source),
                    // BX to ARM switches instruction sets: the interpreter's.
                    k::BX if source & 1 == 0 => return (h, Stop::Refused),
                    _ => source,
                } & !1;
                go!(h, jump::<N, T>(&mut h, c, t, slot_of(c, target), 0))
            }
            32 => go!(h, logical(&mut h, t, r, op, |a, b| a & b)),
            33 => go!(h, logical(&mut h, t, r, op, |a, b| a ^ b)),
            34 => go!(h, shift::<N, T>(&mut h, c, t, r, op, 0)),
            35 => go!(h, shift::<N, T>(&mut h, c, t, r, op, 1)),
            36 => go!(h, shift::<N, T>(&mut h, c, t, r, op, 2)),
            37 => go!(
                h,
                alu(&mut h, t, |h| {
                    let (a, b) = (r[usize::from(op.rd)], r[usize::from(op.rn)]);
                    let carry = h.carry;
                    let result = a.wrapping_add(b).wrapping_add(carry);
                    r[usize::from(op.rd)] = result;
                    h.set_nz(result);
                    h.carry = u32::from(
                        u64::from(a) + u64::from(b) + u64::from(carry) > u64::from(u32::MAX),
                    );
                    h.overflow = !(a ^ b) & (a ^ result);
                })
            ),
            38 => go!(
                h,
                alu(&mut h, t, |h| {
                    let (a, b) = (r[usize::from(op.rd)], r[usize::from(op.rn)]);
                    let borrow = 1 - h.carry;
                    let result = a.wrapping_sub(b).wrapping_sub(borrow);
                    r[usize::from(op.rd)] = result;
                    h.set_nz(result);
                    h.carry = u32::from(u64::from(a) >= u64::from(b) + u64::from(borrow));
                    h.overflow = (a ^ b) & (a ^ result);
                })
            ),
            39 => go!(h, shift::<N, T>(&mut h, c, t, r, op, 3)),
            40 => go!(
                h,
                alu(&mut h, t, |h| {
                    h.set_nz(r[usize::from(op.rd)] & r[usize::from(op.rn)]);
                })
            ),
            41 => go!(
                h,
                alu(&mut h, t, |h| {
                    let b = r[usize::from(op.rn)];
                    let result = 0u32.wrapping_sub(b);
                    r[usize::from(op.rd)] = result;
                    h.set_sub(0, b, result);
                })
            ),
            42 => go!(
                h,
                alu(&mut h, t, |h| {
                    let (a, b) = (r[usize::from(op.rd)], r[usize::from(op.rn)]);
                    h.set_sub(a, b, a.wrapping_sub(b));
                })
            ),
            43 => go!(
                h,
                alu(&mut h, t, |h| {
                    let (a, b) = (r[usize::from(op.rd)], r[usize::from(op.rn)]);
                    h.set_add(a, b, a.wrapping_add(b));
                })
            ),
            44 => go!(h, logical(&mut h, t, r, op, |a, b| a | b)),
            45 => go!(h, multiply::<N, T>(&mut h, c, t, r, op)),
            46 => go!(h, logical(&mut h, t, r, op, |a, b| a & !b)),
            47 => go!(h, logical(&mut h, t, r, op, |_, b| !b)),
            48 => go!(h, transfer::<N, T, STR>(&mut h, c, ops, r, t, op, indexed(r, op))),
            49 => go!(h, transfer::<N, T, STRH>(&mut h, c, ops, r, t, op, indexed(r, op))),
            50 => go!(h, transfer::<N, T, STRB>(&mut h, c, ops, r, t, op, indexed(r, op))),
            51 => go!(h, transfer::<N, T, LDRSB>(&mut h, c, ops, r, t, op, indexed(r, op))),
            52 => go!(h, transfer::<N, T, LDR>(&mut h, c, ops, r, t, op, indexed(r, op))),
            53 => go!(h, transfer::<N, T, LDRH>(&mut h, c, ops, r, t, op, indexed(r, op))),
            54 => go!(h, transfer::<N, T, LDRB>(&mut h, c, ops, r, t, op, indexed(r, op))),
            55 => go!(h, transfer::<N, T, LDRSH>(&mut h, c, ops, r, t, op, indexed(r, op))),
            56 => go!(h, transfer::<N, T, STR>(&mut h, c, ops, r, t, op, offset(r, op))),
            57 => go!(h, transfer::<N, T, LDR>(&mut h, c, ops, r, t, op, offset(r, op))),
            58 => go!(h, transfer::<N, T, STRH>(&mut h, c, ops, r, t, op, offset(r, op))),
            59 => go!(h, transfer::<N, T, LDRH>(&mut h, c, ops, r, t, op, offset(r, op))),
            60 => go!(h, transfer::<N, T, STRB>(&mut h, c, ops, r, t, op, offset(r, op))),
            61 => go!(h, transfer::<N, T, LDRB>(&mut h, c, ops, r, t, op, offset(r, op))),
            62 | 63 => return (h, Stop::Refused),
            64 => go!(h, branch_if::<N, T, 0>(&mut h, c, t, op)),
            65 => go!(h, branch_if::<N, T, 1>(&mut h, c, t, op)),
            66 => go!(h, branch_if::<N, T, 2>(&mut h, c, t, op)),
            67 => go!(h, branch_if::<N, T, 3>(&mut h, c, t, op)),
            68 => go!(h, branch_if::<N, T, 4>(&mut h, c, t, op)),
            69 => go!(h, branch_if::<N, T, 5>(&mut h, c, t, op)),
            70 => go!(h, branch_if::<N, T, 6>(&mut h, c, t, op)),
            71 => go!(h, branch_if::<N, T, 7>(&mut h, c, t, op)),
            72 => go!(h, branch_if::<N, T, 8>(&mut h, c, t, op)),
            73 => go!(h, branch_if::<N, T, 9>(&mut h, c, t, op)),
            74 => go!(h, branch_if::<N, T, 10>(&mut h, c, t, op)),
            75 => go!(h, branch_if::<N, T, 11>(&mut h, c, t, op)),
            76 => go!(h, branch_if::<N, T, 12>(&mut h, c, t, op)),
            77 => go!(h, branch_if::<N, T, 13>(&mut h, c, t, op)),
            _ => return (h, Stop::Refused),
        }
    }
}

#[cold]
#[inline(never)]
fn decode_slot(ops: &mut [Op], c: &mut Ctx<'_>, idx: u32) {
    let pc = c.pc(idx);
    // A page's last slot is left to the interpreter, which runs its fetch
    // into the next page; so is code the engine may not fetch.
    let op = if idx as usize == ops.len() - 1 { None } else { c.code(pc) };
    ops[idx as usize] = op.map_or(EXIT_OP, decode);
    if c.place != Place::Rom {
        let (size, flags) = if c.place == Place::Ewram {
            (EWRAM_SIZE, &mut c.mem.ewram_code[..])
        } else {
            (IWRAM_SIZE, &mut c.mem.iwram_code[..])
        };
        // The page of the instruction and of its prefetch window hold code.
        for address in [pc, pc.wrapping_add(4)] {
            flags[(address as usize & (size - 1)) >> PAGE_SHIFT] = true;
        }
    }
}

/// An instruction with only its fetch.
#[inline(always)]
fn alu<T: Timing>(h: &mut Hot, t: T, body: impl FnOnce(&mut Hot)) -> Flow {
    let before = h.budget;
    if !h.charge(t.fetch(h)) {
        return Flow::Refused;
    }
    body(h);
    h.idx += 1;
    t.after(h, before, false);
    Flow::Next
}

#[inline(always)]
fn logical<T: Timing>(
    h: &mut Hot,
    t: T,
    r: &mut Registers,
    op: &Op,
    f: impl FnOnce(u32, u32) -> u32,
) -> Flow {
    alu(h, t, |h| {
        let result = f(r[usize::from(op.rd)], r[usize::from(op.rn)]);
        r[usize::from(op.rd)] = result;
        h.set_nz(result);
    })
}

/// An instruction with internal cycles.
#[inline(always)]
fn internal<T: Timing>(
    h: &mut Hot,
    c: &mut Ctx<'_>,
    t: T,
    cycles: i64,
    body: impl FnOnce(&mut Hot),
) -> Flow {
    let before = h.budget;
    if !h.charge(t.fetch(h) + cycles) {
        return Flow::Refused;
    }
    body(h);
    h.idx += 1;
    t.after(h, before, true);
    c.internal_mark = h.budget;
    Flow::Next
}

/// LSL, LSR, ASR and ROR by a register.
#[inline(always)]
fn shift<const N: usize, T: Timing>(
    h: &mut Hot,
    c: &mut Ctx<'_>,
    t: T,
    r: &mut Registers,
    op: &Op,
    kind: u32,
) -> Flow {
    internal(h, c, t, 1, |h| {
        let shifted =
            Core::decode_register_shift(r[usize::from(op.rd)], kind, r[usize::from(op.rn)] as u8);
        r[usize::from(op.rd)] = shifted.value;
        h.set_shift(shifted.value, shifted.carry);
    })
}

#[inline(always)]
fn multiply<const N: usize, T: Timing>(
    h: &mut Hot,
    c: &mut Ctx<'_>,
    t: T,
    r: &mut Registers,
    op: &Op,
) -> Flow {
    let lhs = r[usize::from(op.rd)];
    internal(h, c, t, Core::multiply_iterations(lhs, true) as i64, |h| {
        let result = lhs.wrapping_mul(r[usize::from(op.rn)]);
        r[usize::from(op.rd)] = result;
        h.set_nz(result);
    })
}

/// A branch to `slot` of this page (wrapping below it), charging `extra`
/// cycles between the fetch and the refill.
#[inline(always)]
fn jump<const N: usize, T: Timing>(
    h: &mut Hot,
    c: &mut Ctx<'_>,
    t: T,
    slot: u32,
    extra: i64,
) -> Flow {
    let target = c.pc(slot);
    if (slot as usize) < N && (c.clean || c.code(target).is_some()) {
        let Some(next) = branch(*h, c, t, target, extra) else {
            return Flow::Refused;
        };
        *h = Hot { idx: slot, ..next };
        return Flow::Next;
    }
    far_jump(h, c, t, target, extra)
}

/// The state after a branch to `target`, if it fits.
#[inline(always)]
fn branch<T: Timing>(h: Hot, c: &Ctx<'_>, t: T, target: u32, extra: i64) -> Option<Hot> {
    let mut next = h;
    if !next.charge(t.fetch(&h) + extra) {
        return None;
    }
    t.after(&mut next, h.budget, extra != 0);
    let refill = t.refill(&mut next, target, c.pc(h.idx).wrapping_add(6));
    next.charge(refill).then_some(next)
}

/// A branch out of the page, if the target is code the engine may run in
/// the same region and Game Pak window.
#[cold]
#[inline(never)]
fn far_jump<T: Timing>(h: &mut Hot, c: &mut Ctx<'_>, t: T, target: u32, extra: i64) -> Flow {
    let pc = c.pc(h.idx);
    let same = match c.place {
        Place::Rom => {
            target.wrapping_sub(0x0800_0000) >> 25 == pc.wrapping_sub(0x0800_0000) >> 25
                && c.mem.rom.thumb_code(target).is_some()
        }
        place => {
            let mask = if place == Place::Ewram { EWRAM_SIZE - 1 } else { IWRAM_SIZE - 1 } as u32;
            let region = place.region();
            target >> 24 == region && target & !mask != (region << 24 | (0x00ff_ffff & !mask))
        }
    };
    let Some(next) = same.then(|| branch(*h, c, t, target, extra)).flatten() else {
        return Flow::Refused;
    };
    *h = next;
    Flow::Far(target)
}

/// The page slot of an absolute branch target.
#[inline(always)]
fn slot_of(c: &Ctx<'_>, target: u32) -> u32 {
    (target.wrapping_sub(c.base) as i32 >> 1) as u32
}

#[inline(always)]
fn branch_if<const N: usize, T: Timing, const C: u8>(
    h: &mut Hot,
    c: &mut Ctx<'_>,
    t: T,
    op: &Op,
) -> Flow {
    if std::hint::black_box(h.condition::<C>()) {
        let slot = h.idx.wrapping_add(op.x);
        jump::<N, T>(h, c, t, slot, 0)
    } else {
        alu(h, t, |_| {})
    }
}

/// The address of a register-offset transfer.
#[inline(always)]
fn indexed(r: &Registers, op: &Op) -> u32 {
    r[usize::from(op.rn)].wrapping_add(r[op.x as u8 as usize])
}

/// The address of an immediate-offset transfer.
#[inline(always)]
fn offset(r: &Registers, op: &Op) -> u32 {
    r[usize::from(op.rn)].wrapping_add(op.x)
}

#[inline(always)]
fn running<'o, const N: usize>(c: &Ctx<'_>, ops: &'o [Op; N], idx: u32) -> Running<'o> {
    Running { ops: &ops[..], region: c.place.region(), thumb: true, pc: c.pc(idx) }
}

/// A single transfer at `address`.
#[inline(always)]
fn transfer<const N: usize, T: Timing, const MODE: u8>(
    h: &mut Hot,
    c: &mut Ctx<'_>,
    ops: &[Op; N],
    r: &mut Registers,
    t: T,
    op: &Op,
    address: u32,
) -> Flow {
    let (width, bus_address) = match MODE {
        STR | LDR => (AccessWidth::Word, address & !3),
        STRB | LDRB | LDRSB => (AccessWidth::Byte, address),
        _ => (AccessWidth::Half, address & !1),
    };
    if matches!(MODE, STR | STRH | STRB) {
        let Some(cycles) = c.mem.store_cycles(running(c, ops, h.idx), bus_address, width) else {
            return Flow::Refused;
        };
        let before = h.budget;
        let cost = t.fetch(h) + cycles;
        if !c.mem.store_ends_in_time(bus_address, before - cost) || !h.charge(cost) {
            return Flow::Refused;
        }
        c.mem.store(bus_address, width, r[usize::from(op.rd)]);
        h.idx += 1;
        t.after(h, before, true);
        c.store_mark = h.budget;
        return Flow::Next;
    }
    let Some((value, cycles)) = c.mem.load(bus_address, width) else {
        return Flow::Refused;
    };
    let before = h.budget;
    let rom = bus_address >> 24 >= 8;
    let collision = if rom {
        let Some(collision) = t.rom_load(h, before, &c.mem.rom, bus_address, width) else {
            return Flow::Refused;
        };
        collision
    } else {
        0
    };
    if !h.charge(t.fetch(h) + cycles + collision + 1) {
        return Flow::Refused;
    }
    r[usize::from(op.rd)] = match MODE {
        LDR => value.rotate_right((address & 3) * 8),
        LDRH => value.rotate_right((address & 1) * 8),
        LDRSB => (value as u8 as i8 as i32) as u32,
        LDRSH if address & 1 != 0 => ((value >> 8) as u8 as i8 as i32) as u32,
        LDRSH => (value as u16 as i16 as i32) as u32,
        _ => value,
    };
    h.idx += 1;
    t.after(h, before, true);
    if rom {
        t.discard(h, c);
    }
    c.data = value;
    c.load_mark = h.budget;
    c.internal_mark = h.budget;
    Flow::Next
}

/// The data cycles per word of a RAM block from `first` over `count` words.
#[inline(always)]
fn block_cycles(first: u32, count: u32) -> Option<i64> {
    let last = first.wrapping_add((count - 1) * 4);
    match (first >> 24, last >> 24) {
        (3, 3) => Some(1),
        (2, 2) => Some(6),
        _ => None,
    }
}

/// PUSH: the listed registers, then LR if bit 8 is set.
#[inline(always)]
fn push<const N: usize, T: Timing>(
    h: &mut Hot,
    c: &mut Ctx<'_>,
    ops: &[Op; N],
    r: &mut Registers,
    t: T,
    list: u32,
) -> Flow {
    let count = list.count_ones();
    let start = r[13].wrapping_sub(count * 4);
    let first = start & !3;
    let Some(per_word) = block_cycles(first, count) else {
        return Flow::Refused;
    };
    let run = running(c, ops, h.idx);
    if (0..count).any(|beat| c.mem.store_cycles(run, first + 4 * beat, AccessWidth::Word).is_none())
    {
        return Flow::Refused;
    }
    let before = h.budget;
    if !h.charge(t.fetch(h) + i64::from(count) * per_word) {
        return Flow::Refused;
    }
    let mut address = first;
    let mut remaining = list;
    while remaining != 0 {
        let register = remaining.trailing_zeros();
        remaining &= remaining - 1;
        let value = if register == 8 { r[14] } else { r[register as usize] };
        c.mem.store(address, AccessWidth::Word, value);
        address = address.wrapping_add(4);
    }
    r[13] = start;
    h.idx += 1;
    t.after(h, before, true);
    c.store_mark = h.budget;
    Flow::Next
}

/// POP, with PC when `pc` is set.
#[inline(always)]
fn pop<const N: usize, T: Timing>(
    h: &mut Hot,
    c: &mut Ctx<'_>,
    r: &mut Registers,
    t: T,
    list: u32,
    pc: bool,
) -> Flow {
    let count = list.count_ones() + u32::from(pc);
    let start = r[13];
    let first = start & !3;
    let Some(per_word) = block_cycles(first, count) else {
        return Flow::Refused;
    };
    let data = i64::from(count) * per_word + 1;
    if pc {
        let Some((word, _)) = c.mem.load(first + 4 * (count - 1), AccessWidth::Word) else {
            return Flow::Refused;
        };
        let flow = jump::<N, T>(h, c, t, slot_of(c, word & !1), data);
        if !matches!(flow, Flow::Refused) {
            load_block(c, r, first, list);
            r[13] = start.wrapping_add(count * 4);
        }
        return flow;
    }
    let before = h.budget;
    if !h.charge(t.fetch(h) + data) {
        return Flow::Refused;
    }
    load_block(c, r, first, list);
    r[13] = start.wrapping_add(count * 4);
    h.idx += 1;
    t.after(h, before, true);
    c.load_mark = h.budget;
    c.internal_mark = h.budget;
    Flow::Next
}

#[inline(always)]
fn load_block(c: &mut Ctx<'_>, r: &mut Registers, first: u32, list: u32) {
    let mut address = first;
    let mut remaining = list;
    while remaining != 0 {
        let register = remaining.trailing_zeros() as usize;
        remaining &= remaining - 1;
        let (value, _) = c.mem.load(address, AccessWidth::Word).expect("RAM block");
        r[register] = value;
        c.data = value;
        address = address.wrapping_add(4);
    }
}

/// STMIA and LDMIA.
#[inline(always)]
fn multiple<const N: usize, T: Timing>(
    h: &mut Hot,
    c: &mut Ctx<'_>,
    ops: &[Op; N],
    r: &mut Registers,
    t: T,
    op: &Op,
    load: bool,
) -> Flow {
    let list = op.x;
    let rb = usize::from(op.rn);
    let count = list.count_ones();
    let base = r[rb];
    let first = base & !3;
    let final_base = base.wrapping_add(count * 4);
    let Some(per_word) = block_cycles(first, count) else {
        return Flow::Refused;
    };
    if !load {
        let run = running(c, ops, h.idx);
        if (0..count)
            .any(|beat| c.mem.store_cycles(run, first + 4 * beat, AccessWidth::Word).is_none())
        {
            return Flow::Refused;
        }
    }
    let before = h.budget;
    if !h.charge(t.fetch(h) + i64::from(count) * per_word + i64::from(load)) {
        return Flow::Refused;
    }
    if load {
        load_block(c, r, first, list);
        if list & (1 << rb) == 0 {
            r[rb] = final_base;
        }
    } else {
        let mut address = first;
        let mut remaining = list;
        let mut index = 0;
        while remaining != 0 {
            let register = remaining.trailing_zeros() as usize;
            remaining &= remaining - 1;
            let value = if register == rb && index != 0 { final_base } else { r[register] };
            c.mem.store(address, AccessWidth::Word, value);
            address = address.wrapping_add(4);
            index += 1;
        }
        r[rb] = final_base;
    }
    h.idx += 1;
    t.after(h, before, true);
    if load {
        c.load_mark = h.budget;
        c.internal_mark = h.budget;
    } else {
        c.store_mark = h.budget;
    }
    Flow::Next
}
