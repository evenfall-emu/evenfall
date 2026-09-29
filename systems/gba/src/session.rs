//! Owned, host-independent session translated from `session.zig`.

use crate::bus::{KeyState, TimedInput};
use crate::cartridge::{self, save::Type as SaveType};
use crate::machine::{
    BootMode, Config as MachineConfig, Machine, RunError, RunResult, StateError, StepOutcome,
    StopReason,
};
use crate::rom_image;
use crate::video_recovery::{self, VideoRecovery};
use evenfall_audio::{Resampler, StereoSample};
use evenfall_state::rewind::{self, RewindBuffer};
use sha1::{Digest, Sha1};

pub use rewind::{Compression as RewindCompression, Info as RewindInfo, RewindConfig};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RewindPosition {
    pub frame: u64,
    pub cycle: u64,
}

pub struct Session {
    pub machine: Box<Machine>,
    pub rom: Vec<u8>,
    pub bios: Vec<u8>,
    /// Game identity, SHA-1 of the raw ROM.
    pub rom_sha1: [u8; 20],
    pub rtc_enabled: bool,
    pub resampler: Resampler,
    save_type_override: Option<SaveType>,
    rtc_epoch: u64,
    pub audio_output_enabled: bool,
    pub video_interval: u16,
    video_recovery: Option<VideoRecovery>,
    rewind_history: RewindBuffer,
}

#[derive(Clone, Copy, Debug)]
pub struct Config<'a> {
    pub image: &'a [u8],
    pub zipped: bool,
    pub bios: &'a [u8],
    pub save: Option<&'a [u8]>,
    pub save_type: Option<SaveType>,
    pub rtc_enabled: Option<bool>,
    pub rtc_epoch: u64,
    pub output_rate: u32,
    pub rewind: RewindConfig,
}

impl Default for Config<'_> {
    fn default() -> Self {
        Self {
            image: &[],
            zipped: false,
            bios: &[],
            save: None,
            save_type: None,
            rtc_enabled: None,
            rtc_epoch: 0,
            output_rate: 48_000,
            rewind: RewindConfig::default(),
        }
    }
}

#[derive(Debug)]
pub enum Error {
    InvalidRomSize,
    InvalidBiosSize,
    InvalidSampleRate,
    InvalidRtcEpoch,
    ExecutionBudgetExceeded,
    TargetBeforeCurrentCycle,
    AudioBufferTooSmall,
    StateTooLarge,
    InvalidVideoInterval,
    MachineFaulted,
    InvalidRewindSteps,
    RewindEmpty,
    InvalidFramebufferSize,
    Recovery(video_recovery::Error),
    Image(rom_image::Error),
    Rewind(evenfall_state::Error),
    State(StateError),
    Run(RunError),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}
impl From<rom_image::Error> for Error {
    fn from(error: rom_image::Error) -> Self {
        Self::Image(error)
    }
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
impl From<evenfall_state::Error> for Error {
    fn from(error: evenfall_state::Error) -> Self {
        Self::Rewind(error)
    }
}
impl From<video_recovery::Error> for Error {
    fn from(error: video_recovery::Error) -> Self {
        Self::Recovery(error)
    }
}

impl Session {
    pub fn create(config: Config<'_>) -> Result<Self, Error> {
        Self::create_impl(config, None)
    }

    /// Adopts the raw uploaded ROM on success; a ZIP is extracted then dropped.
    pub fn create_from_owned_buffer(config: Config<'_>, buffer: Vec<u8>) -> Result<Self, Error> {
        assert_eq!(config.image.as_ptr(), buffer.as_ptr());
        assert!(config.image.len() <= buffer.len());
        Self::create_impl(config, Some(buffer))
    }

