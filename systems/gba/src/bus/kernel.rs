//! CPU execution kernels whose local clocks are strictly inside a device horizon.

use super::*;
use crate::cpu::engine::CodeCache;

/// Memory one engine run borrows from the bus. The run owns the only view of
/// RAM, video memory and the op caches until it stops.
pub(crate) struct EngineMemory<'a> {
    pub(crate) iwram: &'a mut [u8; IWRAM_SIZE],
    pub(crate) ewram: &'a mut [u8; EWRAM_SIZE],
    pub(crate) code: &'a mut CodeCache,
    /// The BIOS image for a run of BIOS code, otherwise nothing.
    pub(crate) bios: &'a [u8],
    pub(crate) rom: RomData<'a>,
    pub(crate) video: VideoMemory<'a>,
    pub(crate) registers: RunRegisters<'a>,
    /// Cycles from now during which the PPU shows no visible dot, so that a
    /// VRAM or palette write records no raster history: zero while a
    /// visible dot is being drawn.
    pub(crate) quiet: Cycle,
}

/// The MMIO registers an engine run reads and writes itself: IE, IF, IME,
/// POSTFLG and HALTCNT, and the DMA source, destination and word count
/// latches. Their accesses schedule no device event and read no device
/// state, so they need neither the device batch nor the clock. The bus's
/// own MMIO paths are the reference for [`Self::load`] and [`Self::store`].
pub(crate) struct RunRegisters<'a> {
    pub(crate) irq: &'a mut IrqRegisters,
    pub(crate) dma: &'a mut [DmaChannel; 4],
}

impl RunRegisters<'_> {
    /// Whether an access of `width` at the bus-aligned MMIO `address`
    /// touches only these registers. A DMA control register takes no store
    /// here: enabling a channel schedules its transfer.
    #[inline(always)]
    pub(crate) fn holds(address: u32, width: AccessWidth, store: bool) -> bool {
        // Offsets from DMA0SAD: one compare tells the DMA latches apart.
        let offset = address.wrapping_sub(MMIO_BASE + 0xb0);
        if offset < 0x30 {
            // A bus-aligned access stays within one 12-byte channel.
            return !store || offset % 12 + width.bytes() as u32 <= 10;
        }
        IrqRegisters::holds(address, width, store)
    }

    /// A load [`Self::holds`] admits, as [`Bus::read_mmio`] makes it.
    pub(crate) fn load(&self, address: u32, width: AccessWidth) -> u32 {
        let offset = address.wrapping_sub(MMIO_BASE + 0xb0);
        if offset >= 0x30 {
            return self.irq.load(address, width);
        }
        // Only the control halfword reads back.
        let half = |offset: u32| {
            let channel = &self.dma[(offset / 12) as usize];
            u32::from(if offset % 12 == 10 { channel.control } else { 0 })
        };
        match width {
            AccessWidth::Word => half(offset) | half(offset + 2) << 16,
            AccessWidth::Half => half(offset),
            AccessWidth::Byte => (half(offset & !1) >> ((offset & 1) * 8)) & 0xff,
        }
    }

    /// A store [`Self::holds`] admits, as [`Bus::write_mmio`] makes it.
    pub(crate) fn store(&mut self, address: u32, width: AccessWidth, value: u32) {
        let offset = address.wrapping_sub(MMIO_BASE + 0xb0);
        if offset >= 0x30 {
            return self.irq.store(address, width, value);
        }
        match width {
            AccessWidth::Word => {
                self.store_latch(offset, value as u16);
                self.store_latch(offset + 2, (value >> 16) as u16);
            }
            AccessWidth::Half => self.store_latch(offset, value as u16),
            AccessWidth::Byte => {
                let aligned = offset & !1;
                let channel = &self.dma[(aligned / 12) as usize];
                let old = match aligned % 12 {
                    0 => channel.source as u16,
                    2 => (channel.source >> 16) as u16,
                    4 => channel.destination as u16,
                    6 => (channel.destination >> 16) as u16,
                    _ => channel.word_count,
                };
                let shift = (offset & 1) * 8;
                let merged = (old & !(0xff << shift)) | ((value as u16 & 0xff) << shift);
                self.store_latch(aligned, merged);
            }
        }
    }

    fn store_latch(&mut self, offset: u32, value: u16) {
        let index = (offset / 12) as usize;
        let channel = &mut self.dma[index];
        match offset % 12 {
            0 => channel.write_source_low(value),
            2 => channel.write_source_high(index, value),
            4 => channel.write_destination_low(value),
            6 => channel.write_destination_high(index, value),
            8 => channel.word_count = value,
            _ => unreachable!("a DMA control store runs on the bus"),
        }
    }
}

