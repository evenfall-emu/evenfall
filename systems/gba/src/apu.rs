//! Deterministic GBA sound hardware state, translated from ZiGBA `apu.zig`.

pub use evenfall_audio::StereoSample;
use evenfall_state::{Codec, Error, Result, Sink, Source};

const SOUND1_CONTROL_L: u32 = 0x0400_0060;
const SOUND1_CONTROL_H: u32 = 0x0400_0062;
const SOUND1_CONTROL_X: u32 = 0x0400_0064;
const SOUND2_CONTROL_L: u32 = 0x0400_0068;
const SOUND2_CONTROL_H: u32 = 0x0400_006c;
const SOUND3_CONTROL_L: u32 = 0x0400_0070;
const SOUND3_CONTROL_H: u32 = 0x0400_0072;
const SOUND3_CONTROL_X: u32 = 0x0400_0074;
const SOUND4_CONTROL_L: u32 = 0x0400_0078;
const SOUND4_CONTROL_H: u32 = 0x0400_007c;
const SOUND_CONTROL_L: u32 = 0x0400_0080;
const SOUND_CONTROL_H: u32 = 0x0400_0082;
const SOUND_CONTROL_X: u32 = 0x0400_0084;
const SOUND_BIAS: u32 = 0x0400_0088;
const SOUND_CONTROL_H_MASK: u16 = 0x770f;
const SOUND_BIAS_MASK: u16 = 0xc3fe;
const FIFO_A_LOW: u32 = 0x0400_00a0;
const FIFO_B_LOW: u32 = 0x0400_00a4;
const WAVE_RAM_BASE: u32 = 0x0400_0090;
const WAVE_RAM_END: u32 = 0x0400_00a0;
const SAMPLE_QUEUE_CAPACITY: usize = 2048;

fn replace_byte(value: u16, address: u32, byte: u8) -> u16 {
    let shift = if address & 1 == 0 { 0 } else { 8 };
    (value & !(0xff << shift)) | (u16::from(byte) << shift)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Codec)]
pub struct SquareChannel {
    #[codec(bits = 2)]
    duty: u8, // u2
    #[codec(bits = 4)]
    initial_volume: u8, // u4
    #[codec(bits = 4)]
    current_volume: u8, // u4
    #[codec(bits = 3)]
    envelope_period: u8, // u3
    #[codec(bits = 3)]
    envelope_counter: u8, // u3
    envelope_increase: bool,
    dac_enabled: bool,
    #[codec(bits = 11)]
    pub frequency: u16, // u11
    #[codec(bits = 7)]
    length: u8, // u7
    length_enabled: bool,
    #[codec(bits = 3)]
    phase_index: u8, // u3
    phase_cycles: u64,
    pub enabled: bool,
}

impl SquareChannel {
    fn write_duty_envelope(&mut self, value: u16) {
        self.write_duty_length(value as u8);
        self.write_envelope((value >> 8) as u8);
    }

    fn write_duty_length(&mut self, value: u8) {
        self.duty = value >> 6;
        self.length = 64 - (value & 0x3f);
    }

    fn write_envelope(&mut self, value: u8) {
        self.initial_volume = value >> 4;
        self.envelope_period = value & 7;
        self.envelope_increase = value & 8 != 0;
        self.dac_enabled = value & 0xf8 != 0;
        if !self.dac_enabled {
            self.enabled = false;
        }
    }

    fn write_frequency_control(&mut self, value: u16, extra_length_clock: bool) {
        self.write_frequency_low(value as u8);
        self.write_frequency_high((value >> 8) as u8, extra_length_clock);
    }

    fn write_frequency_low(&mut self, value: u8) {
        self.frequency = (self.frequency & 0x700) | u16::from(value);
    }

    fn write_frequency_high(&mut self, value: u8, extra_length_clock: bool) {
        self.frequency = (self.frequency & 0xff) | (u16::from(value & 7) << 8);
        let was_length_enabled = self.length_enabled;
        self.length_enabled = value & 0x40 != 0;
        if !was_length_enabled && self.length_enabled && extra_length_clock {
            self.clock_length();
        }
        if value & 0x80 != 0 {
            if self.length == 0 {
                self.length = 64;
                if self.length_enabled && extra_length_clock {
                    self.clock_length();
                }
            }
            self.current_volume = self.initial_volume;
            self.envelope_counter = self.envelope_period;
            self.phase_cycles = 0;
            self.enabled = self.dac_enabled;
        }
    }

    fn elapse_enabled(&mut self, cycles: u64) {
        let period = 16 * (2048 - u64::from(self.frequency));
        let accumulated = self.phase_cycles + cycles;
        if accumulated < period {
            self.phase_cycles = accumulated;
            return;
        }
        let steps = accumulated / period;
        self.phase_cycles = accumulated % period;
        self.phase_index = ((u64::from(self.phase_index) + steps) % 8) as u8;
    }

    const DUTY_PATTERNS: [u8; 4] = [0b1000_0000, 0b1000_0001, 0b1110_0001, 0b0111_1110];

    fn sample(&self) -> i16 {
        if !self.enabled {
            return 0;
        }
        let high = (Self::DUTY_PATTERNS[self.duty as usize] >> self.phase_index) & 1 != 0;
        if high { i16::from(self.current_volume) } else { 0 }
    }

    fn clock_length(&mut self) {
        if !self.length_enabled || self.length == 0 {
            return;
        }
        self.length -= 1;
        if self.length == 0 {
            self.enabled = false;
        }
    }

    fn clock_envelope(&mut self) {
        if !self.enabled || self.envelope_period == 0 {
            return;
        }
        if self.envelope_counter > 1 {
            self.envelope_counter -= 1;
            return;
        }
        self.envelope_counter = self.envelope_period;
        if self.envelope_increase {
            if self.current_volume < 15 {
                self.current_volume += 1;
            }
        } else if self.current_volume > 0 {
            self.current_volume -= 1;
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Codec)]
struct FrequencySweep {
    #[codec(bits = 3)]
    period: u8, // u3
    decrease: bool,
    #[codec(bits = 3)]
    shift: u8, // u3
    #[codec(bits = 4)]
    counter: u8, // u4
    #[codec(bits = 11)]
    shadow_frequency: u16, // u11
    enabled: bool,
}

impl FrequencySweep {
    fn write(&mut self, value: u16) {
        self.period = ((value >> 4) & 7) as u8;
        self.decrease = value & 8 != 0;
        self.shift = (value & 7) as u8;
    }

