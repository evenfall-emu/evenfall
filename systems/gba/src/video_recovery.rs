//! Session-local reconstruction of video omitted by selective rendering.

use crate::bus::{KeyState, TimedInput};
use crate::machine::{Config, Machine, RunError, RunResult, Slice, StateError, StepOutcome};

const MAX_OPERATIONS: usize = 4096;
const MAX_INPUTS: usize = 4096;

#[derive(Clone, Debug)]
enum Call {
    Run { slice: Slice, input_start: usize, input_count: usize },
    Keys(KeyState),
    Step,
}

#[derive(Clone, Debug)]
struct Operation {
    call: Call,
    after_cycle: u64,
}

#[derive(Debug)]
pub enum Error {
    TooManyRecoveryInputs,
    MachineFaulted,
    VideoRecoveryDiverged,
    State(StateError),
    Run(RunError),
}

impl From<StateError> for Error {
    fn from(error: StateError) -> Self {
        Self::State(error)
    }
}
impl From<RunError> for Error {
    fn from(error: RunError) -> Self {
        Self::Run(error)
    }
}

pub struct VideoRecovery {
    anchor: Vec<u8>,
    anchor_cycle: u64,
    anchor_frame_id: u64,
    scratch: Box<Machine>,
    operations: Vec<Operation>,
    inputs: Vec<TimedInput>,
}

impl VideoRecovery {
    pub fn create(machine: &Machine) -> Result<Self, Error> {
        let scratch = Machine::new(Config {
            rom: &machine.bus.rom,
            bios: &machine.bus.bios,
            boot: machine.boot_mode,
            ..Config::default()
        });
        let mut recovery = Self {
            anchor: Vec::new(),
            anchor_cycle: 0,
            anchor_frame_id: 0,
            scratch,
            operations: Vec::with_capacity(MAX_OPERATIONS),
            inputs: Vec::with_capacity(MAX_INPUTS),
        };
        recovery.rebase(machine)?;
        Ok(recovery)
    }

    pub fn rebase(&mut self, machine: &Machine) -> Result<(), Error> {
        debug_assert!(machine.bus.is_framebuffer_canonical());
        debug_assert!(machine.fault().is_none());
        self.anchor = machine.write_state_alloc()?;
        self.anchor_cycle = machine.now();
        self.anchor_frame_id = machine.bus.video_frame_id;
        self.operations.clear();
        self.inputs.clear();
        Ok(())
    }

    fn reserve(&mut self, machine: &mut Machine, input_count: usize) -> Result<(), Error> {
        if input_count > MAX_INPUTS {
            return Err(Error::TooManyRecoveryInputs);
        }
        if self.operations.len() == MAX_OPERATIONS
            || self.inputs.len() + input_count > MAX_INPUTS
            || machine.now() - self.anchor_cycle > 32 * Machine::CYCLES_PER_FRAME
        {
            self.materialize(machine)?;
            if machine.fault().is_some() {
                return Err(Error::MachineFaulted);
            }
            self.rebase(machine)?;
        }
        Ok(())
    }

    fn finish(&mut self, machine: &Machine, call: Call) -> Result<(), Error> {
        self.operations.push(Operation { call, after_cycle: machine.now() });
        if machine.bus.is_framebuffer_canonical()
            && machine.fault().is_none()
            && machine.bus.video_frame_id != self.anchor_frame_id
        {
            self.rebase(machine)?;
        }
        Ok(())
    }

    pub fn run(
        &mut self,
        machine: &mut Machine,
        slice: Slice,
        inputs: &[TimedInput],
    ) -> Result<RunResult, Error> {
        if machine.fault().is_some() {
            return Ok(machine.run_slice(slice, inputs)?);
        }
        self.reserve(machine, inputs.len())?;
        let input_start = self.inputs.len();
        self.inputs.extend_from_slice(inputs);
        let result = machine.run_slice(slice, inputs)?;
        self.finish(machine, Call::Run { slice, input_start, input_count: inputs.len() })?;
        Ok(result)
    }

    pub fn set_keys(&mut self, machine: &mut Machine, keys: KeyState) -> Result<(), Error> {
        if machine.fault().is_some() {
            machine.set_keys(keys);
            return Ok(());
        }
        self.reserve(machine, 0)?;
        machine.set_keys(keys);
        self.finish(machine, Call::Keys(keys))
    }

    pub fn step(&mut self, machine: &mut Machine) -> Result<StepOutcome, Error> {
        if machine.fault().is_some() {
            return Ok(machine.step());
        }
        self.reserve(machine, 0)?;
        let result = machine.step();
        self.finish(machine, Call::Step)?;
        Ok(result)
    }

    /// Only pixels are published; the live CPU, cycle and audio remain intact.
    pub fn materialize(&mut self, machine: &mut Machine) -> Result<(), Error> {
        if machine.bus.is_framebuffer_canonical() {
            return Ok(());
        }
        self.scratch.load_state(&self.anchor)?;
        self.scratch.bus.use_full_video();
        self.scratch.set_audio_output(false);
        for operation in &self.operations {
            match operation.call {
                Call::Run { slice, input_start, input_count } => {
                    self.scratch
                        .run_slice(slice, &self.inputs[input_start..input_start + input_count])?;
                }
                Call::Keys(keys) => self.scratch.set_keys(keys),
                Call::Step => {
                    self.scratch.step();
                }
            }
            if self.scratch.now() != operation.after_cycle {
                return Err(Error::VideoRecoveryDiverged);
            }
        }
        if self.scratch.now() != machine.now()
            || self.scratch.cpu != machine.cpu
            || self.scratch.fault() != machine.fault()
        {
            return Err(Error::VideoRecoveryDiverged);
        }
        machine.bus.accept_canonical_framebuffer(self.scratch.framebuffer());
        Ok(())
    }
}
