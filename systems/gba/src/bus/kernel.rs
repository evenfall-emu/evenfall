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
        bus: &'a mut Bus,
        view: &mut InstructionFetchRegion,
        pc: u32,
        yield_at: Cycle,
    ) -> Option<Self> {
        if !bus.batch_active
            || bus.pending_dma != 0
            || bus.servicing_dma
            || bus.batch_deadline <= bus.current_cycle
            || yield_at <= bus.current_cycle
            || bus.waitcnt & (1 << 14) == 0
            || bus.game_pak_timing_key != bus.waitcnt
            || bus.prefetch_disabled_bug_pending
            || bus.prefetch_buffer_was_disabled
        {
            return None;
        }
        let mut region = *view;
        if pc.wrapping_sub(region.address) >= region.valid_starts {
            region = bus.instruction_fetch_region(pc);
        }
        region.bytes?;
        let window = ((region.address - GAME_PAK_BASE) / GAME_PAK_WINDOW_SIZE) as usize;
        let duty = Cycle::from(bus.game_pak_s_cycles[window]);
        let now = bus.current_cycle;
        if now < 8 * duty {
            return None;
        }
        let mut run = Self {
            view_address: region.address,
            view_span: region.valid_starts.saturating_sub(4),
            cycle: now,
            end: yield_at.min(bus.batch_deadline - 1),
            bound: 0,
            head_ready: 0,
            data_end: 0,
            load_end: 0,
            discarded_at: 0,
            duty,
            miss: Cycle::from(bus.game_pak_n_cycles[window]),
            fill: 7 * duty,
            bus,
        };
        if !run.admits(pc) {
            return None;
        }
        let prefetch = &run.bus.game_pak_prefetch;
        let head = pc.wrapping_add(4);
        if prefetch.active && (prefetch.count != 0 || prefetch.countdown != 0) {
            if prefetch.opcode_bytes != 2
                || prefetch.capacity != 8
                || prefetch.duty != duty
                || prefetch.head_address != head
                || run.bus.duty_valid_until != Self::block_end(pc)
            {
                return None;
            }
            let buffered = u32::from(prefetch.count);
            if prefetch.countdown == 0 {
                if buffered != 8 || prefetch.last_address != head.wrapping_add(14) {
                    return None;
                }
                run.head_ready = now - run.fill;
            } else {
                if buffered >= 8
                    || prefetch.last_address != head.wrapping_add(2 * buffered)
                    || prefetch.countdown > duty
                {
                    return None;
                }
                run.head_ready = now + prefetch.countdown - Cycle::from(buffered) * duty;
            }
        } else {
            run.head_ready = now + run.bus.game_pak_half_cycles(head, run.bus.fetch_kind);
        }
        run.enter(pc);
        *view = region;
        Some(run)
    }

    fn block_end(address: u32) -> u32 {
        (address | 0x1ffff).wrapping_add(1)
    }

    pub fn admits(&self, pc: u32) -> bool {
        pc.wrapping_sub(self.view_address) < self.view_span && pc & 0x1ffff <= 0x20000 - 22
    }

    pub fn enter(&mut self, pc: u32) {
        debug_assert!(self.admits(pc));
        let limit = (self.view_address + self.view_span).min(Self::block_end(pc) - 21);
        self.bound = self.end.min(self.cycle + Cycle::from((limit - pc).div_ceil(2)));
    }

    pub fn fits(&self, cycles: Cycle) -> bool {
        self.cycle + cycles <= self.bound
    }

    pub fn fetch(&mut self, address: u32) -> u32 {
        let now = self.cycle;
        let ready = self.head_ready;
        self.cycle = (now + 1).max(ready);
        self.head_ready = ready.max(now - self.fill) + self.duty;
        let offset = ((address - GAME_PAK_BASE) & (GAME_PAK_WINDOW_SIZE - 1)) as usize;
        u32::from(u16::from_le_bytes(self.bus.rom[offset..offset + 2].try_into().unwrap()))
    }

    pub fn complete_data(&mut self, load: bool) {
        self.data_end = self.cycle;
        if load {
            self.load_end = self.cycle;
        }
    }
    pub fn store_cycles(address: u32, width: AccessWidth) -> Option<Cycle> {
        ArmIwramExecutionRegion::ram_cycles(address, width)
    }

    pub fn load_cycles(&self, address: u32, width: AccessWidth) -> Option<Cycle> {
        if let Some(cycles) = ArmIwramExecutionRegion::ram_cycles(address, width) {
            return Some(cycles);
        }
        if !(GAME_PAK_BASE..0x0d00_0000).contains(&address) {
            return None;
        }
        let first = ((address - GAME_PAK_BASE) & (GAME_PAK_WINDOW_SIZE - 1)) as usize;
        let end = first + width.bytes();
        if end > self.bus.rom.len()
            || end > GAME_PAK_WINDOW_SIZE as usize
            || (self.bus.cartridge_gpio.is_some() && first <= 0xc9 && end > 0xc4)
        {
            return None;
        }
        Some(Cycle::from(self.collides()) + self.rom_data_cycles(address, width))
    }

    fn collides(&self) -> bool {
        let next = self.cycle + 1;
        if self.head_ready > next || next - self.head_ready > self.fill {
            return false;
        }
        (next - self.head_ready).is_multiple_of(self.duty)
    }

    fn rom_data_cycles(&self, address: u32, width: AccessWidth) -> Cycle {
        match width {
            AccessWidth::Byte | AccessWidth::Half => {
                self.bus.game_pak_half_cycles(address & !1, AccessKind::Nonsequential)
            }
            AccessWidth::Word => {
                let aligned = address & !3;
                self.bus.game_pak_half_cycles(aligned, AccessKind::Nonsequential)
                    + self.bus.game_pak_half_cycles(aligned.wrapping_add(2), AccessKind::Sequential)
            }
        }
    }

    pub fn read_cpu_data(&mut self, address: u32, width: AccessWidth, kind: AccessKind) -> u32 {
        debug_assert_eq!(kind, AccessKind::Nonsequential);
        let value = match address >> 24 {
            2 => {
                self.cycle += ewram_cycles(width);
                self.bus.read_ewram(address, width)
            }
            3 => {
                self.cycle += 1;
                self.bus.read_iwram(address, width)
            }
            _ => {
                let first = ((address - GAME_PAK_BASE) & (GAME_PAK_WINDOW_SIZE - 1)) as usize;
                self.cycle += Cycle::from(self.collides()) + self.rom_data_cycles(address, width);
                self.discarded_at = self.cycle + 1;
                self.head_ready = self.discarded_at + self.miss;
                match width {
                    AccessWidth::Byte => u32::from(self.bus.rom[first]),
                    AccessWidth::Half => u32::from(u16::from_le_bytes(
                        self.bus.rom[first..first + 2].try_into().unwrap(),
                    )),
                    AccessWidth::Word => {
                        u32::from_le_bytes(self.bus.rom[first..first + 4].try_into().unwrap())
                    }
                }
            }
        };
        debug_assert!(self.cycle <= self.bound);
        self.bus.open_bus_value = value;
        value
    }

    pub fn write_cpu_data(&mut self, access: Access, value: u32) {
        debug_assert_eq!(access.source, AccessSource::CpuData);
        if access.address >> 24 == 3 {
            self.cycle += 1;
            self.bus.write_iwram(access.address, access.width, value);
        } else {
            debug_assert_eq!(access.address >> 24, 2);
            self.cycle += ewram_cycles(access.width);
            self.bus.write_ewram(access.address, access.width, value);
        }
        debug_assert!(self.cycle <= self.bound);
    }

    pub fn elapse_cpu_internal(&mut self, cycles: Cycle) {
        self.cycle += cycles;
        debug_assert!(self.cycle <= self.bound);
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
    pub fn refill(&mut self, head: u32, target: u32) -> [u32; 2] {
        debug_assert!(self.admits(target));
        let now = self.cycle;
        let offset = target.wrapping_sub(head);
        if offset != 0 {
            let lands = self.head_ready.max(now - self.fill) + Cycle::from(offset / 2) * self.duty;
            self.head_ready = if offset < 16 && lands > now && lands - self.duty <= now {
                lands
            } else {
                now + self.miss
            };
        }
        [self.fetch(target), self.fetch(target.wrapping_add(2))]
    }

    pub fn finish(self, pc: u32, last_fetch: u32) {
        let bus = self.bus;
        let now = self.cycle;
        debug_assert!(now > bus.current_cycle && now < bus.batch_deadline);
        bus.current_cycle = now;
        bus.fetch_kind =
            if self.data_end == now { AccessKind::Nonsequential } else { AccessKind::Sequential };
        if self.load_end != now {
            bus.open_bus_value = last_fetch;
        }
        bus.duty_valid_until = Self::block_end(pc);
        bus.game_pak_code_active = true;
        let head = pc.wrapping_add(4);
        bus.game_pak_prefetch = if self.discarded_at == now {
            GamePakPrefetch::default()
        } else if self.head_ready + self.fill <= now {
            GamePakPrefetch {
                active: true,
                head_address: head,
                last_address: head.wrapping_add(14),
                count: 8,
                capacity: 8,
                opcode_bytes: 2,
                countdown: 0,
                duty: self.duty,
            }
        } else {
            let buffered =
                if self.head_ready > now { 0 } else { (now - self.head_ready) / self.duty + 1 };
            GamePakPrefetch {
                active: true,
                head_address: head,
                last_address: head.wrapping_add(2 * buffered as u32),
                count: buffered as u8,
                capacity: 8,
                opcode_bytes: 2,
                countdown: self.head_ready + buffered * self.duty - now,
                duty: self.duty,
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Zig: "Thumb ROM kernel matches ordinary stepping for random code, queues and horizons".
    /// Compare the closed-form schedule against the scalar fetch/prefetch state
    /// machine, including full, in-flight, discarded and idle queues, branch
    /// refills, ROM data collisions, block/image edges and short horizons.
    #[test]
    fn thumb_rom_run_matches_scalar_prefetch() {
        let mut rom = vec![0; 0x30000];
        let mut seed = 0x7eed_7b01u32;
        for byte in &mut rom {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            *byte = seed as u8;
        }
        let mut compared = 0;
        let mut refused = 0;
        let mut rom_loads = 0;
        let mut branches = 0;
        let mut full_queues = 0;
        for wait in [0x4000, 0x4317, 0x43a8] {
            for pc in [0x0800_00d0, 0x0801_ffd0, 0x0a02_ffd0, 0x0c00_0100] {
                for count in [0, 1, 4, 7, 8, 9] {
                    for pause in [0, 1, 3, 30] {
                        for offset in [None, Some(0), Some(2), Some(6), Some(18), Some(0xffff_fff4)]
                        {
                            let mut fast = Bus::init(&rom);
                            let mut scalar = Bus::init(&rom);
                            for bus in [&mut fast, &mut scalar] {
                                bus.write_waitcnt(wait);
                                bus.refresh_game_pak_timing();
                                bus.begin_device_batch();
                                bus.elapse(100);
                                let duty = Cycle::from(
                                    bus.game_pak_s_cycles
                                        [((pc - GAME_PAK_BASE) / GAME_PAK_WINDOW_SIZE) as usize],
                                );
                                let head = pc + 4;
                                bus.game_pak_code_active = true;
                                bus.duty_valid_until = ThumbRomRun::block_end(pc);
                                bus.game_pak_prefetch = if count == 9 {
                                    bus.fetch_kind = AccessKind::Nonsequential;
                                    GamePakPrefetch::default()
                                } else {
                                    GamePakPrefetch {
                                        active: true,
                                        head_address: head,
                                        last_address: head + 2 * u32::from(count.min(7)),
                                        count: count.min(8),
                                        capacity: 8,
                                        opcode_bytes: 2,
                                        countdown: if count == 8 { 0 } else { duty - 1 },
                                        duty,
                                    }
                                };
                            }
                            let yield_at = fast.now() + if pause == 0 { 2 } else { 250 };
                            let mut view = InstructionFetchRegion::default();
                            let Some(mut run) =
                                ThumbRomRun::begin(&mut fast, &mut view, pc, yield_at)
                            else {
                                refused += 1;
                                continue;
                            };
                            let fetched = run.fetch(pc + 4);
                            if !run.fits(pause) {
                                refused += 1;
                                continue;
                            }
                            let actual = scalar.fetch_instruction(pc + 4, AccessWidth::Half);
                            assert_eq!(fetched, actual, "pc {pc:x} count {count}");
                            run.elapse_cpu_internal(pause);
                            scalar.elapse_cpu_internal(pause);
                            if pause == 3 {
                                let address = (pc & 0xfe00_0000) + 0x120;
                                let Some(cost) = run.load_cycles(address, AccessWidth::Word) else {
                                    refused += 1;
                                    continue;
                                };
                                if !run.fits(cost + 1) {
                                    refused += 1;
                                    continue;
                                }
                                let value = run.read_cpu_data(
                                    address,
                                    AccessWidth::Word,
                                    AccessKind::Nonsequential,
                                );
                                let reference = scalar.read_cpu_data(
                                    address,
                                    AccessWidth::Word,
                                    AccessKind::Nonsequential,
                                );
                                assert_eq!(value, reference);
                                run.elapse_cpu_internal(1);
                                scalar.elapse_cpu_internal(1);
                                run.complete_data(true);
                            }
                            let (waiting_pc, last_fetch) = if let Some(offset) = offset {
                                let target = (pc + 6).wrapping_add(offset);
                                if !run.admits(target) {
                                    refused += 1;
                                    continue;
                                }
                                let words = run.refill(pc + 6, target);
                                if run.cycle > run.bound {
                                    refused += 1;
                                    continue;
                                }
                                scalar.fetch_kind = AccessKind::Nonsequential;
                                let first = scalar.fetch_instruction(target, AccessWidth::Half);
                                scalar.fetch_kind = AccessKind::Sequential;
                                let second =
                                    scalar.fetch_instruction(target + 2, AccessWidth::Half);
                                assert_eq!(
                                    words,
                                    [first, second],
                                    "pc {pc:x} target {target:x} count {count}"
                                );
                                (target, second)
                            } else {
                                (pc + 2, fetched)
                            };
                            run.finish(waiting_pc, last_fetch);
                            assert_eq!(
                                fast.now(),
                                scalar.now(),
                                "pc {pc:x} count {count} pause {pause} offset {offset:?}"
                            );
                            assert_eq!(fast.open_bus_value, scalar.open_bus_value);
                            assert_eq!(fast.fetch_kind, scalar.fetch_kind);
                            assert_eq!(
                                fast.game_pak_prefetch, scalar.game_pak_prefetch,
                                "pc {pc:x} count {count} pause {pause} offset {offset:?}"
                            );
                            assert_eq!(fast.duty_valid_until, scalar.duty_valid_until);
                            assert_eq!(fast.game_pak_code_active, scalar.game_pak_code_active);
                            compared += 1;
                            rom_loads += usize::from(pause == 3);
                            branches += usize::from(offset.is_some());
                            full_queues += usize::from(count == 8);
                        }
                    }
                }
            }
        }
        assert!(compared > 800, "only {compared} schedules compared");
        assert!(rom_loads > 50 && branches > 500 && full_queues > 50);
        assert!(refused > 0, "short horizons must refuse some runs");
    }

    #[test]
    fn thumb_rom_run_refuses_image_and_block_ends_without_changing_view() {
        let rom = vec![0; 0x30000];
        let mut bus = Bus::init(&rom);
        bus.write_waitcnt(0x4000);
        bus.refresh_game_pak_timing();
        bus.begin_device_batch();
        bus.elapse(100);
        let mut view = bus.instruction_fetch_region(0x0800_0100);
        for pc in [0x0801_fff0, 0x0802_fffa] {
            let before = view;
            let now = bus.now();
            assert!(ThumbRomRun::begin(&mut bus, &mut view, pc, now + 100).is_none());
            assert_eq!(view.address, before.address);
            assert_eq!(view.valid_starts, before.valid_starts);
            assert_eq!(bus.now(), now);
        }
        bus.end_device_batch();
    }
}