    fn trigger(&mut self, frequency: u16) -> bool {
        self.shadow_frequency = frequency;
        self.counter = if self.period == 0 { 8 } else { self.period };
        self.enabled = self.period != 0 || self.shift != 0;
        self.shift == 0 || self.calculate(frequency).is_some()
    }

    fn clock(&mut self, square: &mut SquareChannel) -> bool {
        if !self.enabled {
            return true;
        }
        self.counter -= 1;
        if self.counter != 0 {
            return true;
        }
        self.counter = if self.period == 0 { 8 } else { self.period };
        if self.shift == 0 {
            return true;
        }
        let Some(frequency) = self.calculate(self.shadow_frequency) else {
            return false;
        };
        self.shadow_frequency = frequency;
        square.frequency = frequency;
        self.calculate(frequency).is_some()
    }

    fn calculate(&self, frequency: u16) -> Option<u16> {
        let delta = frequency >> self.shift;
        if self.decrease {
            return Some(frequency - delta);
        }
        let result = frequency + delta;
        if result > 2047 { None } else { Some(result) }
    }
}

fn rotate_nibbles_left(span: &mut [u8], nibbles: u64) {
    let n = nibbles % (span.len() as u64 * 2);
    if n == 0 {
        return;
    }
    let mut tmp = [0u8; 32];
    let buf = &mut tmp[..span.len()];
    let byte_shift = (n / 2) as usize;
    if n.is_multiple_of(2) {
        for (i, byte) in buf.iter_mut().enumerate() {
            *byte = span[(i + byte_shift) % span.len()];
        }
    } else {
        for (i, byte) in buf.iter_mut().enumerate() {
            let hi = span[(i + byte_shift) % span.len()];
            let lo = span[(i + byte_shift + 1) % span.len()];
            *byte = (hi << 4) | (lo >> 4);
        }
    }
    span.copy_from_slice(buf);
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Codec)]
pub struct WaveChannel {
    pub ram: [u8; 32],
    two_bank: bool,
    #[codec(bits = 1)]
    selected_bank: u8, // u1
    dac_enabled: bool,
    enabled: bool,
    #[codec(bits = 2)]
    volume: u8, // u2
    force_75_percent: bool,
    #[codec(bits = 11)]
    frequency: u16, // u11
    phase_cycles: u64,
    #[codec(bits = 9)]
    length: u16, // u9
    length_enabled: bool,
}

impl WaveChannel {
    fn write_bank_control(&mut self, value: u16) {
        self.two_bank = value & 0x20 != 0;
        self.selected_bank = u8::from(value & 0x40 != 0);
        self.dac_enabled = value & 0x80 != 0;
        if !self.dac_enabled {
            self.enabled = false;
        }
    }
    fn write_length_volume(&mut self, value: u16) {
        self.write_length(value as u8);
        self.write_volume((value >> 8) as u8);
    }
    fn write_length(&mut self, value: u8) {
        self.length = 256 - u16::from(value);
    }
    fn write_volume(&mut self, value: u8) {
        self.volume = (value >> 5) & 3;
        self.force_75_percent = value & 0x80 != 0;
    }
    fn write_frequency_control(&mut self, value: u16, extra_length_clock: bool) {
        self.write_frequency_low(value as u8);
        self.write_frequency_high((value >> 8) as u8, extra_length_clock);
    }
    fn write_frequency_low(&mut self, value: u8) {
        self.frequency = (self.frequency & 0x700) | u16::from(value);
    }
    fn write_frequency_high(&mut self, value: u8, extra_length_clock: bool) {
        self.frequency = (self.frequency & 0xff) | (u16::from(value & 7) << 8);
        let was_length_enabled = self.length_enabled;
        self.length_enabled = value & 0x40 != 0;
        if !was_length_enabled && self.length_enabled && extra_length_clock {
            self.clock_length();
        }
        if value & 0x80 != 0 {
            if self.length == 0 {
                self.length = 256;
                if self.length_enabled && extra_length_clock {
                    self.clock_length();
                }
            }
            self.phase_cycles = 0;
            self.enabled = self.dac_enabled;
        }
    }
    fn read_ram_half(&self, address: u32, master_enabled: bool) -> u16 {
        let index = self.ram_index(address, master_enabled);
        u16::from(self.ram[index]) | (u16::from(self.ram[index + 1]) << 8)
    }
    fn write_ram_half(&mut self, address: u32, value: u16, master_enabled: bool) {
        let index = self.ram_index(address, master_enabled);
        self.ram[index] = value as u8;
        self.ram[index + 1] = (value >> 8) as u8;
    }
    fn ram_index(&self, address: u32, master_enabled: bool) -> usize {
        let cpu_bank = if master_enabled { self.selected_bank ^ 1 } else { 1 };
        usize::from(cpu_bank) * 16 + (address - WAVE_RAM_BASE) as usize
    }
    fn elapse_enabled(&mut self, cycles: u64) {
        let period = 8 * (2048 - u64::from(self.frequency));
        let accumulated = self.phase_cycles + cycles;
        if accumulated < period {
            self.phase_cycles = accumulated;
            return;
        }
        let steps = accumulated / period;
        self.phase_cycles = accumulated % period;
        if steps == 0 {
            return;
        }
        let span = if self.two_bank {
            &mut self.ram[..]
        } else {
            let start = usize::from(self.selected_bank) * 16;
            &mut self.ram[start..start + 16]
        };
        rotate_nibbles_left(span, steps);
    }
    #[inline]
    fn silent(&self) -> bool {
        !self.enabled || self.volume == 0 && !self.force_75_percent
    }
    fn sample(&self) -> i16 {
        if self.silent() {
            return 0;
        }
        let tap = if self.two_bank { 0 } else { usize::from(self.selected_bank) * 16 };
        let raw = i16::from(self.ram[tap] >> 4);
        if self.force_75_percent {
            return raw * 3 / 4;
        }
        raw >> (self.volume - 1)
    }
    fn clock_length(&mut self) {
        if !self.length_enabled || self.length == 0 {
            return;
        }
        self.length -= 1;
        if self.length == 0 {
            self.enabled = false;
        }
    }
    fn reset_registers(&mut self) {
        let ram = self.ram;
        *self = Self { ram, ..Self::default() };
    }
}

const fn lfsr_advance_combined(state: u16, feedback: u16) -> u16 {
    let low = state & 1;
    let shifted = (state & 0x7fff) >> 1;
    (if low != 0 { shifted ^ feedback } else { shifted }) | (low << 15)
}

