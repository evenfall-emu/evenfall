//! Thumb decoder and transfers on ARM7TDMI (ARMv4T).

use crate::{AccessKind, AccessWidth, Bus, Core, ExecuteResult};

impl Core {
    /// Execute the latched Thumb halfword. The caller handles fetch-ahead,
    /// branch refills, IRQ boundaries and the platform's SWI implementation.
    pub fn execute_thumb<B: Bus>(&mut self, bus: &mut B, instruction: u16) -> ExecuteResult {
        let high = (instruction >> 8) as u8;
        match high {
            0x00..=0x17 => self.thumb_shift_immediate(instruction),
            0x18..=0x1f => self.thumb_add_subtract(instruction),
            0x20..=0x3f => self.thumb_immediate_alu(instruction),
            0x40..=0x43 => self.thumb_register_alu(bus, instruction),
            0x44..=0x47 => self.thumb_high_register(instruction),
            0x48..=0x4f => {
                let address =
                    (self.registers[15] & !3).wrapping_add(u32::from(instruction & 255) << 2);
                self.registers[((instruction >> 8) & 7) as usize] =
                    bus.read_cpu_data(address, AccessWidth::Word, AccessKind::Nonsequential);
                bus.elapse_cpu_internal(1);
                ExecuteResult::default()
            }
            0x50..=0x5f => self.thumb_register_offset_transfer(bus, instruction),
            0x60..=0x7f => self.thumb_immediate_transfer(bus, instruction),
            0x80..=0x8f => self.thumb_half_transfer(bus, instruction),
            0x90..=0x9f => self.thumb_stack_transfer(bus, instruction),
            0xa0..=0xaf => {
                let base = if instruction & (1 << 11) != 0 {
                    self.registers[13]
                } else {
                    self.registers[15] & !3
                };
                self.registers[((instruction >> 8) & 7) as usize] =
                    base.wrapping_add(u32::from(instruction & 255) << 2);
                ExecuteResult { bus_free: true, ..ExecuteResult::default() }
            }
            0xb0 => {
                let offset = u32::from(instruction & 127) << 2;
                self.registers[13] = if instruction & (1 << 7) != 0 {
                    self.registers[13].wrapping_sub(offset)
                } else {
                    self.registers[13].wrapping_add(offset)
                };
                ExecuteResult { bus_free: true, ..ExecuteResult::default() }
            }
            0xb4 | 0xb5 | 0xbc | 0xbd => self.thumb_push_pop(bus, instruction),
            0xc0..=0xcf => self.thumb_multiple_transfer(bus, instruction),
            0xd0..=0xdd => {
                if !self.condition_passed(u32::from(high & 15)) {
                    return ExecuteResult { bus_free: true, ..ExecuteResult::default() };
                }
                let offset = ((i32::from(instruction as u8 as i8)) << 1) as u32;
                self.next_instruction_address = self.registers[15].wrapping_add(offset);
                ExecuteResult { branched: true, ..ExecuteResult::default() }
            }
            0xdf => ExecuteResult {
                software_interrupt: Some(instruction as u8),
                ..ExecuteResult::default()
            },
            0xe0..=0xe7 => {
                let offset = (((u32::from(instruction & 0x7ff) << 21) as i32) >> 20) as u32;
                self.next_instruction_address = self.registers[15].wrapping_add(offset);
                ExecuteResult { branched: true, ..ExecuteResult::default() }
            }
            0xf0..=0xf7 => {
                let offset = (((u32::from(instruction & 0x7ff) << 21) as i32) >> 9) as u32;
                self.registers[14] = self.registers[15].wrapping_add(offset);
                ExecuteResult { bus_free: true, ..ExecuteResult::default() }
            }
            0xf8..=0xff => {
                let target = self.registers[14].wrapping_add(u32::from(instruction & 0x7ff) << 1);
                self.registers[14] = self.next_instruction_address.wrapping_add(2) | 1;
                self.next_instruction_address = target & !1;
                ExecuteResult { branched: true, ..ExecuteResult::default() }
            }
            // ARMv4T Undefined encodings (including the ARMv5 BLX suffix).
            0xb1..=0xb3 | 0xb6..=0xbb | 0xbe | 0xbf | 0xde | 0xe8..=0xef => {
                self.enter_undefined(self.next_instruction_address.wrapping_add(2));
                ExecuteResult { branched: true, ..ExecuteResult::default() }
            }
        }
    }