    fn create_impl(config: Config<'_>, owned: Option<Vec<u8>>) -> Result<Self, Error> {
        if config.image.is_empty() || config.image.len() > 40 * 1024 * 1024 {
            return Err(Error::InvalidRomSize);
        }
        if !config.bios.is_empty() && config.bios.len() != 16_384 {
            return Err(Error::InvalidBiosSize);
        }
        if !(8_000..=192_000).contains(&config.output_rate) {
            return Err(Error::InvalidSampleRate);
        }
        if config.rtc_epoch > 253_402_300_799 {
            return Err(Error::InvalidRtcEpoch);
        }
        let image_len = config.image.len();
        let rom = if config.zipped {
            rom_image::extract_single_gba(config.image)?
        } else if let Some(mut owned) = owned {
            owned.truncate(image_len);
            owned
        } else {
            config.image.to_vec()
        };
        if rom.is_empty() || rom.len() > rom_image::MAX_ROM_SIZE {
            return Err(Error::InvalidRomSize);
        }
        let bios = config.bios.to_vec();
        let save_type = rom_image::choose_save_type(&rom, config.save_type, config.save)?;
        let save_data =
            config.save.map(|save| rom_image::save_prefix(save_type, save)).transpose()?;
        let features = cartridge::detect_features(&rom);
        let rtc_enabled = config.rtc_enabled.unwrap_or(features.rtc);
        let machine = Machine::new(MachineConfig {
            rom: &rom,
            bios: &bios,
            immutable_media: true,
            boot: if bios.is_empty() { BootMode::Hle } else { BootMode::ExternalBios },
            save_type,
            save_data,
            rtc_epoch_seconds: rtc_enabled.then_some(config.rtc_epoch),
            solar_level: features.solar_sensor.then_some(128),
            gyro_sample: features.gyro.then_some(0x6c0),
            rumble: features.rumble,
            tilt_sample: features.tilt.then_some(Default::default()),
        });
        let rom_sha1 = Sha1::digest(&rom).into();
        let resampler = Resampler::new(machine.audio_sample_rate(), config.output_rate)
            .map_err(|_| Error::InvalidSampleRate)?;
        let rewind_history = RewindBuffer::new(config.rewind)?;
        Ok(Self {
            machine,
            rom,
            bios,
            rom_sha1,
            rtc_enabled,
            resampler,
            save_type_override: config.save_type,
            rtc_epoch: config.rtc_epoch,
            audio_output_enabled: true,
            video_interval: 1,
            video_recovery: None,
            rewind_history,
        })
    }

    /// One call cannot occupy the host beyond one emulated frame.
    pub fn advance(&mut self, deadline: u64) -> Result<RunResult, Error> {
        self.run_slice_with_input(deadline, deadline, &[])
    }

    pub fn run_slice_with_input(
        &mut self,
        deadline: u64,
        input_deadline: u64,
        inputs: &[TimedInput],
    ) -> Result<RunResult, Error> {
        if deadline > self.machine.now()
            && deadline - self.machine.now() > Machine::CYCLES_PER_FRAME
        {
            return Err(Error::ExecutionBudgetExceeded);
        }
        if self.machine.fault().is_none()
            && (deadline < self.machine.now() || input_deadline < deadline)
        {
            return Err(Error::TargetBeforeCurrentCycle);
        }
        let Some(recovery) = &mut self.video_recovery else {
            let result = self.machine.run_slice_with_input(deadline, input_deadline, inputs)?;
            self.maybe_push_rewind();
            return Ok(result);
        };
        let started_at = self.machine.now();
        let mut consumed = 0;
        loop {
            let target = deadline.min(self.machine.bus.next_rendered_frame_cycle());
            let mut result = recovery.run(
                &mut self.machine,
                crate::machine::Slice { yield_at: target, input_deadline, stop_at: Some(deadline) },
                &inputs[consumed..],
            )?;
            consumed += result.inputs_consumed;
            result.started_at = started_at;
            result.inputs_consumed = consumed;
            match result.reason {
                StopReason::Faulted(_) => return Ok(result),
                StopReason::PowerStopped => {
                    self.maybe_push_rewind();
                    return Ok(result);
                }
                _ if self.machine.now() >= deadline => {
                    result.reason = if self.machine.now() == deadline {
                        StopReason::Deadline
                    } else {
                        StopReason::InstructionBoundaryAfterDeadline
                    };
                    self.maybe_push_rewind();
                    return Ok(result);
                }
                _ => {}
            }
        }
    }