const fn noise_jump_tables(feedback: u16) -> [[u16; 256]; 8] {
    let mut tables = [[0; 256]; 8];
    let mut steps = 1;
    while steps <= 8 {
        let mut index = 0;
        while index < 256 {
            let mut state = (index & ((1 << steps) - 1)) as u16;
            let mut count = 0;
            while count < steps {
                state = lfsr_advance_combined(state, feedback);
                count += 1;
            }
            tables[steps - 1][index] = state;
            index += 1;
        }
        steps += 1;
    }
    tables
}

const NOISE_JUMP_WIDE: [[u16; 256]; 8] = noise_jump_tables(0x6000);
const NOISE_JUMP_NARROW: [[u16; 256]; 8] = noise_jump_tables(0x0060);

#[inline]
fn lfsr_jump(state: u16, tables: &[[u16; 256]; 8], steps: u64) -> u16 {
    debug_assert!((1..=8).contains(&steps));
    let mask = (1u16 << steps) - 1;
    ((state & 0x7fff) >> steps) ^ tables[(steps - 1) as usize][(state & mask) as usize]
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Codec)]
struct NoiseChannel {
    #[codec(bits = 4)]
    initial_volume: u8, // u4
    #[codec(bits = 4)]
    current_volume: u8, // u4
    #[codec(bits = 3)]
    envelope_period: u8, // u3
    #[codec(bits = 3)]
    envelope_counter: u8, // u3
    envelope_increase: bool,
    dac_enabled: bool,
    #[codec(bits = 7)]
    length: u8, // u7
    length_enabled: bool,
    #[codec(bits = 3)]
    divisor: u8, // u3
    #[codec(bits = 4)]
    clock_shift: u8, // u4
    narrow: bool,
    #[codec(bits = 15)]
    lfsr: u16, // u15
    phase_cycles: u64,
    output_high: bool,
    enabled: bool,
}

