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

use crate::bus::{
    AccessWidth, Bus, EWRAM_SIZE, EngineMemory, IWRAM_SIZE, PowerState, RunRegisters,
};
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

/// How a load makes a register value of the bus value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Load {
    Word,
    Byte,
    Half,
    SignedByte,
    SignedHalf,
}

impl Load {
    /// The register value a load at `address` makes of bus value `value`:
    /// misaligned words and halfwords rotate, and a misaligned signed
    /// halfword loads its high byte.
    #[inline(always)]
    pub(super) fn extend(self, value: u32, address: u32) -> u32 {
        match self {
            Self::Word => value.rotate_right((address & 3) * 8),
            Self::Half => value.rotate_right((address & 1) * 8),
            Self::SignedByte => (value as u8 as i8 as i32) as u32,
            Self::SignedHalf if address & 1 != 0 => ((value >> 8) as u8 as i8 as i32) as u32,
            Self::SignedHalf => (value as u16 as i16 as i32) as u32,
            Self::Byte => value,
        }
    }

    pub(super) fn width(self) -> AccessWidth {
        match self {
            Self::Word => AccessWidth::Word,
            Self::Byte | Self::SignedByte => AccessWidth::Byte,
            Self::Half | Self::SignedHalf => AccessWidth::Half,
        }
    }
}

/// A single transfer to MMIO that a run stops at and its driver completes:
/// the driver charges the instruction, makes the access on the bus at its
/// cycle and writes the registers, then resumes the run.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Mmio {
    /// The address the instruction computed.
    pub(super) address: u32,
    /// The value a store writes, or the register a load writes and how.
    pub(super) store: Option<(u32, AccessWidth)>,
    pub(super) load: Option<(u8, Load)>,
    /// A base register write after the access (ARM).
    pub(super) writeback: Option<(u8, u32)>,
}

impl Mmio {
    /// The width and bus-aligned address of the access.
    pub(super) fn access(&self) -> (AccessWidth, u32) {
        let width = match (self.store, self.load) {
            (Some((_, width)), _) => width,
            (_, Some((_, load))) => load.width(),
            _ => unreachable!("an MMIO access loads or stores"),
        };
        let aligned = match width {
            AccessWidth::Word => self.address & !3,
            AccessWidth::Half => self.address & !1,
            AccessWidth::Byte => self.address,
        };
        (width, aligned)
    }

    /// Whether the bus lets the run make this access itself.
    pub(super) fn allowed(&self, bus: &Bus) -> bool {
        let (width, address) = self.access();
        if self.store.is_some() {
            bus.engine_mmio_storable(address, width)
        } else {
            bus.engine_mmio_loadable(address, width)
        }
    }

    /// Whether the access touches only registers a run reads and writes
    /// itself ([`RunRegisters`], [`Self::perform_local`]).
    #[inline(always)]
    pub(super) fn local(&self) -> bool {
        let (width, address) = self.access();
        RunRegisters::holds(address, width, self.store.is_some())
    }

    /// Makes an access [`Self::local`] admits on the run's own view of the
    /// registers, and writes the CPU registers. Returns the bus value.
    pub(super) fn perform_local(
        &self,
        local: &mut RunRegisters<'_>,
        registers: &mut [u32; 16],
    ) -> u32 {
        #[cfg(test)]
        LOCAL_ACCESSES.with(|count| count.set(count.get() + 1));
        let (width, address) = self.access();
        let value = match (self.store, self.load) {
            (Some((value, _)), _) => {
                local.store(address, width, value);
                value
            }
            (_, Some((rd, load))) => {
                let value = local.load(address, width);
                registers[usize::from(rd)] = load.extend(value, self.address);
                value
            }
            _ => unreachable!("an MMIO access loads or stores"),
        };
        if let Some((rn, base)) = self.writeback {
            registers[usize::from(rn)] = base;
        }
        value
    }

    /// Makes the access, whose data cycle follows the instruction's fetch
    /// ending at `fetched`, and writes the registers. Returns the bus value.
    pub(super) fn perform(&self, bus: &mut Bus, registers: &mut [u32; 16], fetched: u64) -> u32 {
        #[cfg(test)]
        MMIO_ACCESSES.with(|count| count.set(count.get() + 1));
        let (width, address) = self.access();
        let value = match (self.store, self.load) {
            (Some((value, _)), _) => {
                bus.engine_mmio_store(fetched + 1, address, width, value);
                value
            }
            (_, Some((rd, load))) => {
                let value = bus.engine_mmio_load(fetched, address, width);
                registers[usize::from(rd)] = load.extend(value, self.address);
                value
            }
            _ => unreachable!("an MMIO access loads or stores"),
        };
        if let Some((rn, base)) = self.writeback {
            registers[usize::from(rn)] = base;
        }
        value
    }
}

#[cfg(test)]
thread_local! {
    /// MMIO accesses engine runs made on this thread, for tests to show
    /// they exercised them.
    pub(super) static MMIO_ACCESSES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// Branches between the BIOS and IWRAM inside engine runs.
    pub(super) static SWITCHES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// Interrupt register accesses engine runs made on their own view.
    pub(super) static LOCAL_ACCESSES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Decoded-op caches: one array per RAM region and instruction set, with a
/// slot per instruction address, and lazily allocated Thumb pages for ROM.
/// ARM arrays are twice as long as the region, with sentinels ending the
/// mirror after its last slot, so that a masked index needs no bounds check.
pub(crate) struct CodeCache {
    /// `[EWRAM, IWRAM]` by `[ARM, Thumb]`.
    ram: [[Option<Box<[Op]>>; 2]; 2],
    /// ROM never changes, so its pages are never forgotten; nor does the
    /// BIOS, whose ARM ops run in the IWRAM loop ([`arm::run_bios`]).
    rom_thumb: Vec<Option<Box<[Op]>>>,
    bios_arm: Option<Box<[Op]>>,
    /// Pages that have held a decoded op or the prefetch window of one.
    iwram_code: [bool; IWRAM_SIZE >> PAGE_SHIFT],
    ewram_code: [bool; EWRAM_SIZE >> PAGE_SHIFT],
}

impl Default for CodeCache {
    fn default() -> Self {
        Self {
            ram: Default::default(),
            rom_thumb: Vec::new(),
            bios_arm: None,
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
    let CodeCache { ram, rom_thumb, iwram_code, ewram_code, .. } = memory.code;
    let others = ram.each_ref().map(|isa| isa.each_ref().map(|ops| ops.as_deref()));
    let mem = Mem {
        iwram: memory.iwram,
        ewram: memory.ewram,
        iwram_code,
        ewram_code,
        rom: memory.rom,
        others,
        video: memory.video,
        registers: memory.registers,
        video_floor: budget.saturating_sub(i64::try_from(memory.quiet).unwrap_or(i64::MAX)),
    };
    (mem, rom_thumb)
}

impl Cpu {
    /// Runs decoded instructions from the next instruction address. Returns
    /// whether any instruction ran; if none did, nothing changed.
    pub(crate) fn run_engine(&mut self, bus: &mut Bus, yield_at: u64) -> bool {
        if (bus.irq.irq_line && self.cpsr & (1 << 7) == 0)
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
        let bios_ok = self.low_code_is_ordinary();
        match self.core.next_instruction_address >> 24 {
            0 if bios_ok => arm::run_bios(&mut self.core, bus, limit),
            2 => arm::run::<true>(&mut self.core, bus, limit, false),
            3 => arm::run::<false>(&mut self.core, bus, limit, bios_ok),
            _ => false,
        }
    }
}