    pub fn set_keys(&mut self, keys: KeyState) -> Result<(), Error> {
        if let Some(recovery) = &mut self.video_recovery {
            recovery.set_keys(&mut self.machine, keys)?;
        } else {
            self.machine.set_keys(keys);
        }
        Ok(())
    }
    pub fn step(&mut self) -> Result<StepOutcome, Error> {
        let outcome = if let Some(recovery) = &mut self.video_recovery {
            recovery.step(&mut self.machine)?
        } else {
            self.machine.step()
        };
        if !matches!(outcome, StepOutcome::Faulted(_)) {
            self.maybe_push_rewind();
        }
        Ok(outcome)
    }

    /// A failed or noncanonical capture is a skipped host-policy tick.
    fn maybe_push_rewind(&mut self) {
        if self.machine.fault().is_some() {
            return;
        }
        let frame = self.machine.completed_frames();
        if !self.rewind_history.due_for_capture(frame) {
            return;
        }
        let state = match self.write_state() {
            Ok(bytes) => bytes,
            Err(_) => {
                self.rewind_history.note_skipped(frame);
                return;
            }
        };
        if self.rewind_history.push(frame, self.machine.now(), &state).is_err() {
            self.rewind_history.note_skipped(frame);
        }
    }

    pub fn rewind(&mut self, steps: usize) -> Result<RewindPosition, Error> {
        if steps == 0 {
            return Err(Error::InvalidRewindSteps);
        }
        let index = self
            .rewind_history
            .target_older_than(self.machine.completed_frames(), steps)
            .ok_or(Error::RewindEmpty)?;
        let info = self.rewind_history.info(index).ok_or(Error::RewindEmpty)?;
        let state = self.rewind_history.decode(index)?;
        self.machine.load_state(&state)?;
        self.machine.set_audio_output(self.audio_output_enabled);
        if let Some(recovery) = &mut self.video_recovery {
            recovery.rebase(&self.machine)?;
        }
        self.machine.bus.set_video_interval(self.video_interval);
        self.resampler.reset();
        self.rewind_history.truncate_newer(index);
        Ok(RewindPosition { frame: info.frame, cycle: info.cycle })
    }

    pub fn set_rewind_config(&mut self, config: RewindConfig) -> Result<(), Error> {
        self.rewind_history.set_config(config)?;
        Ok(())
    }
    pub fn set_rewind_speed_scale(&mut self, scale_q8: u16) -> Result<(), Error> {
        self.rewind_history.set_speed_scale(scale_q8)?;
        Ok(())
    }
    pub fn clear_rewind(&mut self) {
        self.rewind_history.clear();
    }
    pub fn rewind_depth(&self) -> usize {
        self.rewind_history.depth()
    }
    pub fn rewind_memory_bytes(&self) -> usize {
        self.rewind_history.memory_bytes()
    }
    pub fn rewind_info(&self, index: usize) -> Option<RewindInfo> {
        self.rewind_history.info(index)
    }

    pub fn rewind_framebuffer(
        &self,
        index: usize,
        output: &mut [u16],
    ) -> Result<RewindInfo, Error> {
        if output.len() != 240 * 160 {
            return Err(Error::InvalidFramebufferSize);
        }
        let state = self.rewind_history.decode(index)?;
        let mut scratch = Machine::new(MachineConfig {
            rom: &self.rom,
            bios: &self.bios,
            boot: if self.bios.is_empty() { BootMode::Hle } else { BootMode::ExternalBios },
            ..MachineConfig::default()
        });
        scratch.load_state(&state)?;
        output.copy_from_slice(scratch.framebuffer());
        self.rewind_history.info(index).ok_or(Error::InvalidRewindSteps)
    }