impl NoiseChannel {
    fn write_length_envelope(&mut self, value: u16) {
        self.write_length(value as u8);
        self.write_envelope((value >> 8) as u8);
    }
    fn write_length(&mut self, value: u8) {
        self.length = 64 - (value & 0x3f);
    }
    fn write_envelope(&mut self, value: u8) {
        self.envelope_period = value & 7;
        self.envelope_increase = value & 8 != 0;
        self.initial_volume = value >> 4;
        self.dac_enabled = value & 0xf8 != 0;
        if !self.dac_enabled {
            self.enabled = false;
        }
    }
    fn write_control(&mut self, value: u16, extra_length_clock: bool) {
        self.write_polynomial(value as u8);
        self.write_trigger((value >> 8) as u8, extra_length_clock);
    }
    fn write_polynomial(&mut self, value: u8) {
        self.divisor = value & 7;
        self.narrow = value & 8 != 0;
        self.clock_shift = value >> 4;
    }
    fn write_trigger(&mut self, value: u8, extra_length_clock: bool) {
        let was_length_enabled = self.length_enabled;
        self.length_enabled = value & 0x40 != 0;
        if !was_length_enabled && self.length_enabled && extra_length_clock {
            self.clock_length();
        }
        if value & 0x80 != 0 {
            if self.length == 0 {
                self.length = 64;
                if self.length_enabled && extra_length_clock {
                    self.clock_length();
                }
            }
            self.current_volume = self.initial_volume;
            self.envelope_counter = self.envelope_period;
            self.lfsr = if self.narrow { 0x0040 } else { 0x4000 };
            self.phase_cycles = 0;
            self.output_high = false;
            self.enabled = self.dac_enabled;
        }
    }
    fn elapse_enabled(&mut self, cycles: u64) {
        let divisor_factor = if self.divisor == 0 { 1 } else { 2 * u64::from(self.divisor) };
        let period = (divisor_factor << self.clock_shift) * 32;
        let accumulated = self.phase_cycles + cycles;
        if accumulated < period {
            self.phase_cycles = accumulated;
            return;
        }
        let mut steps = accumulated / period;
        self.phase_cycles = accumulated % period;
        let mut state = self.lfsr | (u16::from(self.output_high) << 15);
        let tables = if self.narrow { &NOISE_JUMP_NARROW } else { &NOISE_JUMP_WIDE };
        while steps >= 8 {
            state = lfsr_jump(state, tables, 8);
            steps -= 8;
        }
        if steps != 0 {
            state = lfsr_jump(state, tables, steps);
        }
        self.lfsr = state & 0x7fff;
        self.output_high = state & 0x8000 != 0;
    }
    fn sample(&self) -> i16 {
        if !self.enabled || !self.output_high { 0 } else { i16::from(self.current_volume) }
    }
    fn clock_length(&mut self) {
        if !self.length_enabled || self.length == 0 {
            return;
        }
        self.length -= 1;
        if self.length == 0 {
            self.enabled = false;
        }
    }
    fn clock_envelope(&mut self) {
        if !self.enabled || self.envelope_period == 0 {
            return;
        }
        if self.envelope_counter > 1 {
            self.envelope_counter -= 1;
            return;
        }
        self.envelope_counter = self.envelope_period;
        if self.envelope_increase {
            if self.current_volume < 15 {
                self.current_volume += 1;
            }
        } else if self.current_volume > 0 {
            self.current_volume -= 1;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FifoId {
    A,
    B,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Codec)]
struct DirectSoundFifo {
    words: [u32; 8],
    #[codec(bits = 3)]
    read_index: u8, // u3
    #[codec(bits = 3)]
    write_index: u8, // u3
    playback_word: u32,
    #[codec(bits = 3)]
    playback_bytes_remaining: u8, // u3
}

impl DirectSoundFifo {
    fn push_word(&mut self, value: u32) {
        self.words[usize::from(self.write_index)] = value;
        self.write_index = (self.write_index + 1) & 7;
    }
    fn push_partial_word(&mut self, value: u32, lane: u8, byte_count: u8) {
        if lane == 0 && byte_count == 4 {
            self.push_word(value);
            return;
        }
        let mut word = self.words[usize::from(self.write_index)];
        for byte_index in 0..byte_count {
            let shift = u32::from(lane + byte_index) * 8;
            let mask = 0xffu32 << shift;
            word = (word & !mask) | (((value >> (u32::from(byte_index) * 8)) & 0xff) << shift);
        }
        self.push_word(word);
    }
    fn queued_words(&self) -> u8 {
        if self.write_index >= self.read_index {
            self.write_index - self.read_index
        } else {
            8 - self.read_index + self.write_index
        }
    }
    fn needs_refill(&self) -> bool {
        self.queued_words() < 4
    }
    fn overflows_until_refill(&self) -> u64 {
        let queued = self.queued_words();
        if queued < 4 {
            return 1;
        }
        u64::from(self.playback_bytes_remaining) + 2 + 4 * u64::from(queued - 4)
    }
    fn next_sample(&mut self) -> i8 {
        if self.playback_bytes_remaining == 0 {
            if self.queued_words() == 0 {
                return 0;
            }
            self.playback_word = self.words[usize::from(self.read_index)];
            self.playback_bytes_remaining = 4;
            self.read_index = (self.read_index + 1) & 7;
        }
        let value = self.playback_word as u8 as i8;
        self.playback_word >>= 8;
        self.playback_bytes_remaining -= 1;
        value
    }
    fn clear_queued_words(&mut self) {
        self.read_index = 0;
        self.write_index = 0;
    }
}

/// Delegated FIFO-only timer overflows, with absolute next cycles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FifoClock {
    pub next: [u64; 2],
    pub period: [u64; 2],
}

impl Default for FifoClock {
    fn default() -> Self {
        Self { next: [Self::inactive; 2], period: [0; 2] }
    }
}
impl FifoClock {
    #[allow(non_upper_case_globals)] // ZiGBA's public FifoClock.inactive name.
    pub const inactive: u64 = u64::MAX;
    pub fn delegated(&self) -> bool {
        self.next[0].min(self.next[1]) != Self::inactive
    }
}

/// Master-clock sound hardware and the downstream host sample queue.
#[derive(Clone, Debug, PartialEq, Eq, Codec)]
#[codec(module = "zigba.apu.state.v5")]
pub struct Apu {
    pub sound1_control_l: u16,
    pub sound1_control_h: u16,
    pub sound1_control_x: u16,
    pub sound2_control_l: u16,
    pub sound2_control_h: u16,
    pub sound3_control_l: u16,
    pub sound3_control_h: u16,
    pub sound3_control_x: u16,
    pub sound4_control_l: u16,
    pub sound4_control_h: u16,
    pub sound_control_l: u16,
    pub sound_control_h: u16,
    pub master_enabled: bool,
    pub sound_bias: u16,
    fifo_a: DirectSoundFifo,
    fifo_b: DirectSoundFifo,
    pub sample_a: i8,
    pub sample_b: i8,
    #[codec(encode_with = Apu::encode_square1)]
    square1: SquareChannel,
    sweep1: FrequencySweep,
    #[codec(encode_with = Apu::encode_square2)]
    pub square2: SquareChannel,
    pub wave: WaveChannel,
    #[codec(encode_with = Apu::encode_noise)]
    noise: NoiseChannel,
    #[codec(skip)]
    pending_square_noise_cycles: u64,
    sample_phase: u64,
    frame_phase: u64,
    #[codec(bits = 3)]
    frame_step: u8, // u3
    #[codec(skip)]
    samples: [StereoSample; SAMPLE_QUEUE_CAPACITY],
    #[codec(skip)]
    sample_read_index: usize,
    #[codec(skip)]
    sample_write_index: usize,
    #[codec(skip)]
    sample_count: usize,
    #[codec(skip)]
    pub sample_output_enabled: bool,
    #[codec(skip)]
    pub high_speed_audio_enabled: bool,
}

impl Default for Apu {
    fn default() -> Self {
        Self {
            sound1_control_l: 0,
            sound1_control_h: 0,
            sound1_control_x: 0,
            sound2_control_l: 0,
            sound2_control_h: 0,
            sound3_control_l: 0,
            sound3_control_h: 0,
            sound3_control_x: 0,
            sound4_control_l: 0,
            sound4_control_h: 0,
            sound_control_l: 0,
            sound_control_h: 0,
            master_enabled: false,
            sound_bias: 0x200,
            fifo_a: DirectSoundFifo::default(),
            fifo_b: DirectSoundFifo::default(),
            sample_a: 0,
            sample_b: 0,
            square1: SquareChannel::default(),
            sweep1: FrequencySweep::default(),
            square2: SquareChannel::default(),
            wave: WaveChannel::default(),
            noise: NoiseChannel::default(),
            pending_square_noise_cycles: 0,
            sample_phase: 0,
            frame_phase: 0,
            frame_step: 7,
            samples: [StereoSample::default(); SAMPLE_QUEUE_CAPACITY],
            sample_read_index: 0,
            sample_write_index: 0,
            sample_count: 0,
            sample_output_enabled: true,
            high_speed_audio_enabled: false,
        }
    }
}

impl Apu {
    fn encode_square1(&self, sink: &mut Sink<'_>) -> Result<()> {
        self.materialized_square_noise().0.encode(sink)
    }
    fn encode_square2(&self, sink: &mut Sink<'_>) -> Result<()> {
        self.materialized_square_noise().1.encode(sink)
    }
    fn encode_noise(&self, sink: &mut Sink<'_>) -> Result<()> {
        self.materialized_square_noise().2.encode(sink)
    }

    pub fn write_state(&self, sink: &mut Sink<'_>) -> Result<()> {
        self.validate_state()?;
        self.encode(sink)
    }

    pub fn write_state_schema(sink: &mut Sink<'_>) -> Result<()> {
        Self::schema(sink)
    }

    pub fn read_state(&mut self, source: &mut Source<'_>) -> Result<()> {
        self.decode(source)?;
        self.pending_square_noise_cycles = 0;
        self.validate_state()?;
        self.samples.fill(StereoSample::default());
        self.clear_samples();
        Ok(())
    }

    fn validate_state(&self) -> Result<()> {
        if self.sample_phase >= self.sample_interval()
            || self.frame_phase >= 32_768
            || self.pending_square_noise_cycles > self.frame_phase
            || (self.sweep1.enabled && !(1..=8).contains(&self.sweep1.counter))
            || self.fifo_a.playback_bytes_remaining > 4
            || self.fifo_b.playback_bytes_remaining > 4
        {
            return Err(Error::InvalidSaveState);
        }
        Ok(())
    }

    fn psg_blocked(&self, address: u32) -> bool {
        !self.master_enabled && (SOUND1_CONTROL_L..SOUND_CONTROL_H).contains(&address)
    }