    fn thumb_shift_immediate(&mut self, instruction: u16) -> ExecuteResult {
        let shift_type = u32::from(instruction >> 11);
        let amount = u32::from((instruction >> 6) & 31);
        let source = self.registers[((instruction >> 3) & 7) as usize];
        let shifted = match (shift_type, amount) {
            (0, 0) => crate::ShifterResult { value: source, carry: None },
            (0, n) => crate::ShifterResult {
                value: source << n,
                carry: Some(source & (1 << (32 - n)) != 0),
            },
            (1, 0) => crate::ShifterResult { value: 0, carry: Some(source & (1 << 31) != 0) },
            (1, n) => crate::ShifterResult {
                value: source >> n,
                carry: Some(source & (1 << (n - 1)) != 0),
            },
            (2, 0) => crate::ShifterResult {
                value: ((source as i32) >> 31) as u32,
                carry: Some(source & (1 << 31) != 0),
            },
            (2, n) => crate::ShifterResult {
                value: ((source as i32) >> n) as u32,
                carry: Some(source & (1 << (n - 1)) != 0),
            },
            _ => unreachable!(),
        };
        self.registers[(instruction & 7) as usize] = shifted.value;
        self.set_logical_flags(shifted.value, shifted.carry);
        ExecuteResult { bus_free: true, ..ExecuteResult::default() }
    }

    fn thumb_add_subtract(&mut self, instruction: u16) -> ExecuteResult {
        let operand = ((instruction >> 6) & 7) as usize;
        let source = ((instruction >> 3) & 7) as usize;
        let destination = (instruction & 7) as usize;
        let lhs = self.registers[source];
        let rhs =
            if instruction & (1 << 10) != 0 { operand as u32 } else { self.registers[operand] };
        let subtract = instruction & (1 << 9) != 0;
        let result = if subtract { lhs.wrapping_sub(rhs) } else { lhs.wrapping_add(rhs) };
        self.registers[destination] = result;
        if subtract {
            self.set_sub_flags(lhs, rhs, 0, result);
        } else {
            self.set_add_flags(lhs, rhs, 0, result);
        }
        ExecuteResult { bus_free: true, ..ExecuteResult::default() }
    }

    fn thumb_immediate_alu(&mut self, instruction: u16) -> ExecuteResult {
        let rd = ((instruction >> 8) & 7) as usize;
        let rhs = u32::from(instruction & 255);
        let lhs = self.registers[rd];
        match (instruction >> 11) & 3 {
            0 => {
                self.registers[rd] = rhs;
                self.set_logical_flags(rhs, None);
            }
            1 => self.set_sub_flags(lhs, rhs, 0, lhs.wrapping_sub(rhs)),
            2 => {
                let result = lhs.wrapping_add(rhs);
                self.registers[rd] = result;
                self.set_add_flags(lhs, rhs, 0, result);
            }
            3 => {
                let result = lhs.wrapping_sub(rhs);
                self.registers[rd] = result;
                self.set_sub_flags(lhs, rhs, 0, result);
            }
            _ => unreachable!(),
        }
        ExecuteResult { bus_free: true, ..ExecuteResult::default() }
    }

