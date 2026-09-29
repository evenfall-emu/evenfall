//! Pre-decoded execution engine.
//!
//! The engine runs instructions from caches of decoded [`Op`]s instead of
//! fetching and decoding each word. A run keeps the registers and a local
//! CPSR, advances a local clock by each instruction's exact cost (the cost
//! the ordinary interpreter charges) and stops before any instruction that
//! would pass the device horizon or the caller's yield point, touches memory
//! the engine does not serve, or needs the interpreter (mode and
//! instruction-set changes, exceptions, SWI, PSR writes). The pipeline, r15,
//! the open-bus latch and the next fetch's access kind are materialized only
//! when a run stops, so the machine is left exactly where the interpreter
//! would leave it after the same instructions, however execution is sliced.
//!
//! Caches are transient. A slot decodes itself on first execution. Every RAM
//! write through the bus clears the slots it covers once its 256-byte page
//! has held decoded code, and a run stops before any store that reaches a
//! decoded op or one of the two prefetched instructions, so the interpreter
//! executes stores that change code. See `docs/research/execution-engine.md`.

mod arm;
mod mem;
#[cfg(test)]
mod tests;
mod thumb;

use crate::bus::{Bus, EWRAM_SIZE, EngineMemory, IWRAM_SIZE, PowerState};
use crate::cpu::Cpu;
use mem::Mem;

/// Code pages are 256 bytes: the granularity of the "holds code" flags.
pub(crate) const PAGE_SHIFT: u32 = 8;

/// One decoded instruction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C, align(8))]
pub(crate) struct Op {
    /// The handler; zero marks an undecoded slot.
    pub(crate) kind: u8,
    /// The handler a conditional wrapper runs when its condition passes.
    pub(crate) inner: u8,
    /// Usually the destination and first operand registers.
    pub(crate) rd: u8,
    pub(crate) rn: u8,
    /// An immediate, packed operand fields or the raw instruction word.
    pub(crate) x: u32,
}

/// Thumb ops per Game Pak ROM page: 32 KiB of ROM.
pub(crate) const ROM_PAGE_SLOTS: usize = 1 << 14;

/// Decoded-op caches: one array per RAM region and instruction set, with a
/// slot per instruction address, and lazily allocated Thumb pages for ROM.
/// ARM arrays are twice as long as the region, with sentinels ending the
/// mirror after its last slot, so that a masked index needs no bounds check.
pub(crate) struct CodeCache {
    /// `[EWRAM, IWRAM]` by `[ARM, Thumb]`.
    ram: [[Option<Box<[Op]>>; 2]; 2],
    /// ROM never changes, so its pages are never forgotten.
    rom_thumb: Vec<Option<Box<[Op]>>>,
    /// Pages that have held a decoded op or the prefetch window of one.
    iwram_code: [bool; IWRAM_SIZE >> PAGE_SHIFT],
    ewram_code: [bool; EWRAM_SIZE >> PAGE_SHIFT],
}

impl Default for CodeCache {
    fn default() -> Self {
        Self {
            ram: Default::default(),
            rom_thumb: Vec::new(),
            iwram_code: [false; IWRAM_SIZE >> PAGE_SHIFT],
            ewram_code: [false; EWRAM_SIZE >> PAGE_SHIFT],
        }
    }
}

impl CodeCache {
    /// Forgets every decoded op, after a bulk memory change.
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    /// Forgets the ops covering `bytes` bytes of IWRAM from `index`.
    #[inline(always)]
    pub(crate) fn write_iwram(&mut self, index: usize, bytes: usize) {
        let last = (index + bytes - 1) & (IWRAM_SIZE - 1);
        if self.iwram_code[index >> PAGE_SHIFT] || self.iwram_code[last >> PAGE_SHIFT] {
            forget(&mut self.ram[1], index, bytes);
        }
    }

    /// Forgets the ops covering `bytes` bytes of EWRAM from `index`.
    #[inline(always)]
    pub(crate) fn write_ewram(&mut self, index: usize, bytes: usize) {
        let last = (index + bytes - 1) & (EWRAM_SIZE - 1);
        if self.ewram_code[index >> PAGE_SHIFT] || self.ewram_code[last >> PAGE_SHIFT] {
            forget(&mut self.ram[0], index, bytes);
        }
    }

    /// Takes a RAM region's op array out for a run, allocating it the first
    /// time; [`Self::put`] returns it.
    fn take(&mut self, region: u32, thumb: bool, fresh: impl FnOnce() -> Box<[Op]>) -> Box<[Op]> {
        self.ram[(region & 1) as usize][usize::from(thumb)].take().unwrap_or_else(fresh)
    }

    fn put(&mut self, region: u32, thumb: bool, ops: Box<[Op]>) {
        self.ram[(region & 1) as usize][usize::from(thumb)] = Some(ops);
    }
}

#[cold]
fn forget(region: &mut [Option<Box<[Op]>>; 2], index: usize, bytes: usize) {
    let [arm, thumb] = region;
    for offset in index..index + bytes {
        if let Some(ops) = arm {
            let slots = ops.len() / 2;
            ops[(offset / 4) & (slots - 1)] = Op::default();
        }
        if let Some(ops) = thumb {
            let slots = ops.len();
            ops[(offset / 2) & (slots - 1)] = Op::default();
        }
    }
}

/// An op array of `slots` undecoded slots, then `tail`.
fn fresh_ops(slots: usize, tail: &[Op]) -> Box<[Op]> {
    let mut ops = vec![Op::default(); slots];
    ops.extend_from_slice(tail);
    ops.into_boxed_slice()
}

/// Splits the bus memory for a run of `budget` cycles whose op array is out
/// of the cache.
fn split_memory(memory: EngineMemory<'_>, budget: i64) -> (Mem<'_>, &mut Vec<Option<Box<[Op]>>>) {
    let CodeCache { ram, rom_thumb, iwram_code, ewram_code } = memory.code;
    let others = ram.each_ref().map(|isa| isa.each_ref().map(|ops| ops.as_deref()));
    let mem = Mem {
        iwram: memory.iwram,
        ewram: memory.ewram,
        iwram_code,
        ewram_code,
        rom: memory.rom,
        others,
        video: memory.video,
        video_floor: budget.saturating_sub(i64::try_from(memory.quiet).unwrap_or(i64::MAX)),
    };
    (mem, rom_thumb)
}

impl Cpu {
    /// Runs decoded instructions from the next instruction address. Returns
    /// whether any instruction ran; if none did, nothing changed.
    pub(crate) fn run_engine(&mut self, bus: &mut Bus, yield_at: u64) -> bool {
        if (bus.irq_line && self.cpsr & (1 << 7) == 0)
            || self.hle_service.is_some()
            || bus.power_state() != PowerState::Running
        {
            return false;
        }
        let Some(limit) = bus.engine_limit(yield_at) else {
            return false;
        };
        if self.is_thumb() {
            return thumb::run(&mut self.core, bus, limit);
        }
        match self.core.next_instruction_address >> 24 {
            2 => arm::run::<true>(&mut self.core, bus, limit),
            3 => arm::run::<false>(&mut self.core, bus, limit),
            _ => false,
        }
    }
}