/// The interrupt registers of [`RunRegisters`].
impl IrqRegisters {
    /// Whether an access of `width` at the bus-aligned MMIO `address`
    /// touches only these registers: IE and IF, IME, and for stores POSTFLG
    /// and HALTCNT.
    #[inline(always)]
    pub(crate) fn holds(address: u32, width: AccessWidth, store: bool) -> bool {
        // Offsets from IE: one compare turns away the other registers.
        let offset = address.wrapping_sub(MMIO_BASE + 0x200);
        if offset > 0x101 {
            return false;
        }
        let last = offset + width.bytes() as u32 - 1;
        last <= 0x003
            || (0x008 <= offset && last <= 0x00b)
            || (store && 0x100 <= offset && last <= 0x101)
    }

    fn half(&self, offset: u32) -> Option<u16> {
        match offset {
            0x200 => Some(self.interrupt_enable_mask),
            0x202 => Some(self.interrupt_request_flags),
            0x208 => Some(u16::from(self.interrupt_master_enabled)),
            _ => None,
        }
    }

    /// A load [`Self::holds`] admits, as [`Bus::read_mmio`] makes it.
    pub(crate) fn load(&self, address: u32, width: AccessWidth) -> u32 {
        let offset = address.wrapping_sub(MMIO_BASE);
        let half = |offset: u32| u32::from(self.half(offset).unwrap_or(0));
        match width {
            AccessWidth::Word => half(offset) | half(offset + 2) << 16,
            AccessWidth::Half => half(offset),
            AccessWidth::Byte => (half(offset & !1) >> ((offset & 1) * 8)) & 0xff,
        }
    }

    /// A store [`Self::holds`] admits, as [`Bus::write_mmio`] makes it.
    pub(crate) fn store(&mut self, address: u32, width: AccessWidth, value: u32) {
        let offset = address.wrapping_sub(MMIO_BASE);
        match width {
            AccessWidth::Word => {
                self.store_half(offset, value as u16);
                self.store_half(offset + 2, (value >> 16) as u16);
            }
            AccessWidth::Half => self.store_half(offset, value as u16),
            AccessWidth::Byte => match offset {
                0x300 => self.post_boot = value as u8 & 1,
                0x301 => {
                    self.power_state =
                        if value & 0x80 != 0 { PowerState::Stopped } else { PowerState::Halted };
                }
                0x202 | 0x203 => {
                    self.interrupt_request_flags &= !((value as u16 & 0xff) << ((offset & 1) * 8));
                    self.update_line();
                }
                _ => {
                    let Some(old) = self.half(offset & !1) else {
                        return;
                    };
                    let shift = (offset & 1) * 8;
                    let merged = (old & !(0xff << shift)) | ((value as u16 & 0xff) << shift);
                    self.store_half(offset & !1, merged);
                }
            },
        }
    }

    fn store_half(&mut self, offset: u32, value: u16) {
        match offset {
            0x200 => self.interrupt_enable_mask = value & 0x3fff,
            0x202 => self.interrupt_request_flags &= !value,
            0x208 => self.interrupt_master_enabled = value & 1 != 0,
            0x300 => {
                self.post_boot = (value & 1) as u8;
                self.power_state =
                    if value & 0x8000 != 0 { PowerState::Stopped } else { PowerState::Halted };
            }
            _ => return,
        }
        self.update_line();
    }

    fn update_line(&mut self) {
        self.irq_line = self.interrupt_master_enabled
            && self.interrupt_enable_mask & self.interrupt_request_flags != 0;
    }
}

/// Palette RAM, VRAM and OAM with the render caches their writes
/// invalidate. Bus accesses and engine runs both go through this view.
pub(crate) struct VideoMemory<'a> {
    pub(super) palette: &'a mut [u8; PALETTE_SIZE],
    pub(super) vram: &'a mut [u8; VRAM_SIZE],
    pub(super) oam: &'a mut [u8; OAM_SIZE],
    pub(super) memo: &'a mut scanline_memo::Memo<ScanlineRegisters, SCREEN_HEIGHT>,
    pub(super) oam_compact_valid: &'a mut bool,
    pub(super) object_line_index: &'a mut u16,
    pub(super) dispcnt: u16,
}

