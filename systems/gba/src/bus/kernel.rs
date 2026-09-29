//! CPU execution kernels whose local clocks are strictly inside a device horizon.

use super::*;

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

/// The RAM mirror admitted for an ARM block transfer. Bytes are live and
/// accesses to the CPU-visible word align and wrap with `mask`.
pub struct RamWords<'a> {
    pub bytes: &'a mut [u8],
    pub mask: u32,
    pub cycles: Cycle,
}

pub struct ArmIwramExecutionRegion<'a> {
    bus: &'a mut Bus,
    pub cycle: Cycle,
    pub end: Cycle,
    pub data_end: Cycle,
    pub data_value: u32,
    pub data_read: bool,
}

impl<'a> ArmIwramExecutionRegion<'a> {
    pub fn begin(bus: &'a mut Bus, yield_at: Cycle) -> Option<Self> {
        if !bus.batch_active
            || bus.pending_dma != 0
            || bus.servicing_dma
            || bus.batch_deadline <= bus.current_cycle
            || yield_at <= bus.current_cycle
        {
            return None;
        }
        let end = yield_at.min(bus.batch_deadline - 1);
        if end <= bus.current_cycle {
            return None;
        }
        debug_assert!(!bus.game_pak_code_active && !bus.game_pak_prefetch.active);
        Some(Self {
            cycle: bus.current_cycle,
            end,
            bus,
            data_end: Cycle::MAX,
            data_value: 0,
            data_read: false,
        })
    }

    pub fn fetch(&mut self, address: u32) -> u32 {
        debug_assert!(self.cycle < self.end && is_arm_iwram_slice_fetch(address));
        self.cycle += 1;
        self.bus.read_iwram(address, AccessWidth::Word)
    }

    pub fn ram_cycles(address: u32, width: AccessWidth) -> Option<Cycle> {
        match address >> 24 {
            2 => Some(ewram_cycles(width)),
            3 => Some(1),
            _ => None,
        }
    }

    pub fn load_cycles(&mut self, address: u32, width: AccessWidth) -> Option<Cycle> {
        if let Some(cycles) = Self::ram_cycles(address, width) {
            return Some(cycles);
        }
        if !(8..=13).contains(&(address >> 24))
            || self.bus.game_pak_prefetch.active
            || (width == AccessWidth::Half
                && self.bus.save.is_eeprom()
                && self.bus.is_eeprom_address(address))
        {
            return None;
        }
        Some(self.bus.game_pak_cycles(Access {
            address,
            width,
            kind: AccessKind::Nonsequential,
            source: AccessSource::CpuData,
        }))
    }

    pub fn read_cpu_data(&mut self, address: u32, width: AccessWidth, _kind: AccessKind) -> u32 {
        self.data_value = match address >> 24 {
            2 => self.bus.read_ewram(address, width),
            3 => self.bus.read_iwram(address, width),
            _ => self.bus.read_game_pak_rom(address, width),
        };
        if let Some(cycles) = Self::ram_cycles(address, width) {
            self.cycle += cycles;
        } else {
            debug_assert!(!self.bus.game_pak_prefetch.active);
            self.bus.game_pak_prefetch = GamePakPrefetch::default();
            self.cycle += self.bus.game_pak_cycles(Access {
                address,
                width,
                kind: AccessKind::Nonsequential,
                source: AccessSource::CpuData,
            });
        }
        debug_assert!(self.cycle <= self.end);
        self.data_read = true;
        self.data_value
    }

    pub fn write(&mut self, access: Access, value: u32) -> WriteResult {
        let cycles = Self::ram_cycles(access.address, access.width).expect("admitted RAM write");
        self.cycle += cycles;
        debug_assert!(self.cycle <= self.end);
        if access.address >> 24 == 3 {
            self.bus.write_iwram(access.address, access.width, value);
        } else {
            self.bus.write_ewram(access.address, access.width, value);
        }
        self.data_read = false;
        WriteResult { cycles }
    }