    pub fn read_half(&self, address: u32) -> Option<u16> {
        if (WAVE_RAM_BASE..WAVE_RAM_END).contains(&address) && address & 1 == 0 {
            return Some(self.wave.read_ram_half(address, self.master_enabled));
        }
        Some(match address {
            SOUND1_CONTROL_L => self.sound1_control_l,
            SOUND1_CONTROL_H => self.sound1_control_h,
            SOUND1_CONTROL_X => self.sound1_control_x,
            SOUND2_CONTROL_L => self.sound2_control_l,
            SOUND2_CONTROL_H => self.sound2_control_h,
            SOUND3_CONTROL_L => self.sound3_control_l,
            SOUND3_CONTROL_H => self.sound3_control_h,
            SOUND3_CONTROL_X => self.sound3_control_x,
            SOUND4_CONTROL_L => self.sound4_control_l,
            SOUND4_CONTROL_H => self.sound4_control_h,
            SOUND_CONTROL_L => self.sound_control_l,
            SOUND_CONTROL_H => self.sound_control_h,
            SOUND_CONTROL_X => {
                (u16::from(self.master_enabled) << 7)
                    | u16::from(self.square1.enabled)
                    | (u16::from(self.square2.enabled) << 1)
                    | (u16::from(self.wave.enabled) << 2)
                    | (u16::from(self.noise.enabled) << 3)
            }
            SOUND_BIAS => self.sound_bias,
            _ => return None,
        })
    }

    pub fn write_byte(&mut self, address: u32, value: u8) -> bool {
        if (WAVE_RAM_BASE..WAVE_RAM_END).contains(&address) {
            let index =
                self.wave.ram_index(address & !1, self.master_enabled) + (address & 1) as usize;
            self.wave.ram[index] = value;
            return true;
        }
        if self.psg_blocked(address) {
            return true;
        }
        let aligned = address & !1;
        let high_lane = address & 1 != 0;
        let extra_length_clock = self.frame_step & 1 == 0;
        match aligned {
            SOUND1_CONTROL_L => {
                self.flush_square_noise();
                self.sound1_control_l = replace_byte(self.sound1_control_l, address, value) & 0x7f;
                self.sweep1.write(self.sound1_control_l);
            }
            SOUND1_CONTROL_H => {
                self.flush_square_noise();
                self.sound1_control_h =
                    replace_byte(self.sound1_control_h, address, value) & 0xffc0;
                if high_lane {
                    self.square1.write_envelope(value);
                } else {
                    self.square1.write_duty_length(value);
                }
            }
            SOUND1_CONTROL_X => {
                self.flush_square_noise();
                self.sound1_control_x =
                    replace_byte(self.sound1_control_x, address, value) & 0x4000;
                if high_lane {
                    self.square1.write_frequency_high(value, extra_length_clock);
                    if value & 0x80 != 0 && !self.sweep1.trigger(self.square1.frequency) {
                        self.square1.enabled = false;
                    }
                } else {
                    self.square1.write_frequency_low(value);
                }
            }
            SOUND2_CONTROL_L => {
                self.flush_square_noise();
                self.sound2_control_l =
                    replace_byte(self.sound2_control_l, address, value) & 0xffc0;
                if high_lane {
                    self.square2.write_envelope(value);
                } else {
                    self.square2.write_duty_length(value);
                }
            }
            SOUND2_CONTROL_H => {
                self.flush_square_noise();
                self.sound2_control_h =
                    replace_byte(self.sound2_control_h, address, value) & 0x4000;
                if high_lane {
                    self.square2.write_frequency_high(value, extra_length_clock);
                } else {
                    self.square2.write_frequency_low(value);
                }
            }
            SOUND3_CONTROL_L => {
                self.sound3_control_l = replace_byte(self.sound3_control_l, address, value) & 0xe0;
                if !high_lane {
                    self.wave.write_bank_control(u16::from(value));
                }
            }
            SOUND3_CONTROL_H => {
                self.sound3_control_h =
                    replace_byte(self.sound3_control_h, address, value) & 0xe000;
                if high_lane {
                    self.wave.write_volume(value);
                } else {
                    self.wave.write_length(value);
                }
            }
            SOUND3_CONTROL_X => {
                self.sound3_control_x =
                    replace_byte(self.sound3_control_x, address, value) & 0x4000;
                if high_lane {
                    self.wave.write_frequency_high(value, extra_length_clock);
                } else {
                    self.wave.write_frequency_low(value);
                }
            }
            SOUND4_CONTROL_L => {
                self.flush_square_noise();
                self.sound4_control_l =
                    replace_byte(self.sound4_control_l, address, value) & 0xff00;
                if high_lane {
                    self.noise.write_envelope(value);
                } else {
                    self.noise.write_length(value);
                }
            }
            SOUND4_CONTROL_H => {
                self.flush_square_noise();
                self.sound4_control_h =
                    replace_byte(self.sound4_control_h, address, value) & 0x40ff;
                if high_lane {
                    self.noise.write_trigger(value, extra_length_clock);
                } else {
                    self.noise.write_polynomial(value);
                }
            }
            SOUND_CONTROL_L | SOUND_CONTROL_H | SOUND_CONTROL_X | SOUND_BIAS => {
                let current = self.read_half(aligned).unwrap();
                let _ = self.write_half(aligned, replace_byte(current, address, value));
            }
            _ => return false,
        }
        true
    }