impl VideoMemory<'_> {
    /// A read of palette RAM, VRAM or OAM (regions 5 to 7).
    #[inline(always)]
    pub(crate) fn read(&self, address: u32, width: AccessWidth) -> u32 {
        match address >> 24 {
            5 => palette_read(self.palette, address, width),
            6 => vram_read(self.vram, self.dispcnt, address, width),
            _ => oam_read(self.oam, address, width),
        }
    }

    /// A palette write: a byte lands in both bytes of its halfword.
    #[inline(always)]
    pub(crate) fn write_palette(&mut self, address: u32, width: AccessWidth, value: u32) {
        let address = (if width == AccessWidth::Byte { address & !1 } else { address }) as usize
            & (PALETTE_SIZE - 1);
        let mut changed = false;
        for offset in 0..width.bytes().max(2) {
            let byte = if width == AccessWidth::Byte {
                value as u8
            } else {
                (value >> (offset * 8)) as u8
            };
            let index = (address + offset) & (PALETTE_SIZE - 1);
            changed |= self.palette[index] != byte;
            self.palette[index] = byte;
        }
        if changed {
            self.memo.mark_palette();
        }
    }

    /// A VRAM write: a byte lands in both bytes of its halfword, and not at
    /// all in object tiles.
    #[inline(always)]
    pub(crate) fn write_vram(&mut self, address: u32, width: AccessWidth, value: u32) {
        if width == AccessWidth::Byte {
            let Some(index) = vram_index(address & !1, self.dispcnt) else {
                return;
            };
            if index >= vram_obj_boundary(self.dispcnt) {
                return;
            }
            for index in index..index + 2 {
                self.write_vram_byte(index, value as u8);
            }
            return;
        }
        if width == AccessWidth::Half && address & 1 == 0 {
            // Both bytes map alike and share a memo page.
            let Some(index) = vram_index(address, self.dispcnt) else {
                return;
            };
            let bytes = (value as u16).to_le_bytes();
            let slot: &mut [u8; 2] =
                (&mut self.vram[index..index + 2]).try_into().expect("two bytes");
            if *slot != bytes {
                self.memo.mark_vram(index);
                *slot = bytes;
            }
            return;
        }
        self.write_vram_bytes(address, width, value);
    }

    /// A halfword or word VRAM write a byte at a time: the reference for
    /// the halfword path above.
    fn write_vram_bytes(&mut self, address: u32, width: AccessWidth, value: u32) {
        for offset in 0..width.bytes() {
            if let Some(index) = vram_index(address.wrapping_add(offset as u32), self.dispcnt) {
                self.write_vram_byte(index, (value >> (offset * 8)) as u8);
            }
        }
    }

    #[inline(always)]
    fn write_vram_byte(&mut self, index: usize, value: u8) {
        if self.vram[index] != value {
            self.memo.mark_vram(index);
        }
        self.vram[index] = value;
    }

    /// An OAM write; byte writes are ignored.
    #[inline(always)]
    pub(crate) fn write_oam(&mut self, address: u32, width: AccessWidth, value: u32) {
        if width == AccessWidth::Byte {
            return;
        }
        write_ram(self.oam, address as usize & (OAM_SIZE - 1), width, value);
        self.memo.mark_oam();
        *self.oam_compact_valid = false;
        *self.object_line_index = 0xffff;
    }
}

pub(super) fn palette_read(palette: &[u8; PALETTE_SIZE], address: u32, width: AccessWidth) -> u32 {
    read_ram(palette, address as usize & (PALETTE_SIZE - 1), width)
}

pub(super) fn vram_read(
    vram: &[u8; VRAM_SIZE],
    dispcnt: u16,
    address: u32,
    width: AccessWidth,
) -> u32 {
    (0..width.bytes()).fold(0, |value, offset| {
        let byte =
            vram_index(address.wrapping_add(offset as u32), dispcnt).map_or(0, |index| vram[index]);
        value | (u32::from(byte) << (offset * 8))
    })
}

pub(super) fn oam_read(oam: &[u8; OAM_SIZE], address: u32, width: AccessWidth) -> u32 {
    read_ram(oam, address as usize & (OAM_SIZE - 1), width)
}

/// Game Pak ROM as CPU data loads see it while no prefetch is active.
pub(crate) struct RomData<'a> {
    bytes: &'a [u8],
    gpio: bool,
    eeprom: bool,
    large: bool,
    n: [u8; 3],
    s: [u8; 3],
}

impl RomData<'_> {
    /// A nonsequential CPU data load of the bus-aligned `address`: its value
    /// and cycles, or None where the ordinary bus path must decide (EEPROM,
    /// GPIO ports, reads beyond the image, save memory).
    #[inline]
    pub(crate) fn load(&self, address: u32, width: AccessWidth) -> Option<(u32, Cycle)> {
        if !(GAME_PAK_BASE..0x0e00_0000).contains(&address)
            || (width == AccessWidth::Half
                && self.eeprom
                && address >= 0x0d00_0000
                && (!self.large || address >= 0x0dff_ff00))
        {
            return None;
        }
        let first = ((address - GAME_PAK_BASE) & (GAME_PAK_WINDOW_SIZE - 1)) as usize;
        let end = first + width.bytes();
        if self.gpio && first <= 0xc9 && end > 0xc4 {
            return None;
        }
        let window = ((address - GAME_PAK_BASE) / GAME_PAK_WINDOW_SIZE) as usize;
        let n = Cycle::from(self.n[window]);
        if end > self.bytes.len() {
            return Some((self.beyond_image(address, width), self.cycles(address, width)));
        }
        Some(match width {
            AccessWidth::Byte => (u32::from(self.bytes[first]), n),
            AccessWidth::Half => {
                (u32::from(u16::from_le_bytes([self.bytes[first], self.bytes[first + 1]])), n)
            }
            AccessWidth::Word => (
                u32::from_le_bytes(self.bytes[first..end].try_into().expect("four bytes")),
                self.cycles(address, width),
            ),
        })
    }
}

