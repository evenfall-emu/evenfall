//! The host-facing interface every Evenfall system implements.
//!
//! A system is a deterministic machine with no host I/O. Hosts (the CLI, the
//! WebAssembly adapter, the parity harness, benchmarks and tests) feed it
//! bytes and timestamped input and read back pixels, samples, save media and
//! state. The same media, configuration and timestamped inputs produce the
//! same observations however a host slices execution into calls.
//!
//! The trait is deliberately small. System-specific controls (cartridge
//! sensors, link cables, BIOS services) stay on the system's own type; this
//! crate covers only what generic hosts need.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;

pub use evenfall_audio::StereoSample;

/// A count of a system's master clock. Time only moves forward.
pub type Cycle = u64;

/// A host input change applied at an absolute master cycle.
///
/// Inputs due at the current cycle apply before any guest work, and inputs
/// with the same cycle keep the caller's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Timed<I> {
    pub at: Cycle,
    pub input: I,
}

impl<I> Timed<I> {
    pub const fn new(at: Cycle, input: I) -> Self {
        Self { at, input }
    }
}

/// Why a bounded run returned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stop<F> {
    /// The run ended exactly at its yield point.
    Deadline,
    /// An atomic instruction carried the run past its yield point; the
    /// overshoot is `stopped_at - yield_at`.
    InstructionBoundaryAfterDeadline,
    /// The guest entered a power state that only an external event can end.
    PowerStopped,
    /// The guest reached behavior the system refuses to guess at. Faults are
    /// sticky: later runs return the same fault without advancing time.
    Faulted(F),
}

/// The outcome of one bounded run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunResult<F> {
    pub started_at: Cycle,
    pub stopped_at: Cycle,
    pub reason: Stop<F>,
    /// How many of the caller's timed inputs the run applied.
    pub inputs_consumed: usize,
}

/// A run request that cannot be honored. The machine is left unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunError {
    TargetBeforeCurrentCycle,
    InputBeforeCurrentCycle,
    InputsOutOfOrder,
}

/// A memory inspection that would wrap the guest address space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AddressOverflow;

/// The shape of a system's completed video frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoFormat {
    pub width: u32,
    pub height: u32,
    pub pixel: PixelFormat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    /// 15-bit color in a `u16`: red in bits 0–4, green 5–9, blue 10–14.
    Bgr555,
}

/// A deterministic emulated system.
///
/// Every method is a function of emulated time: none reads a host clock, and
/// observation (`now`, `framebuffer`, `save_media`, `inspect`) costs zero
/// emulated cycles and has no side effects.
pub trait System {
    /// One timestamped host input (keys, sensors, and so on). The system's
    /// native event type includes its absolute master cycle, so a host can
    /// pass its input slice through without allocating a converted stream.
    type Input: Copy;
    /// A typed diagnostic for guest behavior the system will not guess at.
    type Fault: Clone + core::fmt::Debug;
    /// Why a state could not be written or loaded.
    type StateError: core::fmt::Debug;
    /// The pixel storage of [`System::framebuffer`].
    type Pixel: Copy;

    /// Master-clock frequency in Hz.
    const MASTER_CLOCK_HZ: u64;
    /// Master cycles in one nominal video frame.
    const CYCLES_PER_FRAME: Cycle;
    const VIDEO: VideoFormat;

    /// The current master cycle.
    fn now(&self) -> Cycle;

    /// Runs until `yield_at`, applying `inputs` whose cycle is at or before
    /// `input_deadline` at their exact cycles.
    ///
    /// Instructions are atomic, so a run may end at the first instruction
    /// boundary past `yield_at`. Separating the input deadline from the yield
    /// point lets a host split one logical request into several calls without
    /// changing which inputs apply where.
    fn run(
        &mut self,
        yield_at: Cycle,
        input_deadline: Cycle,
        inputs: &[Self::Input],
    ) -> Result<RunResult<Self::Fault>, RunError>;

    /// The sticky fault, if the guest reached one.
    fn fault(&self) -> Option<&Self::Fault>;

    /// Video frames completed since power-on.
    fn completed_frames(&self) -> u64;

    /// The most recently completed frame, row-major.
    fn framebuffer(&self) -> &[Self::Pixel];

    /// Moves produced audio samples into `output`; returns how many it wrote.
    fn drain_audio(&mut self, output: &mut [StereoSample]) -> usize;

    /// The rate, in Hz, of samples returned by [`System::drain_audio`].
    fn audio_sample_rate(&self) -> u32;

    /// The cartridge's or console's battery-backed save memory.
    fn save_media(&self) -> &[u8];

    /// Serializes a portable save state.
    fn write_state(&self) -> Result<Vec<u8>, Self::StateError>;

    /// Replaces the machine with a saved one. On error the machine is
    /// unchanged.
    fn load_state(&mut self, bytes: &[u8]) -> Result<(), Self::StateError>;

    /// Copies guest memory at `address` into `output` without side effects.
    fn inspect(&self, address: u32, output: &mut [u8]) -> Result<(), AddressOverflow>;
}