    fn thumb_register_alu<B: Bus>(&mut self, bus: &mut B, instruction: u16) -> ExecuteResult {
        let opcode = (instruction >> 6) & 15;
        let rs = ((instruction >> 3) & 7) as usize;
        let rd = (instruction & 7) as usize;
        let lhs = self.registers[rd];
        let rhs = self.registers[rs];
        let carry = u32::from(self.cpsr & (1 << 29) != 0);
        match opcode {
            2 | 3 | 4 | 7 => {
                let kind = match opcode {
                    2 => 0,
                    3 => 1,
                    4 => 2,
                    7 => 3,
                    _ => unreachable!(),
                };
                let shifted = Self::decode_register_shift(lhs, kind, rhs as u8);
                self.registers[rd] = shifted.value;
                self.set_logical_flags(shifted.value, shifted.carry);
                bus.elapse_cpu_internal(1);
            }
            5 => {
                let result = lhs.wrapping_add(rhs).wrapping_add(carry);
                self.registers[rd] = result;
                self.set_add_flags(lhs, rhs, carry, result);
            }
            6 => {
                let borrow = 1 - carry;
                let result = lhs.wrapping_sub(rhs).wrapping_sub(borrow);
                self.registers[rd] = result;
                self.set_sub_flags(lhs, rhs, borrow, result);
            }
            9 => {
                let result = 0u32.wrapping_sub(rhs);
                self.registers[rd] = result;
                self.set_sub_flags(0, rhs, 0, result);
            }
            8 => self.set_logical_flags(lhs & rhs, None),
            10 => self.set_sub_flags(lhs, rhs, 0, lhs.wrapping_sub(rhs)),
            11 => self.set_add_flags(lhs, rhs, 0, lhs.wrapping_add(rhs)),
            13 => {
                self.registers[rd] = lhs.wrapping_mul(rhs);
                self.set_logical_flags(self.registers[rd], None);
                bus.elapse_cpu_internal(Self::multiply_iterations(lhs, true));
            }
            0 | 1 | 12 | 14 | 15 => {
                let result = match opcode {
                    0 => lhs & rhs,
                    1 => lhs ^ rhs,
                    12 => lhs | rhs,
                    14 => lhs & !rhs,
                    15 => !rhs,
                    _ => unreachable!(),
                };
                self.registers[rd] = result;
                self.set_logical_flags(result, None);
            }
            _ => unreachable!(),
        }
        ExecuteResult {
            bus_free: !matches!(opcode, 2 | 3 | 4 | 7 | 13),
            ..ExecuteResult::default()
        }
    }

    fn thumb_high_register(&mut self, instruction: u16) -> ExecuteResult {
        let operation = (instruction >> 8) & 3;
        let rs = ((instruction >> 3) & 15) as usize;
        let rd = ((instruction & 7) | ((instruction >> 4) & 8)) as usize;
        let source = self.registers[rs];
        match operation {
            0 => {
                let result = self.registers[rd].wrapping_add(source);
                if rd == 15 {
                    self.next_instruction_address = result & !1;
                    return ExecuteResult { branched: true, ..ExecuteResult::default() };
                }
                self.registers[rd] = result;
            }
            1 => {
                let lhs = self.registers[rd];
                self.set_sub_flags(lhs, source, 0, lhs.wrapping_sub(source));
            }
            2 => {
                if rd == 15 {
                    self.next_instruction_address = source & !1;
                    return ExecuteResult { branched: true, ..ExecuteResult::default() };
                }
                self.registers[rd] = source;
            }
            3 => {
                if source & 1 != 0 {
                    self.cpsr |= 1 << 5;
                    self.next_instruction_address = source & !1;
                } else {
                    self.cpsr &= !(1 << 5);
                    self.next_instruction_address = source & !3;
                }
                return ExecuteResult { branched: true, ..ExecuteResult::default() };
            }
            _ => unreachable!(),
        }
        ExecuteResult { bus_free: true, ..ExecuteResult::default() }
    }

    fn thumb_register_offset_transfer<B: Bus>(
        &mut self,
        bus: &mut B,
        instruction: u16,
    ) -> ExecuteResult {
        let operation = (instruction >> 9) & 7;
        let offset = self.registers[((instruction >> 6) & 7) as usize];
        let base = self.registers[((instruction >> 3) & 7) as usize];
        let rd = (instruction & 7) as usize;
        let address = base.wrapping_add(offset);
        let width = match operation {
            0 | 4 => AccessWidth::Word,
            1 | 5 | 7 => AccessWidth::Half,
            _ => AccessWidth::Byte,
        };
        let aligned = if crate::byte_wide_save_address(address) {
            address
        } else {
            match width {
                AccessWidth::Word => address & !3,
                AccessWidth::Half => address & !1,
                AccessWidth::Byte => address,
            }
        };
        if operation >= 3 {
            let read = bus.read_cpu_data(aligned, width, AccessKind::Nonsequential);
            self.registers[rd] = match operation {
                3 => (read as u8 as i8 as i32) as u32,
                4 => read.rotate_right((address & 3) * 8),
                5 => read.rotate_right((address & 1) * 8),
                7 if address & 1 != 0 => ((read >> 8) as u8 as i8 as i32) as u32,
                7 => (read as u16 as i16 as i32) as u32,
                _ => read,
            };
            bus.elapse_cpu_internal(1);
        } else {
            bus.write_cpu_data(aligned, width, AccessKind::Nonsequential, self.registers[rd]);
        }
        ExecuteResult::default()
    }