impl RomData<'_> {
    /// The Thumb instruction at `address` if an engine run may execute it:
    /// it, the halfword after it and its own fetch of `address + 4` read
    /// plain image bytes, and neither that fetch nor the prefetch buffer
    /// ahead of it leaves the 128 KiB block.
    pub(crate) fn thumb_code(&self, address: u32) -> Option<u16> {
        if !(GAME_PAK_BASE..0x0e00_0000 - 4).contains(&address) {
            return None;
        }
        let fetch = address + 4;
        let first = ((address - GAME_PAK_BASE) & (GAME_PAK_WINDOW_SIZE - 1)) as usize;
        let end = first + 6;
        // Near a 128 KiB block end the fetch or the prefetch buffer would
        // cross into the next block, whose first access is nonsequential.
        if end > self.bytes.len()
            || (self.gpio && first < 0xca && end > 0xc4)
            || (self.eeprom && fetch >= 0x0d00_0000 && (!self.large || fetch >= 0x0dff_ff00))
            || address & 0x1ffff > 0x2_0000 - 22
        {
            return None;
        }
        Some(u16::from_le_bytes([self.bytes[first], self.bytes[first + 1]]))
    }

    /// Whether `bytes` bytes from `address` read as plain image bytes: no
    /// GPIO port, EEPROM or end of the image among them.
    pub(crate) fn plain(&self, address: u32, bytes: u32) -> bool {
        let first = ((address - GAME_PAK_BASE) & (GAME_PAK_WINDOW_SIZE - 1)) as usize;
        let end = first + bytes as usize;
        let last = address + bytes - 1;
        end <= self.bytes.len()
            && !(self.gpio && first < 0xca && end > 0xc4)
            && !(self.eeprom && last >= 0x0d00_0000 && (!self.large || last >= 0x0dff_ff00))
    }

    pub(crate) fn len(&self) -> usize {
        self.bytes.len()
    }

    /// The nonsequential and sequential halfword cycles of the Game Pak
    /// window holding `address`.
    pub(crate) fn window_cycles(&self, address: u32) -> (Cycle, Cycle) {
        let window = ((address - GAME_PAK_BASE) / GAME_PAK_WINDOW_SIZE) as usize;
        (Cycle::from(self.n[window]), Cycle::from(self.s[window]))
    }

    /// A halfword of the image, for a pipeline the engine hands back.
    pub(crate) fn half(&self, address: u32) -> u32 {
        let first = ((address - GAME_PAK_BASE) & (GAME_PAK_WINDOW_SIZE - 1)) as usize;
        u32::from(u16::from_le_bytes([self.bytes[first], self.bytes[first + 1]]))
    }

    fn cycles(&self, address: u32, width: AccessWidth) -> Cycle {
        let window = ((address - GAME_PAK_BASE) / GAME_PAK_WINDOW_SIZE) as usize;
        let n = Cycle::from(self.n[window]);
        if width != AccessWidth::Word {
            return n;
        }
        n + if (address + 2) & 0x1ffff == 0 { n } else { Cycle::from(self.s[window]) }
    }

    /// Bytes past the image read as the low address halfword, as in
    /// [`Bus::read_game_pak_rom`].
    #[cold]
    fn beyond_image(&self, address: u32, width: AccessWidth) -> u32 {
        (0..width.bytes()).fold(0, |value, offset| {
            let byte_address = address.wrapping_add(offset as u32);
            let index = ((byte_address - GAME_PAK_BASE) & (GAME_PAK_WINDOW_SIZE - 1)) as usize;
            let byte = self.bytes.get(index).copied().unwrap_or_else(|| {
                let halfword = (byte_address >> 1) as u16;
                if byte_address & 1 == 0 { halfword as u8 } else { (halfword >> 8) as u8 }
            });
            value | u32::from(byte) << (offset * 8)
        })
    }
}

