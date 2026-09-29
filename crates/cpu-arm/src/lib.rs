//! ARM7TDMI instruction state and bus-neutral execution primitives.
//!
//! The bus, not the interpreter, prices each N/S/I access. The type parameter
//! permits both the hardware bus and the local RAM/ROM kernel views to inline.

pub type Cycle = u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstructionSet {
    Arm,
    Thumb,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessWidth {
    Byte,
    Half,
    Word,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessKind {
    Nonsequential,
    Sequential,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pipeline {
    pub instructions: [u32; 2],
    pub instruction_set: InstructionSet,
}

/// Only operations issued by the instruction executors, not scheduling,
/// prefetch or device inspection. Implementations are statically dispatched.
pub trait Bus {
    fn read_cpu_data(&mut self, address: u32, width: AccessWidth, kind: AccessKind) -> u32;
    fn write_cpu_data(&mut self, address: u32, width: AccessWidth, kind: AccessKind, value: u32);
    fn elapse_cpu_internal(&mut self, cycles: Cycle);

    #[inline]
    fn write_cpu_word(&mut self, address: u32, value: u32, kind: AccessKind) {
        self.write_cpu_data(address, AccessWidth::Word, kind, value);
    }

    /// Fast RAM block admission; None retains the ordinary per-beat path.
    #[inline]
    fn cpu_ram_block_cycles(&self, _first: u32, _beats: u32, _internal: Cycle) -> Option<Cycle> {
        None
    }

    fn cpu_ram_word(&self, address: u32) -> u32;
    fn set_cpu_ram_word(&mut self, address: u32, value: u32);
    fn finish_cpu_ram_load_block(&mut self, cycles: Cycle, last_word: u32);
    fn finish_cpu_ram_store_block(&mut self, cycles: Cycle);
}

/// The flat prefix of ZiGBA's `Cpu` state, through `spsr_undefined`.
/// The GBA wrapper flattens this into the same durable field order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Core {
    pub registers: [u32; 16],
    pub cpsr: u32,
    pub next_instruction_address: u32,
    pub pipeline: Option<Pipeline>,
    pub user_r8_12: [u32; 5],
    pub user_r13: u32,
    pub user_r14: u32,
    pub fiq_r8_14: [u32; 7],
    pub spsr_fiq: u32,
    pub irq_r13: u32,
    pub irq_r14: u32,
    pub spsr_irq: u32,
    pub supervisor_r13: u32,
    pub supervisor_r14: u32,
    pub spsr_supervisor: u32,
    pub abort_r13: u32,
    pub abort_r14: u32,
    pub spsr_abort: u32,
    pub undefined_r13: u32,
    pub undefined_r14: u32,
    pub spsr_undefined: u32,
}

impl Core {
    pub fn new(entry_address: u32, initial_cpsr: u32) -> Self {
        let mut core = Self {
            registers: [0; 16],
            cpsr: initial_cpsr,
            next_instruction_address: entry_address,
            pipeline: None,
            user_r8_12: [0; 5],
            user_r13: 0x0300_7F00,
            user_r14: 0,
            fiq_r8_14: [0; 7],
            spsr_fiq: 0,
            irq_r13: 0x0300_7FA0,
            irq_r14: 0,
            spsr_irq: 0,
            supervisor_r13: 0x0300_7FE0,
            supervisor_r14: 0,
            spsr_supervisor: 0,
            abort_r13: 0,
            abort_r14: 0,
            spsr_abort: 0,
            undefined_r13: 0,
            undefined_r14: 0,
            spsr_undefined: 0,
        };
        core.registers[13] = match initial_cpsr & 0x1F {
            0x10 | 0x1F => core.user_r13,
            0x12 => core.irq_r13,
            0x13 => core.supervisor_r13,
            _ => 0,
        };
        core.registers[15] = entry_address.wrapping_add(8);
        core
    }

    #[inline]
    pub fn is_thumb(&self) -> bool {
        self.cpsr & (1 << 5) != 0
    }

    pub fn switch_banked_mode(&mut self, new_mode: u32) {
        let old_mode = self.cpsr & 0x1F;
        if new_mode == old_mode {
            return;
        }
        if old_mode == 0x11 {
            for index in 0..5 {
                self.fiq_r8_14[index] = self.registers[8 + index];
                self.registers[8 + index] = self.user_r8_12[index];
            }
        } else {
            self.user_r8_12.copy_from_slice(&self.registers[8..13]);
        }
        match old_mode {
            0x10 | 0x1F => {
                self.user_r13 = self.registers[13];
                self.user_r14 = self.registers[14];
            }
            0x11 => self.fiq_r8_14[5..7].copy_from_slice(&self.registers[13..15]),
            0x12 => {
                self.irq_r13 = self.registers[13];
                self.irq_r14 = self.registers[14];
            }
            0x13 => {
                self.supervisor_r13 = self.registers[13];
                self.supervisor_r14 = self.registers[14];
            }
            0x17 => {
                self.abort_r13 = self.registers[13];
                self.abort_r14 = self.registers[14];
            }
            0x1B => {
                self.undefined_r13 = self.registers[13];
                self.undefined_r14 = self.registers[14];
            }
            _ => unreachable!("validated processor mode"),
        }
        match new_mode {
            0x10 | 0x1F => {
                self.registers[13] = self.user_r13;
                self.registers[14] = self.user_r14;
            }
            0x11 => {
                self.registers[8..13].copy_from_slice(&self.fiq_r8_14[..5]);
                self.registers[13..15].copy_from_slice(&self.fiq_r8_14[5..7]);
            }
            0x12 => {
                self.registers[13] = self.irq_r13;
                self.registers[14] = self.irq_r14;
            }
            0x13 => {
                self.registers[13] = self.supervisor_r13;
                self.registers[14] = self.supervisor_r14;
            }
            0x17 => {
                self.registers[13] = self.abort_r13;
                self.registers[14] = self.abort_r14;
            }
            0x1B => {
                self.registers[13] = self.undefined_r13;
                self.registers[14] = self.undefined_r14;
            }
            _ => unreachable!("validated processor mode"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ZiGBA `switchBankedMode`: FIQ has private r8–r14 while System and User share their bank.
    #[test]
    fn switch_banked_mode() {
        let mut cpu = Core::new(0x0800_0000, 0x1F);
        cpu.registers[8..15].copy_from_slice(&[8, 9, 10, 11, 12, 13, 14]);
        cpu.switch_banked_mode(0x11);
        cpu.cpsr = 0x11;
        cpu.registers[8..15].copy_from_slice(&[18, 19, 20, 21, 22, 23, 24]);
        cpu.switch_banked_mode(0x12);
        cpu.cpsr = 0x12;
        assert_eq!(&cpu.registers[8..15], &[8, 9, 10, 11, 12, 0x0300_7FA0, 0]);
        cpu.switch_banked_mode(0x11);
        assert_eq!(&cpu.registers[8..15], &[18, 19, 20, 21, 22, 23, 24]);
    }
}
