use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use evenfall_audio::StereoSample;

/// Full scale of the GBA's mixed output, as the web player converts it.
const FULL_SCALE: f32 = 512.0;

/// Samples waiting for the sound device, at most `capacity` stereo frames.
///
/// Emulation and the device run on separate clocks, so the queue absorbs
/// their drift. When it overflows, the oldest samples go, which bounds the
/// delay. When it runs dry, the device plays silence.
#[derive(Debug)]
pub struct SampleQueue {
    samples: VecDeque<[f32; 2]>,
    capacity: usize,
}

impl SampleQueue {
    pub fn new(capacity: usize) -> Self {
        Self { samples: VecDeque::with_capacity(capacity), capacity }
    }

    pub fn push(&mut self, input: &[StereoSample]) {
        for sample in input {
            if self.samples.len() == self.capacity {
                self.samples.pop_front();
            }
            self.samples.push_back(
                [sample.left, sample.right]
                    .map(|channel| (f32::from(channel) / FULL_SCALE).clamp(-1.0, 1.0)),
            );
        }
    }

    /// The next stereo frame, or silence when none is waiting.
    pub fn pop(&mut self) -> [f32; 2] {
        self.samples.pop_front().unwrap_or([0.0; 2])
    }

    pub fn clear(&mut self) {
        self.samples.clear();
    }
}

/// The default output device, fed from a [`SampleQueue`].
pub struct Audio {
    _stream: cpal::Stream,
    queue: Arc<Mutex<SampleQueue>>,
    rate: u32,
}

impl Audio {
    /// Opens the default output device at its preferred rate.
    pub fn open() -> Result<Self, String> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or_else(|| "no sound output device".to_owned())?;
        let supported = device.default_output_config().map_err(|error| error.to_string())?;
        let config = supported.config();
        let rate = config.sample_rate;
        // A tenth of a second: enough to ride out a late frame, short
        // enough that sound stays close to the picture.
        let queue = Arc::new(Mutex::new(SampleQueue::new(rate as usize / 10)));
        let stream = match supported.sample_format() {
            SampleFormat::F32 => build::<f32>(&device, config, &queue),
            SampleFormat::I16 => build::<i16>(&device, config, &queue),
            SampleFormat::U16 => build::<u16>(&device, config, &queue),
            SampleFormat::I32 => build::<i32>(&device, config, &queue),
            other => return Err(format!("unsupported sample format {other}")),
        }?;
        stream.play().map_err(|error| error.to_string())?;
        Ok(Self { _stream: stream, queue, rate })
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    pub fn push(&self, samples: &[StereoSample]) {
        if let Ok(mut queue) = self.queue.lock() {
            queue.push(samples);
        }
    }

    pub fn clear(&self) {
        if let Ok(mut queue) = self.queue.lock() {
            queue.clear();
        }
    }
}

fn build<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    queue: &Arc<Mutex<SampleQueue>>,
) -> Result<cpal::Stream, String> {
    let channels = usize::from(config.channels);
    let queue = Arc::clone(queue);
    device
        .build_output_stream(
            config,
            move |output: &mut [T], _| {
                let Ok(mut queue) = queue.lock() else { return };
                for frame in output.chunks_mut(channels) {
                    let [left, right] = queue.pop();
                    for (channel, sample) in frame.iter_mut().enumerate() {
                        let value = match (channels, channel) {
                            (1, _) => (left + right) / 2.0,
                            (_, 0) => left,
                            (_, 1) => right,
                            _ => 0.0,
                        };
                        *sample = T::from_sample(value);
                    }
                }
            },
            |error| eprintln!("sound: {error}"),
            None,
        )
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 512 is full scale, and louder samples clip instead of wrapping.
    #[test]
    fn samples_scale_to_the_web_players_range() {
        let mut queue = SampleQueue::new(4);
        queue.push(&[StereoSample::new(256, -512), StereoSample::new(1000, -1000)]);
        assert_eq!(queue.pop(), [0.5, -1.0]);
        assert_eq!(queue.pop(), [1.0, -1.0]);
    }

    /// A full queue drops its oldest samples, so the newest always play.
    #[test]
    fn overflow_drops_the_oldest_samples() {
        let mut queue = SampleQueue::new(2);
        queue.push(&[1, 2, 3].map(|n| StereoSample::new(n * 64, 0)));
        assert_eq!(queue.pop(), [0.25, 0.0]);
        assert_eq!(queue.pop(), [0.375, 0.0]);
        assert_eq!(queue.pop(), [0.0, 0.0]);
    }

    /// An empty queue plays silence rather than repeating a sample.
    #[test]
    fn underrun_plays_silence() {
        let mut queue = SampleQueue::new(2);
        queue.push(&[StereoSample::new(512, 512)]);
        assert_eq!(queue.pop(), [1.0, 1.0]);
        assert_eq!(queue.pop(), [0.0, 0.0]);
    }
}