impl Bus {
    /// The last cycle an engine run starting now may reach: before the next
    /// device event and no later than `yield_at`. None while DMA is pending.
    pub(crate) fn engine_limit(&self, yield_at: Cycle) -> Option<Cycle> {
        if !self.batch_active
            || self.pending_dma != 0
            || self.servicing_dma
            || self.batch_deadline <= self.current_cycle
            || yield_at <= self.current_cycle
        {
            return None;
        }
        let limit = yield_at.min(self.batch_deadline - 1);
        (limit > self.current_cycle).then_some(limit)
    }

    /// Whether code fetches have left the Game Pak: no prefetch state that
    /// an engine run in RAM would have to advance.
    pub(crate) fn game_pak_code_idle(&self) -> bool {
        !self.game_pak_code_active && !self.game_pak_prefetch.active
    }

    /// Memory for an engine run; the BIOS image only for a run of BIOS code.
    /// Inlined: a view returned through memory is read back wider than it
    /// was written, which stalls.
    #[inline(always)]
    pub(crate) fn engine_memory(&mut self, bios: bool) -> EngineMemory<'_> {
        // A stale cache is not refreshed here: that would change what the
        // ordinary bus later sees.
        let (n, s) = if self.game_pak_timing_key == self.waitcnt {
            (self.game_pak_n_cycles, self.game_pak_s_cycles)
        } else {
            game_pak_timing(self.waitcnt)
        };
        let size = self.rom.len().min(GAME_PAK_WINDOW_SIZE as usize);
        let quiet = self.video_quiet_cycles();
        EngineMemory {
            iwram: &mut self.iwram,
            ewram: &mut self.ewram,
            code: &mut self.code,
            bios: if bios && self.bios.len() == BIOS_SIZE { &self.bios[..] } else { &[] },
            rom: RomData {
                bytes: &self.rom[..size],
                gpio: self.cartridge_gpio.is_some(),
                eeprom: self.save.is_eeprom(),
                large: self.rom.len() > 16 * 1024 * 1024,
                n,
                s,
            },
            video: VideoMemory {
                palette: &mut self.palette,
                vram: &mut self.vram,
                oam: &mut self.oam,
                memo: &mut self.scanline_memo,
                oam_compact_valid: &mut self.oam_compact_valid,
                object_line_index: &mut self.object_line_index,
                dispcnt: self.dispcnt,
            },
            registers: RunRegisters { irq: &mut self.irq, dma: &mut self.dma_channels },
            quiet,
        }
    }

    /// Cycles from now, within the device batch, during which the PPU
    /// shows no visible dot: to the next line's start in HBlank, to the
    /// frame's end in VBlank, zero while a visible dot is drawn.
    #[inline(never)]
    fn video_quiet_cycles(&self) -> Cycle {
        let mut timing = self.ppu_timing;
        if self.batch_active {
            timing.elapse_before_boundary(self.current_cycle - self.batch_device_cycle);
        }
        let (line, dot) = (Cycle::from(timing.line), Cycle::from(timing.cycle_in_line));
        if line >= SCREEN_HEIGHT as Cycle {
            (228 - line) * 1232 - dot
        } else if dot >= 960 {
            1232 - dot
        } else {
            0
        }
    }

    /// Publishes the end of an engine run that executed at least one
    /// instruction: its clock, the open-bus latch and whether the last
    /// instruction ended with a data access.
    pub(crate) fn engine_finish(&mut self, cycle: Cycle, open_bus: u32, data_last: bool) {
        debug_assert!(cycle >= self.current_cycle);
        debug_assert!(cycle < self.batch_deadline || cycle == self.current_cycle);
        self.current_cycle = cycle;
        self.open_bus_value = open_bus;
        self.fetch_kind =
            if data_last { AccessKind::Nonsequential } else { AccessKind::Sequential };
    }

    /// Whether an engine run may make a CPU load of `width` at MMIO
    /// `address` itself: one whose value never includes the open-bus latch,
    /// which the bus does not hold during a run. As [`Bus::read_mmio`]
    /// reads; a register that reads only through a side path counts as
    /// unmapped, which only leaves more to the interpreter.
    pub(crate) fn engine_mmio_loadable(&self, address: u32, width: AccessWidth) -> bool {
        if address >> 24 != 4 {
            return false;
        }
        let half = address & !1;
        let other = if width == AccessWidth::Word { half + 2 } else { half ^ 2 };
        self.peek_mmio_half(half).is_some() || self.peek_mmio_half(other).is_some()
    }

    /// Whether an engine run may make a CPU store at MMIO `address` itself:
    /// any register but WAITCNT, which changes the Game Pak timing and
    /// prefetch state a run keeps to itself.
    pub(crate) fn engine_mmio_storable(&self, address: u32, width: AccessWidth) -> bool {
        let last = address + width.bytes() as u32 - 1;
        address >> 24 == 4 && (last < MMIO_BASE + 0x204 || address > MMIO_BASE + 0x207)
    }

    /// A CPU load an engine run makes from MMIO, starting at `cycle`: what
    /// [`Bus::read`] does for it, except the Game Pak prefetch buffer, whose
    /// state the run keeps. Leaves the clock after the access's cycle.
    pub(crate) fn engine_mmio_load(
        &mut self,
        cycle: Cycle,
        address: u32,
        width: AccessWidth,
    ) -> u32 {
        debug_assert!(self.engine_mmio_loadable(address, width));
        debug_assert!(self.batch_active && cycle >= self.current_cycle);
        debug_assert!(cycle + 1 < self.batch_deadline);
        self.current_cycle = cycle;
        self.flush_device_batch();
        self.fetch_kind = AccessKind::Sequential;
        let value = self.read_mmio(address, width);
        self.open_bus_value = value;
        self.current_cycle = cycle + 1;
        value
    }

    /// A CPU store an engine run makes to MMIO, whose cycle ends at
    /// `cycle`: what [`Bus::write`] does for it, except the Game Pak
    /// prefetch buffer, whose state the run keeps, and pending DMA, which
    /// [`Self::engine_may_go_on`] leaves to the caller.
    pub(crate) fn engine_mmio_store(
        &mut self,
        cycle: Cycle,
        address: u32,
        width: AccessWidth,
        value: u32,
    ) {
        debug_assert!(self.engine_mmio_storable(address, width));
        debug_assert!(self.batch_active && cycle > self.current_cycle);
        debug_assert!(cycle < self.batch_deadline);
        // The clock moves inside the batch, which then brings the PPU to it:
        // the store's cycle meets no device event.
        self.current_cycle = cycle;
        self.fetch_kind = AccessKind::Sequential;
        self.flush_device_batch();
        let deadline = self.batch_deadline;
        self.batch_active = false;
        self.batch_deadline = 0;
        self.write_mmio(address, width, value);
        self.batch_active = true;
        self.batch_device_cycle = self.current_cycle;
        // A register that schedules no device event leaves the next one
        // where it was: no need to find it again.
        self.batch_deadline = if schedules_nothing(address, width) {
            debug_assert_eq!(deadline, self.next_device_boundary(), "{address:08x}");
            deadline
        } else {
            self.next_device_boundary()
        };
    }

    /// Whether a run may go on to `limit` after an MMIO access: no DMA is
    /// pending, the CPU still runs, no IRQ it would take is raised
    /// (`irq_masked` is the CPSR's I bit) and no device event came before
    /// the limit.
    pub(crate) fn engine_may_go_on(&self, limit: Cycle, irq_masked: bool) -> bool {
        self.pending_dma == 0
            && self.irq.power_state == PowerState::Running
            && (irq_masked || !self.irq.irq_line)
            && limit < self.batch_deadline
    }
}