    pub fn write_half(&mut self, address: u32, value: u16) -> bool {
        if (WAVE_RAM_BASE..WAVE_RAM_END).contains(&address) && address & 1 == 0 {
            self.wave.write_ram_half(address, value, self.master_enabled);
            return true;
        }
        if self.psg_blocked(address) {
            return true;
        }
        let extra_length_clock = self.frame_step & 1 == 0;
        match address {
            SOUND1_CONTROL_L => {
                self.flush_square_noise();
                self.sound1_control_l = value & 0x7f;
                self.sweep1.write(value);
            }
            SOUND1_CONTROL_H => {
                self.flush_square_noise();
                self.sound1_control_h = value & 0xffc0;
                self.square1.write_duty_envelope(value);
            }
            SOUND1_CONTROL_X => {
                self.flush_square_noise();
                self.sound1_control_x = value & 0x4000;
                self.square1.write_frequency_control(value, extra_length_clock);
                if value & 0x8000 != 0 && !self.sweep1.trigger(self.square1.frequency) {
                    self.square1.enabled = false;
                }
            }
            SOUND2_CONTROL_L => {
                self.flush_square_noise();
                self.sound2_control_l = value & 0xffc0;
                self.square2.write_duty_envelope(value);
            }
            SOUND2_CONTROL_H => {
                self.flush_square_noise();
                self.sound2_control_h = value & 0x4000;
                self.square2.write_frequency_control(value, extra_length_clock);
            }
            SOUND3_CONTROL_L => {
                self.sound3_control_l = value & 0xe0;
                self.wave.write_bank_control(value);
            }
            SOUND3_CONTROL_H => {
                self.sound3_control_h = value & 0xe000;
                self.wave.write_length_volume(value);
            }
            SOUND3_CONTROL_X => {
                self.sound3_control_x = value & 0x4000;
                self.wave.write_frequency_control(value, extra_length_clock);
            }
            SOUND4_CONTROL_L => {
                self.flush_square_noise();
                self.sound4_control_l = value & 0xff00;
                self.noise.write_length_envelope(value);
            }
            SOUND4_CONTROL_H => {
                self.flush_square_noise();
                self.sound4_control_h = value & 0x40ff;
                self.noise.write_control(value, extra_length_clock);
            }
            SOUND_CONTROL_L => self.sound_control_l = value & 0xff77,
            SOUND_CONTROL_H => {
                self.sound_control_h = value & SOUND_CONTROL_H_MASK;
                if value & 0x800 != 0 {
                    self.fifo_a.clear_queued_words();
                }
                if value & 0x8000 != 0 {
                    self.fifo_b.clear_queued_words();
                }
            }
            SOUND_CONTROL_X => {
                self.flush_square_noise();
                let was_enabled = self.master_enabled;
                self.master_enabled = value & 0x80 != 0;
                if !self.master_enabled {
                    self.sound1_control_l = 0;
                    self.sound1_control_h = 0;
                    self.sound1_control_x = 0;
                    self.sound2_control_l = 0;
                    self.sound2_control_h = 0;
                    self.sound3_control_l = 0;
                    self.sound3_control_h = 0;
                    self.sound3_control_x = 0;
                    self.sound4_control_l = 0;
                    self.sound4_control_h = 0;
                    self.sound_control_l = 0;
                    self.sound_control_h &= 0xff00;
                    self.square1 = SquareChannel::default();
                    self.sweep1 = FrequencySweep::default();
                    self.square2 = SquareChannel::default();
                    self.wave.reset_registers();
                    self.noise = NoiseChannel::default();
                    self.frame_phase = 0;
                    self.frame_step = 7;
                } else if !was_enabled {
                    self.frame_phase = 0;
                    self.frame_step = 7;
                }
            }
            SOUND_BIAS => {
                self.flush_square_noise();
                self.sound_bias = value & SOUND_BIAS_MASK;
                self.sample_phase %= self.sample_interval();
            }
            _ => return false,
        }
        true
    }

    /// BIOS RegisterRamReset bit 6, without rewriting Direct Sound FIFO ports.
    pub fn reset_registers(&mut self) {
        for address in [
            SOUND1_CONTROL_L,
            SOUND1_CONTROL_H,
            SOUND1_CONTROL_X,
            SOUND2_CONTROL_L,
            SOUND2_CONTROL_H,
            SOUND3_CONTROL_L,
            SOUND3_CONTROL_H,
            SOUND3_CONTROL_X,
            SOUND4_CONTROL_L,
            SOUND4_CONTROL_H,
            SOUND_CONTROL_L,
            SOUND_CONTROL_H,
            SOUND_CONTROL_X,
        ] {
            let _ = self.write_half(address, 0);
        }
        let _ = self.write_half(SOUND_BIAS, 0x200);
        self.wave.ram.fill(0);
    }

    pub fn fifo_for_address(address: u32) -> Option<FifoId> {
        match address {
            FIFO_A_LOW => Some(FifoId::A),
            FIFO_B_LOW => Some(FifoId::B),
            _ => None,
        }
    }
    pub fn accepts_fifo_write(address: u32, byte_count: u8) -> bool {
        Self::fifo_write_target(address)
            .is_some_and(|(_, lane)| u16::from(lane) + u16::from(byte_count) <= 4)
    }
    pub fn write_fifo(&mut self, address: u32, value: u32, byte_count: u8) {
        let (fifo, lane) = Self::fifo_write_target(address).unwrap();
        match fifo {
            FifoId::A => self.fifo_a.push_partial_word(value, lane, byte_count),
            FifoId::B => self.fifo_b.push_partial_word(value, lane, byte_count),
        }
    }
    fn fifo_write_target(address: u32) -> Option<(FifoId, u8)> {
        if (FIFO_A_LOW..FIFO_A_LOW + 4).contains(&address) {
            return Some((FifoId::A, (address - FIFO_A_LOW) as u8));
        }
        if (FIFO_B_LOW..FIFO_B_LOW + 4).contains(&address) {
            return Some((FifoId::B, (address - FIFO_B_LOW) as u8));
        }
        None
    }

    /// Consume delegated overflows in (from, to], after DAC and frame endpoints.
    pub fn elapse_clocked(&mut self, from: u64, to: u64, clock: &mut FifoClock) -> u8 {
        debug_assert!(from <= to);
        let mixer = if self.high_speed_audio_enabled || !self.sample_output_enabled {
            None
        } else {
            Some(Mixer::init(self))
        };
        let mut now = from;
        let mut refill_requests = 0;
        loop {
            let next = clock.next[0].min(clock.next[1]);
            if next > to {
                break;
            }
            debug_assert!(next > now);
            self.elapse_segment(next - now, mixer.as_ref());
            now = next;
            for index in 0..2 {
                if clock.next[index] != next {
                    continue;
                }
                refill_requests |= self.timer_overflow(index, 1);
                clock.next[index] += clock.period[index];
            }
        }
        self.elapse_segment(to - now, mixer.as_ref());
        refill_requests
    }
    #[inline]
    fn elapse_segment(&mut self, cycles: u64, mixer: Option<&Mixer>) {
        if let Some(mixer) = mixer {
            self.elapse_mixed(cycles, mixer);
        } else {
            self.elapse(cycles);
        }
    }
    pub fn overflows_until_refill(&self, timer_index: usize) -> Option<u64> {
        if !self.master_enabled {
            return None;
        }
        let mut overflows = None;
        if self.sound_control_h & 0x300 != 0
            && usize::from(self.sound_control_h & 0x400 != 0) == timer_index
        {
            overflows = Some(self.fifo_a.overflows_until_refill());
        }
        if self.sound_control_h & 0x3000 != 0
            && usize::from(self.sound_control_h & 0x4000 != 0) == timer_index
        {
            let b = self.fifo_b.overflows_until_refill();
            overflows = Some(overflows.map_or(b, |a: u64| a.min(b)));
        }
        overflows
    }
    pub fn fifo_needs_refill(&self, address: u32) -> bool {
        match Self::fifo_write_target(address).unwrap().0 {
            FifoId::A => self.fifo_a.needs_refill(),
            FifoId::B => self.fifo_b.needs_refill(),
        }
    }
    pub fn timer_overflow(&mut self, timer_index: usize, overflows: u64) -> u8 {
        let mut requests = 0;
        for _ in 0..overflows {
            let a_running = self.master_enabled && self.sound_control_h & 0x300 != 0;
            let b_running = self.master_enabled && self.sound_control_h & 0x3000 != 0;
            if a_running && usize::from(self.sound_control_h & 0x400 != 0) == timer_index {
                if self.fifo_a.needs_refill() {
                    requests |= 1;
                }
                self.sample_a = self.fifo_a.next_sample();
            }
            if b_running && usize::from(self.sound_control_h & 0x4000 != 0) == timer_index {
                if self.fifo_b.needs_refill() {
                    requests |= 2;
                }
                self.sample_b = self.fifo_b.next_sample();
            }
        }
        requests
    }

