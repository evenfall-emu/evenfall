//! Owned, host-independent session translated from `session.zig`.

use crate::bus::{KeyState, Media, TimedInput};
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
    /// The live machine is inspectable; replacing it wholesale bypasses the
    /// session's resident-media binding. Use load_state/reset_with_save for
    /// resident media, or construct a new Session for different media.
    pub machine: Box<Machine>,
    rom: Media,
    bios: Media,
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
    OutOfMemory,
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
    pub fn rom(&self) -> &[u8] {
        &self.rom
    }

    pub fn bios(&self) -> &[u8] {
        &self.bios
    }

    pub fn create(config: Config<'_>) -> Result<Self, Error> {
        Self::create_impl(config, None).map_err(|(error, _)| error)
    }

    /// Uses the prefix of the uploaded buffer as the image. Failed construction
    /// returns the original allocation; success shares the Machine's immutable
    /// media handle, preserving the uploaded bytes' address. `config.image` is
    /// ignored. A ZIP's extracted ROM takes its own allocation instead.
    pub fn create_from_owned_buffer(
        config: Config<'_>,
        buffer: Vec<u8>,
        image_len: usize,
    ) -> Result<Self, (Error, Vec<u8>)> {
        Self::create_impl(config, Some((buffer, image_len)))
            .map_err(|(error, buffer)| (error, buffer.expect("owned upload returned on error")))
    }

    fn create_impl(
        config: Config<'_>,
        upload: Option<(Vec<u8>, usize)>,
    ) -> Result<Self, (Error, Option<Vec<u8>>)> {
        // All fallible setup precedes adoption, so an invalid configuration
        // returns the original uploaded Vec, including late rewind errors.
        let prepared = (|| -> Result<_, Error> {
            let image = if let Some((buffer, image_len)) = upload.as_ref() {
                buffer.get(..*image_len).ok_or(Error::InvalidRomSize)?
            } else {
                config.image
            };
            if image.is_empty() || image.len() > 40 * 1024 * 1024 {
                return Err(Error::InvalidRomSize);
            }
            if !config.bios.is_empty() && config.bios.len() != 16_384 {
                return Err(Error::InvalidBiosSize);
            }
            if !(8_000..=192_000).contains(&config.output_rate) {
                return Err(Error::InvalidSampleRate);
            }
            if config.rtc_epoch > crate::cartridge::gpio::Rtc::MAX_EPOCH_SECONDS {
                return Err(Error::InvalidRtcEpoch);
            }
            let extracted =
                config.zipped.then(|| rom_image::extract_single_gba(image)).transpose()?;
            let rom = extracted.as_deref().unwrap_or(image);
            if rom.is_empty() || rom.len() > rom_image::MAX_ROM_SIZE {
                return Err(Error::InvalidRomSize);
            }
            let save_type = rom_image::choose_save_type(rom, config.save_type, config.save)?;
            let save_data =
                config.save.map(|save| rom_image::save_prefix(save_type, save)).transpose()?;
            let features = cartridge::detect_features(rom);
            let rtc_enabled = config.rtc_enabled.unwrap_or(features.rtc);
            let rom_sha1 = Sha1::digest(rom).into();
            let rewind_history = RewindBuffer::new(config.rewind)?;
            let copied = if upload.is_none() && extracted.is_none() {
                let mut copy = Vec::new();
                copy.try_reserve_exact(image.len()).map_err(|_| Error::OutOfMemory)?;
                copy.extend_from_slice(image);
                Some(copy)
            } else {
                None
            };
            let mut bios_copy = Vec::new();
            bios_copy.try_reserve_exact(config.bios.len()).map_err(|_| Error::OutOfMemory)?;
            bios_copy.extend_from_slice(config.bios);
            Ok((
                extracted,
                copied,
                bios_copy,
                save_type,
                save_data,
                features,
                rtc_enabled,
                rom_sha1,
                rewind_history,
            ))
        })();
        let (
            extracted,
            copied,
            bios_copy,
            save_type,
            save_data,
            features,
            rtc_enabled,
            rom_sha1,
            rewind_history,
        ) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => return Err((error, upload.map(|(buffer, _)| buffer))),
        };
        let rom = Media::from_vec(if let Some(rom) = extracted {
            rom
        } else if let Some((mut buffer, image_len)) = upload {
            buffer.truncate(image_len);
            buffer
        } else {
            copied.expect("borrowed raw ROM was copied during preparation")
        });
        let bios = Media::from_vec(bios_copy);
        let machine = Machine::new_with_owned_media(
            MachineConfig {
                boot: if config.bios.is_empty() { BootMode::Hle } else { BootMode::ExternalBios },
                save_type,
                save_data,
                rtc_epoch_seconds: rtc_enabled.then_some(config.rtc_epoch),
                solar_level: features.solar_sensor.then_some(128),
                gyro_sample: features.gyro.then_some(0x6c0),
                rumble: features.rumble,
                tilt_sample: features.tilt.then_some(Default::default()),
                ..MachineConfig::default()
            },
            rom.clone(),
            bios.clone(),
        );
        // The output rate is checked above and a freshly initialized APU
        // always has a nonzero sample rate.
        let resampler = Resampler::new(machine.audio_sample_rate(), config.output_rate)
            .expect("validated output rate and nonzero APU sample rate");
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
            recovery.rebase(&self.machine);
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
        let mut scratch = Machine::new_with_shared_media(
            &self.machine,
            MachineConfig {
                boot: if self.bios.is_empty() { BootMode::Hle } else { BootMode::ExternalBios },
                ..MachineConfig::default()
            },
        );
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
                self.video_recovery = Some(VideoRecovery::create(&self.machine));
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

    /// Fallible caller-sized output for the WASM ABI; native writes keep the fast path.
    pub fn write_state_fallible(&mut self) -> Result<Vec<u8>, Error> {
        self.canonical_framebuffer()?;
        Ok(self.machine.write_state_alloc_fallible()?)
    }

    /// Portable live state plus the exact retained rewind policy and history.
    pub fn write_rewind_bundle(&mut self) -> Result<Vec<u8>, Error> {
        let live = self.write_state()?;
        Ok(self.rewind_history.write_bundle(&live)?)
    }

    pub fn write_rewind_bundle_fallible(&mut self) -> Result<Vec<u8>, Error> {
        let live = self.write_state_fallible()?;
        Ok(self.rewind_history.write_bundle(&live)?)
    }

    pub fn load_rewind_bundle(&mut self, bytes: &[u8]) -> Result<(), Error> {
        if bytes.len() > rewind::MAX_BUNDLE_BYTES {
            return Err(evenfall_state::Error::RewindBundleTooLarge.into());
        }
        self.load_rewind_bundle_owned(bytes.to_vec())
    }

    /// Consumes an upload without copying its stored history payloads. A
    /// failed import drops the upload and leaves the live session untouched.
    pub fn load_rewind_bundle_owned(&mut self, bytes: Vec<u8>) -> Result<(), Error> {
        // A decoded checkpoint is never trusted just because the outer digest
        // matches. Reuse one scratch machine to validate all anchors/entries.
        let scratch = std::cell::RefCell::new(Machine::new_with_shared_media(
            &self.machine,
            MachineConfig {
                boot: if self.bios.is_empty() { BootMode::Hle } else { BootMode::ExternalBios },
                ..MachineConfig::default()
            },
        ));
        let (live, history) = RewindBuffer::read_bundle_owned(bytes, |state| {
            let mut next = scratch.borrow_mut();
            next.load_state_in_place(state).map_err(|error| match error {
                StateError::Codec(error) => error,
                _ => evenfall_state::Error::InvalidSaveState,
            })?;
            Ok((next.completed_frames(), next.now()))
        })
        .map_err(|(error, _)| error)?;
        let mut next = scratch.into_inner();
        next.load_state_in_place(&live)?;
        next.set_audio_output(self.audio_output_enabled);
        next.bus.set_video_interval(self.video_interval);
        if let Some(recovery) = &mut self.video_recovery {
            recovery.rebase(&next);
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
            recovery.rebase(&self.machine);
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
        let mut next = Machine::new_with_shared_media(
            &self.machine,
            MachineConfig {
                rom: &self.rom,
                bios: &self.bios,
                boot: if self.bios.is_empty() { BootMode::Hle } else { BootMode::ExternalBios },
                save_type,
                save_data,
                rtc_epoch_seconds: self.rtc_enabled.then_some(self.rtc_epoch),
                solar_level: features.solar_sensor.then_some(128),
                gyro_sample: features.gyro.then_some(0x6c0),
                rumble: features.rumble,
                tilt_sample: features.tilt.then_some(Default::default()),
            },
        );
        next.set_audio_output(self.audio_output_enabled);
        if let Some(recovery) = &mut self.video_recovery {
            recovery.rebase(&next);
        }
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

    /// Evenfall-only: digest-valid states whose counters would make the next
    /// oscillator advance or RTC date read run for years of host time are
    /// rejected, by explicit loads and inside rewind bundles, and neither the
    /// live machine nor the history changes.
    #[test]
    fn digest_valid_states_with_unbounded_counters_are_rejected_atomically() {
        let mut session = Session::create(Config {
            image: LOOP_ROM,
            rtc_enabled: Some(true),
            rewind: RewindConfig { interval_frames: 1, ..RewindConfig::default() },
            ..Config::default()
        })
        .unwrap();
        for _ in 0..3 {
            session.advance(session.machine.now() + Machine::CYCLES_PER_FRAME).unwrap();
        }
        let state = session.write_state().unwrap();
        let bundle = session.write_rewind_bundle().unwrap();
        let live = 16..16 + state.len();
        assert_eq!(bundle[live.clone()], state);
        let fields = session.machine.state_field_ranges().unwrap();
        for (name, value) in [
            ("bus.apu.noise.phase_cycles", 1u64 << 63),
            ("bus.cartridge_gpio.rtc.epoch_seconds", u64::MAX),
        ] {
            let range = &fields.iter().find(|field| field.0 == name).unwrap().1;
            let at = Machine::STATE_HEADER_SIZE + range.start as usize;
            let mut bad = state.clone();
            bad[at..at + 8].copy_from_slice(&value.to_le_bytes());
            let end = bad.len() - 32;
            let digest = blake3::hash(&bad[Machine::STATE_HEADER_SIZE..end]);
            bad[end..].copy_from_slice(digest.as_bytes());
            assert!(session.load_state(&bad).is_err(), "{name}");
            assert_eq!(session.write_state().unwrap(), state, "{name}");

            let mut bad_bundle = bundle.clone();
            bad_bundle[live.clone()].copy_from_slice(&bad);
            let end = bad_bundle.len() - 32;
            let digest = blake3::hash(&bad_bundle[..end]);
            bad_bundle[end..].copy_from_slice(digest.as_bytes());
            assert!(session.load_rewind_bundle(&bad_bundle).is_err(), "{name}");
            assert_eq!(session.write_rewind_bundle().unwrap(), bundle, "{name}");
        }
    }

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

    /// Zig: "owned buffer creation consumes exactly the uploaded allocation"
    /// and "upload ownership transfers only on successful construction".
    #[test]
    fn owned_buffer_transfers_only_after_successful_construction() {
        let buffer = [LOOP_ROM, &[0xa5, 0x36]].concat();
        let address = buffer.as_ptr();
        let session =
            Session::create_from_owned_buffer(Config::default(), buffer, LOOP_ROM.len()).unwrap();
        assert_eq!(session.rom.as_ptr(), address);
        assert_eq!(session.rom.as_slice(), LOOP_ROM);
        assert_eq!(session.rom.as_ptr(), session.machine.bus.rom.as_ptr());

        let buffer = LOOP_ROM.to_vec();
        let address = buffer.as_ptr();
        let (error, returned) = Session::create_from_owned_buffer(
            Config { output_rate: 0, ..Config::default() },
            buffer,
            LOOP_ROM.len(),
        )
        .err()
        .unwrap();
        assert!(matches!(error, Error::InvalidSampleRate));
        assert_eq!(returned.as_ptr(), address);
        assert_eq!(returned, LOOP_ROM);

        let buffer = LOOP_ROM.to_vec();
        let address = buffer.as_ptr();
        let (error, returned) = Session::create_from_owned_buffer(
            Config {
                rewind: RewindConfig { interval_frames: 0, ..RewindConfig::default() },
                ..Config::default()
            },
            buffer,
            LOOP_ROM.len(),
        )
        .err()
        .unwrap();
        assert!(matches!(error, Error::Rewind(evenfall_state::Error::InvalidRewindConfig)));
        assert_eq!(returned.as_ptr(), address);
        assert_eq!(returned, LOOP_ROM);
    }

    /// Zig: "buffered snapshots match streamed state bytes for owned and borrowed media".
    /// The mutable borrowed-ROM mutation/rejection half is deliberately not
    /// ported: Machine copies or adopts immutable shared media at construction.
    #[test]
    fn buffered_snapshots_match_streamed_state_bytes_for_owned_and_shared_media() {
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
            let rom = session.rom.clone();
            let mut restored = Machine::new_with_owned_media(
                MachineConfig {
                    save_type,
                    rtc_epoch_seconds: Some(12345),
                    ..MachineConfig::default()
                },
                rom,
                session.bios.clone(),
            );
            assert_eq!(session.rom.as_ptr(), restored.bus.rom.as_ptr());
            restored.load_state(&snapshot).unwrap();
            assert_eq!(restored.write_state_alloc().unwrap(), snapshot);
        }
    }

    /// Zig: "borrowed BIOS identity stays live while owned BIOS snapshots stay stable".
    /// The live-mutation half is deliberately not ported: both Rust paths
    /// snapshot caller bytes into immutable shared media at construction.
    #[test]
    fn bios_identity_is_fixed_at_construction_and_foreign_bios_is_rejected() {
        let mut caller_bios = vec![0; 16_384];
        let mut session =
            Session::create(Config { image: LOOP_ROM, bios: &caller_bios, ..Config::default() })
                .unwrap();
        let state = session.write_state().unwrap();
        caller_bios[100] = 1;
        assert_eq!(session.write_state().unwrap(), state);
        assert_eq!(session.bios.as_ptr(), session.machine.bus.bios.as_ptr());

        let mut foreign = Machine::new(MachineConfig {
            rom: LOOP_ROM,
            bios: &caller_bios,
            boot: BootMode::ExternalBios,
            ..MachineConfig::default()
        });
        let before = foreign.write_state_alloc().unwrap();
        assert!(matches!(
            foreign.load_state(&state),
            Err(StateError::Codec(evenfall_state::Error::SaveStateBiosMismatch))
        ));
        assert_eq!(foreign.write_state_alloc().unwrap(), before);
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

    /// Zig: "session imports save memory with arbitrary trailing bytes but rejects a short save"
    #[test]
    fn session_imports_save_prefix_and_rejects_short_or_conflicting_saves() {
        let rom = b"....FLASH1M_V103";
        let mut save = vec![0xff; 128 * 1024 + 19];
        save[0x1234] = 0x35;
        save[0x10042] = 0x7a;
        save[128 * 1024..].fill(0x55);
        let mut session =
            Session::create(Config { image: rom, save: Some(&save), ..Config::default() }).unwrap();
        assert_eq!(session.machine.save_type(), SaveType::Flash128k);
        assert_eq!(session.machine.save_data(), &save[..128 * 1024]);
        session.reset_with_save(&save).unwrap();
        assert_eq!(session.machine.save_data(), &save[..128 * 1024]);
        assert!(matches!(
            session.reset_with_save(&save[..128 * 1024 - 1]),
            Err(Error::Image(rom_image::Error::InvalidSaveFileSize))
        ));
        assert_eq!(session.machine.save_data(), &save[..128 * 1024]);
        assert!(matches!(
            session.reset_with_save(&save[..64 * 1024]),
            Err(Error::Image(rom_image::Error::SaveTypeConflict))
        ));
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

    /// Zig: "rewind escapes a faulted machine with the last good state"
    #[test]
    fn rewind_escapes_a_faulted_machine_with_the_last_good_state() {
        let rom = video_recovery_rom(true);
        let mut session = Session::create(Config {
            image: &rom,
            rewind: RewindConfig { interval_frames: 1, ..RewindConfig::default() },
            ..Config::default()
        })
        .unwrap();
        for _ in 0..3 {
            session.advance(session.machine.now() + Machine::CYCLES_PER_FRAME).unwrap();
        }
        let healthy_depth = session.rewind_depth();
        assert!(healthy_depth > 0);
        session.set_keys(KeyState { a: true, ..KeyState::default() }).unwrap();
        let result = session.advance(session.machine.now() + 100).unwrap();
        assert!(matches!(result.reason, StopReason::Faulted(_)));
        assert!(session.machine.fault().is_some());
        assert_eq!(session.rewind_depth(), healthy_depth);
        let restored = session.rewind(1).unwrap();
        assert!(session.machine.fault().is_none());
        assert_eq!(restored.frame, session.machine.completed_frames());
        session.advance(session.machine.now() + Machine::CYCLES_PER_FRAME).unwrap();
        assert!(session.machine.fault().is_none());
    }

    /// Zig: "explicit loads and resets start a new rewind timeline"
    #[test]
    fn explicit_loads_and_resets_start_a_new_rewind_timeline() {
        let mut session = Session::create(Config {
            image: LOOP_ROM,
            rewind: RewindConfig { interval_frames: 1, ..RewindConfig::default() },
            ..Config::default()
        })
        .unwrap();
        for _ in 0..2 {
            session.advance(session.machine.now() + Machine::CYCLES_PER_FRAME).unwrap();
        }
        assert_eq!(session.rewind_depth(), 2);
        let state = session.write_state().unwrap();
        session.advance(session.machine.now() + Machine::CYCLES_PER_FRAME).unwrap();
        assert_eq!(session.rewind_depth(), 3);
        session.load_state(&state).unwrap();
        assert_eq!(session.rewind_depth(), 0);
        assert!(matches!(session.rewind(1), Err(Error::RewindEmpty)));
        session.advance(session.machine.now() + Machine::CYCLES_PER_FRAME).unwrap();
        assert_eq!(session.rewind_depth(), 1);
        session.reset_with_save(&[]).unwrap();
        assert_eq!(session.rewind_depth(), 0);
    }

    /// Zig: "session validates rewind configs and preserves history on reconfig failure"
    #[test]
    fn invalid_rewind_reconfiguration_preserves_history() {
        assert!(matches!(
            Session::create(Config {
                image: LOOP_ROM,
                rewind: RewindConfig { interval_frames: 0, ..RewindConfig::default() },
                ..Config::default()
            }),
            Err(Error::Rewind(evenfall_state::Error::InvalidRewindConfig))
        ));
        let mut session = Session::create(Config {
            image: LOOP_ROM,
            rewind: RewindConfig { interval_frames: 1, ..RewindConfig::default() },
            ..Config::default()
        })
        .unwrap();
        session.advance(Machine::CYCLES_PER_FRAME).unwrap();
        assert_eq!(session.rewind_depth(), 1);
        assert!(
            session
                .set_rewind_config(RewindConfig {
                    budget_bytes: rewind::MAX_BUDGET_BYTES + 1,
                    ..RewindConfig::default()
                })
                .is_err()
        );
        assert!(
            session
                .set_rewind_config(RewindConfig {
                    age_exponent: f64::NAN,
                    ..RewindConfig::default()
                })
                .is_err()
        );
        assert_eq!(session.rewind_depth(), 1);
        session
            .set_rewind_config(RewindConfig { budget_bytes: 0, ..RewindConfig::default() })
            .unwrap();
        assert_eq!(session.rewind_depth(), 0);
        session.advance(2 * Machine::CYCLES_PER_FRAME).unwrap();
        assert_eq!(session.rewind_depth(), 0);
    }

    /// Zig: "rewind history compresses by default and honors explicit off"
    #[test]
    fn rewind_history_compresses_by_default_and_honors_explicit_off() {
        let mut session = Session::create(Config {
            image: LOOP_ROM,
            rewind: RewindConfig { interval_frames: 1, ..RewindConfig::default() },
            ..Config::default()
        })
        .unwrap();
        for _ in 0..2 {
            session.advance(session.machine.now() + Machine::CYCLES_PER_FRAME).unwrap();
        }
        let info = session.rewind_info(1).unwrap();
        assert_eq!(info.encoding, rewind::Encoding::CopyDelta);
        let previous_frame = session.rewind_info(0).unwrap().frame;
        session.rewind(1).unwrap();
        assert_eq!(session.machine.completed_frames(), previous_frame);
        assert!(info.bytes < session.write_state().unwrap().len() / 2);

        session
            .set_rewind_config(RewindConfig {
                interval_frames: 1,
                compression: RewindCompression::Off,
                ..RewindConfig::default()
            })
            .unwrap();
        assert_eq!(session.rewind_depth(), 0);
        session.advance(session.machine.now() + Machine::CYCLES_PER_FRAME).unwrap();
        assert_eq!(session.rewind_info(0).unwrap().encoding, rewind::Encoding::Raw);
        let at_frame = session.machine.completed_frames();
        session.advance(session.machine.now() + Machine::CYCLES_PER_FRAME).unwrap();
        session.rewind(1).unwrap();
        assert_eq!(session.machine.completed_frames(), at_frame);
    }

    /// Zig: "rewind framebuffer observation is exact and does not consume history"
    #[test]
    fn rewind_framebuffer_observation_is_exact_and_does_not_consume_history() {
        let rom = video_recovery_rom(false);
        let mut session = Session::create(Config {
            image: &rom,
            rewind: RewindConfig { interval_frames: 1, ..RewindConfig::default() },
            ..Config::default()
        })
        .unwrap();
        session.machine.bus.dispcnt = 0;
        session.set_keys(KeyState { a: true, ..KeyState::default() }).unwrap();
        session.advance(Machine::CYCLES_PER_FRAME).unwrap();
        let oldest_pixels = session.machine.framebuffer().to_vec();
        session.set_keys(KeyState { b: true, ..KeyState::default() }).unwrap();
        session.advance(2 * Machine::CYCLES_PER_FRAME).unwrap();
        session.set_keys(KeyState { select: true, ..KeyState::default() }).unwrap();
        session.advance(3 * Machine::CYCLES_PER_FRAME).unwrap();

        let before = session.write_state().unwrap();
        let depth = session.rewind_depth();
        let mut pixels = vec![0; 240 * 160];
        let info = session.rewind_framebuffer(0, &mut pixels).unwrap();
        assert_eq!(info.frame, session.rewind_info(0).unwrap().frame);
        assert_eq!(session.rewind_depth(), depth);
        assert_eq!(pixels, oldest_pixels);
        assert_ne!(pixels, session.machine.framebuffer());
        assert_eq!(session.write_state().unwrap(), before);
        assert!(matches!(
            session.rewind_framebuffer(depth, &mut pixels),
            Err(Error::Rewind(evenfall_state::Error::InvalidRewindSteps))
        ));
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
            if compression == RewindCompression::Off {
                b.load_rewind_bundle(&bundle).unwrap();
            } else {
                b.load_rewind_bundle_owned(bundle.clone()).unwrap();
            }
            assert_eq!(b.write_state().unwrap(), live);
            assert_eq!(b.rewind_depth(), 3);
            assert_eq!(b.write_rewind_bundle().unwrap(), bundle);
            assert!(!b.audio_output_enabled);

            a.advance(a.machine.now() + Machine::CYCLES_PER_FRAME).unwrap();
            b.advance(b.machine.now() + Machine::CYCLES_PER_FRAME).unwrap();
            assert_eq!(a.write_rewind_bundle().unwrap(), b.write_rewind_bundle().unwrap());
        }
    }

    /// Zig: "rewind bundles reject corrupt envelopes metadata codecs and foreign media atomically".
    /// Also checks the owned-import rollback boundary from "rewind bundle allocation
    /// failures preserve both live state and old history" for recoverable errors.
    /// Zig's injected allocator-failure half is deliberately not ported: a
    /// Rust allocation failure normally aborts instead of returning an error.
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
            (8, 3),
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
            assert!(session.load_rewind_bundle_owned(corrupt).is_err(), "offset {offset}");
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

    /// Zig: "bundle restores empty tiny-budget and evicted multi-anchor histories after branching"
    #[test]
    fn bundle_restores_empty_tiny_budget_and_evicted_histories_after_branching() {
        for budget in [0, 1, 400_000, rewind::DEFAULT_BUDGET_BYTES] {
            let mut session = Session::create(Config {
                image: LOOP_ROM,
                rewind: RewindConfig {
                    interval_frames: 1,
                    budget_bytes: budget,
                    ..RewindConfig::default()
                },
                ..Config::default()
            })
            .unwrap();
            let mut random = 71_u32;
            for frame in 0..9_u8 {
                if frame % 3 == 0 {
                    for byte in session.machine.bus.ewram.iter_mut() {
                        random ^= random << 13;
                        random ^= random >> 17;
                        random ^= random << 5;
                        *byte = random as u8;
                    }
                }
                session.advance(session.machine.now() + Machine::CYCLES_PER_FRAME).unwrap();
            }
            let bundle = session.write_rewind_bundle().unwrap();
            if session.rewind_depth() > 1 {
                session.rewind(1).unwrap();
            }
            session.set_keys(KeyState { a: true, ..KeyState::default() }).unwrap();
            session.advance(session.machine.now() + Machine::CYCLES_PER_FRAME).unwrap();
            session.load_rewind_bundle(&bundle).unwrap();
            assert_eq!(session.write_rewind_bundle().unwrap(), bundle);
            if session.rewind_depth() > 1 {
                let oldest_frame = session.rewind_info(0).unwrap().frame;
                session.rewind(session.rewind_depth() - 1).unwrap();
                assert_eq!(session.machine.completed_frames(), oldest_frame);
            }
        }
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
        for (bytes, word) in rom.as_chunks_mut::<4>().0.iter_mut().zip(words) {
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

    /// Zig: "selective recovery reserves bounded history and preserves failing instruction pixels"
    #[test]
    fn selective_recovery_bounds_history_and_preserves_faulting_instruction_pixels() {
        let rom = video_recovery_rom(true);
        let mut full = Session::create(Config { image: &rom, ..Config::default() }).unwrap();
        let mut selective = Session::create(Config { image: &rom, ..Config::default() }).unwrap();
        full.machine.bus.dispcnt = 0;
        selective.machine.bus.dispcnt = 0;
        selective.set_video_interval(16).unwrap();
        for _ in 0..4 {
            full.advance(full.machine.now() + Machine::CYCLES_PER_FRAME).unwrap();
            selective.advance(selective.machine.now() + Machine::CYCLES_PER_FRAME).unwrap();
        }
        assert!(!selective.machine.bus.is_framebuffer_canonical());
        for _ in 0..4100 {
            full.set_keys(KeyState::default()).unwrap();
            selective.set_keys(KeyState::default()).unwrap();
        }
        full.advance(full.machine.now() + Machine::CYCLES_PER_FRAME).unwrap();
        selective.advance(selective.machine.now() + Machine::CYCLES_PER_FRAME).unwrap();
        full.set_keys(KeyState { a: true, ..KeyState::default() }).unwrap();
        selective.set_keys(KeyState { a: true, ..KeyState::default() }).unwrap();
        let expected = full.advance(full.machine.now() + 100).unwrap();
        let actual = selective.advance(selective.machine.now() + 100).unwrap();
        assert_eq!(actual, expected);
        assert!(selective.machine.fault().is_some());
        for _ in 0..4100 {
            selective.set_keys(KeyState::default()).unwrap();
        }
        let cycle = selective.machine.now();
        assert_eq!(full.machine.framebuffer(), selective.canonical_framebuffer().unwrap());
        assert_eq!(selective.machine.now(), cycle);
        assert!(matches!(selective.write_state(), Err(Error::State(StateError::MachineFaulted))));
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
        let mut fresh = Session::create(Config { image: &rom, ..Config::default() }).unwrap();
        for _ in 0..2 {
            session.advance(session.machine.now() + Machine::CYCLES_PER_FRAME).unwrap();
            fresh.advance(fresh.machine.now() + Machine::CYCLES_PER_FRAME).unwrap();
        }
        assert_eq!(session.write_state().unwrap(), fresh.write_state().unwrap());
    }

    /// Zig: "internal video yields preserve STOP serial horizon and unreachable future inputs"
    #[test]
    fn internal_video_yields_preserve_stop_serial_horizon_and_future_inputs() {
        let rom = [0, 0, 3, 0xef, 0xfe, 0xff, 0xff, 0xea]; // Stop; b .
        for (future_input, wakes) in [(false, true), (true, true), (false, false)] {
            let mut full = Session::create(Config { image: &rom, ..Config::default() }).unwrap();
            let mut selective =
                Session::create(Config { image: &rom, ..Config::default() }).unwrap();
            for session in [&mut full, &mut selective] {
                session.machine.bus.rcnt = 0xc000;
                session.machine.bus.joy_control = 0x40;
                session.machine.bus.irq.interrupt_enable_mask = 1 << 7;
                session.machine.bus.update_irq_line();
                session
                    .machine
                    .bus
                    .schedule_joy_bus(
                        if wakes {
                            crate::bus::JoyBusCommand::Reset
                        } else {
                            crate::bus::JoyBusCommand::Status
                        },
                        250_000,
                    )
                    .unwrap();
            }
            selective.set_video_interval(16).unwrap();
            let future = [TimedInput {
                at: 220_000,
                change: crate::bus::TimedInputChange::Keys(KeyState {
                    a: true,
                    ..KeyState::default()
                }),
            }];
            let inputs = if future_input { &future[..] } else { &[][..] };
            let expected = full
                .run_slice_with_input(Machine::CYCLES_PER_FRAME, Machine::CYCLES_PER_FRAME, inputs)
                .unwrap();
            let actual = selective
                .run_slice_with_input(Machine::CYCLES_PER_FRAME, Machine::CYCLES_PER_FRAME, inputs)
                .unwrap();
            assert_eq!(actual, expected);
            if future_input {
                assert_eq!(actual.reason, StopReason::PowerStopped);
                let mut reference =
                    Session::create(Config { image: &rom, ..Config::default() }).unwrap();
                reference.step().unwrap();
                assert_eq!(reference.machine.now(), actual.stopped_at);
                assert_eq!(selective.machine.bus.joy_completion_cycle, Some(250_000));
                assert_eq!(actual.inputs_consumed, 0);
            } else if !wakes {
                assert_eq!(actual.reason, StopReason::PowerStopped);
                assert_eq!(actual.stopped_at, 250_000);
            } else {
                assert!(actual.stopped_at >= Machine::CYCLES_PER_FRAME);
            }
            assert_eq!(full.write_state().unwrap(), selective.write_state().unwrap());
        }
    }
}