/// Whether a store to these MMIO bytes can change no device's next event:
/// background, window, mosaic and blend registers, IE and IF, IME, POSTFLG
/// and HALTCNT, whose writes schedule nothing.
fn schedules_nothing(address: u32, width: AccessWidth) -> bool {
    let offset = address.wrapping_sub(MMIO_BASE);
    let last = offset + width.bytes() as u32 - 1;
    (0x008 <= offset && last <= 0x05f)
        || (0x200 <= offset && last <= 0x203)
        || (0x208 <= offset && last <= 0x20b)
        || (0x300 <= offset && last <= 0x301)
}

pub fn begin_arm_iwram_slice(bus: &mut Bus) {
    bus.game_pak_code_active = false;
    bus.game_pak_prefetch = GamePakPrefetch::default();
}

pub fn fetch_arm_iwram_in_slice(bus: &mut Bus, address: u32) -> u32 {
    debug_assert!(is_arm_iwram_slice_fetch(address));
    bus.fetch_kind = AccessKind::Sequential;
    let value = bus.read_iwram(address, AccessWidth::Word);
    bus.complete_read(value, 1).value
}

/// The closed-form Game Pak prefetch schedule of a Thumb ROM run: the
/// buffer head's halfword is ready at `head_ready`, each later halfword one
/// `duty` after the previous, and the buffer holds at most `fill / duty + 1`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PrefetchClock {
    pub(crate) head_ready: Cycle,
    pub(crate) duty: Cycle,
    pub(crate) fill: Cycle,
}

