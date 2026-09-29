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
//! has held decoded code, and a run stops before any store into such a page,
//! so the interpreter executes stores that could change code or the two
//! prefetched instructions.

mod arm;
#[cfg(test)]
mod tests;

use crate::bus::{Bus, EWRAM_SIZE, IWRAM_SIZE, PowerState};
use crate::cpu::Cpu;

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

const IWRAM_ARM_SLOTS: usize = IWRAM_SIZE / 4;
const EWRAM_ARM_SLOTS: usize = EWRAM_SIZE / 4;

/// Decoded-op caches, one slot per instruction address of each region. The
/// arrays are twice as long as the region, with sentinels ending the
/// mirror after its last slot, so that a masked index needs no bounds check.
pub(crate) struct CodeCache {
    iwram_arm: Option<Box<[Op; 2 * IWRAM_ARM_SLOTS]>>,
    ewram_arm: Option<Box<[Op; 2 * EWRAM_ARM_SLOTS]>>,
    /// Pages that have held a decoded op or the prefetch window of one.
    iwram_code: [bool; IWRAM_SIZE >> PAGE_SHIFT],
    ewram_code: [bool; EWRAM_SIZE >> PAGE_SHIFT],
}

impl Default for CodeCache {
    fn default() -> Self {
        Self {
            iwram_arm: None,
            ewram_arm: None,
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
            forget(self.iwram_arm.as_deref_mut(), index, bytes);
        }
    }

    /// Forgets the ops covering `bytes` bytes of EWRAM from `index`.
    #[inline(always)]
    pub(crate) fn write_ewram(&mut self, index: usize, bytes: usize) {
        let last = (index + bytes - 1) & (EWRAM_SIZE - 1);
        if self.ewram_code[index >> PAGE_SHIFT] || self.ewram_code[last >> PAGE_SHIFT] {
            forget(self.ewram_arm.as_deref_mut(), index, bytes);
        }
    }
}

#[cold]
fn forget<const N: usize>(arm: Option<&mut [Op; N]>, index: usize, bytes: usize) {
    let slots = N / 2;
    if let Some(ops) = arm {
        for offset in index..index + bytes {
            ops[(offset / 4) & (slots - 1)] = Op::default();
        }
    }
}

/// A region's op array with every slot undecoded, then the mirror-end
/// sentinels and, last, the stop sentinel.
fn fresh_ops<const N: usize>(end: Op, stop: Op) -> Box<[Op; N]> {
    let mut ops = vec![Op::default(); N];
    ops[N / 2..].fill(end);
    ops[N - 1] = stop;
    ops.into_boxed_slice().try_into().expect("fixed-size op array")
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
            return false;
        }
        match self.core.next_instruction_address >> 24 {
            2 => arm::run::<true>(&mut self.core, bus, limit),
            3 => arm::run::<false>(&mut self.core, bus, limit),
            _ => false,
        }
    }
}
