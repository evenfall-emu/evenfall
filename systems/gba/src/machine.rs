//! Deterministic host-facing hardware seam, translated from `machine.zig`.

use crate::bus::{self, Bus, KeyState, TimedInput};
use crate::cpu::{self, Cpu};
use crate::save::Type as SaveType;
use crate::cartridge_tilt::Sample as TiltSample;
use evenfall_host::Cycle;

pub use cpu::FaultReason;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fault {
    pub cycle: Cycle,
    pub instruction_address: u32,
    pub instruction: u32,
    pub instruction_set: cpu::InstructionSet,
    pub reason: FaultReason,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    Deadline,
    InstructionBoundaryAfterDeadline,
    PowerStopped,
    Faulted(Fault),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunResult {
    pub started_at: Cycle,
    pub stopped_at: Cycle,
    pub reason: StopReason,
    pub inputs_consumed: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WaitResult {
    pub started_at: Cycle,
    pub stopped_at: Cycle,
    pub reason: cpu::WaitReason,
    pub woke: bool,
    pub inputs_consumed: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunError {
    TargetBeforeCurrentCycle,
    InputBeforeCurrentCycle,
    InputsOutOfOrder,
    CpuNotWaiting,
    MachineFaulted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CpuSnapshot {
    pub cycle: Cycle,
    pub registers: [u32; 16],
    pub next_instruction_address: u32,
    pub cpsr: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AddressOverflow;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkedFault {
    pub machine_index: usize,
    pub fault: Fault,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkedStopReason {
    Deadline,
    PowerStopped,
    Faulted(LinkedFault),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkedRunResult {
    pub earliest_cycle: Cycle,
    pub latest_cycle: Cycle,
    pub reason: LinkedStopReason,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkedRunError {
    Link(bus::SioLinkError),
    DuplicateMachine,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootMode {
    Hle,
    SkipBios,
    ExternalBios,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StepResult {
    pub started_at: Cycle,
    pub stopped_at: Cycle,
    pub instruction_address: u32,
    pub instruction: u32,
    pub instruction_set: cpu::InstructionSet,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepOutcome {
    Executed(StepResult),
    Waiting(cpu::WaitReason),
    Faulted(Fault),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StepWithInputResult {
    pub outcome: StepOutcome,
    pub inputs_consumed: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct Config<'a> {
    pub rom: &'a [u8],
    pub bios: &'a [u8],
    /// The owner promises to keep the ROM and BIOS immutable.
    pub immutable_media: bool,
    pub boot: BootMode,
    pub save_type: SaveType,
    pub save_data: Option<&'a [u8]>,
    pub rtc_epoch_seconds: Option<u64>,
    pub solar_level: Option<u8>,
    pub gyro_sample: Option<u16>,
    pub rumble: bool,
    pub tilt_sample: Option<TiltSample>,
}

impl Default for Config<'_> {
    fn default() -> Self {
        Self {
            rom: &[], bios: &[], immutable_media: false, boot: BootMode::Hle,
            save_type: SaveType::Auto, save_data: None, rtc_epoch_seconds: None,
            solar_level: None, gyro_sample: None, rumble: false, tilt_sample: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MediaIdentity {
    rom: [u8; 32],
    bios: [u8; 32],
}

impl MediaIdentity {
    fn new(rom: &[u8], bios: &[u8]) -> Self {
        Self { rom: *blake3::hash(rom).as_bytes(), bios: *blake3::hash(bios).as_bytes() }
    }
}

pub struct Machine {
    pub boot_mode: BootMode,
    pub bus: Bus,
    pub cpu: Cpu,
    latched_fault: Option<Fault>,
    media_identity: Option<MediaIdentity>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slice {
    pub yield_at: Cycle,
    pub input_deadline: Cycle,
    pub stop_at: Option<Cycle>,
}

impl Machine {
    /// Allocates the large machine directly in its final heap location.
    pub fn new(config: Config<'_>) -> Box<Self> {
        assert!(matches!(config.boot, BootMode::Hle) == config.bios.is_empty(),
            "HLE boot does not accept BIOS; external boot requires BIOS");
        let entry_address = if config.boot == BootMode::ExternalBios { 0 } else { 0x0800_0000 };
        let initial_cpsr = if config.boot == BootMode::ExternalBios { 0xD3 } else { 0x1F };
        let mut storage = Box::<Self>::new_uninit();
        let ptr = storage.as_mut_ptr();
        // SAFETY: Each field is written once before assume_init; Bus writes its
        // own large arrays directly into the allocated field, never on stack.
        unsafe {
            std::ptr::addr_of_mut!((*ptr).boot_mode).write(config.boot);
            Bus::init_in_place(std::ptr::addr_of_mut!((*ptr).bus), config.rom, config.bios,
                config.save_type, config.save_data);
            std::ptr::addr_of_mut!((*ptr).cpu).write(Cpu::init(entry_address, initial_cpsr, config.boot == BootMode::Hle));
            std::ptr::addr_of_mut!((*ptr).latched_fault).write(None);
            std::ptr::addr_of_mut!((*ptr).media_identity).write(
                config.immutable_media.then(|| MediaIdentity::new(config.rom, config.bios)));
            let machine = storage.assume_init();
            if let Some(epoch) = config.rtc_epoch_seconds { machine.bus.enable_rtc(epoch); }
            if let Some(level) = config.solar_level { machine.bus.enable_solar_sensor(level); }
            if let Some(sample) = config.gyro_sample { machine.bus.enable_gyro(sample); }
            if config.rumble { machine.bus.enable_rumble(); }
            if let Some(sample) = config.tilt_sample { machine.bus.enable_tilt(sample); }
            machine
        }
    }

    pub fn now(&self) -> Cycle { self.bus.now() }
    pub fn fault(&self) -> Option<Fault> { self.latched_fault }

    fn latch_bus_fault(&mut self) -> Option<Fault> {
        if self.latched_fault.is_none() {
            let livelock = self.bus.dma_livelock()?;
            let cpu_fault = self.cpu.boundary_fault(&self.bus, FaultReason::DmaLivelock {
                channel: livelock.channel, started_at: livelock.started_at,
            });
            self.latched_fault = Some(Fault {
                cycle: self.now(), instruction_address: cpu_fault.instruction_address,
                instruction: cpu_fault.instruction, instruction_set: cpu_fault.instruction_set,
                reason: cpu_fault.reason,
            });
        }
        self.latched_fault
    }

    pub fn step(&mut self) -> StepOutcome {
        if let Some(fault) = self.latched_fault { return StepOutcome::Faulted(fault); }
        let started_at = self.now();
        let outcome = self.cpu.step(&mut self.bus);
        if let Some(fault) = self.latch_bus_fault() { return StepOutcome::Faulted(fault); }
        match outcome {
            cpu::StepOutcome::Waiting(reason) => StepOutcome::Waiting(reason),
            cpu::StepOutcome::Executed(step) => StepOutcome::Executed(StepResult {
                started_at, stopped_at: self.now(), instruction_address: step.instruction_address,
                instruction: step.instruction, instruction_set: step.instruction_set,
            }),
            cpu::StepOutcome::Faulted(step) => {
                let fault = Fault { cycle: self.now(), instruction_address: step.instruction_address,
                    instruction: step.instruction, instruction_set: step.instruction_set, reason: step.reason };
                self.latched_fault = Some(fault);
                StepOutcome::Faulted(fault)
            }
        }
    }

    pub fn step_with_input(&mut self, inputs: &[TimedInput]) -> Result<StepWithInputResult, RunError> {
        if let Some(fault) = self.latched_fault {
            return Ok(StepWithInputResult { outcome: StepOutcome::Faulted(fault), inputs_consumed: 0 });
        }
        self.validate_inputs(inputs)?;
        self.bus.begin_external_inputs(inputs);
        let outcome = self.step();
        let inputs_consumed = self.bus.external_inputs_consumed();
        self.bus.end_external_inputs();
        Ok(StepWithInputResult { outcome, inputs_consumed })
    }

    fn validate_inputs(&self, inputs: &[TimedInput]) -> Result<(), RunError> {
        let mut previous_at = self.now();
        for input in inputs {
            if input.at < self.now() { return Err(RunError::InputBeforeCurrentCycle); }
            if input.at < previous_at { return Err(RunError::InputsOutOfOrder); }
            previous_at = input.at;
        }
        Ok(())
    }

    fn begin_inputs_until(&mut self, inputs: &[TimedInput], deadline: Cycle) {
        let count = inputs.partition_point(|input| input.at <= deadline);
        self.bus.begin_external_inputs(&inputs[..count]);
    }

    pub fn wait_until(&mut self, deadline: Cycle) -> Result<WaitResult, RunError> {
        self.wait_until_with_input(deadline, &[])
    }

    pub fn wait_until_with_input(&mut self, deadline: Cycle, inputs: &[TimedInput]) -> Result<WaitResult, RunError> {
        if self.latched_fault.is_some() { return Err(RunError::MachineFaulted); }
        if deadline < self.now() { return Err(RunError::TargetBeforeCurrentCycle); }
        self.validate_inputs(inputs)?;
        if self.cpu.inside_hle_bulk_service() || self.bus.power_state() == bus::PowerState::Running {
            return Err(RunError::CpuNotWaiting);
        }
        self.begin_inputs_until(inputs, deadline);
        let started_at = self.now();
        let (reason, woke) = match self.bus.power_state() {
            bus::PowerState::Halted => (cpu::WaitReason::Halted, self.bus.idle_halt_until(deadline)),
            bus::PowerState::Stopped => (cpu::WaitReason::Stopped, self.bus.idle_stop_until(deadline)),
            bus::PowerState::Running => unreachable!(),
        };
        let _ = self.latch_bus_fault();
        let result = WaitResult { started_at, stopped_at: self.now(), reason, woke,
            inputs_consumed: self.bus.external_inputs_consumed() };
        self.bus.end_external_inputs();
        Ok(result)
    }

    pub fn read_register(&self, index: u8) -> u32 { self.cpu.read_register(index) }
    pub fn next_instruction_address(&self) -> u32 { self.cpu.next_instruction_address() }
    pub fn read_cpsr(&self) -> u32 { self.cpu.read_cpsr() }
    pub fn inspect_cpu(&self) -> CpuSnapshot {
        CpuSnapshot { cycle: self.now(), registers: self.cpu.registers,
            next_instruction_address: self.cpu.next_instruction_address(), cpsr: self.cpu.read_cpsr() }
    }

    pub fn inspect_memory(&self, address: u32, output: &mut [u8]) -> Result<(), AddressOverflow> {
        if output.len as u64 > u64::from(u32::MAX) - u64::from(address) + 1 { return Err(AddressOverflow); }
        for (offset, byte) in output.iter_mut().enumerate() {
            *byte = self.bus.inspect_byte(address + offset as u32);
        }
        Ok(())
    }

    pub fn set_keys(&mut self, keys: KeyState) { self.bus.set_keys(keys); }
    pub fn set_solar_level(&mut self, level: u8) { self.bus.set_solar_level(level); }
    pub fn set_gyro_sample(&mut self, sample: u16) { self.bus.set_gyro_sample(sample); }
    pub fn rumble_active(&self) -> bool { self.bus.rumble_active() }
    pub fn set_tilt(&mut self, sample: TiltSample) { self.bus.set_tilt(sample); }
    pub fn framebuffer(&self) -> &[u16] { self.bus.framebuffer() }
    pub fn completed_frames(&self) -> Cycle { self.bus.completed_frames() }
    pub fn drain_audio_samples(&mut self, output: &mut [bus::StereoSample]) -> usize { self.bus.drain_audio_samples(output) }
    pub fn set_audio_output(&mut self, enabled: bool) { self.bus.set_audio_output(enabled); }
    pub fn set_high_speed_audio(&mut self, enabled: bool) { self.bus.set_high_speed_audio(enabled); }
    pub fn audio_sample_rate(&self) -> u32 { self.bus.audio_sample_rate() }
    pub fn save_data(&self) -> &[u8] { self.bus.save_data() }
    pub fn save_type(&self) -> SaveType { self.bus.save_type() }

    pub fn run_until(&mut self, deadline: Cycle) -> Result<RunResult, RunError> {
        self.run_until_with_input(deadline, &[])
    }

    pub fn run_until_with_input(&mut self, deadline: Cycle, inputs: &[TimedInput]) -> Result<RunResult, RunError> {
        self.run_slice_with_input(deadline, deadline, inputs)
    }

    pub fn run_slice_with_input(&mut self, yield_at: Cycle, deadline: Cycle, inputs: &[TimedInput]) -> Result<RunResult, RunError> {
        self.run_slice(Slice { yield_at, input_deadline: deadline, stop_at: None }, inputs)
    }

    pub fn run_slice(&mut self, slice: Slice, inputs: &[TimedInput]) -> Result<RunResult, RunError> {
        let yield_at = slice.yield_at;
        let deadline = slice.input_deadline;
        let stop_at = slice.stop_at.unwrap_or(yield_at);
        if let Some(fault) = self.latched_fault {
            return Ok(RunResult { started_at: self.now(), stopped_at: self.now(),
                reason: StopReason::Faulted(fault), inputs_consumed: 0 });
        }
        if yield_at < self.now() || stop_at < yield_at || deadline < stop_at {
            return Err(RunError::TargetBeforeCurrentCycle);
        }
        self.validate_inputs(inputs)?;
        self.begin_inputs_until(inputs, deadline);
        self.bus.begin_device_batch();
        let result = self.run_slice_inner(yield_at, stop_at);
        self.bus.end_device_batch();
        self.bus.end_external_inputs();
        Ok(result)
    }

    fn run_slice_inner(&mut self, yield_at: Cycle, stop_at: Cycle) -> RunResult {
        let started_at = self.now();
        let mut execution_fault = None;
        let mut chunk_fault = None;
        while self.now() < yield_at {
            if let Some(fault) = self.latch_bus_fault() {
                return self.run_result(started_at, StopReason::Faulted(fault));
            }
            let chunk_end = yield_at.min(self.now().saturating_add(Bus::MAX_DMA_BURST_CYCLES));
            if self.bus.power_state() == bus::PowerState::Stopped && !self.bus.stop_wake_pending()
                && !self.cpu.inside_hle_bulk_service() {
                self.bus.idle_stop_until(stop_at);
                if self.bus.power_state() == bus::PowerState::Stopped && !self.bus.stop_wake_pending() {
                    return self.run_result(started_at, StopReason::PowerStopped);
                }
            }
            if self.bus.power_state() == bus::PowerState::Halted && !self.bus.halt_wake_pending()
                && !self.cpu.inside_hle_bulk_service() {
                self.bus.idle_halt_until(chunk_end);
                if self.now() >= chunk_end || self.bus.dma_livelock().is_some() { continue; }
            }
            let outcome = match self.cpu.run_normal_until(&mut self.bus, chunk_end, &mut execution_fault, &mut chunk_fault) {
                cpu::ChunkOutcome::Deadline => continue,
                cpu::ChunkOutcome::Generic => self.cpu.run_step(&mut self.bus, &mut execution_fault),
                cpu::ChunkOutcome::Faulted => Some(cpu::StepOutcome::Faulted(chunk_fault.take().expect("chunk fault"))),
            };
            if let Some(cpu::StepOutcome::Faulted(step)) = outcome {
                let fault = Fault { cycle: self.now(), instruction_address: step.instruction_address,
                    instruction: step.instruction, instruction_set: step.instruction_set, reason: step.reason };
                self.latched_fault = Some(fault);
                return self.run_result(started_at, StopReason::Faulted(fault));
            }
        }
        if let Some(fault) = self.latch_bus_fault() {
            return self.run_result(started_at, StopReason::Faulted(fault));
        }
        self.run_result(started_at, if self.now() == yield_at { StopReason::Deadline }
            else { StopReason::InstructionBoundaryAfterDeadline })
    }

    fn run_result(&self, started_at: Cycle, reason: StopReason) -> RunResult {
        RunResult { started_at, stopped_at: self.now(), reason,
            inputs_consumed: self.bus.external_inputs_consumed() }
    }
}
