//! ARM7TDMI instruction state and bus-neutral execution primitives.
//!
//! The bus, not the interpreter, prices each N/S/I access. The type parameter
//! lets the system's bus inline into the executors.

use evenfall_state::Codec;

mod multiply;
mod thumb;

pub use multiply::{MultiplyKind, multiply_carry};

pub type Cycle = u64;

/// For each ARM condition, bit `NZCV` is set when the condition passes under
/// those flags (N is bit 3 of the index, V bit 0).
const CONDITION_TRUTH: [u16; 16] = {
    let mut table = [0u16; 16];
    let mut condition = 0;
    while condition < 16 {
        let mut flags = 0;
        while flags < 16 {
            let (n, z, c, v) = (flags & 8 != 0, flags & 4 != 0, flags & 2 != 0, flags & 1 != 0);
            let passes = match condition {
                0 => z,
                1 => !z,
                2 => c,
                3 => !c,
                4 => n,
                5 => !n,
                6 => v,
                7 => !v,
                8 => c && !z,
                9 => !c || z,
                10 => n == v,
                11 => n != v,
                12 => !z && n == v,
                13 => z || n != v,
                14 => true,
                _ => false,
            };
            if passes {
                table[condition] |= 1 << flags;
            }
            flags += 1;
        }
        condition += 1;
    }
    table
};

