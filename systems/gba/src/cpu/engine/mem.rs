//! Data paths shared by the ARM and Thumb engines: RAM and video memory
//! loads and stores at their bus cost, Game Pak ROM loads, and the checks
//! that keep stores into code, and VRAM or palette stores that a visible
//! dot could observe, for the interpreter.

use super::{Op, PAGE_SHIFT};
use crate::bus::{
    AccessWidth, EWRAM_SIZE, IWRAM_SIZE, RomData, RunRegisters, VideoMemory, read_ram, write_ram,
};

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
    pub(super) video: VideoMemory<'a>,
    /// The MMIO registers runs access themselves.
    pub(super) registers: RunRegisters<'a>,
    /// A VRAM or palette store may run only if it ends with more budget
    /// left than this: before the next visible dot.
    pub(super) video_floor: i64,
    /// The run's budget when it took this view, where `registers.ppu` is.
    pub(super) start: i64,
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
            5..=7 => self.load_video(address, width),
            _ => self.rom.load(address, width).map(|(value, cycles)| (value, cycles as i64)),
        }
    }

    /// [`Self::load`] for IWRAM and EWRAM only.
    #[inline(always)]
    pub(super) fn ram_load(&self, address: u32, width: AccessWidth) -> Option<(u32, i64)> {
        match address >> 24 {
            3 => Some((read_ram(self.iwram, (address as usize) & (IWRAM_SIZE - 1), width), 1)),
            2 => Some((
                read_ram(self.ewram, (address as usize) & (EWRAM_SIZE - 1), width),
                if width == AccessWidth::Word { 6 } else { 3 },
            )),
            _ => None,
        }
    }

    /// The data cycles of a store to IWRAM or EWRAM in a page that has never
    /// held code, which needs no further check.
    #[inline(always)]
    pub(super) fn plain_ram_store(&self, address: u32, width: AccessWidth) -> Option<i64> {
        match address >> 24 {
            3 => (!self.iwram_code[((address as usize) & (IWRAM_SIZE - 1)) >> PAGE_SHIFT])
                .then_some(1),
            2 => (!self.ewram_code[((address as usize) & (EWRAM_SIZE - 1)) >> PAGE_SHIFT])
                .then_some(if width == AccessWidth::Word { 6 } else { 3 }),
            _ => None,
        }
    }

    #[inline(never)]
    fn load_video(&self, address: u32, width: AccessWidth) -> Option<(u32, i64)> {
        Some((self.video.read(address, width), video_cycles(address, width)))
    }

    /// The cost of an instruction whose fetch costs `fetch` and which ends
    /// with a store at a bus-aligned address, starting with `budget` left,
    /// if the engine may perform the store: to RAM, but not to a decoded
    /// instruction of either set or to one of the instructions prefetched
    /// after the running one; to OAM; or to VRAM and palette RAM, if the
    /// store ends before the next visible dot.
    #[inline(always)]
    pub(super) fn store_cost(
        &self,
        running: Running<'_>,
        address: u32,
        width: AccessWidth,
        fetch: i64,
        budget: i64,
    ) -> Option<i64> {
        let cycles = match address >> 24 {
            3 => 1,
            2 if width == AccessWidth::Word => 6,
            2 => 3,
            _ => return self.video_store_cost(address, width, fetch, budget),
        };
        (!self.store_reaches_code(running, address, width)).then_some(fetch + cycles)
    }

    #[inline(never)]
    fn video_store_cost(
        &self,
        address: u32,
        width: AccessWidth,
        fetch: i64,
        budget: i64,
    ) -> Option<i64> {
        let region = address >> 24;
        if !(5..=7).contains(&region) {
            return None;
        }
        let cost = fetch + video_cycles(address, width);
        (region == 7 || budget - cost > self.video_floor).then_some(cost)
    }

    /// Whether the code page of a RAM address has held decoded code.
    #[inline(always)]
    pub(super) fn holds_code(&self, address: u32) -> bool {
        if address >> 24 == 3 {
            self.iwram_code[((address as usize) & (IWRAM_SIZE - 1)) >> PAGE_SHIFT]
        } else {
            self.ewram_code[((address as usize) & (EWRAM_SIZE - 1)) >> PAGE_SHIFT]
        }
    }

    /// Whether a RAM store reaches a decoded instruction of either set or
    /// one of the instructions prefetched after the running one.
    #[inline(always)]
    pub(super) fn store_reaches_code(
        &self,
        running: Running<'_>,
        address: u32,
        width: AccessWidth,
    ) -> bool {
        let code = if address >> 24 == 3 {
            self.iwram_code[((address as usize) & (IWRAM_SIZE - 1)) >> PAGE_SHIFT]
        } else {
            self.ewram_code[((address as usize) & (EWRAM_SIZE - 1)) >> PAGE_SHIFT]
        };
        code && self.store_hits_code(running, address, width)
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

    /// A store [`Self::store_cost`] admitted.
    #[inline(always)]
    pub(super) fn store(&mut self, address: u32, width: AccessWidth, value: u32) {
        match address >> 24 {
            3 => write_ram(self.iwram, (address as usize) & (IWRAM_SIZE - 1), width, value),
            2 => write_ram(self.ewram, (address as usize) & (EWRAM_SIZE - 1), width, value),
            _ => self.store_video(address, width, value),
        }
    }

    #[inline(never)]
    fn store_video(&mut self, address: u32, width: AccessWidth, value: u32) {
        #[cfg(test)]
        VIDEO_STORES.with(|stores| stores.set(stores.get() + 1));
        match address >> 24 {
            5 => self.video.write_palette(address, width, value),
            6 => self.video.write_vram(address, width, value),
            _ => self.video.write_oam(address, width, value),
        }
    }
}

#[cfg(test)]
thread_local! {
    /// Video memory stores engine runs made on this thread, for tests to
    /// show they exercised them.
    pub(super) static VIDEO_STORES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Palette RAM and VRAM take a cycle per halfword, OAM a cycle per access.
#[inline(always)]
fn video_cycles(address: u32, width: AccessWidth) -> i64 {
    if address >> 24 != 7 && width == AccessWidth::Word { 2 } else { 1 }
}