    fn materialized_square_noise(&self) -> (SquareChannel, SquareChannel, NoiseChannel) {
        let (mut square1, mut square2, mut noise) = (self.square1, self.square2, self.noise);
        let cycles = self.pending_square_noise_cycles;
        if cycles != 0 {
            if square1.enabled {
                square1.elapse_enabled(cycles);
            }
            if square2.enabled {
                square2.elapse_enabled(cycles);
            }
            if noise.enabled {
                noise.elapse_enabled(cycles);
            }
        }
        (square1, square2, noise)
    }

    pub fn current_sample(&self) -> StereoSample {
        if !self.master_enabled || self.high_speed_audio_enabled {
            return StereoSample::default();
        }
        let mixer = Mixer::init(self);
        if self.pending_square_noise_cycles != 0 {
            let (square1, square2, noise) = self.materialized_square_noise();
            return self.mix_channels(
                &mixer,
                [square1.sample(), square2.sample(), self.wave.sample(), noise.sample()],
            );
        }
        self.mix_current(&mixer)
    }
    #[inline]
    fn mix_current(&self, mixer: &Mixer) -> StereoSample {
        self.mix_channels(
            mixer,
            [self.square1.sample(), self.square2.sample(), self.wave.sample(), self.noise.sample()],
        )
    }
    #[inline]
    fn mix_channels(&self, mixer: &Mixer, psg: [i16; 4]) -> StereoSample {
        let a = i32::from(self.sample_a);
        let b = i32::from(self.sample_b);
        let mut left = a * mixer.fifo_left[0] + b * mixer.fifo_left[1];
        let mut right = a * mixer.fifo_right[0] + b * mixer.fifo_right[1];
        for (channel, sample) in psg.into_iter().enumerate() {
            let square = i32::from(sample) << 3;
            left += (square * mixer.psg_left[channel]) >> mixer.psg_shift;
            right += (square * mixer.psg_right[channel]) >> mixer.psg_shift;
        }
        StereoSample { left: mixer.apply_bias(left), right: mixer.apply_bias(right) }
    }

    pub fn elapse(&mut self, cycles: u64) {
        if self.high_speed_audio_enabled {
            self.elapse_silent::<false>(cycles);
            return;
        }
        if !self.sample_output_enabled {
            self.elapse_silent::<true>(cycles);
            return;
        }
        let mixer = Mixer::init(self);
        self.elapse_mixed(cycles, &mixer);
    }
    fn elapse_mixed(&mut self, cycles: u64, mixer: &Mixer) {
        let mut remaining = cycles;
        while remaining > 0 {
            let step = if self.master_enabled {
                remaining.min(32_768 - self.frame_phase)
            } else {
                remaining
            };
            if self.square_noise_audible() || !self.wave.silent() {
                self.elapse_audible(step, mixer);
            } else {
                self.elapse_constant(step, mixer);
            }
            remaining -= step;
            if self.master_enabled {
                self.frame_phase += step;
                if self.frame_phase == 32_768 {
                    self.advance_square_noise();
                    self.frame_phase = 0;
                    self.clock_frame_sequencer();
                }
            }
        }
    }
    fn elapse_audible(&mut self, cycles: u64, mixer: &Mixer) {
        let interval = self.sample_interval();
        let audible = self.square_noise_audible();
        let mut remaining = cycles;
        while remaining > 0 {
            let step = remaining.min(interval - self.sample_phase);
            if self.square1.enabled || self.square2.enabled || self.noise.enabled {
                self.pending_square_noise_cycles += step;
            }
            if self.wave.enabled {
                self.wave.elapse_enabled(step);
            }
            self.sample_phase += step;
            remaining -= step;
            if self.sample_phase == interval {
                self.sample_phase = 0;
                if audible {
                    self.advance_square_noise();
                }
                self.push_sample(if self.master_enabled {
                    self.mix_current(mixer)
                } else {
                    StereoSample::default()
                });
            }
        }
    }
    fn elapse_constant(&mut self, cycles: u64, mixer: &Mixer) {
        if self.square1.enabled || self.square2.enabled || self.noise.enabled {
            self.pending_square_noise_cycles += cycles;
        }
        if self.wave.enabled {
            self.wave.elapse_enabled(cycles);
        }
        let shift = self.sample_interval_shift();
        let phase = self.sample_phase + cycles;
        self.sample_phase = phase & ((1 << shift) - 1);
        let count = phase >> shift;
        if count == 0 {
            return;
        }
        self.push_samples(
            if self.master_enabled {
                self.mix_channels(mixer, [0; 4])
            } else {
                StereoSample::default()
            },
            count,
        );
    }