    /// Host preference survives state loads and resets, but is not serialized.
    pub fn set_audio_output(&mut self, enabled: bool) {
        if self.audio_output_enabled == enabled {
            return;
        }
        self.audio_output_enabled = enabled;
        self.machine.set_audio_output(enabled);
        self.resampler.reset();
    }

    pub fn drain_audio(&mut self, output: &mut [StereoSample]) -> Result<usize, Error> {
        let rate = self.machine.audio_sample_rate();
        if rate != self.resampler.source_rate() {
            self.resampler = Resampler::new(rate, self.resampler.output_rate())
                .map_err(|_| Error::InvalidSampleRate)?;
        }
        let mut raw = [StereoSample::default(); 2048];
        let count = self.machine.drain_audio_samples(&mut raw);
        let result = self.resampler.process(&raw[..count], output);
        if result.consumed != count {
            return Err(Error::AudioBufferTooSmall);
        }
        Ok(result.written)
    }

    pub fn set_video_interval(&mut self, interval: u16) -> Result<(), Error> {
        if !(1..=16).contains(&interval) {
            return Err(Error::InvalidVideoInterval);
        }
        if interval == self.video_interval {
            return Ok(());
        }
        if interval == 1 {
            if let Some(recovery) = &mut self.video_recovery {
                recovery.materialize(&mut self.machine)?;
                self.video_recovery = None;
            }
            self.machine.bus.use_full_video();
        } else {
            if self.machine.fault().is_some() {
                return Err(Error::MachineFaulted);
            }
            if self.video_recovery.is_none() {
                self.video_recovery = Some(VideoRecovery::create(&self.machine)?);
            }
            self.machine.bus.set_video_interval(interval);
        }
        self.video_interval = interval;
        Ok(())
    }

    pub fn canonical_framebuffer(&mut self) -> Result<&[u16], Error> {
        if let Some(recovery) = &mut self.video_recovery {
            recovery.materialize(&mut self.machine)?;
        }
        Ok(self.machine.framebuffer())
    }

    pub fn write_state(&mut self) -> Result<Vec<u8>, Error> {
        self.canonical_framebuffer()?;
        Ok(self.machine.write_state_alloc()?)
    }

    /// Portable live state plus the exact retained rewind policy and history.
    pub fn write_rewind_bundle(&mut self) -> Result<Vec<u8>, Error> {
        let live = self.write_state()?;
        Ok(self.rewind_history.write_bundle(&live)?)
    }

    pub fn load_rewind_bundle(&mut self, bytes: &[u8]) -> Result<(), Error> {
        // A decoded checkpoint is never trusted just because the outer digest
        // matches. Reuse one scratch machine to validate all anchors/entries.
        let scratch = std::cell::RefCell::new(Machine::new(MachineConfig {
            rom: &self.rom,
            bios: &self.bios,
            boot: if self.bios.is_empty() { BootMode::Hle } else { BootMode::ExternalBios },
            ..MachineConfig::default()
        }));
        let (live, history) = RewindBuffer::read_bundle(bytes, |state| {
            let mut next = scratch.borrow_mut();
            next.load_state(state).map_err(|error| match error {
                StateError::Codec(error) => error,
                _ => evenfall_state::Error::InvalidSaveState,
            })?;
            Ok((next.completed_frames(), next.now()))
        })?;
        let mut next = scratch.into_inner();
        next.load_state(&live)?;
        next.set_audio_output(self.audio_output_enabled);
        next.bus.set_video_interval(self.video_interval);
        if let Some(recovery) = &mut self.video_recovery {
            recovery.rebase(&next)?;
        }
        self.machine = next;
        self.rewind_history = history;
        self.resampler.reset();
        Ok(())
    }