    pub fn ram_words(&mut self, address: u32) -> RamWords<'_> {
        match address >> 24 {
            2 => {
                RamWords { bytes: &mut self.bus.ewram[..], mask: EWRAM_SIZE as u32 - 1, cycles: 6 }
            }
            3 => {
                RamWords { bytes: &mut self.bus.iwram[..], mask: IWRAM_SIZE as u32 - 1, cycles: 1 }
            }
            _ => unreachable!("admitted RAM block"),
        }
    }

    pub fn write_cpu_word(&mut self, address: u32, value: u32, kind: AccessKind) {
        self.write(
            Access { address, width: AccessWidth::Word, kind, source: AccessSource::CpuData },
            value,
        );
    }

    pub fn elapse_cpu_internal(&mut self, cycles: Cycle) {
        self.cycle += cycles;
        debug_assert!(self.cycle <= self.end);
    }

    pub fn complete_data(&mut self) {
        self.data_end = self.cycle;
    }

    /// Publishes the local clock and final latch; it consumes the exclusive
    /// Bus borrow so the caller may resume ordinary device access afterwards.
    pub fn finish(self, last_fetch: u32) {
        let bus = self.bus;
        if self.cycle == bus.current_cycle {
            return;
        }
        debug_assert!(self.cycle < bus.batch_deadline);
        let data_last = self.data_end == self.cycle;
        bus.current_cycle = self.cycle;
        bus.open_bus_value = if data_last && self.data_read { self.data_value } else { last_fetch };
        bus.fetch_kind = if data_last { AccessKind::Nonsequential } else { AccessKind::Sequential };
    }
}

/// An event-free Thumb ROM run. The Bus is exclusively borrowed until finish.
pub struct ThumbRomRun<'a> {
    bus: &'a mut Bus,
    pub view_address: u32,
    pub view_span: u32,
    pub cycle: Cycle,
    pub end: Cycle,
    pub bound: Cycle,
    pub head_ready: Cycle,
    pub data_end: Cycle,
    pub load_end: Cycle,
    pub discarded_at: Cycle,
    pub duty: Cycle,
    pub miss: Cycle,
    pub fill: Cycle,
}

impl<'a> ThumbRomRun<'a> {
    pub fn begin(
        _bus: &'a mut Bus,
        _view: &mut InstructionFetchRegion,
        _pc: u32,
        _yield_at: Cycle,
    ) -> Option<Self> {
        todo!("closed-form Thumb ROM admission")
    }

    pub fn admits(&self, _pc: u32) -> bool {
        todo!()
    }
    pub fn enter(&mut self, _pc: u32) {
        todo!()
    }
    pub fn fits(&self, _cycles: Cycle) -> bool {
        todo!()
    }
    pub fn fetch(&mut self, _address: u32) -> u32 {
        todo!()
    }
    pub fn complete_data(&mut self, _load: bool) {
        todo!()
    }
    pub fn store_cycles(address: u32, width: AccessWidth) -> Option<Cycle> {
        ArmIwramExecutionRegion::ram_cycles(address, width)
    }
    pub fn load_cycles(&self, _address: u32, _width: AccessWidth) -> Option<Cycle> {
        todo!()
    }
    pub fn read_cpu_data(&mut self, _address: u32, _width: AccessWidth, _kind: AccessKind) -> u32 {
        todo!()
    }
    pub fn write_cpu_data(&mut self, _access: Access, _value: u32) {
        todo!()
    }
    pub fn elapse_cpu_internal(&mut self, _cycles: Cycle) {
        todo!()
    }
    pub fn cpu_ram_block_cycles(&self, first: u32, beats: u32, internal: Cycle) -> Option<Cycle> {
        Self::block_cycles(first, beats, internal)
    }
    pub fn block_cycles(first: u32, beats: u32, internal: Cycle) -> Option<Cycle> {
        debug_assert!(first & 3 == 0 && beats != 0);
        let last = first.wrapping_add((beats - 1).wrapping_mul(4));
        if last < first || last >> 24 != first >> 24 {
            return None;
        }
        ArmIwramExecutionRegion::ram_cycles(first, AccessWidth::Word)
            .map(|per_word| per_word * Cycle::from(beats) + internal)
    }
    pub fn cpu_ram_word(&self, address: u32) -> u32 {
        self.bus.cpu_ram_word(address)
    }
    pub fn set_cpu_ram_word(&mut self, address: u32, value: u32) {
        self.bus.set_cpu_ram_word(address, value);
    }
    pub fn finish_cpu_ram_store_block(&mut self, cycles: Cycle) {
        self.elapse_cpu_internal(cycles);
    }
    pub fn finish_cpu_ram_load_block(&mut self, cycles: Cycle, last_word: u32) {
        self.bus.open_bus_value = last_word;
        self.elapse_cpu_internal(cycles);
    }
    pub fn refill(&mut self, _head: u32, _target: u32) -> [u32; 2] {
        todo!()
    }
    pub fn finish(self, _pc: u32, _last_fetch: u32) {
        todo!()
    }
}