/// Whether `condition` passes under the NZCV flags of `cpsr`.
#[inline(always)]
pub fn condition_passes(cpsr: u32, condition: u32) -> bool {
    (CONDITION_TRUTH[(condition & 15) as usize] >> (cpsr >> 28)) & 1 != 0
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Codec)]
#[codec(tag_bits = 1)]
pub enum InstructionSet {
    #[default]
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
pub enum PsrStateOperation {
    MsrCpsr,
    DataProcessingExceptionReturn,
    BlockTransferExceptionReturn,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvalidPsrStateKind {
    InvalidMode,
    IllegalTBitChange,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessorMode {
    User,
    Fiq,
    Irq,
    Supervisor,
    Abort,
    Undefined,
    System,
}

impl ProcessorMode {
    pub fn parse(bits: u32) -> Option<Self> {
        Some(match bits & 0x1f {
            0x10 => Self::User,
            0x11 => Self::Fiq,
            0x12 => Self::Irq,
            0x13 => Self::Supervisor,
            0x17 => Self::Abort,
            0x1b => Self::Undefined,
            0x1f => Self::System,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpsrOperation {
    Mrs,
    Msr,
    DataProcessingExceptionReturn,
    BlockTransferExceptionReturn,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArmInstructionClass {
    Mrs,
    Multiply,
    Swap,
    MultiplyLong,
    HalfwordTransfer,
    SingleTransfer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultReason {
    InvalidPsrState { operation: PsrStateOperation, kind: InvalidPsrStateKind, value: u32 },
    MissingSpsr { operation: SpsrOperation, mode: ProcessorMode },
    UnpredictableArmEncoding { instruction_class: ArmInstructionClass, offending_registers: u16 },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExecuteResult {
    pub branched: bool,
    pub bus_free: bool,
    /// The platform decides whether to vector or handle this through HLE.
    pub software_interrupt: Option<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShifterResult {
    pub value: u32,
    pub carry: Option<bool>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Codec)]
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

/// Predecoded ARM ALU handler. Decoding identifies the instruction class but
/// does not admit accesses or validate unpredictable register combinations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArmAluHandler(u8);

impl ArmAluHandler {
    #[inline]
    pub fn decode(instruction: u32) -> Option<Self> {
        let class = (instruction >> 25) & 7;
        (class == 1
            || (class == 0
                && instruction & 0x0fc0_00f0 != 0x0000_0090
                && !(instruction & 0x0e00_0090 == 0x0000_0090 && (instruction >> 5) & 3 != 0)))
            .then_some(Self(((instruction >> 20) & 31) as u8))
    }

    /// Caller must admit the opcode first: no PC destination, invalid test
    /// encoding, exception return or ISA fault may reach this fast path.
    #[inline(always)]
    pub fn execute<B: Bus>(self, core: &mut Core, bus: &mut B, instruction: u32) -> ExecuteResult {
        match self.0 {
            0 => core.execute_arm_kernel_alu::<0, false, _>(bus, instruction),
            1 => core.execute_arm_kernel_alu::<0, true, _>(bus, instruction),
            2 => core.execute_arm_kernel_alu::<1, false, _>(bus, instruction),
            3 => core.execute_arm_kernel_alu::<1, true, _>(bus, instruction),
            4 => core.execute_arm_kernel_alu::<2, false, _>(bus, instruction),
            5 => core.execute_arm_kernel_alu::<2, true, _>(bus, instruction),
            6 => core.execute_arm_kernel_alu::<3, false, _>(bus, instruction),
            7 => core.execute_arm_kernel_alu::<3, true, _>(bus, instruction),
            8 => core.execute_arm_kernel_alu::<4, false, _>(bus, instruction),
            9 => core.execute_arm_kernel_alu::<4, true, _>(bus, instruction),
            10 => core.execute_arm_kernel_alu::<5, false, _>(bus, instruction),
            11 => core.execute_arm_kernel_alu::<5, true, _>(bus, instruction),
            12 => core.execute_arm_kernel_alu::<6, false, _>(bus, instruction),
            13 => core.execute_arm_kernel_alu::<6, true, _>(bus, instruction),
            14 => core.execute_arm_kernel_alu::<7, false, _>(bus, instruction),
            15 => core.execute_arm_kernel_alu::<7, true, _>(bus, instruction),
            17 => core.execute_arm_kernel_alu::<8, true, _>(bus, instruction),
            19 => core.execute_arm_kernel_alu::<9, true, _>(bus, instruction),
            21 => core.execute_arm_kernel_alu::<10, true, _>(bus, instruction),
            23 => core.execute_arm_kernel_alu::<11, true, _>(bus, instruction),
            24 => core.execute_arm_kernel_alu::<12, false, _>(bus, instruction),
            25 => core.execute_arm_kernel_alu::<12, true, _>(bus, instruction),
            26 => core.execute_arm_kernel_alu::<13, false, _>(bus, instruction),
            27 => core.execute_arm_kernel_alu::<13, true, _>(bus, instruction),
            28 => core.execute_arm_kernel_alu::<14, false, _>(bus, instruction),
            29 => core.execute_arm_kernel_alu::<14, true, _>(bus, instruction),
            30 => core.execute_arm_kernel_alu::<15, false, _>(bus, instruction),
            31 => core.execute_arm_kernel_alu::<15, true, _>(bus, instruction),
            _ => unreachable!("kernel rejected non-flag-setting test instruction"),
        }
    }
}

/// The flat prefix of ZiGBA's `Cpu` state, through `spsr_undefined`.
/// The GBA wrapper flattens this into the same durable field order.
#[derive(Clone, Debug, Default, PartialEq, Eq, Codec)]
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

    /// Whether `condition` passes under the current NZCV flags: one lookup of
    /// bit NZCV in the condition's precomputed truth mask.
    #[inline(always)]
    pub fn condition_passed(&self, condition: u32) -> bool {
        condition_passes(self.cpsr, condition)
    }

    /// The ARM condition table as written in the architecture manual; the
    /// reference for [`CONDITION_TRUTH`].
    pub fn condition_passed_reference(&self, condition: u32) -> bool {
        let n = self.cpsr & (1 << 31) != 0;
        let z = self.cpsr & (1 << 30) != 0;
        let c = self.cpsr & (1 << 29) != 0;
        let v = self.cpsr & (1 << 28) != 0;
        match condition {
            0 => z,
            1 => !z,
            2 => c,
            3 => !c,
            4 => n,
            5 => !n,
            6 => v,
            7 => !v,
            8 => c && !z,
            9 => !c || z,
            10 => n == v,
            11 => n != v,
            12 => !z && n == v,
            13 => z || n != v,
            14 => true,
            15 => false,
            _ => unreachable!("four-bit ARM condition"),
        }
    }

    pub fn read_current_spsr(&self) -> Option<u32> {
        match self.cpsr & 0x1f {
            0x11 => Some(self.spsr_fiq),
            0x12 => Some(self.spsr_irq),
            0x13 => Some(self.spsr_supervisor),
            0x17 => Some(self.spsr_abort),
            0x1b => Some(self.spsr_undefined),
            _ => None,
        }
    }

    fn missing_spsr(&self, operation: SpsrOperation) -> FaultReason {
        FaultReason::MissingSpsr {
            operation,
            mode: ProcessorMode::parse(self.cpsr).expect("validated processor mode"),
        }
    }

    fn invalid_exception_return_psr(
        &self,
        operation: PsrStateOperation,
    ) -> Result<(), FaultReason> {
        let restored = self.read_current_spsr().ok_or_else(|| {
            self.missing_spsr(match operation {
                PsrStateOperation::DataProcessingExceptionReturn => {
                    SpsrOperation::DataProcessingExceptionReturn
                }
                PsrStateOperation::BlockTransferExceptionReturn => {
                    SpsrOperation::BlockTransferExceptionReturn
                }
                PsrStateOperation::MsrCpsr => unreachable!(),
            })
        })?;
        if ProcessorMode::parse(restored).is_none() {
            return Err(FaultReason::InvalidPsrState {
                operation,
                kind: InvalidPsrStateKind::InvalidMode,
                value: restored,
            });
        }
        Ok(())
    }

    pub fn restore_cpsr_from_spsr(&mut self) {
        let restored = self.read_current_spsr().expect("validated SPSR");
        self.switch_banked_mode(restored & 0x1f);
        self.cpsr = restored;
    }

    pub fn enter_exception(&mut self, mode: u32, vector: u32, return_address: u32) {
        let old_cpsr = self.cpsr;
        self.switch_banked_mode(mode);
        match mode {
            0x11 => {
                self.spsr_fiq = old_cpsr;
                self.fiq_r8_14[6] = return_address;
            }
            0x12 => {
                self.spsr_irq = old_cpsr;
                self.irq_r14 = return_address;
            }
            0x13 => {
                self.spsr_supervisor = old_cpsr;
                self.supervisor_r14 = return_address;
            }
            0x17 => {
                self.spsr_abort = old_cpsr;
                self.abort_r14 = return_address;
            }
            0x1b => {
                self.spsr_undefined = old_cpsr;
                self.undefined_r14 = return_address;
            }
            _ => unreachable!("exception mode"),
        }
        self.registers[14] = return_address;
        self.cpsr = (old_cpsr & !0x3f) | mode | (1 << 7);
        self.next_instruction_address = vector;
        self.registers[15] = vector.wrapping_add(8);
    }

    pub fn enter_undefined(&mut self, return_address: u32) {
        self.enter_exception(0x1b, 4, return_address);
    }

    pub fn enter_software_interrupt(&mut self, return_address: u32) {
        self.enter_exception(0x13, 8, return_address);
    }

    fn set_logical_flags(&mut self, result: u32, carry: Option<bool>) {
        self.cpsr =
            (self.cpsr & !0xc000_0000) | (result & 0x8000_0000) | (u32::from(result == 0) << 30);
        if let Some(carry) = carry {
            self.cpsr = (self.cpsr & !(1 << 29)) | (u32::from(carry) << 29);
        }
    }

    fn set_arithmetic_flags(&mut self, result: u32, carry: bool, overflow: bool) {
        self.cpsr = (self.cpsr & !0xf000_0000)
            | (result & 0x8000_0000)
            | (u32::from(result == 0) << 30)
            | (u32::from(carry) << 29)
            | (u32::from(overflow) << 28);
    }

    fn set_add_flags(&mut self, lhs: u32, rhs: u32, carry: u32, result: u32) {
        let wide = u64::from(lhs) + u64::from(rhs) + u64::from(carry);
        let signed = i64::from(lhs as i32) + i64::from(rhs as i32) + i64::from(carry);
        self.set_arithmetic_flags(
            result,
            wide > u64::from(u32::MAX),
            signed > i64::from(i32::MAX) || signed < i64::from(i32::MIN),
        );
    }

    fn set_sub_flags(&mut self, lhs: u32, rhs: u32, borrow: u32, result: u32) {
        let subtrahend = u64::from(rhs) + u64::from(borrow);
        let signed = i64::from(lhs as i32) - i64::from(rhs as i32) - i64::from(borrow);
        self.set_arithmetic_flags(
            result,
            u64::from(lhs) >= subtrahend,
            signed > i64::from(i32::MAX) || signed < i64::from(i32::MIN),
        );
    }

    #[inline]
    pub fn multiply_iterations(multiplier: u32, signed: bool) -> Cycle {
        let magnitude =
            if signed && multiplier & 0x8000_0000 != 0 { !multiplier } else { multiplier };
        1 + u64::from(magnitude > 0xff)
            + u64::from(magnitude > 0xffff)
            + u64::from(magnitude > 0xff_ffff)
    }

    #[inline]
    pub fn decode_operand2(&self, instruction: u32, immediate: bool) -> ShifterResult {
        if immediate {
            let rotate = ((instruction >> 8) & 15) * 2;
            let value = (instruction & 255).rotate_right(rotate);
            return ShifterResult { value, carry: (rotate != 0).then_some(value & (1 << 31) != 0) };
        }

        let rm = (instruction & 15) as usize;
        let shift_type = (instruction >> 5) & 3;
        if instruction & (1 << 4) != 0 {
            let source = self.registers[rm].wrapping_add(if rm == 15 { 4 } else { 0 });
            let rs = ((instruction >> 8) & 15) as usize;
            return Self::decode_register_shift(source, shift_type, self.registers[rs] as u8);
        }

        let source = self.registers[rm];
        let amount = (instruction >> 7) & 31;
        let sign = source & (1 << 31) != 0;
        match (shift_type, amount) {
            (0, 0) => ShifterResult { value: source, carry: None },
            (0, n) => {
                ShifterResult { value: source << n, carry: Some(source & (1 << (32 - n)) != 0) }
            }
            (1, 0) => ShifterResult { value: 0, carry: Some(sign) },
            (1, n) => {
                ShifterResult { value: source >> n, carry: Some(source & (1 << (n - 1)) != 0) }
            }
            (2, 0) => ShifterResult { value: if sign { u32::MAX } else { 0 }, carry: Some(sign) },
            (2, n) => ShifterResult {
                value: ((source as i32) >> n) as u32,
                carry: Some(source & (1 << (n - 1)) != 0),
            },
            (3, 0) => ShifterResult {
                value: ((self.cpsr & (1 << 29)) << 2) | (source >> 1),
                carry: Some(source & 1 != 0),
            },
            (3, n) => {
                let value = source.rotate_right(n);
                ShifterResult { value, carry: Some(value & (1 << 31) != 0) }
            }
            _ => unreachable!(),
        }
    }

    #[inline]
    pub fn decode_register_shift(source: u32, shift_type: u32, amount: u8) -> ShifterResult {
        if amount == 0 {
            return ShifterResult { value: source, carry: None };
        }
        let sign = source & (1 << 31) != 0;
        match shift_type {
            0 if amount < 32 => ShifterResult {
                value: source << amount,
                carry: Some(source & (1 << (32 - amount)) != 0),
            },
            0 if amount == 32 => ShifterResult { value: 0, carry: Some(source & 1 != 0) },
            0 => ShifterResult { value: 0, carry: Some(false) },
            1 if amount < 32 => ShifterResult {
                value: source >> amount,
                carry: Some(source & (1 << (amount - 1)) != 0),
            },
            1 if amount == 32 => ShifterResult { value: 0, carry: Some(sign) },
            1 => ShifterResult { value: 0, carry: Some(false) },
            2 if amount < 32 => ShifterResult {
                value: ((source as i32) >> amount) as u32,
                carry: Some(source & (1 << (amount - 1)) != 0),
            },
            2 => ShifterResult { value: if sign { u32::MAX } else { 0 }, carry: Some(sign) },
            3 if amount & 31 == 0 => ShifterResult { value: source, carry: Some(sign) },
            3 => {
                let value = source.rotate_right(u32::from(amount & 31));
                ShifterResult { value, carry: Some(value & (1 << 31) != 0) }
            }
            _ => unreachable!(),
        }
    }

    /// ARM data processing, including legacy compare-to-PC and exception
    /// returns. The caller has already checked the instruction's condition.
    #[inline(always)]
    pub fn execute_arm_data_processing<B: Bus>(
        &mut self,
        bus: &mut B,
        instruction: u32,
    ) -> Result<ExecuteResult, FaultReason> {
        let opcode = (instruction >> 21) & 15;
        let updates_flags = instruction & (1 << 20) != 0;
        let flags_only = (8..=11).contains(&opcode);
        let shifted = self.decode_operand2(instruction, instruction & (1 << 25) != 0);
        let rhs = shifted.value;
        let rn = ((instruction >> 16) & 15) as usize;
        let register_shifted = instruction & (1 << 25) == 0 && instruction & (1 << 4) != 0;
        let lhs = self.registers[rn].wrapping_add(if rn == 15 && register_shifted { 4 } else { 0 });
        let carry = u32::from(self.cpsr & (1 << 29) != 0);
        let borrow = 1 - carry;
        let result = match opcode {
            0 | 8 => lhs & rhs,
            1 | 9 => lhs ^ rhs,
            2 | 10 => lhs.wrapping_sub(rhs),
            3 => rhs.wrapping_sub(lhs),
            4 | 11 => lhs.wrapping_add(rhs),
            5 => lhs.wrapping_add(rhs).wrapping_add(carry),
            6 => lhs.wrapping_sub(rhs).wrapping_sub(borrow),
            7 => rhs.wrapping_sub(lhs).wrapping_sub(borrow),
            12 => lhs | rhs,
            13 => rhs,
            14 => lhs & !rhs,
            15 => !rhs,
            _ => unreachable!(),
        };
        let rd = ((instruction >> 12) & 15) as usize;
        let legacy_psr_restore = flags_only && updates_flags && rd == 15;
        let mode_has_spsr = matches!(self.cpsr & 31, 0x11 | 0x12 | 0x13 | 0x17 | 0x1b);
        if updates_flags && rd == 15 && mode_has_spsr {
            self.invalid_exception_return_psr(PsrStateOperation::DataProcessingExceptionReturn)?;
        }
        let mut branched = false;
        let mut restored = false;
        if legacy_psr_restore {
            if mode_has_spsr {
                self.restore_cpsr_from_spsr();
            }
        } else if !flags_only {
            if rd == 15 {
                if updates_flags && mode_has_spsr {
                    self.restore_cpsr_from_spsr();
                    restored = true;
                }
                let mask = if self.is_thumb() { 1 } else { 3 };
                self.next_instruction_address = result & !mask;
                branched = true;
            } else {
                self.registers[rd] = result;
            }
        }
        if (updates_flags || flags_only) && !legacy_psr_restore && (!branched || !restored) {
            match opcode {
                0 | 1 | 8 | 9 | 12..=15 => self.set_logical_flags(result, shifted.carry),
                2 | 10 => self.set_sub_flags(lhs, rhs, 0, result),
                3 => self.set_sub_flags(rhs, lhs, 0, result),
                4 | 11 => self.set_add_flags(lhs, rhs, 0, result),
                5 => self.set_add_flags(lhs, rhs, carry, result),
                6 => self.set_sub_flags(lhs, rhs, borrow, result),
                7 => self.set_sub_flags(rhs, lhs, borrow, result),
                _ => unreachable!(),
            }
        }
        if register_shifted {
            bus.elapse_cpu_internal(1);
        }
        Ok(ExecuteResult { branched, bus_free: !register_shifted, software_interrupt: None })
    }

    /// The bounded kernel has already excluded PC writes and exception
    /// returns. Specialize the ALU opcode without changing the interpreter.
    #[inline(always)]
    fn execute_arm_kernel_alu<const OP: u32, const FLAGS: bool, B: Bus>(
        &mut self,
        bus: &mut B,
        instruction: u32,
    ) -> ExecuteResult {
        let shift = self.decode_operand2(instruction, instruction & (1 << 25) != 0);
        let rhs = shift.value;
        let rn = ((instruction >> 16) & 15) as usize;
        let rd = ((instruction >> 12) & 15) as usize;
        let register_shifted = instruction & (1 << 25) == 0 && instruction & (1 << 4) != 0;
        let lhs = self.registers[rn].wrapping_add(if rn == 15 && register_shifted { 4 } else { 0 });
        let carry = u32::from(self.cpsr & (1 << 29) != 0);
        let borrow = 1 - carry;
        let result = match OP {
            0 | 8 => lhs & rhs,
            1 | 9 => lhs ^ rhs,
            2 | 10 => lhs.wrapping_sub(rhs),
            3 => rhs.wrapping_sub(lhs),
            4 | 11 => lhs.wrapping_add(rhs),
            5 => lhs.wrapping_add(rhs).wrapping_add(carry),
            6 => lhs.wrapping_sub(rhs).wrapping_sub(borrow),
            7 => rhs.wrapping_sub(lhs).wrapping_sub(borrow),
            12 => lhs | rhs,
            13 => rhs,
            14 => lhs & !rhs,
            15 => !rhs,
            _ => unreachable!(),
        };
        if !(8..=11).contains(&OP) {
            self.registers[rd] = result;
        }
        if FLAGS || (8..=11).contains(&OP) {
            match OP {
                0 | 1 | 8 | 9 | 12..=15 => self.set_logical_flags(result, shift.carry),
                2 | 10 => self.set_sub_flags(lhs, rhs, 0, result),
                3 => self.set_sub_flags(rhs, lhs, 0, result),
                4 | 11 => self.set_add_flags(lhs, rhs, 0, result),
                5 => self.set_add_flags(lhs, rhs, carry, result),
                6 => self.set_sub_flags(lhs, rhs, borrow, result),
                7 => self.set_sub_flags(rhs, lhs, borrow, result),
                _ => unreachable!(),
            }
        }
        if register_shifted {
            bus.elapse_cpu_internal(1);
        }
        ExecuteResult { bus_free: !register_shifted, ..ExecuteResult::default() }
    }
}

fn unpredictable(instruction_class: ArmInstructionClass, registers: u16) -> FaultReason {
    debug_assert_ne!(registers, 0);
    FaultReason::UnpredictableArmEncoding { instruction_class, offending_registers: registers }
}

fn register_bit(register: usize) -> u16 {
    1 << register
}

fn byte_wide_save_address(address: u32) -> bool {
    address >> 25 == 7
}

/// The address a block-transfer beat at `address` presents to the bus:
/// aligned, except on the 8-bit save bus, which sees it exactly (the mGBA
/// suite's SRAM `CpuSet`/`CpuFastSet` rows).
fn block_address(address: u32) -> u32 {
    if byte_wide_save_address(address) { address } else { address & !3 }
}

impl Core {
    pub fn execute_arm<B: Bus>(
        &mut self,
        bus: &mut B,
        instruction: u32,
    ) -> Result<ExecuteResult, FaultReason> {
        if !self.condition_passed(instruction >> 28) {
            return Ok(ExecuteResult::default());
        }
        self.execute_arm_condition_passed(bus, instruction)
    }

    /// Execute an opcode whose condition was checked by the bounded kernel.
    /// Ordinary execution continues to enter through `execute_arm`.
    #[inline(always)]
    pub fn execute_arm_condition_passed<B: Bus>(
        &mut self,
        bus: &mut B,
        instruction: u32,
    ) -> Result<ExecuteResult, FaultReason> {
        match (instruction >> 25) & 7 {
            0 => {
                if instruction & 0x0fff_fff0 == 0x012f_ff10 {
                    let target = self.registers[(instruction & 15) as usize];
                    if target & 1 != 0 {
                        self.cpsr |= 1 << 5;
                        self.next_instruction_address = target & !1;
                    } else {
                        self.cpsr &= !(1 << 5);
                        self.next_instruction_address = target & !3;
                    }
                    return Ok(ExecuteResult { branched: true, ..ExecuteResult::default() });
                }
                if instruction & 0x0fb0_fff0 == 0x0120_f000 {
                    self.write_psr(instruction, self.registers[(instruction & 15) as usize])?;
                    return Ok(ExecuteResult::default());
                }
                if instruction & 0x0fbf_0fff == 0x010f_0000 {
                    let rd = ((instruction >> 12) & 15) as usize;
                    if rd == 15 {
                        return Err(unpredictable(ArmInstructionClass::Mrs, register_bit(rd)));
                    }
                    self.registers[rd] = if instruction & (1 << 22) != 0 {
                        self.read_current_spsr()
                            .ok_or_else(|| self.missing_spsr(SpsrOperation::Mrs))?
                    } else {
                        self.cpsr
                    };
                    return Ok(ExecuteResult::default());
                }
                if instruction & 0x0fb0_0ff0 == 0x0100_0090 {
                    return self.execute_swap(bus, instruction);
                }
                if instruction & 0x0f80_00f0 == 0x0080_0090 {
                    return self.execute_multiply_long(bus, instruction);
                }
                if instruction & 0x0fc0_00f0 == 0x0000_0090 {
                    return self.execute_multiply(bus, instruction);
                }
                if instruction & 0x0e00_0090 == 0x0000_0090 && (instruction >> 5) & 3 != 0 {
                    return self.execute_halfword_data_transfer(bus, instruction);
                }
            }
            1 if instruction & 0x0fb0_f000 == 0x0320_f000 => {
                self.write_psr(instruction, self.decode_operand2(instruction, true).value)?;
                return Ok(ExecuteResult::default());
            }
            2 | 3 => {
                if instruction & 0x0e00_0010 == 0x0600_0010 {
                    bus.elapse_cpu_internal(1);
                    self.enter_undefined(self.next_instruction_address.wrapping_add(4));
                    return Ok(ExecuteResult { branched: true, ..ExecuteResult::default() });
                }
                return self.execute_single_data_transfer(bus, instruction);
            }
            4 => return self.execute_block_data_transfer(bus, instruction),
            5 => {
                if instruction & (1 << 24) != 0 {
                    self.registers[14] = self.next_instruction_address.wrapping_add(4);
                }
                let offset = (((instruction & 0x00ff_ffff) << 8) as i32 >> 6) as u32;
                self.next_instruction_address = self.registers[15].wrapping_add(offset);
                return Ok(ExecuteResult { branched: true, ..ExecuteResult::default() });
            }
            6 | 7 => {
                if instruction & 0x0f00_0000 == 0x0f00_0000 {
                    return Ok(ExecuteResult {
                        software_interrupt: Some((instruction >> 16) as u8),
                        ..ExecuteResult::default()
                    });
                }
                self.enter_undefined(self.next_instruction_address.wrapping_add(4));
                return Ok(ExecuteResult { branched: true, ..ExecuteResult::default() });
            }
            _ => {}
        }
        self.execute_arm_data_processing(bus, instruction)
    }

    pub fn write_psr(&mut self, instruction: u32, operand: u32) -> Result<(), FaultReason> {
        let fields = (instruction >> 16) & 15;
        if instruction & (1 << 22) != 0 {
            let destination = match self.cpsr & 31 {
                0x11 => &mut self.spsr_fiq,
                0x12 => &mut self.spsr_irq,
                0x13 => &mut self.spsr_supervisor,
                0x17 => &mut self.spsr_abort,
                0x1b => &mut self.spsr_undefined,
                _ => return Err(self.missing_spsr(SpsrOperation::Msr)),
            };
            let mask = (if fields & 8 != 0 { 0xf000_0000 } else { 0 })
                | (if fields & 1 != 0 { 0xff } else { 0 });
            *destination = (*destination & !mask) | (operand & mask);
        } else {
            let control = fields & 1 != 0 && self.cpsr & 31 != 0x10;
            if control {
                let candidate = (self.cpsr & !0xff) | (operand & 0xff);
                if ProcessorMode::parse(candidate).is_none() {
                    return Err(FaultReason::InvalidPsrState {
                        operation: PsrStateOperation::MsrCpsr,
                        kind: InvalidPsrStateKind::InvalidMode,
                        value: candidate,
                    });
                }
                if (candidate ^ self.cpsr) & (1 << 5) != 0 {
                    return Err(FaultReason::InvalidPsrState {
                        operation: PsrStateOperation::MsrCpsr,
                        kind: InvalidPsrStateKind::IllegalTBitChange,
                        value: candidate,
                    });
                }
            }
            if fields & 8 != 0 {
                self.cpsr = (self.cpsr & 0x0fff_ffff) | (operand & 0xf000_0000);
            }
            if control {
                self.switch_banked_mode(operand & 31);
                self.cpsr = (self.cpsr & !0xff) | (operand & 0xff);
            }
        }
        Ok(())
    }

    fn execute_multiply<B: Bus>(
        &mut self,
        bus: &mut B,
        instruction: u32,
    ) -> Result<ExecuteResult, FaultReason> {
        let rd = ((instruction >> 16) & 15) as usize;
        let rn = ((instruction >> 12) & 15) as usize;
        let rs = ((instruction >> 8) & 15) as usize;
        let rm = (instruction & 15) as usize;
        let accumulate = instruction & (1 << 21) != 0;
        let mut offending = 0;
        for reg in [rd, rs, rm] {
            if reg == 15 {
                offending |= register_bit(reg);
            }
        }
        if accumulate && rn == 15 {
            offending |= register_bit(rn);
        }
        if offending != 0 {
            return Err(unpredictable(ArmInstructionClass::Multiply, offending));
        }
        let multiplier = self.registers[rs];
        let multiplicand = self.registers[rm];
        let accumulator = if accumulate { self.registers[rn] } else { 0 };
        let result = multiplicand.wrapping_mul(multiplier).wrapping_add(accumulator);
        self.registers[rd] = result;
        if instruction & (1 << 20) != 0 {
            let carry = multiply_carry(
                MultiplyKind::Short,
                multiplicand,
                multiplier,
                u64::from(accumulator),
            );
            self.set_logical_flags(result, Some(carry));
        }
        bus.elapse_cpu_internal(
            Self::multiply_iterations(multiplier, true) + u64::from(accumulate),
        );
        Ok(ExecuteResult::default())
    }

    fn execute_multiply_long<B: Bus>(
        &mut self,
        bus: &mut B,
        instruction: u32,
    ) -> Result<ExecuteResult, FaultReason> {
        let hi = ((instruction >> 16) & 15) as usize;
        let lo = ((instruction >> 12) & 15) as usize;
        let rs = ((instruction >> 8) & 15) as usize;
        let rm = (instruction & 15) as usize;
        let mut offending = 0;
        for reg in [hi, lo, rs, rm] {
            if reg == 15 {
                offending |= register_bit(reg);
            }
        }
        if hi == lo {
            offending |= register_bit(hi);
        }
        if offending != 0 {
            return Err(unpredictable(ArmInstructionClass::MultiplyLong, offending));
        }
        let signed = instruction & (1 << 22) != 0;
        let accumulate = instruction & (1 << 21) != 0;
        let multiplier = self.registers[rs];
        let multiplicand = self.registers[rm];
        let accumulator = if accumulate {
            (u64::from(self.registers[hi]) << 32) | u64::from(self.registers[lo])
        } else {
            0
        };
        let product = if signed {
            (i64::from(multiplicand as i32) * i64::from(multiplier as i32)) as u64
        } else {
            u64::from(multiplicand) * u64::from(multiplier)
        }
        .wrapping_add(accumulator);
        self.registers[lo] = product as u32;
        self.registers[hi] = (product >> 32) as u32;
        if instruction & (1 << 20) != 0 {
            let kind = if signed { MultiplyKind::LongSigned } else { MultiplyKind::LongUnsigned };
            let carry = multiply_carry(kind, multiplicand, multiplier, accumulator);
            self.cpsr = (self.cpsr & !0xe000_0000)
                | ((product >> 32) as u32 & 0x8000_0000)
                | (u32::from(product == 0) << 30)
                | (u32::from(carry) << 29);
        }
        bus.elapse_cpu_internal(
            Self::multiply_iterations(multiplier, signed) + 1 + u64::from(accumulate),
        );
        Ok(ExecuteResult::default())
    }

    fn execute_swap<B: Bus>(
        &mut self,
        bus: &mut B,
        instruction: u32,
    ) -> Result<ExecuteResult, FaultReason> {
        let byte = instruction & (1 << 22) != 0;
        let rn = ((instruction >> 16) & 15) as usize;
        let rd = ((instruction >> 12) & 15) as usize;
        let rm = (instruction & 15) as usize;
        let offending =
            [rn, rd, rm].iter().filter(|&&r| r == 15).fold(0, |mask, &r| mask | register_bit(r));
        if offending != 0 {
            return Err(unpredictable(ArmInstructionClass::Swap, offending));
        }
        let address = self.registers[rn];
        let aligned = if byte || byte_wide_save_address(address) { address } else { address & !3 };
        let width = if byte { AccessWidth::Byte } else { AccessWidth::Word };
        let old = bus.read_cpu_data(aligned, width, AccessKind::Nonsequential);
        bus.write_cpu_data(aligned, width, AccessKind::Nonsequential, self.registers[rm]);
        self.registers[rd] = if byte { old } else { old.rotate_right((address & 3) * 8) };
        bus.elapse_cpu_internal(1);
        Ok(ExecuteResult::default())
    }

    pub fn execute_single_data_transfer<B: Bus>(
        &mut self,
        bus: &mut B,
        instruction: u32,
    ) -> Result<ExecuteResult, FaultReason> {
        let register_offset = instruction & (1 << 25) != 0;
        let preindexed = instruction & (1 << 24) != 0;
        let add_offset = instruction & (1 << 23) != 0;
        let byte = instruction & (1 << 22) != 0;
        let writeback = instruction & (1 << 21) != 0;
        let load = instruction & (1 << 20) != 0;
        let rn = ((instruction >> 16) & 15) as usize;
        let rd = ((instruction >> 12) & 15) as usize;
        let mut offending = 0;
        if rn == 15 && (!preindexed || writeback) {
            offending |= register_bit(rn);
        }
        if rd == 15 && byte {
            offending |= register_bit(rd);
        }
        if offending != 0 {
            return Err(unpredictable(ArmInstructionClass::SingleTransfer, offending));
        }
        // ARM `011` with bit 4 was handled as Undefined by the decoder.
        debug_assert!(!register_offset || instruction & (1 << 4) == 0);
        let base = self.registers[rn];
        let offset = if register_offset {
            self.decode_operand2(instruction, false).value
        } else {
            instruction & 0xfff
        };
        let adjusted =
            if add_offset { base.wrapping_add(offset) } else { base.wrapping_sub(offset) };
        let address = if preindexed { adjusted } else { base };
        let bus_address =
            if byte || byte_wide_save_address(address) { address } else { address & !3 };
        let width = if byte { AccessWidth::Byte } else { AccessWidth::Word };
        if load {
            let read = bus.read_cpu_data(bus_address, width, AccessKind::Nonsequential);
            let value = if byte { read } else { read.rotate_right((address & 3) * 8) };
            if rd == 15 {
                self.next_instruction_address = value & !3;
            } else {
                self.registers[rd] = value;
            }
            if (!preindexed || writeback) && rn != rd {
                self.registers[rn] = adjusted;
            }
            bus.elapse_cpu_internal(1);
            return Ok(ExecuteResult { branched: rd == 15, ..ExecuteResult::default() });
        }
        let value = self.registers[rd].wrapping_add(if rd == 15 { 4 } else { 0 });
        bus.write_cpu_data(bus_address, width, AccessKind::Nonsequential, value);
        if !preindexed || writeback {
            self.registers[rn] = adjusted;
        }
        Ok(ExecuteResult::default())
    }

    pub fn execute_halfword_data_transfer<B: Bus>(
        &mut self,
        bus: &mut B,
        instruction: u32,
    ) -> Result<ExecuteResult, FaultReason> {
        let preindexed = instruction & (1 << 24) != 0;
        let writeback = instruction & (1 << 21) != 0;
        let load = instruction & (1 << 20) != 0;
        let operation = (instruction >> 5) & 3;
        debug_assert_ne!(operation, 0);
        if !load && (operation == 2 || operation == 3) {
            // ARMv5E doubleword encodings are Undefined on ARM7TDMI.
            bus.elapse_cpu_internal(1);
            self.enter_undefined(self.next_instruction_address.wrapping_add(4));
            return Ok(ExecuteResult { branched: true, ..ExecuteResult::default() });
        }
        let rn = ((instruction >> 16) & 15) as usize;
        let rd = ((instruction >> 12) & 15) as usize;
        let rm = (instruction & 15) as usize;
        let immediate = instruction & (1 << 22) != 0;
        let mut offending = 0;
        if rd == 15 {
            offending |= register_bit(rd);
        }
        if rn == 15 && (!preindexed || writeback) {
            offending |= register_bit(rn);
        }
        if !immediate && rm == 15 {
            offending |= register_bit(rm);
        }
        if offending != 0 {
            return Err(unpredictable(ArmInstructionClass::HalfwordTransfer, offending));
        }
        let base = self.registers[rn];
        let offset = if immediate {
            ((instruction >> 8) & 15) << 4 | (instruction & 15)
        } else {
            self.registers[rm]
        };
        let adjusted = if instruction & (1 << 23) != 0 {
            base.wrapping_add(offset)
        } else {
            base.wrapping_sub(offset)
        };
        let address = if preindexed { adjusted } else { base };
        let bus_address =
            if operation == 2 || byte_wide_save_address(address) { address } else { address & !1 };
        let width = if operation == 2 { AccessWidth::Byte } else { AccessWidth::Half };
        if load {
            let read = bus.read_cpu_data(bus_address, width, AccessKind::Nonsequential);
            self.registers[rd] = match operation {
                1 => read.rotate_right((address & 1) * 8),
                2 => (read as u8 as i8 as i32) as u32,
                3 if address & 1 != 0 => ((read >> 8) as u8 as i8 as i32) as u32,
                3 => (read as u16 as i16 as i32) as u32,
                _ => unreachable!(),
            };
            if (!preindexed || writeback) && rn != rd {
                self.registers[rn] = adjusted;
            }
            bus.elapse_cpu_internal(1);
        } else {
            bus.write_cpu_data(bus_address, width, AccessKind::Nonsequential, self.registers[rd]);
            if !preindexed || writeback {
                self.registers[rn] = adjusted;
            }
        }
        Ok(ExecuteResult::default())
    }

    pub fn execute_block_data_transfer<B: Bus>(
        &mut self,
        bus: &mut B,
        instruction: u32,
    ) -> Result<ExecuteResult, FaultReason> {
        let preindexed = instruction & (1 << 24) != 0;
        let add = instruction & (1 << 23) != 0;
        let user_bank = instruction & (1 << 22) != 0;
        let writeback = instruction & (1 << 21) != 0;
        let load = instruction & (1 << 20) != 0;
        let rn = ((instruction >> 16) & 15) as usize;
        let list = instruction as u16;
        let empty = list == 0;
        let transfer_list = if empty { 1 << 15 } else { list };
        let exception_return = user_bank && load && transfer_list & (1 << 15) != 0;
        if exception_return {
            self.invalid_exception_return_psr(PsrStateOperation::BlockTransferExceptionReturn)?;
        }
        let byte_count = if empty { 64 } else { transfer_list.count_ones() * 4 };
        let base = self.registers[rn];
        let mut address = if add {
            base.wrapping_add(if preindexed { 4 } else { 0 })
        } else {
            base.wrapping_sub(byte_count).wrapping_add(if preindexed { 0 } else { 4 })
        };
        let final_base =
            if add { base.wrapping_add(byte_count) } else { base.wrapping_sub(byte_count) };
        let mut loaded_pc = None;
        for (index, register) in
            (0..16).filter(|&r| transfer_list & register_bit(r) != 0).enumerate()
        {
            let kind = if index == 0 { AccessKind::Nonsequential } else { AccessKind::Sequential };
            if load {
                let read = bus.read_cpu_data(block_address(address), AccessWidth::Word, kind);
                if register == 15 {
                    loaded_pc = Some(read);
                } else if user_bank && !exception_return {
                    self.write_user_bank_register(register, read);
                } else {
                    self.registers[register] = read;
                }
            } else {
                let value = if register == 15 {
                    self.registers[15].wrapping_add(4)
                } else if writeback && register == rn && index != 0 {
                    final_base
                } else if user_bank {
                    self.read_user_bank_register(register)
                } else {
                    self.registers[register]
                };
                bus.write_cpu_word(block_address(address), value, kind);
            }
            address = address.wrapping_add(4);
        }
        let load_replaced_base =
            load && (!user_bank || exception_return) && transfer_list & register_bit(rn) != 0;
        if writeback && !load_replaced_base {
            self.registers[rn] = final_base;
        }
        if load {
            bus.elapse_cpu_internal(1);
        }
        if let Some(pc) = loaded_pc {
            if exception_return {
                self.restore_cpsr_from_spsr();
            }
            self.next_instruction_address = pc & !(if self.is_thumb() { 1 } else { 3 });
        }
        Ok(ExecuteResult { branched: loaded_pc.is_some(), ..ExecuteResult::default() })
    }

    pub fn read_user_bank_register(&self, register: usize) -> u32 {
        let mode = self.cpsr & 31;
        match register {
            0..=7 | 15 => self.registers[register],
            8..=12 if mode == 0x11 => self.user_r8_12[register - 8],
            8..=12 => self.registers[register],
            13 if mode == 0x10 || mode == 0x1f => self.registers[13],
            13 => self.user_r13,
            14 if mode == 0x10 || mode == 0x1f => self.registers[14],
            14 => self.user_r14,
            _ => unreachable!(),
        }
    }

    pub fn write_user_bank_register(&mut self, register: usize, value: u32) {
        let mode = self.cpsr & 31;
        match register {
            0..=7 | 15 => self.registers[register] = value,
            8..=12 if mode == 0x11 => self.user_r8_12[register - 8] = value,
            8..=12 => self.registers[register] = value,
            13 if mode == 0x10 || mode == 0x1f => self.registers[13] = value,
            13 => self.user_r13 = value,
            14 if mode == 0x10 || mode == 0x1f => self.registers[14] = value,
            14 => self.user_r14 = value,
            _ => unreachable!(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The condition truth table agrees with the architecture manual's rules
    /// for all sixteen conditions under all sixteen flag settings.
    #[test]
    fn condition_table_matches_the_reference_for_every_condition_and_flags() {
        let mut core = Core::new(0x0800_0000, 0x1f);
        for flags in 0..16u32 {
            core.cpsr = (flags << 28) | 0x1f;
            for condition in 0..16 {
                assert_eq!(
                    core.condition_passed(condition),
                    core.condition_passed_reference(condition),
                    "condition {condition:x} flags {flags:04b}"
                );
            }
        }
    }

    #[derive(Default)]
    struct TraceBus {
        reads: Vec<(u32, AccessWidth, AccessKind)>,
        writes: Vec<(u32, AccessWidth, AccessKind, u32)>,
        read_value: u32,
        internal: u64,
    }

    impl Bus for TraceBus {
        fn read_cpu_data(&mut self, address: u32, width: AccessWidth, kind: AccessKind) -> u32 {
            self.reads.push((address, width, kind));
            self.read_value
        }
        fn write_cpu_data(
            &mut self,
            address: u32,
            width: AccessWidth,
            kind: AccessKind,
            value: u32,
        ) {
            self.writes.push((address, width, kind, value));
        }
        fn elapse_cpu_internal(&mut self, cycles: Cycle) {
            self.internal += cycles;
        }
        fn cpu_ram_word(&self, _: u32) -> u32 {
            unreachable!()
        }
        fn set_cpu_ram_word(&mut self, _: u32, _: u32) {
            unreachable!()
        }
        fn finish_cpu_ram_load_block(&mut self, _: Cycle, _: u32) {
            unreachable!()
        }
        fn finish_cpu_ram_store_block(&mut self, _: Cycle) {
            unreachable!()
        }
    }

    struct RamBus {
        optimized: bool,
        words: [u32; 8],
        cycles: Cycle,
        last_word: u32,
    }

    impl RamBus {
        fn index(address: u32) -> usize {
            assert_eq!(address & 3, 0);
            ((address - 0x0300_0100) / 4) as usize
        }
    }

    impl Bus for RamBus {
        fn read_cpu_data(&mut self, address: u32, width: AccessWidth, _: AccessKind) -> u32 {
            assert_eq!(width, AccessWidth::Word);
            self.cycles += 1;
            self.last_word = self.words[Self::index(address)];
            self.last_word
        }
        fn write_cpu_data(&mut self, address: u32, width: AccessWidth, _: AccessKind, value: u32) {
            assert_eq!(width, AccessWidth::Word);
            self.cycles += 1;
            self.words[Self::index(address)] = value;
        }
        fn elapse_cpu_internal(&mut self, cycles: Cycle) {
            self.cycles += cycles;
        }
        fn cpu_ram_block_cycles(&self, _first: u32, beats: u32, internal: Cycle) -> Option<Cycle> {
            self.optimized.then_some(Cycle::from(beats) + internal)
        }
        fn cpu_ram_word(&self, address: u32) -> u32 {
            self.words[Self::index(address)]
        }
        fn set_cpu_ram_word(&mut self, address: u32, value: u32) {
            self.words[Self::index(address)] = value;
        }
        fn finish_cpu_ram_load_block(&mut self, cycles: Cycle, last_word: u32) {
            self.cycles += cycles;
            self.last_word = last_word;
        }
        fn finish_cpu_ram_store_block(&mut self, cycles: Cycle) {
            self.cycles += cycles;
        }
    }

    /// ZiGBA `executeThumbMultipleTransfer`: admitted RAM blocks match every
    /// scalar beat, including a listed base register and a trailing load I cycle.
    #[test]
    fn thumb_ram_blocks_match_per_beat_reference() {
        for (instruction, base_register) in [(0xc10b, 1), (0xca25, 2)] {
            let mut core = Core::new(0x0800_0040, 0x3f);
            core.registers[0] = 0x7a19_0365;
            core.registers[1] = 0x0331_0233;
            core.registers[2] = 0x02b5_090a;
            core.registers[3] = 0xdead_00af;
            core.registers[base_register] = 0x0300_0101;
            let words = [0x187a_2596, 0x0300_0073, 0x6ace_0991, 0, 0, 0, 0, 0];
            let mut reference_bus = RamBus { optimized: false, words, cycles: 0, last_word: 0 };
            let mut fast_bus = RamBus { optimized: true, words, cycles: 0, last_word: 0 };
            let mut fast = core.clone();
            let reference = core.execute_thumb(&mut reference_bus, instruction);
            let optimized = fast.execute_thumb(&mut fast_bus, instruction);
            assert_eq!(optimized, reference);
            assert_eq!(fast, core);
            assert_eq!(fast_bus.words, reference_bus.words);
            assert_eq!(fast_bus.cycles, reference_bus.cycles);
            assert_eq!(fast_bus.last_word, reference_bus.last_word);
        }
    }

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

    /// ZiGBA `decodeOperand2`: register LSR #32 carries bit 31; LSR #0 by register does not shift.
    #[test]
    fn register_shift_boundary() {
        let shifted = Core::decode_register_shift(0x8000_0003, 1, 32);
        assert_eq!(shifted, ShifterResult { value: 0, carry: Some(true) });
        let unshifted = Core::decode_register_shift(0x8000_0003, 1, 0);
        assert_eq!(unshifted, ShifterResult { value: 0x8000_0003, carry: None });
    }

    /// ZiGBA `executeArm`: SUBS pc, lr, #4 restores T and its target's bit 1.
    #[test]
    fn data_processing_exception_return_to_thumb() {
        let mut cpu = Core::new(0x0800_0100, 0x92);
        cpu.registers[14] = 0x0800_000a;
        cpu.spsr_irq = 0x6000_003f;
        let mut bus = TraceBus::default();
        assert!(cpu.execute_arm(&mut bus, 0xe25e_f004).unwrap().branched);
        assert_eq!(cpu.next_instruction_address, 0x0800_0006);
        assert_eq!(cpu.cpsr, 0x6000_003f);
        assert_eq!(cpu.registers[13], 0x0300_7f00);
        assert!(bus.reads.is_empty());
    }

    /// ZiGBA `executeSingleDataTransfer`: STR pc stores address +12, not the visible address +8.
    #[test]
    fn str_pc_and_unaligned_ldr_pc() {
        let mut cpu = Core::new(0x0800_0040, 0x1f);
        cpu.registers[1] = 0x0200_0001;
        let mut bus = TraceBus { read_value: 0x0000_0091, ..TraceBus::default() };
        cpu.execute_arm(&mut bus, 0xe581_f000).unwrap();
        assert_eq!(
            bus.writes,
            [(0x0200_0000, AccessWidth::Word, AccessKind::Nonsequential, 0x0800_004c)]
        );
        bus.read_value = 0x0180_0000;
        assert!(cpu.execute_arm(&mut bus, 0xe591_f000).unwrap().branched);
        assert_eq!(bus.reads, [(0x0200_0000, AccessWidth::Word, AccessKind::Nonsequential)]);
        assert_eq!(cpu.next_instruction_address, 0x0001_8000);
        assert_eq!(bus.internal, 1);
    }

    /// ZiGBA `executeSingleDataTransfer` / jsmolka-arm test 353: the I bit of
    /// a transfer selects a shifted register, not a rotated immediate.
    #[test]
    fn register_offset_transfer_indexes_and_writes_back() {
        let mut cpu = Core::new(0x0800_0040, 0x1f);
        cpu.registers[0] = 32;
        cpu.registers[1] = 1;
        cpu.registers[2] = 0x0300_0100;
        let mut bus = TraceBus { read_value: 32, ..TraceBus::default() };
        cpu.execute_arm(&mut bus, 0xe482_0004).unwrap(); // STR r0, [r2], #4
        assert_eq!(cpu.registers[2], 0x0300_0104);
        cpu.execute_arm(&mut bus, 0xe732_3101).unwrap(); // LDR r3, [r2, -r1, LSL #2]!
        assert_eq!(bus.writes, [(0x0300_0100, AccessWidth::Word, AccessKind::Nonsequential, 32)]);
        assert_eq!(bus.reads, [(0x0300_0100, AccessWidth::Word, AccessKind::Nonsequential)]);
        assert_eq!(cpu.registers[3], 32);
        assert_eq!(cpu.registers[2], 0x0300_0100);
        assert_eq!(bus.internal, 1);
    }

    /// ZiGBA `executeSingleDataTransfer`: bit 25 selects a register offset, not a rotated immediate.
    #[test]
    fn arm_str_register_offset_uses_shifted_register_value() {
        let mut cpu = Core::new(0x0800_0040, 0x1f);
        cpu.registers[0] = 0x1111_1111;
        cpu.registers[1] = 0x0600_4000;
        cpu.registers[2] = 7;
        let mut bus = TraceBus::default();
        // STR r0, [r1, r2, LSL #2]: the target is the last word of a 32-byte tile.
        cpu.execute_arm(&mut bus, 0xe781_0102).unwrap();
        assert_eq!(
            bus.writes,
            [(0x0600_401c, AccessWidth::Word, AccessKind::Nonsequential, 0x1111_1111)]
        );
    }

    #[test]
    fn admitted_arm_alu_handlers_match_interpreter_for_every_opcode_and_shift_form() {
        for opcode in 0..16 {
            for flags in [false, true] {
                if (8..=11).contains(&opcode) && !flags {
                    continue; // test opcodes without S are not admitted by the kernel.
                }
                for operand in [(1 << 25) | 0x2a5, 0x372] {
                    let instruction = 0xe000_0000
                        | (opcode << 21)
                        | (u32::from(flags) << 20)
                        | (1 << 16)
                        | (4 << 12)
                        | operand;
                    let mut reference = Core::new(0x0800_0040, 0x1f);
                    reference.cpsr |= 0xb000_0000;
                    reference.registers[1] = 0x7162_5aed;
                    reference.registers[2] = 0x8000_0181;
                    reference.registers[3] = 9;
                    reference.registers[4] = 0x1234_5678;
                    let mut fast = reference.clone();
                    let mut reference_bus = TraceBus::default();
                    let mut fast_bus = TraceBus::default();
                    let expected = reference.execute_arm(&mut reference_bus, instruction).unwrap();
                    let handler = ArmAluHandler::decode(instruction).unwrap();
                    let actual = handler.execute(&mut fast, &mut fast_bus, instruction);
                    assert_eq!(actual, expected, "opcode {opcode}, flags {flags}, {operand:#x}");
                    assert_eq!(fast, reference, "opcode {opcode}, flags {flags}, {operand:#x}");
                    assert_eq!(fast_bus.internal, reference_bus.internal);
                }
            }
        }
    }

    /// ZiGBA `executeBlockDataTransfer`: empty STM transfers one aligned PC beat and steps by 64.
    #[test]
    fn empty_stm_preserves_low_base_bits() {
        let mut cpu = Core::new(0x0800_0020, 0x1f);
        cpu.registers[2] = 0x0300_0003;
        let mut bus = TraceBus::default();
        cpu.execute_arm(&mut bus, 0xe8a2_0000).unwrap();
        assert_eq!(
            bus.writes,
            [(0x0300_0000, AccessWidth::Word, AccessKind::Nonsequential, 0x0800_002c)]
        );
        assert_eq!(cpu.registers[2], 0x0300_0043);
    }

    /// ZiGBA `executeMultiplyRegisters`: timing captures Rs before an aliased destination changes it.
    #[test]
    fn multiply_destination_alias_keeps_original_timing() {
        let mut cpu = Core::new(0, 0x1f);
        cpu.registers[3] = 0x0100_0000;
        cpu.registers[2] = 2;
        let mut bus = TraceBus::default();
        cpu.execute_arm(&mut bus, 0xe003_0392).unwrap();
        assert_eq!(cpu.registers[3], 0x0200_0000);
        assert_eq!(bus.internal, 4);
    }

    /// The mGBA suite's multiply-long table (`src/multiply-long.c`), recorded
    /// on hardware: `SMULLS r9, r8, r0, r1` and `UMULLS r9, r8, r0, r1` from
    /// cleared flags, with `r0` and `r1` each of 0, 1, -1, `7FFFFFFFh`,
    /// `80000000h` and `80000001h`. Each row is `(r0, r1, [r8, r9, CPSR] for
    /// SMULLS, [r8, r9, CPSR] for UMULLS)`; C comes from the multiplier's
    /// carry-save vectors ([`multiply_carry`]).
    #[test]
    fn flag_setting_long_multiplies_match_the_mgba_suite_hardware_table() {
        #[rustfmt::skip]
        let table: [(u32, u32, [u32; 3], [u32; 3]); 36] = [
            (0x00000000, 0x00000000, [0x00000000, 0x00000000, 0x4000001f], [0x00000000, 0x00000000, 0x4000001f]), // 0 *   0
            (0x00000001, 0x00000000, [0x00000000, 0x00000000, 0x4000001f], [0x00000000, 0x00000000, 0x4000001f]), // 1 *   0
            (0xffffffff, 0x00000000, [0x00000000, 0x00000000, 0x4000001f], [0x00000000, 0x00000000, 0x4000001f]), // -1 *   0
            (0x7fffffff, 0x00000000, [0x00000000, 0x00000000, 0x4000001f], [0x00000000, 0x00000000, 0x4000001f]), // $7F *   0
            (0x80000000, 0x00000000, [0x00000000, 0x00000000, 0x4000001f], [0x00000000, 0x00000000, 0x4000001f]), // $80 *   0
            (0x80000001, 0x00000000, [0x00000000, 0x00000000, 0x4000001f], [0x00000000, 0x00000000, 0x4000001f]), // $81 *   0
            (0x00000000, 0x00000001, [0x00000000, 0x00000000, 0x4000001f], [0x00000000, 0x00000000, 0x4000001f]), // 0 *   1
            (0x00000001, 0x00000001, [0x00000000, 0x00000001, 0x0000001f], [0x00000000, 0x00000001, 0x0000001f]), // 1 *   1
            (0xffffffff, 0x00000001, [0xffffffff, 0xffffffff, 0x8000001f], [0x00000000, 0xffffffff, 0x0000001f]), // -1 *   1
            (0x7fffffff, 0x00000001, [0x00000000, 0x7fffffff, 0x0000001f], [0x00000000, 0x7fffffff, 0x0000001f]), // $7F *   1
            (0x80000000, 0x00000001, [0xffffffff, 0x80000000, 0x8000001f], [0x00000000, 0x80000000, 0x0000001f]), // $80 *   1
            (0x80000001, 0x00000001, [0xffffffff, 0x80000001, 0x8000001f], [0x00000000, 0x80000001, 0x0000001f]), // $81 *   1
            (0x00000000, 0xffffffff, [0x00000000, 0x00000000, 0x4000001f], [0x00000000, 0x00000000, 0x4000001f]), // 0 *  -1
            (0x00000001, 0xffffffff, [0xffffffff, 0xffffffff, 0x8000001f], [0x00000000, 0xffffffff, 0x0000001f]), // 1 *  -1
            (0xffffffff, 0xffffffff, [0x00000000, 0x00000001, 0x0000001f], [0xfffffffe, 0x00000001, 0xa000001f]), // -1 *  -1
            (0x7fffffff, 0xffffffff, [0xffffffff, 0x80000001, 0x8000001f], [0x7ffffffe, 0x80000001, 0x2000001f]), // $7F *  -1
            (0x80000000, 0xffffffff, [0x00000000, 0x80000000, 0x0000001f], [0x7fffffff, 0x80000000, 0x0000001f]), // $80 *  -1
            (0x80000001, 0xffffffff, [0x00000000, 0x7fffffff, 0x0000001f], [0x80000000, 0x7fffffff, 0x8000001f]), // $81 *  -1
            (0x00000000, 0x7fffffff, [0x00000000, 0x00000000, 0x4000001f], [0x00000000, 0x00000000, 0x4000001f]), // 0 * $7F
            (0x00000001, 0x7fffffff, [0x00000000, 0x7fffffff, 0x0000001f], [0x00000000, 0x7fffffff, 0x0000001f]), // 1 * $7F
            (0xffffffff, 0x7fffffff, [0xffffffff, 0x80000001, 0xa000001f], [0x7ffffffe, 0x80000001, 0x2000001f]), // -1 * $7F
            (0x7fffffff, 0x7fffffff, [0x3fffffff, 0x00000001, 0x0000001f], [0x3fffffff, 0x00000001, 0x0000001f]), // $7F * $7F
            (0x80000000, 0x7fffffff, [0xc0000000, 0x80000000, 0xa000001f], [0x3fffffff, 0x80000000, 0x2000001f]), // $80 * $7F
            (0x80000001, 0x7fffffff, [0xc0000000, 0xffffffff, 0xa000001f], [0x3fffffff, 0xffffffff, 0x2000001f]), // $81 * $7F
            (0x00000000, 0x80000000, [0x00000000, 0x00000000, 0x6000001f], [0x00000000, 0x00000000, 0x4000001f]), // 0 * $80
            (0x00000001, 0x80000000, [0xffffffff, 0x80000000, 0xa000001f], [0x00000000, 0x80000000, 0x0000001f]), // 1 * $80
            (0xffffffff, 0x80000000, [0x00000000, 0x80000000, 0x0000001f], [0x7fffffff, 0x80000000, 0x2000001f]), // -1 * $80
            (0x7fffffff, 0x80000000, [0xc0000000, 0x80000000, 0xa000001f], [0x3fffffff, 0x80000000, 0x0000001f]), // $7F * $80
            (0x80000000, 0x80000000, [0x40000000, 0x00000000, 0x0000001f], [0x40000000, 0x00000000, 0x2000001f]), // $80 * $80
            (0x80000001, 0x80000000, [0x3fffffff, 0x80000000, 0x0000001f], [0x40000000, 0x80000000, 0x2000001f]), // $81 * $80
            (0x00000000, 0x80000001, [0x00000000, 0x00000000, 0x6000001f], [0x00000000, 0x00000000, 0x4000001f]), // 0 * $81
            (0x00000001, 0x80000001, [0xffffffff, 0x80000001, 0xa000001f], [0x00000000, 0x80000001, 0x0000001f]), // 1 * $81
            (0xffffffff, 0x80000001, [0x00000000, 0x7fffffff, 0x0000001f], [0x80000000, 0x7fffffff, 0xa000001f]), // -1 * $81
            (0x7fffffff, 0x80000001, [0xc0000000, 0xffffffff, 0xa000001f], [0x3fffffff, 0xffffffff, 0x0000001f]), // $7F * $81
            (0x80000000, 0x80000001, [0x3fffffff, 0x80000000, 0x0000001f], [0x40000000, 0x80000000, 0x2000001f]), // $80 * $81
            (0x80000001, 0x80000001, [0x3fffffff, 0x00000001, 0x0000001f], [0x40000001, 0x00000001, 0x2000001f]), // $81 * $81
        ];
        for (rm, rs, smull, umull) in table {
            for (instruction, expected) in [(0xe0d8_9190, smull), (0xe098_9190, umull)] {
                let mut cpu = Core::new(0x0800_0040, 0x1f);
                (cpu.registers[0], cpu.registers[1]) = (rm, rs);
                cpu.execute_arm(&mut TraceBus::default(), instruction).unwrap();
                let got = [cpu.registers[8], cpu.registers[9], cpu.cpsr];
                assert_eq!(got, expected, "{instruction:08x} with {rm:08x} * {rs:08x}");
            }
        }
    }

    /// C after every flag-setting multiply form, for multipliers of one to
    /// four cycles with C clear and set: `(form, Rm, Rs, RdLo or Rn, RdHi,
    /// C)`, with forms 0 to 5 `MULS`, `MLAS`, `UMULLS`, `UMLALS`, `SMULLS`,
    /// `SMLALS`. The expected C comes from the model's published reference
    /// implementation (zaydlang/multiplication-algorithm, `impl.h`), run on
    /// these operands; Thumb `MUL` repeats the `MULS` rows with Rd as the
    /// multiplier.
    #[test]
    fn flag_setting_multiplies_take_c_from_the_reference_multiplier_model() {
        #[rustfmt::skip]
        let table: [(usize, u32, u32, u32, u32, bool); 48] = [
            (0, 0x3d9c2a99, 0xffffff74, 0x00005681, 0xffff9a8b, false),
            (0, 0x000000ff, 0xffffff1e, 0x00007b41, 0x372f9f5e, true),
            (0, 0x7fffffff, 0xfffff7c1, 0xffffff2b, 0xffff4095, false),
            (0, 0x00000000, 0xffff34f1, 0x000000c0, 0x00000100, true),
            (0, 0x7fffffff, 0xffcca63e, 0x00000060, 0x00000055, false),
            (0, 0xadb46ced, 0x009e8185, 0x0000d7ac, 0x00009c6d, true),
            (0, 0xffff737e, 0x7fffffff, 0x00000025, 0x80000001, false),
            (0, 0x000000ce, 0x80000001, 0x00000040, 0xffffb488, true),
            (1, 0x12d6098f, 0xffffff50, 0xffff1378, 0x80000001, false),
            (1, 0x000000ff, 0xffffff56, 0xaa3e1a05, 0xffffffa9, true),
            (1, 0xffff6f9a, 0x00000a76, 0xffffffc1, 0xffffff5d, false),
            (1, 0x00000050, 0xffff489d, 0x0000002b, 0x5563793d, true),
            (1, 0xffff60c3, 0x00975297, 0xe8edad59, 0xc28f9e00, false),
            (1, 0xffffd16c, 0xff9dad72, 0x28c873ad, 0x395e8c2d, true),
            (1, 0xffff87e3, 0x34d7b4ca, 0x00000099, 0xffff47db, false),
            (1, 0x000000ff, 0xb9fd3852, 0xfffff3b5, 0xffffffdd, true),
            (2, 0x0000003b, 0x00000000, 0xffffffff, 0xffffab34, false),
            (2, 0xffffad56, 0x0000009f, 0x000000cb, 0x00008534, true),
            (2, 0x00000097, 0x0000982b, 0x0000007b, 0xffffe409, false),
            (2, 0xffffff10, 0x00005e4b, 0xe84eeb2d, 0xbf3d19d2, true),
            (2, 0xb15beb14, 0x00e9ec5a, 0x0000c5ad, 0x000000d6, false),
            (2, 0x00007107, 0x004b5014, 0x000017fd, 0xffff38e7, true),
            (2, 0x000000a3, 0x74a9d8b5, 0xffff5e38, 0x1c3e9a83, false),
            (2, 0xffffff7a, 0xffffffff, 0xffffae32, 0x000000c0, true),
            (3, 0x7fffffff, 0x000000d0, 0x9aa07b96, 0xfffffceb, false),
            (3, 0xffffffed, 0x00000088, 0x000000ff, 0xffffffff, true),
            (3, 0x000000be, 0x00009f5d, 0xffff132a, 0xffffff68, false),
            (3, 0xffffff04, 0x00005f1f, 0xffffa8ee, 0x00004659, true),
            (3, 0xfee39156, 0x008f0cc3, 0x0000009e, 0xff70fd38, false),
            (3, 0x7e6ff20b, 0x00d23a69, 0x0000a0b9, 0x00004a8f, true),
            (3, 0xffffffd6, 0x22a2894e, 0x8afce860, 0xc1decd0a, false),
            (3, 0x0000b164, 0xffffffcb, 0x0000e64d, 0x09e5b47f, true),
            (4, 0x00000007, 0xffffffff, 0xffffffff, 0x3c603a02, false),
            (4, 0xffffff9d, 0x00000089, 0x0000553f, 0xffff4d27, true),
            (4, 0xffffffff, 0xffff1e4f, 0xffffffb9, 0x0000a25e, false),
            (4, 0x0000994e, 0xffff1149, 0xffff2db6, 0xffffff3b, true),
            (4, 0x95f10095, 0xff5c07d1, 0x00000001, 0x8982a97d, false),
            (4, 0x80000001, 0xff1cb1e5, 0x7fffffff, 0xffffff01, true),
            (4, 0xffffffff, 0x25b35f9d, 0xffffff73, 0xffffa505, false),
            (4, 0xa140cdfa, 0x7772e516, 0xffffffff, 0x00004425, true),
            (5, 0x000022a9, 0x00000007, 0xc0f0296d, 0xfffff0fc, false),
            (5, 0x00000020, 0xffffff70, 0x0000008d, 0x8125e110, true),
            (5, 0x00000b0a, 0x0000b024, 0xffffe03c, 0xffffff9f, false),
            (5, 0xffff7cc2, 0xffff76ad, 0x80000001, 0x268db523, true),
            (5, 0x47ebf780, 0x007061cc, 0xffffffd4, 0xe6da189b, false),
            (5, 0x14f47aff, 0x00ffdc1b, 0x00000067, 0xffffb13e, true),
            (5, 0x928a572f, 0x80000000, 0x0000b395, 0x00000036, false),
            (5, 0xffff61bb, 0x7fffffff, 0x00000000, 0x000031d8, true),
        ];
        const ARM: [u32; 6] =
            [0xe012_0190, 0xe032_3190, 0xe095_4190, 0xe0b5_4190, 0xe0d5_4190, 0xe0f5_4190];
        for (form, rm, rs, low, high, carry) in table {
            let mut cpu = Core::new(0x0800_0040, 0x1f);
            cpu.registers[..6].copy_from_slice(&[rm, rs, 0, low, low, high]);
            cpu.execute_arm(&mut TraceBus::default(), ARM[form]).unwrap();
            let context = format!("form {form}: {rm:08x} * {rs:08x} + {high:08x}:{low:08x}");
            assert_eq!(carry, cpu.cpsr & (1 << 29) != 0, "{context}");
            if form == 0 {
                let mut cpu = Core::new(0x0800_0040, 0x3f);
                (cpu.registers[0], cpu.registers[1]) = (rm, rs);
                cpu.execute_thumb(&mut TraceBus::default(), 0x4341); // MUL r1, r0
                assert_eq!(rm.wrapping_mul(rs), cpu.registers[1], "Thumb {context}");
                assert_eq!(carry, cpu.cpsr & (1 << 29) != 0, "Thumb {context}");
            }
        }
    }

    /// The mGBA suite's memory table (`swi B 32` and `swi C 32` rows,
    /// recorded on hardware): the retail CpuSet and CpuFastSet, whose word
    /// loops are `LDMIA`/`STMIA` (GBATEK, BIOS Memory Copy), load and store
    /// the SRAM byte at an unaligned address itself. A block transfer gives
    /// the 8-bit save bus its exact addresses, stepping by 4 from the base,
    /// as a single transfer does; every other region sees them aligned.
    #[test]
    fn block_transfers_present_exact_addresses_to_the_save_bus() {
        use AccessKind::{Nonsequential as N, Sequential as S};
        use AccessWidth::Word;
        let arm = |base: u32, instruction: u32| {
            let mut cpu = Core::new(0x0800_0040, 0x1f);
            (cpu.registers[0], cpu.registers[1], cpu.registers[2]) =
                (base, 0x1111_1111, 0x2222_2222);
            let mut bus = TraceBus::default();
            cpu.execute_arm(&mut bus, instruction).unwrap();
            (cpu.registers[0], bus.reads, bus.writes)
        };
        // LDMIA r0!, {r1, r2}
        let (base, reads, _) = arm(0x0e00_0001, 0xe8b0_0006);
        assert_eq!(reads, [(0x0e00_0001, Word, N), (0x0e00_0005, Word, S)]);
        assert_eq!(base, 0x0e00_0009);
        let (_, reads, _) = arm(0x0300_0001, 0xe8b0_0006);
        assert_eq!(reads, [(0x0300_0000, Word, N), (0x0300_0004, Word, S)]);
        // STMIA r0, {r1, r2}
        let (_, _, writes) = arm(0x0e00_0003, 0xe880_0006);
        assert_eq!(
            writes,
            [(0x0e00_0003, Word, N, 0x1111_1111), (0x0e00_0007, Word, S, 0x2222_2222)]
        );

        let thumb = |base: u32, instruction: u16| {
            let mut cpu = Core::new(0x0800_0040, 0x3f);
            (cpu.registers[0], cpu.registers[1], cpu.registers[13]) = (base, 0x1111_1111, base);
            let mut bus = TraceBus::default();
            cpu.execute_thumb(&mut bus, instruction);
            (bus.reads, bus.writes)
        };
        // LDMIA r0!, {r1}; STMIA r0!, {r1}; POP {r1}; PUSH {r1}
        assert_eq!(thumb(0x0e00_0002, 0xc802).0, [(0x0e00_0002, Word, N)]);
        assert_eq!(thumb(0x0e00_0002, 0xc002).1, [(0x0e00_0002, Word, N, 0x1111_1111)]);
        assert_eq!(thumb(0x0e00_0003, 0xbc02).0, [(0x0e00_0003, Word, N)]);
        assert_eq!(thumb(0x0e00_0007, 0xb402).1, [(0x0e00_0003, Word, N, 0x1111_1111)]);
    }

    /// ZiGBA `executeThumb`: BX selects ARM only for an even target, without rounding a Thumb return to a word.
    #[test]
    fn thumb_bx_interworks_but_pop_pc_does_not() {
        let mut cpu = Core::new(0x0800_0002, 0x3f);
        cpu.registers[1] = 0x0800_0084;
        let mut bus = TraceBus { read_value: 0x0800_00a3, ..TraceBus::default() };
        assert!(cpu.execute_thumb(&mut bus, 0x4708).branched);
        assert_eq!(cpu.next_instruction_address, 0x0800_0084);
        assert!(!cpu.is_thumb());

        cpu.cpsr |= 1 << 5;
        cpu.registers[13] = 0x0300_0013;
        assert!(cpu.execute_thumb(&mut bus, 0xbd00).branched);
        assert_eq!(bus.reads, [(0x0300_0010, AccessWidth::Word, AccessKind::Nonsequential)]);
        assert_eq!(cpu.next_instruction_address, 0x0800_00a2);
        assert_eq!(cpu.registers[13], 0x0300_0017);
        assert!(cpu.is_thumb());
        assert_eq!(bus.internal, 1);
    }

    /// ZiGBA `executeThumb`: an empty PUSH sends one aligned PC+6 beat and moves SP by 64.
    #[test]
    fn thumb_empty_push_preserves_stack_low_bits() {
        let mut cpu = Core::new(0x0800_0042, 0x3f);
        cpu.registers[15] = 0x0800_0046;
        cpu.registers[13] = 0x0300_0103;
        let mut bus = TraceBus::default();
        cpu.execute_thumb(&mut bus, 0xb400);
        assert_eq!(
            bus.writes,
            [(0x0300_00c0, AccessWidth::Word, AccessKind::Nonsequential, 0x0800_0048)]
        );
        assert_eq!(cpu.registers[13], 0x0300_00c3);
    }
}