    fn thumb_immediate_transfer<B: Bus>(&mut self, bus: &mut B, instruction: u16) -> ExecuteResult {
        let byte = instruction & (1 << 12) != 0;
        let load = instruction & (1 << 11) != 0;
        let immediate = u32::from((instruction >> 6) & 31);
        let base = self.registers[((instruction >> 3) & 7) as usize];
        let rd = (instruction & 7) as usize;
        let address = base.wrapping_add(if byte { immediate } else { immediate << 2 });
        self.thumb_transfer(
            bus,
            address,
            rd,
            if byte { AccessWidth::Byte } else { AccessWidth::Word },
            load,
        )
    }

    fn thumb_half_transfer<B: Bus>(&mut self, bus: &mut B, instruction: u16) -> ExecuteResult {
        let load = instruction & (1 << 11) != 0;
        let address = self.registers[((instruction >> 3) & 7) as usize]
            .wrapping_add(u32::from((instruction >> 6) & 31) << 1);
        self.thumb_transfer(bus, address, (instruction & 7) as usize, AccessWidth::Half, load)
    }

    fn thumb_stack_transfer<B: Bus>(&mut self, bus: &mut B, instruction: u16) -> ExecuteResult {
        let load = instruction & (1 << 11) != 0;
        let address = self.registers[13].wrapping_add(u32::from(instruction & 255) << 2);
        self.thumb_transfer(
            bus,
            address,
            ((instruction >> 8) & 7) as usize,
            AccessWidth::Word,
            load,
        )
    }

    fn thumb_transfer<B: Bus>(
        &mut self,
        bus: &mut B,
        address: u32,
        rd: usize,
        width: AccessWidth,
        load: bool,
    ) -> ExecuteResult {
        let aligned = if crate::byte_wide_save_address(address) {
            address
        } else {
            match width {
                AccessWidth::Word => address & !3,
                AccessWidth::Half => address & !1,
                AccessWidth::Byte => address,
            }
        };
        if load {
            let read = bus.read_cpu_data(aligned, width, AccessKind::Nonsequential);
            self.registers[rd] = read.rotate_right(match width {
                AccessWidth::Word => (address & 3) * 8,
                AccessWidth::Half => (address & 1) * 8,
                AccessWidth::Byte => 0,
            });
            bus.elapse_cpu_internal(1);
        } else {
            bus.write_cpu_data(aligned, width, AccessKind::Nonsequential, self.registers[rd]);
        }
        ExecuteResult::default()
    }

    fn thumb_multiple_transfer<B: Bus>(&mut self, bus: &mut B, instruction: u16) -> ExecuteResult {
        let load = instruction & (1 << 11) != 0;
        let rb = ((instruction >> 8) & 7) as usize;
        let list = instruction as u8;
        let base = self.registers[rb];
        if list == 0 {
            self.registers[rb] = base.wrapping_add(64);
            if load {
                let value =
                    bus.read_cpu_data(base & !3, AccessWidth::Word, AccessKind::Nonsequential);
                self.next_instruction_address = value & !1;
                bus.elapse_cpu_internal(1);
                return ExecuteResult { branched: true, ..ExecuteResult::default() };
            }
            bus.write_cpu_word(
                base & !3,
                self.registers[15].wrapping_add(2),
                AccessKind::Nonsequential,
            );
            return ExecuteResult::default();
        }
        let final_base = base.wrapping_add(list.count_ones() * 4);
        let mut address = base;
        let mut last_word = 0;
        let block_cycles = bus.cpu_ram_block_cycles(base & !3, list.count_ones(), u64::from(load));
        for (index, register) in (0..8).filter(|&r| list & (1 << r) != 0).enumerate() {
            let kind = if index == 0 { AccessKind::Nonsequential } else { AccessKind::Sequential };
            if load {
                let value = if block_cycles.is_some() {
                    bus.cpu_ram_word(address & !3)
                } else {
                    bus.read_cpu_data(address & !3, AccessWidth::Word, kind)
                };
                last_word = value;
                self.registers[register] = value;
            } else {
                let value = if register == rb && index != 0 {
                    final_base
                } else {
                    self.registers[register]
                };
                if block_cycles.is_some() {
                    bus.set_cpu_ram_word(address & !3, value);
                } else {
                    bus.write_cpu_word(address & !3, value, kind);
                }
            }
            address = address.wrapping_add(4);
        }
        if !load || list & (1 << rb) == 0 {
            self.registers[rb] = final_base;
        }
        if let Some(cycles) = block_cycles {
            if load {
                bus.finish_cpu_ram_load_block(cycles, last_word);
            } else {
                bus.finish_cpu_ram_store_block(cycles);
            }
        } else if load {
            bus.elapse_cpu_internal(1);
        }
        ExecuteResult::default()
    }