impl Bus {
    /// The prefetch schedule of a Thumb ROM run whose next instruction is
    /// at `pc`, if the buffer is enabled and its state fits the closed form.
    pub(crate) fn thumb_prefetch_entry(&self, pc: u32) -> Option<PrefetchClock> {
        if self.waitcnt & (1 << 14) == 0
            || self.game_pak_timing_key != self.waitcnt
            || self.prefetch_disabled_bug_pending
            || self.prefetch_buffer_was_disabled
            || !(GAME_PAK_BASE..0x0e00_0000).contains(&pc)
        {
            return None;
        }
        let window = ((pc - GAME_PAK_BASE) / GAME_PAK_WINDOW_SIZE) as usize;
        let duty = Cycle::from(self.game_pak_s_cycles[window]);
        let fill = 7 * duty;
        let now = self.current_cycle;
        if now < 8 * duty {
            return None;
        }
        let prefetch = &self.game_pak_prefetch;
        let head = pc.wrapping_add(4);
        let head_ready = if prefetch.active && (prefetch.count != 0 || prefetch.countdown != 0) {
            if prefetch.opcode_bytes != 2
                || prefetch.capacity != 8
                || prefetch.duty != duty
                || prefetch.head_address != head
                || self.duty_valid_until != block_end(pc)
            {
                return None;
            }
            let buffered = u32::from(prefetch.count);
            if prefetch.countdown == 0 {
                if buffered != 8 || prefetch.last_address != head.wrapping_add(14) {
                    return None;
                }
                now - fill
            } else {
                if buffered >= 8
                    || prefetch.last_address != head.wrapping_add(2 * buffered)
                    || prefetch.countdown > duty
                {
                    return None;
                }
                now + prefetch.countdown - Cycle::from(buffered) * duty
            }
        } else {
            now + self.game_pak_half_cycles(head, self.fetch_kind)
        };
        Some(PrefetchClock { head_ready, duty, fill })
    }

    /// Publishes the end of a Thumb ROM run at `pc` with the buffer
    /// enabled: its clock, access kind, open bus and prefetch state.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn thumb_prefetch_finish(
        &mut self,
        pc: u32,
        now: Cycle,
        clock: &PrefetchClock,
        discarded: bool,
        data_last: bool,
        load_last: bool,
        last_fetch: u32,
    ) {
        debug_assert!(now >= self.current_cycle);
        debug_assert!(now < self.batch_deadline || now == self.current_cycle);
        self.current_cycle = now;
        self.fetch_kind =
            if data_last { AccessKind::Nonsequential } else { AccessKind::Sequential };
        if !load_last {
            self.open_bus_value = last_fetch;
        }
        self.duty_valid_until = block_end(pc);
        self.game_pak_code_active = true;
        let head = pc.wrapping_add(4);
        let (head_ready, duty) = (clock.head_ready, clock.duty);
        self.game_pak_prefetch = if discarded {
            GamePakPrefetch::default()
        } else if head_ready + clock.fill <= now {
            GamePakPrefetch {
                active: true,
                head_address: head,
                last_address: head.wrapping_add(14),
                count: 8,
                capacity: 8,
                opcode_bytes: 2,
                countdown: 0,
                duty,
            }
        } else {
            let buffered = if head_ready > now { 0 } else { (now - head_ready) / duty + 1 };
            GamePakPrefetch {
                active: true,
                head_address: head,
                last_address: head.wrapping_add(2 * buffered as u32),
                count: buffered as u8,
                capacity: 8,
                opcode_bytes: 2,
                countdown: head_ready + buffered * duty - now,
                duty,
            }
        };
    }
}