    pub fn load_state(&mut self, bytes: &[u8]) -> Result<(), Error> {
        if bytes.len() > 2 * 1024 * 1024 {
            return Err(Error::StateTooLarge);
        }
        self.machine.load_state(bytes)?;
        self.machine.set_audio_output(self.audio_output_enabled);
        if let Some(recovery) = &mut self.video_recovery {
            recovery.rebase(&self.machine)?;
        }
        self.machine.bus.set_video_interval(self.video_interval);
        self.resampler.reset();
        self.rewind_history.clear();
        Ok(())
    }

    pub fn reset_with_save(&mut self, save: &[u8]) -> Result<(), Error> {
        let data = (!save.is_empty()).then_some(save);
        let save_type = rom_image::choose_save_type(&self.rom, self.save_type_override, data)?;
        let save_data = data.map(|bytes| rom_image::save_prefix(save_type, bytes)).transpose()?;
        let features = cartridge::detect_features(&self.rom);
        let mut next = Machine::new(MachineConfig {
            rom: &self.rom,
            bios: &self.bios,
            immutable_media: true,
            boot: if self.bios.is_empty() { BootMode::Hle } else { BootMode::ExternalBios },
            save_type,
            save_data,
            rtc_epoch_seconds: self.rtc_enabled.then_some(self.rtc_epoch),
            solar_level: features.solar_sensor.then_some(128),
            gyro_sample: features.gyro.then_some(0x6c0),
            rumble: features.rumble,
            tilt_sample: features.tilt.then_some(Default::default()),
        });
        next.set_audio_output(self.audio_output_enabled);
        next.bus.set_video_interval(self.video_interval);
        self.machine = next;
        self.resampler.reset();
        self.rewind_history.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOOP_ROM: &[u8] = &[0xfe, 0xff, 0xff, 0xea];

    /// Zig: "owned sessions isolate state and preserve snapshot continuation"
    #[test]
    fn owned_sessions_isolate_state_and_preserve_snapshot_continuation() {
        let mut a = Session::create(Config { image: LOOP_ROM, ..Config::default() }).unwrap();
        let mut b = Session::create(Config { image: LOOP_ROM, ..Config::default() }).unwrap();
        a.advance(1000).unwrap();
        assert_eq!(b.machine.now(), 0);
        let state = a.write_state().unwrap();
        b.load_state(&state).unwrap();
        a.advance(2000).unwrap();
        b.advance(2000).unwrap();
        assert_eq!(a.write_state().unwrap(), b.write_state().unwrap());
        let before = a.machine.inspect_cpu();
        assert!(matches!(
            a.advance(a.machine.now() + Machine::CYCLES_PER_FRAME + 1),
            Err(Error::ExecutionBudgetExceeded)
        ));
        assert_eq!(a.machine.inspect_cpu(), before);
        let mut malformed = state;
        malformed[0] ^= 1;
        assert!(a.load_state(&malformed).is_err());
        assert_eq!(a.machine.inspect_cpu(), before);
    }

    /// Zig: "session rejects invalid media and configuration before constructing a machine"
    #[test]
    fn session_rejects_invalid_media_and_configuration_before_constructing_a_machine() {
        assert!(matches!(Session::create(Config::default()), Err(Error::InvalidRomSize)));
        assert!(matches!(
            Session::create(Config { image: &[1], bios: &[1], ..Config::default() }),
            Err(Error::InvalidBiosSize)
        ));
        assert!(matches!(
            Session::create(Config { image: &[1], output_rate: 0, ..Config::default() }),
            Err(Error::InvalidSampleRate)
        ));
        assert!(matches!(
            Session::create(Config { image: &[1], save: Some(&[1]), ..Config::default() }),
            Err(Error::Image(rom_image::Error::InvalidSaveFileSize))
        ));
    }

    /// Zig: "buffered snapshots match streamed state bytes for owned and borrowed media"
    #[test]
    fn buffered_snapshots_match_streamed_state_bytes_for_owned_media() {
        for save_type in [
            SaveType::None,
            SaveType::Sram,
            SaveType::Flash64k,
            SaveType::Flash128k,
            SaveType::Eeprom512,
            SaveType::Eeprom8k,
        ] {
            let mut session = Session::create(Config {
                image: LOOP_ROM,
                save_type: Some(save_type),
                rtc_enabled: Some(true),
                rtc_epoch: 12345,
                ..Config::default()
            })
            .unwrap();
            session.advance(12345).unwrap();
            let snapshot = session.write_state().unwrap();
            let mut stream = Vec::new();
            session.machine.write_state(&mut stream).unwrap();
            assert_eq!(stream, snapshot);
            let mut restored = Machine::new(MachineConfig {
                rom: LOOP_ROM,
                save_type,
                rtc_epoch_seconds: Some(12345),
                ..MachineConfig::default()
            });
            restored.load_state(&snapshot).unwrap();
            assert_eq!(restored.write_state_alloc().unwrap(), snapshot);
        }
    }

    /// Zig: "save restoration boots resident media and failure preserves the running session"
    #[test]
    fn save_restoration_boots_resident_media_and_failure_preserves_the_running_session() {
        let mut session = Session::create(Config {
            image: LOOP_ROM,
            rtc_enabled: Some(true),
            rtc_epoch: 12345,
            ..Config::default()
        })
        .unwrap();
        session.advance(1000).unwrap();
        let before = session.write_state().unwrap();
        assert!(matches!(
            session.reset_with_save(&[1]),
            Err(Error::Image(rom_image::Error::InvalidSaveFileSize))
        ));
        assert_eq!(session.write_state().unwrap(), before);
        let save = vec![0xa5; 32768];
        session.reset_with_save(&save).unwrap();
        let mut loaded = Session::create(Config {
            image: LOOP_ROM,
            save: Some(&save),
            rtc_enabled: Some(true),
            rtc_epoch: 12345,
            ..Config::default()
        })
        .unwrap();
        assert_eq!(session.write_state().unwrap(), loaded.write_state().unwrap());
    }

    /// Zig: "rewind captures per-frame history and restores earlier frames"
    #[test]
    fn rewind_captures_per_frame_history_and_restores_earlier_frames() {
        let mut session = Session::create(Config {
            image: LOOP_ROM,
            rewind: RewindConfig { interval_frames: 1, ..RewindConfig::default() },
            ..Config::default()
        })
        .unwrap();
        assert_eq!(session.rewind_depth(), 0);
        assert!(matches!(session.rewind(1), Err(Error::RewindEmpty)));
        assert!(matches!(session.rewind(0), Err(Error::InvalidRewindSteps)));
        for _ in 0..4 {
            session.advance(session.machine.now() + Machine::CYCLES_PER_FRAME).unwrap();
        }
        assert_eq!(session.machine.completed_frames(), 4);
        assert_eq!(session.rewind_depth(), 4);
        assert_eq!(session.rewind_info(0).unwrap().frame, 1);
        assert_eq!(session.rewind_info(2).unwrap().frame, 3);
        let at_four = session.write_state().unwrap();
        assert_eq!(session.rewind(1).unwrap().frame, 3);
        assert_eq!(session.rewind_depth(), 3);
        session.advance(session.machine.now() + Machine::CYCLES_PER_FRAME).unwrap();
        assert_eq!(session.write_state().unwrap(), at_four);
        assert_eq!(session.rewind(1).unwrap().frame, 3);
        assert_eq!(session.rewind(1).unwrap().frame, 2);
        assert_eq!(session.rewind(1).unwrap().frame, 1);
        assert!(matches!(session.rewind(1), Err(Error::RewindEmpty)));
    }

    /// Zig: "rewind bundles preserve live position, history and capture continuation"
    #[test]
    fn rewind_bundles_preserve_live_position_history_and_capture_continuation() {
        for compression in
            [RewindCompression::Off, RewindCompression::Fast, RewindCompression::Delta]
        {
            let mut a = Session::create(Config {
                image: LOOP_ROM,
                rewind: RewindConfig {
                    interval_frames: 1,
                    compression,
                    age_exponent: 0.25,
                    ..RewindConfig::default()
                },
                ..Config::default()
            })
            .unwrap();
            a.set_rewind_speed_scale(512).unwrap();
            for _ in 0..7 {
                a.advance(a.machine.now() + Machine::CYCLES_PER_FRAME).unwrap();
            }
            let live = a.write_state().unwrap();
            let bundle = a.write_rewind_bundle().unwrap();
            assert_eq!(a.write_state().unwrap(), live);
            assert_eq!(a.rewind_depth(), 3);

            let mut b = Session::create(Config { image: LOOP_ROM, ..Config::default() }).unwrap();
            b.set_audio_output(false);
            b.load_rewind_bundle(&bundle).unwrap();
            assert_eq!(b.write_state().unwrap(), live);
            assert_eq!(b.rewind_depth(), 3);
            assert_eq!(b.write_rewind_bundle().unwrap(), bundle);
            assert!(!b.audio_output_enabled);

            a.advance(a.machine.now() + Machine::CYCLES_PER_FRAME).unwrap();
            b.advance(b.machine.now() + Machine::CYCLES_PER_FRAME).unwrap();
            assert_eq!(a.write_rewind_bundle().unwrap(), b.write_rewind_bundle().unwrap());
        }
    }

    /// Zig: "rewind bundles reject corrupt envelopes metadata codecs and foreign media atomically"
    #[test]
    fn rewind_bundles_reject_corrupt_envelopes_metadata_codecs_and_foreign_media_atomically() {
        let mut session = Session::create(Config {
            image: LOOP_ROM,
            rewind: RewindConfig { interval_frames: 1, ..RewindConfig::default() },
            ..Config::default()
        })
        .unwrap();
        for _ in 0..3 {
            session.advance(session.machine.now() + Machine::CYCLES_PER_FRAME).unwrap();
        }
        let bundle = session.write_rewind_bundle().unwrap();
        session.rewind(2).unwrap();
        let before = session.write_rewind_bundle().unwrap();
        let history = 16 + u32::from_le_bytes(bundle[12..16].try_into().unwrap()) as usize;
        let anchor = history + 51;
        let entry = anchor
            + 8
            + u32::from_le_bytes(bundle[anchor + 4..anchor + 8].try_into().unwrap()) as usize;
        let payload = entry + 33;
        let cases = [
            (0, 0),
            (8, 2),
            (16, 0),
            (history + 3, 255),
            (history + 6, 255),
            (history + 14, 255),
            (history + 25, 2),
            (history + 26, 4),
            (history + 49, 1),
            (anchor + 16, 7),
            (entry + 8, 1),
            (entry + 24, 255),
            (entry + 25, 255),
            (payload + 3, 255),
            (payload + 8, 7),
        ];
        for (offset, value) in cases {
            let mut corrupt = bundle.clone();
            corrupt[offset] = value;
            let hash = blake3::hash(&corrupt[..corrupt.len() - 32]);
            let end = corrupt.len() - 32;
            corrupt[end..].copy_from_slice(hash.as_bytes());
            assert!(session.load_rewind_bundle(&corrupt).is_err(), "offset {offset}");
            assert_eq!(session.write_rewind_bundle().unwrap(), before, "offset {offset}");
        }
        for length in [0, 47, bundle.len() - 1] {
            assert!(session.load_rewind_bundle(&bundle[..length]).is_err());
            assert_eq!(session.write_rewind_bundle().unwrap(), before);
        }
        let mut other =
            Session::create(Config { image: &[0xfd, 0xff, 0xff, 0xea], ..Config::default() })
                .unwrap();
        assert!(matches!(
            other.load_rewind_bundle(&bundle),
            Err(Error::Rewind(evenfall_state::Error::SaveStateRomMismatch))
        ));
    }

    fn video_recovery_rom(fault_on_a: bool) -> [u8; 32] {
        let words = if fault_on_a {
            [
                0xe59f0010_u32,
                0xe1d020b0,
                0xe3120001,
                0x0fff0000,
                0xeafffffb,
                0xe1a00000,
                0x04000130,
                0,
            ]
        } else {
            [
                0xe59f0010, 0xe59f1010, 0xe1d020b0, 0xe1c120b0, 0xeafffffc, 0xe1a00000, 0x04000130,
                0x05000000,
            ]
        };
        let mut rom = [0; 32];
        for (bytes, word) in rom.chunks_exact_mut(4).zip(words) {
            bytes.copy_from_slice(&word.to_le_bytes());
        }
        rom
    }

    /// Zig: "selective Session restores exact arbitrary-cycle states and inputs across internal frame splits"
    #[test]
    fn selective_session_restores_exact_arbitrary_cycle_states_and_inputs_across_internal_frame_splits()
     {
        let rom = video_recovery_rom(false);
        let mut full = Session::create(Config { image: &rom, ..Config::default() }).unwrap();
        let mut selective = Session::create(Config { image: &rom, ..Config::default() }).unwrap();
        full.machine.bus.dispcnt = 0;
        selective.machine.bus.dispcnt = 0;
        full.advance(100_003).unwrap();
        selective.advance(100_003).unwrap();
        selective.set_video_interval(16).unwrap();
        selective.set_audio_output(false);
        let mut reconstructed = false;
        for index in 0..24 {
            let deadline = full.machine.now() + Machine::CYCLES_PER_FRAME;
            let inputs = [
                TimedInput {
                    at: full.machine.now() + 97,
                    change: crate::bus::TimedInputChange::Keys(KeyState {
                        a: index % 2 != 0,
                        b: index % 3 == 0,
                        ..KeyState::default()
                    }),
                },
                TimedInput {
                    at: deadline - 3,
                    change: crate::bus::TimedInputChange::Keys(KeyState {
                        start: index % 2 == 0,
                        ..KeyState::default()
                    }),
                },
            ];
            let expected = full.run_slice_with_input(deadline, deadline, &inputs).unwrap();
            let actual = selective.run_slice_with_input(deadline, deadline, &inputs).unwrap();
            assert_eq!(actual, expected);
            if index % 7 == 3 {
                reconstructed |= !selective.machine.bus.is_framebuffer_canonical();
                let at = selective.machine.now();
                assert_eq!(selective.write_state().unwrap(), full.write_state().unwrap());
                assert_eq!(selective.machine.now(), at);
            }
        }
        assert!(reconstructed);
        selective.set_video_interval(1).unwrap();
        assert!(selective.video_recovery.is_none());
        assert_eq!(selective.write_state().unwrap(), full.write_state().unwrap());
    }

    /// Zig: "selective Session load and reset replace recovery anchors transactionally"
    #[test]
    fn selective_session_load_and_reset_replace_recovery_anchors_transactionally() {
        let rom = video_recovery_rom(false);
        let mut session = Session::create(Config { image: &rom, ..Config::default() }).unwrap();
        session.advance(73_127).unwrap();
        let state = session.write_state().unwrap();
        session.set_video_interval(8).unwrap();
        for _ in 0..5 {
            session.advance(session.machine.now() + Machine::CYCLES_PER_FRAME).unwrap();
        }
        let before = session.machine.now();
        assert!(session.load_state(&[1, 2, 3]).is_err());
        assert_eq!(session.machine.now(), before);
        session.load_state(&state).unwrap();
        assert_eq!(session.write_state().unwrap(), state);
        assert_eq!(session.video_interval, 8);
        session.reset_with_save(&[]).unwrap();
        assert_eq!(session.machine.now(), 0);
        assert_eq!(session.video_interval, 8);
    }
}
