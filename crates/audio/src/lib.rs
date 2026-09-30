//! Audio sample types and allocation-free, deterministic conversion from an
//! emulated DAC rate to a host rate.
//!
//! Systems produce [`StereoSample`]s at their own DAC rate; hosts convert
//! them with a [`Resampler`]. Conversion is exact integer arithmetic, so the
//! same input stream produces the same output on every host, however the
//! stream is split into calls.

#![no_std]

/// One stereo sample on the emulated DAC's signed scale.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct StereoSample {
    pub left: i16,
    pub right: i16,
}

impl StereoSample {
    pub const fn new(left: i16, right: i16) -> Self {
        Self { left, right }
    }
}

/// A source or output rate of zero cannot define an interval.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidSampleRate;

/// How much of one [`Resampler::process`] call's input and output was used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessResult {
    pub consumed: usize,
    pub written: usize,
}

/// Linear-interpolating rate converter with an exact rational phase.
///
/// The phase counts output-rate units within the current source interval, so
/// no rounding error accumulates over any stream length.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resampler {
    source_rate: u32,
    output_rate: u32,
    phase: u64,
    previous: Option<StereoSample>,
}

impl Resampler {
    pub fn new(source_rate: u32, output_rate: u32) -> Result<Self, InvalidSampleRate> {
        if source_rate == 0 || output_rate == 0 {
            return Err(InvalidSampleRate);
        }
        Ok(Self { source_rate, output_rate, phase: 0, previous: None })
    }

    pub fn source_rate(&self) -> u32 {
        self.source_rate
    }

    pub fn output_rate(&self) -> u32 {
        self.output_rate
    }

    /// Discards interpolation history; the next input starts a new stream.
    pub fn reset(&mut self) {
        self.phase = 0;
        self.previous = None;
    }

    /// Converts as much input as fits in `output`.
    ///
    /// The caller keeps `input[result.consumed..]` for the next call. The
    /// first sample of a stream seeds interpolation and is emitted only once
    /// the following sample establishes an interval.
    pub fn process(
        &mut self,
        input: &[StereoSample],
        output: &mut [StereoSample],
    ) -> ProcessResult {
        let mut consumed = 0;
        let mut written = 0;

        if self.previous.is_none() && !input.is_empty() {
            self.previous = Some(input[0]);
            consumed = 1;
        }

        while consumed < input.len() {
            let current = input[consumed];
            let previous = self.previous.expect("a stream in progress has a previous sample");
            while self.phase < u64::from(self.output_rate) {
                if written == output.len() {
                    return ProcessResult { consumed, written };
                }
                output[written] = interpolate(previous, current, self.phase, self.output_rate);
                written += 1;
                self.phase += u64::from(self.source_rate);
            }
            self.phase -= u64::from(self.output_rate);
            self.previous = Some(current);
            consumed += 1;
        }

        ProcessResult { consumed, written }
    }
}

fn interpolate(
    first: StereoSample,
    second: StereoSample,
    phase: u64,
    denominator: u32,
) -> StereoSample {
    StereoSample {
        left: interpolate_channel(first.left, second.left, phase, denominator),
        right: interpolate_channel(first.right, second.right, phase, denominator),
    }
}

fn interpolate_channel(first: i16, second: i16, phase: u64, denominator: u32) -> i16 {
    let start = i64::from(first);
    let delta = i64::from(second) - start;
    let numerator = start * i64::from(denominator) + delta * phase as i64;
    // Truncating division, as in the reference: the result always lies
    // between the two endpoints, so it fits an i16.
    (numerator / i64::from(denominator)) as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn s(left: i16, right: i16) -> StereoSample {
        StereoSample { left, right }
    }

    const fn l(left: i16) -> StereoSample {
        StereoSample { left, right: 0 }
    }

    #[test]
    fn preserves_equal_rate_samples_across_calls() {
        let mut resampler = Resampler::new(48_000, 48_000).unwrap();
        let first = [s(10, -10), s(20, -20)];
        let second = [s(30, -30), s(40, -40)];
        let mut output = [StereoSample::default(); 4];

        let first_result = resampler.process(&first, &mut output);
        let second_result = resampler.process(&second, &mut output[first_result.written..]);

        assert_eq!(first_result, ProcessResult { consumed: 2, written: 1 });
        assert_eq!(second_result, ProcessResult { consumed: 2, written: 2 });
        assert_eq!(output[..3], [first[0], first[1], second[0]]);
    }

    #[test]
    fn reports_unconsumed_input_when_output_fills_mid_interval() {
        let mut resampler = Resampler::new(1, 4).unwrap();
        let input = [s(0, 0), s(100, -100), s(200, -200)];
        let mut first_output = [StereoSample::default(); 2];
        let mut second_output = [StereoSample::default(); 6];

        let first = resampler.process(&input, &mut first_output);
        let second = resampler.process(&input[first.consumed..], &mut second_output);

        assert_eq!(first, ProcessResult { consumed: 1, written: 2 });
        assert_eq!(first_output, [s(0, 0), s(25, -25)]);
        assert_eq!(second, ProcessResult { consumed: 2, written: 6 });
        assert_eq!(
            second_output,
            [s(50, -50), s(75, -75), s(100, -100), s(125, -125), s(150, -150), s(175, -175)]
        );
    }

    #[test]
    fn downsampling_keeps_rational_phase_without_drift() {
        let mut resampler = Resampler::new(4, 2).unwrap();
        let input = [l(0), l(10), l(20), l(30), l(40), l(50)];
        let mut output = [StereoSample::default(); 3];

        let result = resampler.process(&input, &mut output);

        assert_eq!(result, ProcessResult { consumed: input.len(), written: output.len() });
        assert_eq!([output[0].left, output[1].left, output[2].left], [0, 20, 40]);
    }

    #[test]
    fn reset_discards_interpolation_history() {
        let mut resampler = Resampler::new(2, 4).unwrap();
        let mut output = [StereoSample::default(); 2];
        resampler.process(&[l(10), l(20)], &mut output);
        resampler.reset();

        let result = resampler.process(&[l(100), l(200)], &mut output);

        assert_eq!(result.written, 2);
        assert_eq!([output[0].left, output[1].left], [100, 150]);
    }

    #[test]
    fn rejects_zero_rates() {
        assert_eq!(Resampler::new(0, 48_000), Err(InvalidSampleRate));
        assert_eq!(Resampler::new(32_768, 0), Err(InvalidSampleRate));
    }
}