/// The first address after the 128 KiB Game Pak block holding `address`:
/// the prefetch buffer's duty is valid until there.
fn block_end(address: u32) -> u32 {
    (address | 0x1ffff).wrapping_add(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Loads and stores of every width at every byte of IE, IF, IME, POSTFLG,
    /// HALTCNT and the DMA registers that [`RunRegisters`] admits leave the
    /// registers, the IRQ line, the power state and the DMA channels, and
    /// read the values, that `Bus::read` and `Bus::write` do, from all
    /// kinds of starting values.
    #[test]
    fn run_registers_match_the_bus() {
        let mut seed = 0x1e1e_2020_u32;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        let state = |bus: &Bus| {
            let channels = bus.dma_channels.each_ref().map(|channel| {
                (
                    channel.source,
                    channel.destination,
                    channel.word_count,
                    channel.control,
                    channel.latched_source,
                    channel.latched_destination,
                )
            });
            (bus.irq, channels, bus.dma_start_cycles, bus.pending_dma)
        };
        let (mut loads, mut stores, mut dma) = (0, 0, 0);
        for _ in 0..8000 {
            let mut bus = Bus::init(&[]);
            bus.irq.interrupt_enable_mask = next() as u16 & 0x3fff;
            bus.irq.interrupt_request_flags = next() as u16 & 0x3fff;
            bus.irq.interrupt_master_enabled = next() & 1 != 0;
            bus.irq.post_boot = (next() & 1) as u8;
            bus.update_irq_line();
            for channel in &mut bus.dma_channels {
                channel.source = next();
                channel.destination = next();
                channel.word_count = next() as u16;
                channel.control = next() as u16 & 0xffe0;
                channel.latched_source = next();
                channel.latched_destination = next();
            }
            let width =
                [AccessWidth::Byte, AccessWidth::Half, AccessWidth::Word][next() as usize % 3];
            let store = next() & 1 != 0;
            let offset = match next() % 4 {
                0 => 0x200 + next() % 4,
                1 => 0x208 + next() % 4,
                2 => 0x300 + next() % 4,
                _ => 0xb0 + next() % 0x30,
            };
            let address = MMIO_BASE + (offset & !(width.bytes() as u32 - 1));
            if !RunRegisters::holds(address, width, store) {
                continue;
            }
            let value = next();
            let mut engine = Bus::init(&[]);
            engine.irq = bus.irq;
            for (to, from) in engine.dma_channels.iter_mut().zip(&bus.dma_channels) {
                (to.source, to.destination, to.word_count, to.control) =
                    (from.source, from.destination, from.word_count, from.control);
                (to.latched_source, to.latched_destination) =
                    (from.latched_source, from.latched_destination);
            }
            let access = Access {
                address,
                width,
                kind: AccessKind::Nonsequential,
                source: AccessSource::CpuData,
            };
            let context = format!("{address:08x} {width:?} store {store} value {value:08x}");
            let mut registers = engine.engine_memory(false).registers;
            if store {
                registers.store(address, width, value);
                bus.write(access, value);
                stores += 1;
            } else {
                let loaded = registers.load(address, width);
                assert_eq!(loaded, bus.read(access).value, "{context}");
                loads += 1;
            }
            dma += usize::from(offset < 0x200);
            assert_eq!(state(&engine), state(&bus), "{context}");
        }
        assert!(
            loads > 1000 && stores > 1000 && dma > 1000,
            "{loads} loads, {stores} stores, {dma} DMA"
        );
    }

    /// [`RunRegisters::holds`] admits exactly the accesses whose every byte
    /// is IE, IF or IME, or for stores POSTFLG or HALTCNT, or is a DMA
    /// register other than, for stores, a control register.
    #[test]
    fn run_registers_hold_exactly_their_bytes() {
        for store in [false, true] {
            for width in [AccessWidth::Byte, AccessWidth::Half, AccessWidth::Word] {
                let bytes = width.bytes() as u32;
                for offset in (0..0x1000).step_by(bytes as usize) {
                    let bytes = offset..offset + bytes;
                    let interrupt = bytes.clone().all(|byte| {
                        matches!(byte, 0x200..=0x203 | 0x208..=0x20b)
                            || (store && matches!(byte, 0x300 | 0x301))
                    });
                    let dma = bytes.clone().all(|byte| {
                        (0xb0..0xe0).contains(&byte) && !(store && (byte - 0xb0) % 12 >= 10)
                    });
                    let address = MMIO_BASE + offset;
                    assert_eq!(
                        RunRegisters::holds(address, width, store),
                        interrupt || dma,
                        "{address:08x} {width:?} store {store}"
                    );
                }
            }
        }
    }

    /// An aligned halfword VRAM write in one step stores, skips and marks
    /// exactly what two byte writes do, in every display mode, across the
    /// mirrors and the bitmap-mode hole.
    #[test]
    fn halfword_vram_writes_match_byte_writes() {
        let mut seed = 0x1234_5679_u32;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        let (mut fast, mut reference) = (Bus::init(&[]), Bus::init(&[]));
        let mut writes = 0;
        for trial in 0..20_000 {
            let dispcnt = (trial % 6) as u16;
            let address = 0x0600_0000 + (next() & 0x1_fffe);
            let value = if trial % 3 == 0 { next() & 0xff } else { next() };
            fast.dispcnt = dispcnt;
            reference.dispcnt = dispcnt;
            *fast.scanline_memo = Default::default();
            *reference.scanline_memo = Default::default();
            fast.video_memory().write_vram(address, AccessWidth::Half, value);
            reference.video_memory().write_vram_bytes(address, AccessWidth::Half, value);
            assert!(fast.vram[..] == reference.vram[..], "{address:08x} mode {dispcnt}");
            assert!(
                fast.scanline_memo.same_changes(&reference.scanline_memo),
                "marks {address:08x} mode {dispcnt}"
            );
            writes += usize::from(vram_index(address, dispcnt).is_some());
        }
        assert!(writes > 15_000, "only {writes} writes reached VRAM");
    }
}
