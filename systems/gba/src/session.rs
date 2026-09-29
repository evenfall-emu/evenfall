//! Owned, host-independent session translated from `session.zig`.

use crate::bus::{KeyState, TimedInput};
use crate::cartridge::{self, save::Type as SaveType};
use crate::machine::{
    BootMode, Config as MachineConfig, Machine, RunError, RunResult, StateError, StepOutcome,
    StopReason,
};
use crate::rom_image;
use evenfall_audio::{Resampler, StereoSample};
use sha1::{Digest, Sha1};

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
    Image(rom_image::Error),
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
        Ok(self.machine.run_slice_with_input(deadline, input_deadline, inputs)?)
    }

    pub fn set_keys(&mut self, keys: KeyState) {
        self.machine.set_keys(keys);
    }
    pub fn step(&mut self) -> StepOutcome {
        self.machine.step()
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
        if interval == 1 {
            self.machine.bus.use_full_video();
        } else {
            if self.machine.fault().is_some() {
                return Err(Error::MachineFaulted);
            }
            self.machine.bus.set_video_interval(interval);
        }
        self.video_interval = interval;
        Ok(())
    }

    pub fn canonical_framebuffer(&mut self) -> Result<&[u16], Error> {
        Ok(self.machine.framebuffer())
    }

    pub fn write_state(&mut self) -> Result<Vec<u8>, Error> {
        self.canonical_framebuffer()?;
        Ok(self.machine.write_state_alloc()?)
    }

    pub fn load_state(&mut self, bytes: &[u8]) -> Result<(), Error> {
        if bytes.len() > 2 * 1024 * 1024 {
            return Err(Error::StateTooLarge);
        }
        self.machine.load_state(bytes)?;
        self.machine.set_audio_output(self.audio_output_enabled);
        self.machine.bus.set_video_interval(self.video_interval);
        self.resampler.reset();
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
        Ok(())
    }
}
