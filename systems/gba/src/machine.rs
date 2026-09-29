//! Deterministic host-facing hardware seam, translated from `machine.zig`.

use crate::bus::{self, Bus, KeyState, TimedInput};
use crate::cartridge::{save::Type as SaveType, tilt::Sample as TiltSample};
use crate::cpu::{self, Cpu};
use evenfall_host::Cycle;
use evenfall_state::{Codec, Sink, Source};
use std::io::Write;
use std::ops::Range;

pub use cpu::FaultReason;

impl From<evenfall_host::Timed<bus::TimedInputChange>> for TimedInput {
    fn from(input: evenfall_host::Timed<bus::TimedInputChange>) -> Self {
        Self { at: input.at, change: input.input }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fault {
    pub cycle: Cycle,
    pub instruction_address: u32,
    pub instruction: u32,
    pub instruction_set: cpu::InstructionSet,
    pub reason: FaultReason,
}

impl Fault {
    /// One stable, host-agnostic diagnostic line, as in ZiGBA.
    pub fn write_diagnostic(&self, writer: &mut impl Write) -> std::io::Result<()> {
        write!(
            writer,
            "emulation fault cycle={} pc={:08x} {}={:08x} reason=",
            self.cycle,
            self.instruction_address,
            tag_name(self.instruction_set),
            self.instruction
        )?;
        match self.reason {
            FaultReason::Isa(reason) => match reason {
                evenfall_cpu_arm::FaultReason::InvalidPsrState { operation, kind, value } => {
                    writeln!(
                        writer,
                        "invalid_psr_state operation={} kind={} value={value:08x}",
                        tag_name(operation),
                        tag_name(kind)
                    )
                }
                evenfall_cpu_arm::FaultReason::MissingSpsr { operation, mode } => writeln!(
                    writer,
                    "missing_spsr operation={} mode={}",
                    tag_name(operation),
                    tag_name(mode)
                ),
                evenfall_cpu_arm::FaultReason::UnpredictableArmEncoding {
                    instruction_class,
                    offending_registers,
                } => writeln!(
                    writer,
                    "unpredictable_arm_encoding class={} offending_registers={offending_registers:04x}",
                    tag_name(instruction_class)
                ),
            },
            FaultReason::UnsupportedHleSwi { service } => {
                writeln!(writer, "unsupported_hle_swi service={service:02x}")
            }
            FaultReason::InvalidHleMemoryAccess { service, operation, address, width } => writeln!(
                writer,
                "invalid_hle_memory_access service={service:02x} operation={} address={address:08x} width={}",
                tag_name(operation),
                tag_name(width)
            ),
            FaultReason::HleIrqNestingLimit { depth } => {
                writeln!(writer, "hle_irq_nesting_limit depth={depth}")
            }
            FaultReason::SampleFreqSetModeZero { mode } => {
                writeln!(writer, "sample_freq_set_mode_zero mode={mode}")
            }
            FaultReason::SoundChannelClearInternalCallback { target } => {
                writeln!(writer, "sound_channel_clear_internal_callback target={target:08x}")
            }
            FaultReason::TrackStopCgbCallback { target } => {
                writeln!(writer, "track_stop_cgb_callback target={target:08x}")
            }
            FaultReason::Mp2kPlayerChainLimit { player } => {
                writeln!(writer, "mp2k_player_chain_limit player={player:08x}")
            }
            FaultReason::Mp2kMissingRunningStatus { track, event } => {
                writeln!(writer, "mp2k_missing_running_status track={track:08x} event={event:02x}")
            }
            FaultReason::Mp2kInternalCallback { target } => {
                writeln!(writer, "mp2k_internal_callback target={target:08x}")
            }
            FaultReason::Mp2kReplacementCallback { command, target } => {
                writeln!(writer, "mp2k_replacement_callback command={command} target={target:08x}")
            }
            FaultReason::SoundDriverMainInternalCallback { target } => {
                writeln!(writer, "sound_driver_main_internal_callback target={target:08x}")
            }
            FaultReason::DmaLivelock { channel, started_at } => {
                writeln!(writer, "dma_livelock channel={channel} started_at={started_at}")
            }
        }
    }
}

fn tag_name(tag: impl std::fmt::Debug) -> String {
    let name = format!("{tag:?}");
    let chars: Vec<char> = name.chars().collect();
    let mut result = String::with_capacity(name.len() + 4);
    for (index, &ch) in chars.iter().enumerate() {
        if ch.is_uppercase()
            && index > 0
            && (chars[index - 1].is_lowercase()
                || chars[index - 1].is_ascii_digit()
                || (chars[index - 1].is_uppercase()
                    && chars.get(index + 1).is_some_and(|next| next.is_lowercase())))
        {
            result.push('_');
        }
        result.extend(ch.to_lowercase());
    }
    result
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

/// Advances two to four machines on the same deterministic cable timeline.
pub fn run_linked_until(
    machines: &mut [&mut Machine],
    deadline: Cycle,
    connection: bus::SioConnection,
) -> Result<LinkedRunResult, LinkedRunError> {
    if !(2..=4).contains(&machines.len()) {
        return Err(LinkedRunError::Link(bus::SioLinkError::UnsupportedPeerCount));
    }
    for index in 0..machines.len() {
        for other in index + 1..machines.len() {
            if std::ptr::eq(machines[index], machines[other]) {
                return Err(LinkedRunError::DuplicateMachine);
            }
        }
    }
    loop {
        for (index, machine) in machines.iter().enumerate() {
            if let Some(fault) = machine.fault() {
                return Ok(linked_run_result(
                    machines,
                    LinkedStopReason::Faulted(LinkedFault { machine_index: index, fault }),
                ));
            }
        }
        let (first, rest) = machines.split_first_mut().unwrap();
        let (second, rest) = rest.split_first_mut().unwrap();
        let link_result = match rest {
            [] => bus::synchronize_sio(&mut [&mut first.bus, &mut second.bus], connection),
            [third] => bus::synchronize_sio(
                &mut [&mut first.bus, &mut second.bus, &mut third.bus],
                connection,
            ),
            [third, fourth] => bus::synchronize_sio(
                &mut [&mut first.bus, &mut second.bus, &mut third.bus, &mut fourth.bus],
                connection,
            ),
            _ => unreachable!(),
        };
        match link_result {
            Ok(_) | Err(bus::SioLinkError::UnsupportedMode) => {}
            Err(error) => return Err(LinkedRunError::Link(error)),
        }
        if machines.iter().all(|machine| machine.now() >= deadline) {
            return Ok(linked_run_result(machines, LinkedStopReason::Deadline));
        }

        let runnable = machines
            .iter()
            .enumerate()
            .filter(|(_, machine)| {
                machine.now() < deadline
                    && (machine.cpu.inside_hle_bulk_service()
                        || match machine.bus.power_state() {
                            bus::PowerState::Running => true,
                            bus::PowerState::Halted => machine.bus.halt_wake_pending(),
                            bus::PowerState::Stopped => machine.bus.stop_wake_pending(),
                        })
            })
            .min_by_key(|(_, machine)| machine.now())
            .map(|(index, _)| index);
        if let Some(index) = runnable {
            if let StepOutcome::Faulted(fault) = machines[index].step() {
                return Ok(linked_run_result(
                    machines,
                    LinkedStopReason::Faulted(LinkedFault { machine_index: index, fault }),
                ));
            }
            continue;
        }
        let halted = machines
            .iter()
            .enumerate()
            .filter(|(_, machine)| {
                machine.now() < deadline && machine.bus.power_state() == bus::PowerState::Halted
            })
            .min_by_key(|(_, machine)| machine.now())
            .map(|(index, _)| index);
        if let Some(index) = halted {
            machines[index].bus.idle_halt_until(deadline);
            let _ = machines[index].latch_bus_fault();
            continue;
        }
        let mut stopped_progress = false;
        for machine in machines.iter_mut() {
            if machine.bus.now() >= deadline
                || machine.bus.power_state() != bus::PowerState::Stopped
            {
                continue;
            }
            let before = machine.now();
            let woke = machine.bus.idle_stop_until(deadline);
            if woke || machine.now() != before {
                stopped_progress = true;
            }
        }
        if stopped_progress {
            continue;
        }
        return Ok(linked_run_result(machines, LinkedStopReason::PowerStopped));
    }
}

fn linked_run_result(machines: &[&mut Machine], reason: LinkedStopReason) -> LinkedRunResult {
    LinkedRunResult {
        earliest_cycle: machines.iter().map(|machine| machine.now()).min().expect("two peers"),
        latest_cycle: machines.iter().map(|machine| machine.now()).max().expect("two peers"),
        reason,
    }
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
            rom: &[],
            bios: &[],
            immutable_media: false,
            boot: BootMode::Hle,
            save_type: SaveType::Auto,
            save_data: None,
            rtc_epoch_seconds: None,
            solar_level: None,
            gyro_sample: None,
            rumble: false,
            tilt_sample: None,
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
    pub const CYCLES_PER_FRAME: Cycle = 280_896;
    pub const STATE_MAGIC: &'static [u8; 8] = b"ZIGBAST1";
    pub const STATE_VERSION: u32 = 6;
    pub const MAX_STATE_PAYLOAD: usize = 1024 * 1024;
    pub const STATE_HEADER_SIZE: usize = 133;

    /// Allocates the large machine directly in its final heap location.
    pub fn new(config: Config<'_>) -> Box<Self> {
        assert!(
            matches!(config.boot, BootMode::Hle) == config.bios.is_empty(),
            "HLE boot does not accept BIOS; external boot requires BIOS"
        );
        let entry_address = if config.boot == BootMode::ExternalBios { 0 } else { 0x0800_0000 };
        let initial_cpsr = if config.boot == BootMode::ExternalBios { 0xD3 } else { 0x1F };
        let mut storage = Box::<Self>::new_uninit();
        let ptr = storage.as_mut_ptr();
        // SAFETY: Each field is written once before assume_init; Bus writes its
        // own large arrays directly into the allocated field, never on stack.
        unsafe {
            std::ptr::addr_of_mut!((*ptr).boot_mode).write(config.boot);
            Bus::init_in_place(
                std::ptr::addr_of_mut!((*ptr).bus),
                config.rom,
                config.bios,
                config.save_type,
                config.save_data,
            );
            std::ptr::addr_of_mut!((*ptr).cpu).write(Cpu::init(
                entry_address,
                initial_cpsr,
                config.boot == BootMode::Hle,
            ));
            std::ptr::addr_of_mut!((*ptr).latched_fault).write(None);
            std::ptr::addr_of_mut!((*ptr).media_identity)
                .write(config.immutable_media.then(|| MediaIdentity::new(config.rom, config.bios)));
            let mut machine = storage.assume_init();
            if let Some(epoch) = config.rtc_epoch_seconds {
                machine.bus.enable_rtc(epoch);
            }
            if let Some(level) = config.solar_level {
                machine.bus.enable_solar_sensor(level);
            }
            if let Some(sample) = config.gyro_sample {
                machine.bus.enable_gyro(sample);
            }
            if config.rumble {
                machine.bus.enable_rumble();
            }
            if let Some(sample) = config.tilt_sample {
                machine.bus.enable_tilt(sample);
            }
            machine
        }
    }

    pub fn now(&self) -> Cycle {
        self.bus.now()
    }
    pub fn fault(&self) -> Option<Fault> {
        self.latched_fault
    }

    fn latch_bus_fault(&mut self) -> Option<Fault> {
        if self.latched_fault.is_none() {
            let livelock = self.bus.dma_livelock()?;
            let cpu_fault = self.cpu.boundary_fault(
                &self.bus,
                FaultReason::DmaLivelock {
                    channel: livelock.channel,
                    started_at: livelock.started_at,
                },
            );
            self.latched_fault = Some(Fault {
                cycle: self.now(),
                instruction_address: cpu_fault.instruction_address,
                instruction: cpu_fault.instruction,
                instruction_set: cpu_fault.instruction_set,
                reason: cpu_fault.reason,
            });
        }
        self.latched_fault
    }

    pub fn step(&mut self) -> StepOutcome {
        if let Some(fault) = self.latched_fault {
            return StepOutcome::Faulted(fault);
        }
        let started_at = self.now();
        let outcome = self.cpu.step(&mut self.bus);
        if let Some(fault) = self.latch_bus_fault() {
            return StepOutcome::Faulted(fault);
        }
        match outcome {
            cpu::StepOutcome::Waiting(reason) => StepOutcome::Waiting(reason),
            cpu::StepOutcome::Executed(step) => StepOutcome::Executed(StepResult {
                started_at,
                stopped_at: self.now(),
                instruction_address: step.instruction_address,
                instruction: step.instruction,
                instruction_set: step.instruction_set,
            }),
            cpu::StepOutcome::Faulted(step) => {
                let fault = Fault {
                    cycle: self.now(),
                    instruction_address: step.instruction_address,
                    instruction: step.instruction,
                    instruction_set: step.instruction_set,
                    reason: step.reason,
                };
                self.latched_fault = Some(fault);
                StepOutcome::Faulted(fault)
            }
        }
    }

    pub fn step_with_input(
        &mut self,
        inputs: &[TimedInput],
    ) -> Result<StepWithInputResult, RunError> {
        if let Some(fault) = self.latched_fault {
            return Ok(StepWithInputResult {
                outcome: StepOutcome::Faulted(fault),
                inputs_consumed: 0,
            });
        }
        self.validate_inputs(inputs)?;
        // SAFETY: The caller's slice stays allocated and immutable until
        // end_external_inputs below; step is synchronous and retains nothing.
        unsafe { self.bus.begin_external_inputs(inputs) };
        let outcome = self.step();
        let inputs_consumed = self.bus.external_inputs_consumed();
        self.bus.end_external_inputs();
        Ok(StepWithInputResult { outcome, inputs_consumed })
    }

    fn validate_inputs(&self, inputs: &[TimedInput]) -> Result<(), RunError> {
        let mut previous_at = self.now();
        for input in inputs {
            if input.at < self.now() {
                return Err(RunError::InputBeforeCurrentCycle);
            }
            if input.at < previous_at {
                return Err(RunError::InputsOutOfOrder);
            }
            previous_at = input.at;
        }
        Ok(())
    }

    fn begin_inputs_until(&mut self, inputs: &[TimedInput], deadline: Cycle) {
        let count = inputs.partition_point(|input| input.at <= deadline);
        // SAFETY: Both bounded call sites synchronously finish the bus run
        // and call end_external_inputs before returning to the input owner.
        unsafe { self.bus.begin_external_inputs(&inputs[..count]) };
    }

    pub fn wait_until(&mut self, deadline: Cycle) -> Result<WaitResult, RunError> {
        self.wait_until_with_input(deadline, &[])
    }

    pub fn wait_until_with_input(
        &mut self,
        deadline: Cycle,
        inputs: &[TimedInput],
    ) -> Result<WaitResult, RunError> {
        if self.latched_fault.is_some() {
            return Err(RunError::MachineFaulted);
        }
        if deadline < self.now() {
            return Err(RunError::TargetBeforeCurrentCycle);
        }
        self.validate_inputs(inputs)?;
        if self.cpu.inside_hle_bulk_service() || self.bus.power_state() == bus::PowerState::Running
        {
            return Err(RunError::CpuNotWaiting);
        }
        self.begin_inputs_until(inputs, deadline);
        let started_at = self.now();
        let (reason, woke) = match self.bus.power_state() {
            bus::PowerState::Halted => {
                (cpu::WaitReason::Halted, self.bus.idle_halt_until(deadline))
            }
            bus::PowerState::Stopped => {
                (cpu::WaitReason::Stopped, self.bus.idle_stop_until(deadline))
            }
            bus::PowerState::Running => unreachable!(),
        };
        let _ = self.latch_bus_fault();
        let result = WaitResult {
            started_at,
            stopped_at: self.now(),
            reason,
            woke,
            inputs_consumed: self.bus.external_inputs_consumed(),
        };
        self.bus.end_external_inputs();
        Ok(result)
    }

    pub fn read_register(&self, index: u8) -> u32 {
        self.cpu.read_register(usize::from(index))
    }
    pub fn next_instruction_address(&self) -> u32 {
        self.cpu.next_instruction_address()
    }
    pub fn read_cpsr(&self) -> u32 {
        self.cpu.read_cpsr()
    }
    pub fn inspect_cpu(&self) -> CpuSnapshot {
        CpuSnapshot {
            cycle: self.now(),
            registers: self.cpu.registers,
            next_instruction_address: self.cpu.next_instruction_address(),
            cpsr: self.cpu.read_cpsr(),
        }
    }

    pub fn inspect_memory(&self, address: u32, output: &mut [u8]) -> Result<(), AddressOverflow> {
        if output.len() as u64 > u64::from(u32::MAX) - u64::from(address) + 1 {
            return Err(AddressOverflow);
        }
        for (offset, byte) in output.iter_mut().enumerate() {
            *byte = self.bus.inspect_byte(address + offset as u32);
        }
        Ok(())
    }

    pub fn set_keys(&mut self, keys: KeyState) {
        self.bus.set_keys(keys);
    }
    pub fn set_solar_level(&mut self, level: u8) {
        self.bus.set_solar_level(level);
    }
    pub fn set_gyro_sample(&mut self, sample: u16) {
        self.bus.set_gyro_sample(sample);
    }
    pub fn rumble_active(&self) -> bool {
        self.bus.rumble_active()
    }
    pub fn set_tilt(&mut self, sample: TiltSample) {
        self.bus.set_tilt(sample);
    }
    pub fn framebuffer(&self) -> &[u16] {
        self.bus.framebuffer()
    }
    pub fn completed_frames(&self) -> Cycle {
        self.bus.completed_frames()
    }
    pub fn drain_audio_samples(&mut self, output: &mut [evenfall_audio::StereoSample]) -> usize {
        self.bus.drain_audio_samples(output)
    }
    pub fn set_audio_output(&mut self, enabled: bool) {
        self.bus.set_audio_output(enabled);
    }
    pub fn set_high_speed_audio(&mut self, enabled: bool) {
        self.bus.set_high_speed_audio(enabled);
    }
    pub fn audio_sample_rate(&self) -> u32 {
        self.bus.audio_sample_rate()
    }
    pub fn save_data(&self) -> &[u8] {
        self.bus.save_data()
    }
    pub fn save_type(&self) -> SaveType {
        self.bus.save_type()
    }

    pub fn run_until(&mut self, deadline: Cycle) -> Result<RunResult, RunError> {
        self.run_until_with_input(deadline, &[])
    }

    pub fn run_until_with_input(
        &mut self,
        deadline: Cycle,
        inputs: &[TimedInput],
    ) -> Result<RunResult, RunError> {
        self.run_slice_with_input(deadline, deadline, inputs)
    }

    pub fn run_slice_with_input(
        &mut self,
        yield_at: Cycle,
        deadline: Cycle,
        inputs: &[TimedInput],
    ) -> Result<RunResult, RunError> {
        self.run_slice(Slice { yield_at, input_deadline: deadline, stop_at: None }, inputs)
    }

    pub fn run_slice(
        &mut self,
        slice: Slice,
        inputs: &[TimedInput],
    ) -> Result<RunResult, RunError> {
        let yield_at = slice.yield_at;
        let deadline = slice.input_deadline;
        let stop_at = slice.stop_at.unwrap_or(yield_at);
        if let Some(fault) = self.latched_fault {
            return Ok(RunResult {
                started_at: self.now(),
                stopped_at: self.now(),
                reason: StopReason::Faulted(fault),
                inputs_consumed: 0,
            });
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
            if self.bus.power_state() == bus::PowerState::Stopped
                && !self.bus.stop_wake_pending()
                && !self.cpu.inside_hle_bulk_service()
            {
                self.bus.idle_stop_until(stop_at);
                if self.bus.power_state() == bus::PowerState::Stopped
                    && !self.bus.stop_wake_pending()
                {
                    return self.run_result(started_at, StopReason::PowerStopped);
                }
            }
            if self.bus.power_state() == bus::PowerState::Halted
                && !self.bus.halt_wake_pending()
                && !self.cpu.inside_hle_bulk_service()
            {
                self.bus.idle_halt_until(chunk_end);
                if self.now() >= chunk_end || self.bus.dma_livelock().is_some() {
                    continue;
                }
            }
            let outcome = match self.cpu.run_normal_until(
                &mut self.bus,
                chunk_end,
                &mut execution_fault,
                &mut chunk_fault,
            ) {
                cpu::ChunkOutcome::Deadline => continue,
                cpu::ChunkOutcome::Generic => {
                    self.cpu.run_step(&mut self.bus, &mut execution_fault)
                }
                cpu::ChunkOutcome::Faulted => {
                    Some(cpu::StepOutcome::Faulted(chunk_fault.take().expect("chunk fault")))
                }
            };
            if let Some(cpu::StepOutcome::Faulted(step)) = outcome {
                let fault = Fault {
                    cycle: self.now(),
                    instruction_address: step.instruction_address,
                    instruction: step.instruction,
                    instruction_set: step.instruction_set,
                    reason: step.reason,
                };
                self.latched_fault = Some(fault);
                return self.run_result(started_at, StopReason::Faulted(fault));
            }
        }
        if let Some(fault) = self.latch_bus_fault() {
            return self.run_result(started_at, StopReason::Faulted(fault));
        }
        self.run_result(
            started_at,
            if self.now() == yield_at {
                StopReason::Deadline
            } else {
                StopReason::InstructionBoundaryAfterDeadline
            },
        )
    }

    fn run_result(&self, started_at: Cycle, reason: StopReason) -> RunResult {
        RunResult {
            started_at,
            stopped_at: self.now(),
            reason,
            inputs_consumed: self.bus.external_inputs_consumed(),
        }
    }

    /// Writes the portable v6 envelope, counting before the second payload
    /// traversal to keep header framing independent of the output stream.
    pub fn write_state(&self, writer: &mut impl Write) -> Result<(), StateError> {
        self.validate_state_write()?;
        let mut counter = Sink::new(None, None);
        self.write_state_payload(&mut counter)?;
        let mut header = Vec::with_capacity(Self::STATE_HEADER_SIZE);
        self.write_state_header(&mut header, counter.count)?;
        writer.write_all(&header)?;
        // The codec's Sink writes to Vec, so this is currently a bounded
        // payload buffer rather than an unbuffered stream to the writer.
        let mut payload = Vec::with_capacity(counter.count as usize);
        let mut hasher = blake3::Hasher::new();
        let mut sink = Sink::new(Some(&mut payload), Some(&mut hasher));
        self.write_state_payload(&mut sink)?;
        if sink.count != counter.count {
            return Err(StateError::Codec(evenfall_state::Error::InvalidSaveState));
        }
        writer.write_all(&payload)?;
        writer.write_all(hasher.finalize().as_bytes())?;
        Ok(())
    }

    /// Single payload traversal with the header filled after encoding.
    pub fn write_state_alloc(&self) -> Result<Vec<u8>, StateError> {
        self.validate_state_write()?;
        let mut output = vec![0; Self::STATE_HEADER_SIZE];
        let payload_len = {
            let mut sink = Sink::new(Some(&mut output), None);
            self.write_state_payload(&mut sink)?;
            sink.count
        };
        let digest = blake3::hash(&output[Self::STATE_HEADER_SIZE..]);
        output.extend_from_slice(digest.as_bytes());
        let mut header = Vec::with_capacity(Self::STATE_HEADER_SIZE);
        self.write_state_header(&mut header, payload_len)?;
        output[..Self::STATE_HEADER_SIZE].copy_from_slice(&header);
        Ok(output)
    }

    fn validate_state_write(&self) -> Result<(), StateError> {
        if self.latched_fault.is_some() {
            return Err(StateError::MachineFaulted);
        }
        if !self.bus.is_framebuffer_canonical() {
            return Err(StateError::FramebufferNotCanonical);
        }
        if self.cpu.hle_bios != (self.boot_mode == BootMode::Hle) {
            return Err(StateError::Codec(evenfall_state::Error::InvalidSaveState));
        }
        Ok(())
    }

    fn write_state_header(&self, header: &mut Vec<u8>, payload_len: u64) -> Result<(), StateError> {
        if payload_len > Self::MAX_STATE_PAYLOAD as u64 {
            return Err(StateError::Codec(evenfall_state::Error::InvalidSaveState));
        }
        let identity = self
            .media_identity
            .unwrap_or_else(|| MediaIdentity::new(&self.bus.rom, &self.bus.bios));
        let mut sink = Sink::new(Some(header), None);
        sink.write_all(Self::STATE_MAGIC)?;
        Self::STATE_VERSION.encode(&mut sink)?;
        sink.write_all(&Self::schema_digest()?)?;
        payload_len.encode(&mut sink)?;
        sink.write_all(&[self.boot_mode as u8])?;
        (self.bus.rom.len() as u64).encode(&mut sink)?;
        sink.write_all(&identity.rom)?;
        (self.bus.bios.len() as u64).encode(&mut sink)?;
        sink.write_all(&identity.bios)?;
        debug_assert_eq!(sink.count as usize, Self::STATE_HEADER_SIZE);
        Ok(())
    }

    fn write_state_payload(&self, sink: &mut Sink<'_>) -> Result<(), StateError> {
        sink.field("cycle", |sink| self.now().encode(sink))?;
        sink.field("cpu", |sink| self.cpu.write_state(sink, self.now()))?;
        sink.field("bus", |sink| self.bus.write_state(sink))?;
        Ok(())
    }

    /// Durable payload fields with byte ranges relative to the first payload
    /// byte (after the 133-byte envelope header), from the real codec writer.
    pub fn state_field_ranges(&self) -> Result<Vec<(String, Range<u64>)>, StateError> {
        self.validate_state_write()?;
        let mut ranges = Vec::new();
        let mut sink = Sink::new(None, None);
        sink.trace = Some(&mut ranges);
        self.write_state_payload(&mut sink)?;
        Ok(ranges)
    }

    pub fn schema_description() -> Result<Vec<u8>, StateError> {
        let mut description = Vec::new();
        let mut sink = Sink::new(Some(&mut description), None);
        evenfall_state::schema_name(&mut sink, "zigba.state-codec.le-minimal.v1")?;
        evenfall_state::schema_name(&mut sink, "zigba.machine.payload.v6")?;
        <Cycle as Codec>::schema(&mut sink)?;
        Cpu::write_state_schema(&mut sink)?;
        Bus::write_state_schema(&mut sink)?;
        Ok(description)
    }

    pub fn schema_digest() -> Result<[u8; 32], StateError> {
        Ok(*blake3::hash(&Self::schema_description()?).as_bytes())
    }

    /// Verifies into heap-allocated scratch and publishes only on success.
    pub fn load_state(&mut self, bytes: &[u8]) -> Result<(), StateError> {
        let mut input = Source::new(bytes);
        let mut magic = [0; 8];
        input.read_all(&mut magic)?;
        if magic != *Self::STATE_MAGIC {
            return Err(evenfall_state::Error::InvalidSaveState.into());
        }
        let mut version = 0u32;
        version.decode(&mut input)?;
        if version != Self::STATE_VERSION {
            return Err(evenfall_state::Error::UnsupportedSaveStateVersion.into());
        }
        let mut schema = [0; 32];
        input.read_all(&mut schema)?;
        if schema != Self::schema_digest()? {
            return Err(evenfall_state::Error::SaveStateSchemaMismatch.into());
        }
        let mut payload_len = 0u64;
        payload_len.decode(&mut input)?;
        if payload_len > Self::MAX_STATE_PAYLOAD as u64 {
            return Err(evenfall_state::Error::InvalidSaveState.into());
        }
        let mut boot = [0];
        input.read_all(&mut boot)?;
        if boot[0] != self.boot_mode as u8 {
            return Err(evenfall_state::Error::SaveStateBootModeMismatch.into());
        }
        let mut rom_len = 0u64;
        let mut rom_digest = [0; 32];
        rom_len.decode(&mut input)?;
        input.read_all(&mut rom_digest)?;
        let identity = self
            .media_identity
            .unwrap_or_else(|| MediaIdentity::new(&self.bus.rom, &self.bus.bios));
        if rom_len != self.bus.rom.len() as u64 || rom_digest != identity.rom {
            return Err(evenfall_state::Error::SaveStateRomMismatch.into());
        }
        let mut bios_len = 0u64;
        let mut bios_digest = [0; 32];
        bios_len.decode(&mut input)?;
        input.read_all(&mut bios_digest)?;
        if bios_len != self.bus.bios.len() as u64 || bios_digest != identity.bios {
            return Err(evenfall_state::Error::SaveStateBiosMismatch.into());
        }
        let expected_len = payload_len as usize + 32;
        if input.input.len() != expected_len {
            return Err(evenfall_state::Error::InvalidSaveState.into());
        }
        let (payload, digest) = input.input.split_at(payload_len as usize);
        let mut scratch = Self::new(Config {
            rom: &self.bus.rom,
            bios: &self.bus.bios,
            boot: self.boot_mode,
            ..Config::default()
        });
        scratch.media_identity = self.media_identity;
        let mut payload_source = Source::new(payload);
        let mut cycle = 0;
        cycle.decode(&mut payload_source)?;
        scratch.cpu.read_state(&mut payload_source, cycle)?;
        scratch.bus.current_cycle = cycle;
        scratch.bus.read_state(&mut payload_source)?;
        if !payload_source.input.is_empty()
            || blake3::hash(payload).as_bytes() != digest
            || scratch.cpu.hle_bios != (self.boot_mode == BootMode::Hle)
        {
            return Err(evenfall_state::Error::InvalidSaveState.into());
        }
        std::mem::swap(self, &mut scratch);
        Ok(())
    }
}

#[derive(Debug)]
pub enum StateError {
    Codec(evenfall_state::Error),
    Io(std::io::Error),
    MachineFaulted,
    FramebufferNotCanonical,
}

impl From<evenfall_state::Error> for StateError {
    fn from(error: evenfall_state::Error) -> Self {
        Self::Codec(error)
    }
}
impl From<std::io::Error> for StateError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl evenfall_host::System for Machine {
    type Input = TimedInput;
    type Fault = Fault;
    type StateError = StateError;
    type Pixel = u16;

    const MASTER_CLOCK_HZ: u64 = 16_777_216;
    const CYCLES_PER_FRAME: Cycle = 280_896;
    const VIDEO: evenfall_host::VideoFormat = evenfall_host::VideoFormat {
        width: 240,
        height: 160,
        pixel: evenfall_host::PixelFormat::Bgr555,
    };

    fn now(&self) -> Cycle {
        Machine::now(self)
    }

    fn run(
        &mut self,
        yield_at: Cycle,
        input_deadline: Cycle,
        inputs: &[Self::Input],
    ) -> Result<evenfall_host::RunResult<Self::Fault>, evenfall_host::RunError> {
        let result = self.run_slice_with_input(yield_at, input_deadline, inputs).map_err(
            |error| match error {
                RunError::TargetBeforeCurrentCycle => {
                    evenfall_host::RunError::TargetBeforeCurrentCycle
                }
                RunError::InputBeforeCurrentCycle => {
                    evenfall_host::RunError::InputBeforeCurrentCycle
                }
                RunError::InputsOutOfOrder => evenfall_host::RunError::InputsOutOfOrder,
                RunError::CpuNotWaiting | RunError::MachineFaulted => {
                    unreachable!("run_slice never returns wait errors")
                }
            },
        )?;
        let reason = match result.reason {
            StopReason::Deadline => evenfall_host::Stop::Deadline,
            StopReason::InstructionBoundaryAfterDeadline => {
                evenfall_host::Stop::InstructionBoundaryAfterDeadline
            }
            StopReason::PowerStopped => evenfall_host::Stop::PowerStopped,
            StopReason::Faulted(fault) => evenfall_host::Stop::Faulted(fault),
        };
        Ok(evenfall_host::RunResult {
            started_at: result.started_at,
            stopped_at: result.stopped_at,
            reason,
            inputs_consumed: result.inputs_consumed,
        })
    }

    fn fault(&self) -> Option<&Self::Fault> {
        self.latched_fault.as_ref()
    }
    fn completed_frames(&self) -> u64 {
        Machine::completed_frames(self)
    }
    fn framebuffer(&self) -> &[Self::Pixel] {
        Machine::framebuffer(self)
    }
    fn drain_audio(&mut self, output: &mut [evenfall_host::StereoSample]) -> usize {
        self.drain_audio_samples(output)
    }
    fn audio_sample_rate(&self) -> u32 {
        Machine::audio_sample_rate(self)
    }
    fn save_media(&self) -> &[u8] {
        Machine::save_data(self)
    }
    fn write_state(&self) -> Result<Vec<u8>, Self::StateError> {
        self.write_state_alloc()
    }
    fn load_state(&mut self, bytes: &[u8]) -> Result<(), Self::StateError> {
        Machine::load_state(self, bytes)
    }
    fn inspect(
        &self,
        address: u32,
        output: &mut [u8],
    ) -> Result<(), evenfall_host::AddressOverflow> {
        self.inspect_memory(address, output).map_err(|_| evenfall_host::AddressOverflow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ZiGBA's stable diagnostic line includes width, instruction set, cycle,
    /// and the reason payload; Debug's CamelCase names would be incompatible.
    #[test]
    fn fault_writes_stable_diagnostic_line() {
        let fault = Fault {
            cycle: 73,
            instruction_address: 0x0800_0014,
            instruction: 0xEF00_007F,
            instruction_set: cpu::InstructionSet::Arm,
            reason: FaultReason::InvalidHleMemoryAccess {
                service: 0x0b,
                operation: cpu::HleMemoryOperation::Write,
                address: 0x0400_0122,
                width: bus::AccessWidth::Half,
            },
        };
        let mut output = Vec::new();
        fault.write_diagnostic(&mut output).unwrap();
        assert_eq!(output,
            b"emulation fault cycle=73 pc=08000014 arm=ef00007f reason=invalid_hle_memory_access service=0b operation=write address=04000122 width=half\n");
    }

    /// ZiGBA's version-six schema description and envelope digest are
    /// independent fixtures generated by its own schema writer.
    #[test]
    fn schema_matches_zigba_version_six() {
        let expected = include_bytes!("../../../tools/parity/fixtures/schema-v6.bin");
        let actual = Machine::schema_description().unwrap();
        if actual != expected {
            let first = actual.iter().zip(expected.iter()).position(|(a, b)| a != b);
            let offset = first.unwrap_or(actual.len().min(expected.len()));
            panic!(
                "schema description differs at offset {offset} (Rust length {}, Zig length {}): Rust {:?}, Zig {:?}",
                actual.len(),
                expected.len(),
                actual.get(offset),
                expected.get(offset),
            );
        }
        assert_eq!(
            blake3::hash(&actual).to_hex().as_str(),
            include_str!("../../../tools/parity/fixtures/schema-v6.blake3").trim()
        );
        assert_eq!(Machine::schema_digest().unwrap(), *blake3::hash(expected).as_bytes());
    }

    #[test]
    fn state_field_ranges_are_payload_relative_and_cover_encoded_bytes() {
        let machine = Machine::new(Config { rom: &[0xfe, 0xff, 0xff, 0xea], ..Config::default() });
        let ranges = machine.state_field_ranges().unwrap();
        let range = |name: &str| ranges.iter().find(|(path, _)| path == name).unwrap().1.clone();
        let payload = machine.write_state_alloc().unwrap();
        assert_eq!(range("cycle"), 0..8);
        assert_eq!(range("cpu").start, 8);
        assert_eq!(range("cpu").end, range("bus").start);
        assert_eq!(range("bus").end as usize, payload.len() - Machine::STATE_HEADER_SIZE - 32);
        assert!(ranges.iter().any(|(path, _)| path.starts_with("cpu.")));
        assert!(ranges.iter().any(|(path, _)| path.starts_with("bus.")));
    }
}
