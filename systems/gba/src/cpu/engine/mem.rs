//! Data paths shared by the ARM and Thumb engines: RAM loads and stores at
//! their bus cost, Game Pak ROM loads, and the check that keeps stores into
//! code for the interpreter.

use super::{Op, PAGE_SHIFT};
use crate::bus::{AccessWidth, EWRAM_SIZE, IWRAM_SIZE, RomData, read_ram, write_ram};

/// Memory an engine run works on. The array the run executes from is held
/// apart; `others` sees every other RAM op array, `[EWRAM, IWRAM]` by
/// `[ARM, Thumb]`.
pub(super) struct Mem<'a> {
    pub(super) iwram: &'a mut [u8; IWRAM_SIZE],
    pub(super) ewram: &'a mut [u8; EWRAM_SIZE],
    pub(super) iwram_code: &'a mut [bool; IWRAM_SIZE >> PAGE_SHIFT],
    pub(super) ewram_code: &'a mut [bool; EWRAM_SIZE >> PAGE_SHIFT],
    pub(super) rom: RomData<'a>,
    pub(super) others: [[Option<&'a [Op]>; 2]; 2],
}

/// The code a run executes: its op array, RAM region (2 or 3, else ROM),
/// instruction set and the address of the executing instruction.
#[derive(Clone, Copy)]
pub(super) struct Running<'a> {
    pub(super) ops: &'a [Op],
    pub(super) region: u32,
    pub(super) thumb: bool,
    pub(super) pc: u32,
}

impl Mem<'_> {
    /// A data load at a bus-aligned address: its bus value and data cycles,
    /// or None when the ordinary bus must perform it.
    #[inline(always)]
    pub(super) fn load(&self, address: u32, width: AccessWidth) -> Option<(u32, i64)> {
        match address >> 24 {
            3 => Some((read_ram(self.iwram, (address as usize) & (IWRAM_SIZE - 1), width), 1)),
            2 => Some((
                read_ram(self.ewram, (address as usize) & (EWRAM_SIZE - 1), width),
                if width == AccessWidth::Word { 6 } else { 3 },
            )),
            _ => self.rom.load(address, width).map(|(value, cycles)| (value, cycles as i64)),
        }
    }

    /// The data cycles of a store at a bus-aligned address that the engine
    /// may perform: to RAM, and not to a decoded instruction of either set
    /// or to one of the instructions prefetched after the running one.
    #[inline(always)]
    pub(super) fn store_cycles(
        &self,
        running: Running<'_>,
        address: u32,
        width: AccessWidth,
    ) -> Option<i64> {
        let (cycles, code) = match address >> 24 {
            3 => (1, self.iwram_code[((address as usize) & (IWRAM_SIZE - 1)) >> PAGE_SHIFT]),
            2 => (
                if width == AccessWidth::Word { 6 } else { 3 },
                self.ewram_code[((address as usize) & (EWRAM_SIZE - 1)) >> PAGE_SHIFT],
            ),
            _ => return None,
        };
        (!code || !self.store_hits_code(running, address, width)).then_some(cycles)
    }

    /// Whether a store in a page that has held code reaches a decoded op of
    /// either instruction set or the running code's prefetch window.
    #[cold]
    #[inline(never)]
    fn store_hits_code(&self, running: Running<'_>, address: u32, width: AccessWidth) -> bool {
        let region = address >> 24;
        let mask = if region == 2 { EWRAM_SIZE - 1 } else { IWRAM_SIZE - 1 } as u32;
        let offset = address & mask;
        let bytes = width.bytes() as u32;
        if region == running.region {
            // Two instructions after the running one are already fetched.
            let (start, length) = if running.thumb {
                (running.pc.wrapping_add(2), 4)
            } else {
                (running.pc.wrapping_add(4), 8)
            };
            let start = start & mask;
            if offset.wrapping_sub(start) & mask < length
                || start.wrapping_sub(offset) & mask < bytes
            {
                return true;
            }
        }
        let decoded = |thumb: bool| {
            let ops = if region == running.region && thumb == running.thumb {
                Some(running.ops)
            } else {
                self.others[(region & 1) as usize][usize::from(thumb)]
            };
            let Some(ops) = ops else {
                return false;
            };
            let (first, last) = if thumb {
                (offset >> 1, (offset + bytes - 1) >> 1)
            } else {
                (offset >> 2, offset >> 2)
            };
            (first..=last).any(|slot| ops[slot as usize].kind != 0)
        };
        decoded(false) || decoded(true)
    }

    #[inline(always)]
    pub(super) fn store(&mut self, address: u32, width: AccessWidth, value: u32) {
        if address >> 24 == 3 {
            write_ram(self.iwram, (address as usize) & (IWRAM_SIZE - 1), width, value);
        } else {
            write_ram(self.ewram, (address as usize) & (EWRAM_SIZE - 1), width, value);
        }
    }
}