    pub fn set_sample_output(&mut self, enabled: bool) {
        if self.sample_output_enabled == enabled {
            return;
        }
        self.flush_square_noise();
        self.sample_output_enabled = enabled;
        self.clear_samples();
    }
    pub fn set_high_speed_audio(&mut self, enabled: bool) {
        if self.high_speed_audio_enabled == enabled {
            return;
        }
        self.flush_square_noise();
        self.high_speed_audio_enabled = enabled;
        self.clear_samples();
    }
    fn clear_samples(&mut self) {
        self.sample_read_index = 0;
        self.sample_write_index = 0;
        self.sample_count = 0;
    }
    fn elapse_silent<const PRESERVE_OSCILLATORS: bool>(&mut self, cycles: u64) {
        let sample_mask = self.sample_interval() - 1;
        let mut remaining = cycles;
        while remaining > 0 {
            let until_frame =
                if self.master_enabled { 32_768 - self.frame_phase } else { remaining };
            let step = remaining.min(until_frame);
            if PRESERVE_OSCILLATORS
                && (self.square1.enabled || self.square2.enabled || self.noise.enabled)
            {
                self.pending_square_noise_cycles += step;
            }
            if self.wave.enabled {
                self.wave.elapse_enabled(step);
            }
            self.sample_phase = self.sample_phase.wrapping_add(step) & sample_mask;
            if self.master_enabled {
                self.frame_phase += step;
            }
            remaining -= step;
            if self.master_enabled && self.frame_phase == 32_768 {
                if PRESERVE_OSCILLATORS {
                    self.flush_square_noise();
                }
                self.frame_phase = 0;
                self.clock_frame_sequencer();
            }
        }
    }
    fn flush_square_noise(&mut self) {
        self.advance_square_noise();
    }
    #[inline]
    fn square_noise_audible(&self) -> bool {
        (self.square1.enabled && self.square1.current_volume != 0)
            || (self.square2.enabled && self.square2.current_volume != 0)
            || (self.noise.enabled && self.noise.current_volume != 0)
    }
    #[inline]
    fn advance_square_noise(&mut self) {
        let cycles = self.pending_square_noise_cycles;
        if cycles == 0 {
            return;
        }
        if self.square1.enabled {
            self.square1.elapse_enabled(cycles);
        }
        if self.square2.enabled {
            self.square2.elapse_enabled(cycles);
        }
        if self.noise.enabled {
            self.noise.elapse_enabled(cycles);
        }
        self.pending_square_noise_cycles = 0;
    }

    pub fn drain_samples(&mut self, output: &mut [StereoSample]) -> usize {
        let count = output.len().min(self.sample_count);
        for sample in &mut output[..count] {
            *sample = self.samples[self.sample_read_index];
            self.sample_read_index = (self.sample_read_index + 1) % SAMPLE_QUEUE_CAPACITY;
        }
        self.sample_count -= count;
        count
    }
    pub fn sample_rate(&self) -> u32 {
        32_768 << (self.sound_bias >> 14)
    }
    fn sample_interval(&self) -> u64 {
        1 << self.sample_interval_shift()
    }
    fn sample_interval_shift(&self) -> u32 {
        9 - u32::from(self.sound_bias >> 14)
    }
    fn clock_frame_sequencer(&mut self) {
        self.frame_step = (self.frame_step + 1) & 7;
        match self.frame_step {
            0 | 4 => {
                self.square1.clock_length();
                self.square2.clock_length();
                self.wave.clock_length();
                self.noise.clock_length();
            }
            2 | 6 => {
                if !self.sweep1.clock(&mut self.square1) {
                    self.square1.enabled = false;
                }
                self.square1.clock_length();
                self.square2.clock_length();
                self.wave.clock_length();
                self.noise.clock_length();
            }
            7 => {
                self.square1.clock_envelope();
                self.square2.clock_envelope();
                self.noise.clock_envelope();
            }
            _ => {}
        }
    }
    fn push_sample(&mut self, sample: StereoSample) {
        self.samples[self.sample_write_index] = sample;
        self.sample_write_index = (self.sample_write_index + 1) % SAMPLE_QUEUE_CAPACITY;
        if self.sample_count < SAMPLE_QUEUE_CAPACITY {
            self.sample_count += 1;
        } else {
            self.sample_read_index = (self.sample_read_index + 1) % SAMPLE_QUEUE_CAPACITY;
        }
    }
    fn push_samples(&mut self, sample: StereoSample, count: u64) {
        let written = count.min(SAMPLE_QUEUE_CAPACITY as u64) as usize;
        let first = written.min(SAMPLE_QUEUE_CAPACITY - self.sample_write_index);
        self.samples[self.sample_write_index..self.sample_write_index + first].fill(sample);
        self.samples[..written - first].fill(sample);
        self.sample_write_index =
            ((self.sample_write_index as u64 + count) % SAMPLE_QUEUE_CAPACITY as u64) as usize;
        if count < (SAMPLE_QUEUE_CAPACITY - self.sample_count) as u64 {
            self.sample_count += count as usize;
        } else {
            self.sample_count = SAMPLE_QUEUE_CAPACITY;
            self.sample_read_index = self.sample_write_index;
        }
    }
}

#[derive(Clone, Copy)]
struct Mixer {
    fifo_left: [i32; 2],
    fifo_right: [i32; 2],
    psg_left: [i32; 4],
    psg_right: [i32; 4],
    psg_shift: u32,
    bias: i32,
    precision_mask: i32,
}
impl Mixer {
    fn init(apu: &Apu) -> Self {
        let cnt_l = i32::from(apu.sound_control_l);
        let cnt_h = apu.sound_control_h;
        let scale_a = if cnt_h & 4 != 0 { 4 } else { 2 };
        let scale_b = if cnt_h & 8 != 0 { 4 } else { 2 };
        let left_volume = ((cnt_l >> 4) & 7) + 1;
        let right_volume = (cnt_l & 7) + 1;
        let mut mixer = Self {
            fifo_left: [
                if cnt_h & 0x200 != 0 { scale_a } else { 0 },
                if cnt_h & 0x2000 != 0 { scale_b } else { 0 },
            ],
            fifo_right: [
                if cnt_h & 0x100 != 0 { scale_a } else { 0 },
                if cnt_h & 0x1000 != 0 { scale_b } else { 0 },
            ],
            psg_left: [0; 4],
            psg_right: [0; 4],
            psg_shift: 4 - u32::from((cnt_h & 3).min(2)),
            bias: i32::from(apu.sound_bias & 0x3ff),
            precision_mask: 0,
        };
        for channel in 0..4 {
            mixer.psg_left[channel] =
                if (cnt_l >> (12 + channel)) & 1 != 0 { left_volume } else { 0 };
            mixer.psg_right[channel] =
                if (cnt_l >> (8 + channel)) & 1 != 0 { right_volume } else { 0 };
        }
        let discarded_bits = u32::from(apu.sound_bias >> 14) + 1;
        mixer.precision_mask = !((1i32 << discarded_bits) - 1);
        mixer
    }
    fn apply_bias(&self, mixed: i32) -> i16 {
        let clipped = (mixed + self.bias).clamp(0, 0x3ff);
        ((clipped & self.precision_mask) - (self.bias & self.precision_mask)) as i16
    }
}

#[cfg(test)]
mod tests;
