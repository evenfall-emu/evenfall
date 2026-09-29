//! Session-local reconstruction of video omitted by selective rendering.

use crate::bus::{KeyState, TimedInput};
use crate::machine::{Config, Machine, RunError, RunResult, Slice, StepOutcome};

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
    Run(RunError),
}

impl From<RunError> for Error {
    fn from(error: RunError) -> Self {
        Self::Run(error)
    }
}

pub struct VideoRecovery {
    /// A copy of the live machine at the anchor. Reconstruction replays the
    /// journal on it, after which it equals the live machine and serves as
    /// the next anchor.
    replay: Box<Machine>,
    /// False after a reconstruction failed part-way, which leaves `replay`
    /// neither at the anchor nor at the live machine.
    anchored: bool,
    anchor_cycle: u64,
    anchor_frame_id: u64,
    operations: Vec<Operation>,
    inputs: Vec<TimedInput>,
}

impl VideoRecovery {
    pub fn create(machine: &Machine) -> Self {
        let replay = Machine::new_with_shared_media(
            machine,
            Config { boot: machine.boot_mode, ..Config::default() },
        );
        let mut recovery = Self {
            replay,
            anchored: false,
            anchor_cycle: 0,
            anchor_frame_id: 0,
            operations: Vec::with_capacity(MAX_OPERATIONS),
            inputs: Vec::with_capacity(MAX_INPUTS),
        };
        recovery.rebase(machine);
        recovery
    }

    pub fn rebase(&mut self, machine: &Machine) {
        debug_assert!(machine.bus.is_framebuffer_canonical());
        debug_assert!(machine.fault().is_none());
        self.replay.copy_from(machine);
        self.anchored = true;
        self.start_journal(machine);
    }

    fn start_journal(&mut self, machine: &Machine) {
        self.anchor_cycle = machine.now();
        self.anchor_frame_id = machine.bus.video_frame_id;
        self.operations.clear();
        self.inputs.clear();
    }

    fn reserve(&mut self, machine: &mut Machine, input_count: usize) -> Result<(), Error> {
        if input_count > MAX_INPUTS {
            return Err(Error::TooManyRecoveryInputs);
        }
        if self.operations.len() == MAX_OPERATIONS
            || self.inputs.len() + input_count > MAX_INPUTS
            || machine.now() - self.anchor_cycle > 32 * Machine::CYCLES_PER_FRAME
        {
            let reconstructed = !machine.bus.is_framebuffer_canonical();
            self.materialize(machine)?;
            if machine.fault().is_some() {
                return Err(Error::MachineFaulted);
            }
            if !reconstructed {
                self.rebase(machine);
            }
        }
        Ok(())
    }

    fn finish(&mut self, machine: &Machine, call: Call) {
        self.operations.push(Operation { call, after_cycle: machine.now() });
        if machine.bus.is_framebuffer_canonical()
            && machine.fault().is_none()
            && machine.bus.video_frame_id != self.anchor_frame_id
        {
            self.rebase(machine);
        }
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
        let result = machine.run_slice(slice, inputs)?;
        self.inputs.extend_from_slice(inputs);
        self.finish(machine, Call::Run { slice, input_start, input_count: inputs.len() });
        Ok(result)
    }

    pub fn set_keys(&mut self, machine: &mut Machine, keys: KeyState) -> Result<(), Error> {
        if machine.fault().is_some() {
            machine.set_keys(keys);
            return Ok(());
        }
        self.reserve(machine, 0)?;
        machine.set_keys(keys);
        self.finish(machine, Call::Keys(keys));
        Ok(())
    }

    pub fn step(&mut self, machine: &mut Machine) -> Result<StepOutcome, Error> {
        if machine.fault().is_some() {
            return Ok(machine.step());
        }
        self.reserve(machine, 0)?;
        let result = machine.step();
        self.finish(machine, Call::Step);
        Ok(result)
    }

    /// Only pixels are published; the live CPU, cycle and audio remain
    /// intact. The replayed machine then equals the live one and becomes the
    /// anchor of an empty journal.
    pub fn materialize(&mut self, machine: &mut Machine) -> Result<(), Error> {
        if machine.bus.is_framebuffer_canonical() {
            return Ok(());
        }
        if !self.anchored {
            return Err(Error::VideoRecoveryDiverged);
        }
        self.anchored = false;
        self.replay.bus.use_full_video();
        self.replay.set_audio_output(false);
        for operation in &self.operations {
            match operation.call {
                Call::Run { slice, input_start, input_count } => {
                    self.replay
                        .run_slice(slice, &self.inputs[input_start..input_start + input_count])?;
                }
                Call::Keys(keys) => self.replay.set_keys(keys),
                Call::Step => {
                    self.replay.step();
                }
            }
            if self.replay.now() != operation.after_cycle {
                return Err(Error::VideoRecoveryDiverged);
            }
        }
        if self.replay.now() != machine.now()
            || self.replay.cpu != machine.cpu
            || self.replay.fault() != machine.fault()
        {
            return Err(Error::VideoRecoveryDiverged);
        }
        machine.bus.accept_canonical_framebuffer(self.replay.framebuffer());
        self.anchored = true;
        self.start_journal(machine);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::TimedInputChange;

    #[test]
    fn rejected_run_does_not_append_inputs_to_replay_history() {
        let mut machine =
            Machine::new(Config { rom: &[0xfe, 0xff, 0xff, 0xea], ..Config::default() });
        let mut recovery = VideoRecovery::create(&machine);
        let inputs = [TimedInput { at: 1, change: TimedInputChange::Keys(KeyState::default()) }];
        assert!(matches!(
            recovery.run(
                &mut machine,
                Slice { yield_at: 0, input_deadline: 0, stop_at: Some(1) },
                &inputs,
            ),
            Err(Error::Run(RunError::TargetBeforeCurrentCycle))
        ));
        assert!(recovery.inputs.is_empty());
        assert!(recovery.operations.is_empty());
    }
}