    fn thumb_push_pop<B: Bus>(&mut self, bus: &mut B, instruction: u16) -> ExecuteResult {
        let pop = instruction & (1 << 11) != 0;
        let extra = instruction & (1 << 8) != 0;
        let list = instruction as u8;
        let count = list.count_ones() + u32::from(extra);
        let base = self.registers[13];
        if count == 0 {
            let address = if pop { base } else { base.wrapping_sub(64) };
            self.registers[13] = if pop { base.wrapping_add(64) } else { address };
            if pop {
                let value =
                    bus.read_cpu_data(address & !3, AccessWidth::Word, AccessKind::Nonsequential);
                self.next_instruction_address = value & !1;
                bus.elapse_cpu_internal(1);
                return ExecuteResult { branched: true, ..ExecuteResult::default() };
            }
            bus.write_cpu_word(
                address & !3,
                self.registers[15].wrapping_add(2),
                AccessKind::Nonsequential,
            );
            return ExecuteResult::default();
        }
        let start = if pop { base } else { base.wrapping_sub(count * 4) };
        let block_cycles = bus.cpu_ram_block_cycles(start & !3, count, u64::from(pop));
        let mut address = start;
        let mut last_word = 0;
        if !pop && block_cycles.is_none() {
            self.registers[13] = start;
        }
        for (index, register) in (0..8).filter(|&r| list & (1 << r) != 0).enumerate() {
            let kind = if index == 0 { AccessKind::Nonsequential } else { AccessKind::Sequential };
            if pop {
                let value = if block_cycles.is_some() {
                    bus.cpu_ram_word(address & !3)
                } else {
                    bus.read_cpu_data(address & !3, AccessWidth::Word, kind)
                };
                last_word = value;
                self.registers[register] = value;
            } else if block_cycles.is_some() {
                bus.set_cpu_ram_word(address & !3, self.registers[register]);
            } else {
                bus.write_cpu_word(address & !3, self.registers[register], kind);
            }
            address = address.wrapping_add(4);
        }
        if extra {
            let kind = if list == 0 { AccessKind::Nonsequential } else { AccessKind::Sequential };
            if pop {
                last_word = if block_cycles.is_some() {
                    bus.cpu_ram_word(address & !3)
                } else {
                    bus.read_cpu_data(address & !3, AccessWidth::Word, kind)
                };
                self.next_instruction_address = last_word & !1;
            } else if block_cycles.is_some() {
                bus.set_cpu_ram_word(address & !3, self.registers[14]);
            } else {
                bus.write_cpu_word(address & !3, self.registers[14], kind);
            }
            address = address.wrapping_add(4);
        }
        if pop {
            self.registers[13] = address;
        } else {
            self.registers[13] = start;
        }
        if let Some(cycles) = block_cycles {
            if pop {
                bus.finish_cpu_ram_load_block(cycles, last_word);
            } else {
                bus.finish_cpu_ram_store_block(cycles);
            }
        } else if pop {
            bus.elapse_cpu_internal(1);
        }
        ExecuteResult { branched: pop && extra, ..ExecuteResult::default() }
    }
}
