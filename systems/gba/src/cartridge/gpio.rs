//! Cartridge-owned GPIO port and the S3511-compatible real-time clock.

use crate::types::Cycle;
use evenfall_state::Codec;

pub const MASTER_CLOCK_HZ: Cycle = 16_777_216;

#[derive(Clone, Default, Codec)]
pub struct Gpio {
    pub read_enabled: bool,
    #[codec(bits = 4)]
    pub direction: u8,
    #[codec(bits = 4)]
    pub output_latch: u8,
    pub rtc: Option<Rtc>,
    pub solar: Option<SolarSensor>,
    pub gyro: Option<Gyro>,
    pub rumble_enabled: bool,
    pub rumble_active: bool,
}

impl Gpio {
    pub fn init() -> Self {
        Self::default()
    }
    pub fn init_rtc(epoch_seconds: u64) -> Self {
        Self { rtc: Some(Rtc::init(epoch_seconds)), ..Self::default() }
    }
    pub fn attach_rtc(&mut self, epoch_seconds: u64) {
        self.rtc = Some(Rtc::init(epoch_seconds));
    }
    pub fn attach_solar_sensor(&mut self, light_level: u8) {
        self.solar = Some(SolarSensor::init(light_level));
    }
    pub fn set_solar_level(&mut self, light_level: u8) {
        if let Some(solar) = &mut self.solar {
            solar.light_level = light_level;
        }
    }
    pub fn attach_gyro(&mut self, sample: u16) {
        self.gyro = Some(Gyro::init(sample));
    }
    pub fn set_gyro_sample(&mut self, sample: u16) {
        if let Some(gyro) = &mut self.gyro {
            gyro.sample = sample & 0xfff;
        }
    }
    pub fn attach_rumble(&mut self) {
        self.rumble_enabled = true;
    }
    pub fn is_rumbling(&self) -> bool {
        self.rumble_active
    }
    pub fn validate_state(&self, current_cycle: Cycle) -> Result<(), InvalidSaveState> {
        if !self.rumble_enabled && self.rumble_active {
            return Err(InvalidSaveState);
        }
        if let Some(rtc) = &self.rtc {
            rtc.validate_state(current_cycle)?;
        }
        Ok(())
    }
    pub fn read_byte(&self, offset: u32) -> Option<u8> {
        if !self.read_enabled {
            return None;
        }
        match offset {
            0xc4 => Some(self.pin_state()),
            0xc5 | 0xc7 | 0xc9 => Some(0),
            0xc6 => Some(self.direction),
            0xc8 => Some(u8::from(self.read_enabled)),
            _ => None,
        }
    }
    pub fn is_register(offset: u32) -> bool {
        (0xc4..=0xc9).contains(&offset)
    }
    pub fn write_half(&mut self, offset: u32, value: u16, cycle: Cycle) -> bool {
        match offset {
            0xc4 => {
                let mask = self.direction;
                self.output_latch = (self.output_latch & !mask) | ((value as u8 & 0xf) & mask);
                self.update_rtc(cycle);
            }
            0xc6 => {
                self.direction = value as u8 & 0xf;
                self.update_rtc(cycle);
            }
            0xc8 => self.read_enabled = value & 1 != 0,
            _ => {}
        }
        if let Some(rtc) = &mut self.rtc {
            return rtc.take_irq();
        }
        false
    }
    fn pin_state(&self) -> u8 {
        let outputs = self.output_latch & self.direction;
        let mut device_inputs = 0;
        if let Some(rtc) = &self.rtc {
            device_inputs |= rtc.output_pins();
        }
        if let Some(solar) = &self.solar {
            device_inputs |= solar.output_pins();
        }
        if let Some(gyro) = &self.gyro {
            device_inputs |= gyro.output_pins();
        }
        outputs | (device_inputs & !self.direction)
    }
    fn update_rtc(&mut self, cycle: Cycle) {
        let pins = self.output_latch & self.direction;
        if let Some(rtc) = &mut self.rtc {
            rtc.update_pins(pins, self.direction, cycle);
        }
        if let Some(solar) = &mut self.solar {
            solar.update_pins(pins, self.direction);
        }
        if let Some(gyro) = &mut self.gyro {
            gyro.update_pins(pins, self.direction);
        }
        self.rumble_active = self.rumble_enabled && self.direction & 8 != 0 && pins & 8 != 0;
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct InvalidSaveState;

#[derive(Clone, Default, Codec)]
pub struct Gyro {
    #[codec(bits = 12)]
    pub sample: u16,
    pub shift: u16,
    pub serial_bit: bool,
    pub previous_clock: bool,
}
impl Gyro {
    fn init(sample: u16) -> Self {
        Self { sample: sample & 0xfff, shift: 0, serial_bit: false, previous_clock: false }
    }
    fn output_pins(&self) -> u8 {
        if self.serial_bit { 1 << 2 } else { 0 }
    }
    fn update_pins(&mut self, pins: u8, direction: u8) {
        let start = direction & 1 != 0 && pins & 1 != 0;
        let clock = direction & 2 != 0 && pins & 2 != 0;
        if start {
            self.shift = self.sample;
            self.output_next_bit();
        } else if self.previous_clock && !clock {
            self.output_next_bit();
        }
        self.previous_clock = clock;
    }
    fn output_next_bit(&mut self) {
        self.serial_bit = self.shift & 0x8000 != 0;
        self.shift <<= 1;
    }
}

#[derive(Clone, Default, Codec)]
pub struct SolarSensor {
    pub light_level: u8,
    pub counter: u8,
    pub previous_clock: bool,
}
impl SolarSensor {
    fn init(light_level: u8) -> Self {
        Self { light_level, counter: 0, previous_clock: false }
    }
    fn output_pins(&self) -> u8 {
        if u16::from(self.counter) > 255 - u16::from(self.light_level) { 1 << 3 } else { 0 }
    }
    fn update_pins(&mut self, pins: u8, direction: u8) {
        let selected = direction & 4 != 0 && pins & 4 == 0;
        if !selected {
            return;
        }
        let clock = direction & 1 != 0 && pins & 1 != 0;
        let reset = direction & 2 != 0 && pins & 2 != 0;
        if reset {
            self.counter = 0;
        } else if self.previous_clock && !clock {
            self.counter = self.counter.wrapping_add(1);
        }
        self.previous_clock = clock;
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Codec)]
#[codec(tag_bits = 3)]
pub enum Transfer {
    #[default]
    Idle,
    Armed,
    Command,
    Receive,
    Send,
    Complete,
}

#[derive(Clone, Default, Codec)]
pub struct Rtc {
    pub epoch_seconds: u64,
    pub epoch_cycle: Cycle,
    pub weekday_bias: i8,
    pub control: u8,
    pub transfer: Transfer,
    pub previous_sck: bool,
    pub previous_cs: bool,
    pub output_sio: bool,
    pub shift: u8,
    #[codec(bits = 4)]
    pub bit_index: u8,
    #[codec(bits = 4)]
    pub byte_index: u8,
    #[codec(bits = 3)]
    pub register: u8,
    #[codec(bits = 4)]
    pub payload_length: u8,
    pub payload: [u8; 7],
    pub irq_pending: bool,
}
impl Rtc {
    /// 9999-12-31T23:59:59Z, the latest epoch a host may configure. Guest
    /// writes set dates up to 2099, so no reachable state exceeds it.
    pub const MAX_EPOCH_SECONDS: u64 = 253_402_300_799;

    fn init(epoch_seconds: u64) -> Self {
        Self {
            epoch_seconds,
            epoch_cycle: 0,
            weekday_bias: 0,
            control: 0x40,
            transfer: Transfer::Idle,
            previous_sck: false,
            previous_cs: false,
            output_sio: false,
            shift: 0,
            bit_index: 0,
            byte_index: 0,
            register: 0,
            payload_length: 0,
            payload: [0; 7],
            irq_pending: false,
        }
    }
    fn validate_state(&self, current_cycle: Cycle) -> Result<(), InvalidSaveState> {
        if self.bit_index >= 8
            || self.payload_length as usize > self.payload.len()
            || self.epoch_cycle > current_cycle
            // Date reads convert year by year, so a larger epoch would run
            // for years of host time.
            || self.epoch_seconds > Self::MAX_EPOCH_SECONDS
        {
            return Err(InvalidSaveState);
        }
        if matches!(self.transfer, Transfer::Send | Transfer::Receive) {
            let expected_length = match self.register {
                2 => 7,
                4 => 1,
                6 => 3,
                _ => return Err(InvalidSaveState),
            };
            if self.payload_length != expected_length || self.byte_index >= self.payload_length {
                return Err(InvalidSaveState);
            }
        }
        Ok(())
    }
    fn output_pins(&self) -> u8 {
        if self.output_sio && self.previous_cs { 1 << 1 } else { 0 }
    }
    fn take_irq(&mut self) -> bool {
        let pending = self.irq_pending;
        self.irq_pending = false;
        pending
    }
    fn update_pins(&mut self, pins: u8, direction: u8, cycle: Cycle) {
        let sck = direction & 1 != 0 && pins & 1 != 0;
        let sio = direction & 2 != 0 && pins & 2 != 0;
        let cs = direction & 4 != 0 && pins & 4 != 0;
        match self.transfer {
            Transfer::Idle => {
                if sck && !cs {
                    self.transfer = Transfer::Armed;
                }
            }
            Transfer::Armed => {
                if sck && cs {
                    self.start_command();
                } else if !sck || cs {
                    self.transfer = Transfer::Idle;
                }
            }
            Transfer::Command | Transfer::Receive | Transfer::Send | Transfer::Complete => {
                if !cs {
                    self.finish_transfer(sck);
                } else if !sck {
                    if matches!(self.transfer, Transfer::Command | Transfer::Receive) {
                        let mask = 1_u8 << self.bit_index;
                        self.shift = if sio { self.shift | mask } else { self.shift & !mask };
                    }
                    self.output_sio = false;
                } else if !self.previous_sck {
                    self.clock_rising(cycle);
                }
            }
        }
        self.previous_sck = sck;
        self.previous_cs = cs;
    }
    fn start_command(&mut self) {
        self.transfer = Transfer::Command;
        self.shift = 0;
        self.bit_index = 0;
        self.byte_index = 0;
        self.payload_length = 0;
        self.output_sio = false;
    }
    fn finish_transfer(&mut self, sck: bool) {
        self.transfer = if sck { Transfer::Armed } else { Transfer::Idle };
        self.output_sio = false;
        self.shift = 0;
        self.bit_index = 0;
        self.byte_index = 0;
    }
    fn clock_rising(&mut self, cycle: Cycle) {
        match self.transfer {
            Transfer::Command | Transfer::Receive => {
                self.bit_index += 1;
                if self.bit_index != 8 {
                    return;
                }
                self.bit_index = 0;
                if self.transfer == Transfer::Command {
                    self.process_command(cycle);
                } else {
                    self.receive_byte(cycle);
                }
                self.shift = 0;
            }
            Transfer::Send => {
                let byte = self.payload[self.byte_index as usize];
                self.output_sio = byte & (1 << self.bit_index) != 0;
                self.bit_index += 1;
                if self.bit_index == 8 {
                    self.bit_index = 0;
                    self.byte_index += 1;
                    if self.byte_index == self.payload_length {
                        self.transfer = Transfer::Complete;
                    }
                }
            }
            _ => {}
        }
    }
    fn process_command(&mut self, cycle: Cycle) {
        let mut command = self.shift;
        if command >> 4 == 6 {
            command = command.reverse_bits();
        }
        if command & 0x0f != 6 {
            self.transfer = Transfer::Complete;
            return;
        }
        self.register = (command >> 4) & 7;
        self.payload_length = [0, 0, 7, 0, 1, 0, 3, 0][self.register as usize];
        self.byte_index = 0;
        let reading = command & 0x80 != 0;
        if self.register == 0 {
            self.control = 0;
            self.set_calendar(
                cycle,
                DateParts {
                    year: 2000,
                    month: 1,
                    day: 1,
                    weekday: 0,
                    hour: 0,
                    minute: 0,
                    second: 0,
                },
            );
            self.transfer = Transfer::Complete;
            return;
        }
        if self.register == 3 {
            self.irq_pending = true;
            self.transfer = Transfer::Complete;
            return;
        }
        if reading {
            match self.register {
                2 => self.latch_date_time(cycle),
                4 => {
                    self.payload[0] = self.control;
                    self.control &= 0x7f;
                }
                6 => self.latch_time(cycle),
                _ => {}
            }
            self.transfer =
                if self.payload_length == 0 { Transfer::Complete } else { Transfer::Send };
        } else {
            self.transfer =
                if self.payload_length == 0 { Transfer::Complete } else { Transfer::Receive };
        }
    }
    fn receive_byte(&mut self, cycle: Cycle) {
        self.payload[self.byte_index as usize] = self.shift;
        self.byte_index += 1;
        if self.byte_index != self.payload_length {
            return;
        }
        match self.register {
            2 => self.apply_date_time_write(cycle),
            4 => self.control = self.payload[0] & 0x6a,
            6 => self.apply_time_write(cycle),
            _ => {}
        }
        self.transfer = Transfer::Complete;
    }
    fn apply_date_time_write(&mut self, cycle: Cycle) {
        let (
            Some(year),
            Some(month),
            Some(day),
            Some(weekday),
            Some(hour),
            Some(minute),
            Some(second),
        ) = (
            decode_bcd(self.payload[0], 99),
            decode_bcd(self.payload[1], 12),
            decode_bcd(self.payload[2], 31),
            decode_bcd(self.payload[3], 6),
            self.decode_hour(self.payload[4]),
            decode_bcd(self.payload[5], 59),
            decode_bcd(self.payload[6], 59),
        )
        else {
            return;
        };
        if month == 0 || day == 0 || day > days_in_month(2000 + u16::from(year), month) {
            return;
        }
        self.set_calendar(
            cycle,
            DateParts { year: 2000 + u16::from(year), month, day, weekday, hour, minute, second },
        );
    }
    fn apply_time_write(&mut self, cycle: Cycle) {
        let (Some(hour), Some(minute), Some(second)) = (
            self.decode_hour(self.payload[0]),
            decode_bcd(self.payload[1], 59),
            decode_bcd(self.payload[2], 59),
        ) else {
            return;
        };
        let mut parts = self.date_parts(cycle);
        parts.hour = hour;
        parts.minute = minute;
        parts.second = second;
        self.set_calendar(cycle, parts);
    }
    fn set_calendar(&mut self, cycle: Cycle, parts: DateParts) {
        self.epoch_seconds = epoch_seconds_for(parts);
        self.epoch_cycle = cycle;
        let natural_weekday = ((self.epoch_seconds / 86400 + 4) % 7) as i8;
        self.weekday_bias = parts.weekday as i8 - natural_weekday;
    }
    fn latch_date_time(&mut self, cycle: Cycle) {
        let parts = self.date_parts(cycle);
        self.payload = [
            bcd((parts.year % 100) as u8),
            bcd(parts.month),
            bcd(parts.day),
            bcd(parts.weekday),
            self.encode_hour(parts.hour),
            bcd(parts.minute),
            bcd(parts.second),
        ];
    }
    fn latch_time(&mut self, cycle: Cycle) {
        let parts = self.date_parts(cycle);
        let hour = self.encode_hour(parts.hour);
        self.payload[..3].copy_from_slice(&[hour, bcd(parts.minute), bcd(parts.second)]);
    }
    fn encode_hour(&self, hour: u8) -> u8 {
        if self.control & 0x40 != 0 {
            bcd(hour)
        } else {
            bcd(hour % 12) | if hour >= 12 { 0x80 } else { 0 }
        }
    }
    fn decode_hour(&self, encoded: u8) -> Option<u8> {
        if self.control & 0x40 != 0 {
            return decode_bcd(encoded, 23);
        }
        let hour12 = decode_bcd(encoded & 0x7f, 12)?;
        let normalized = if hour12 == 12 { 0 } else { hour12 };
        Some(normalized + if encoded & 0x80 != 0 { 12 } else { 0 })
    }
    fn date_parts(&self, cycle: Cycle) -> DateParts {
        let elapsed_seconds = cycle.saturating_sub(self.epoch_cycle) / MASTER_CLOCK_HZ;
        let epoch = self.epoch_seconds.saturating_add(elapsed_seconds);
        let mut days = epoch / 86400;
        let day_seconds = epoch % 86400;
        let mut year = 1970_u16;
        while days >= u64::from(days_in_year(year)) {
            days -= u64::from(days_in_year(year));
            year += 1;
        }
        let mut month = 1_u8;
        while days >= u64::from(days_in_month(year, month)) {
            days -= u64::from(days_in_month(year, month));
            month += 1;
        }
        DateParts {
            year,
            month,
            day: days as u8 + 1,
            weekday: (((epoch / 86400 + 4) % 7) as i64 + i64::from(self.weekday_bias)).rem_euclid(7)
                as u8,
            hour: (day_seconds / 3600) as u8,
            minute: ((day_seconds / 60) % 60) as u8,
            second: (day_seconds % 60) as u8,
        }
    }
}

#[derive(Clone, Copy)]
struct DateParts {
    year: u16,
    month: u8,
    day: u8,
    weekday: u8,
    hour: u8,
    minute: u8,
    second: u8,
}
fn bcd(value: u8) -> u8 {
    ((value / 10) << 4) | (value % 10)
}
fn decode_bcd(value: u8, maximum: u8) -> Option<u8> {
    let (high, low) = (value >> 4, value & 0xf);
    if high > 9 || low > 9 {
        return None;
    }
    let decoded = high * 10 + low;
    (decoded <= maximum).then_some(decoded)
}
fn days_in_year(year: u16) -> u16 {
    if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) {
        366
    } else {
        365
    }
}
fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        2 => {
            if days_in_year(year) == 366 {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}
fn epoch_seconds_for(parts: DateParts) -> u64 {
    let mut days = 0_u64;
    for year in 1970..parts.year {
        days += u64::from(days_in_year(year));
    }
    for month in 1..parts.month {
        days += u64::from(days_in_month(parts.year, month));
    }
    days += u64::from(parts.day - 1);
    days * 86400
        + u64::from(parts.hour) * 3600
        + u64::from(parts.minute) * 60
        + u64::from(parts.second)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rtc_state_rejects_indices_and_payload_shapes_that_can_escape_its_buffers() {
        let mut gpio = Gpio::init_rtc(0);
        assert_eq!(gpio.validate_state(0), Ok(()));
        let rtc = gpio.rtc.as_mut().unwrap();
        rtc.bit_index = 8;
        assert_eq!(rtc.validate_state(0), Err(InvalidSaveState));
        rtc.bit_index = 0;
        rtc.payload_length = 8;
        assert_eq!(rtc.validate_state(0), Err(InvalidSaveState));
        rtc.payload_length = 7;
        rtc.transfer = Transfer::Send;
        rtc.register = 2;
        rtc.byte_index = 6;
        assert_eq!(rtc.validate_state(0), Ok(()));
        rtc.byte_index = 7;
        assert_eq!(rtc.validate_state(0), Err(InvalidSaveState));
        rtc.byte_index = 0;
        rtc.payload_length = 1;
        assert_eq!(rtc.validate_state(0), Err(InvalidSaveState));
        rtc.register = 4;
        assert_eq!(rtc.validate_state(0), Ok(()));
        rtc.transfer = Transfer::Receive;
        rtc.register = 6;
        rtc.payload_length = 3;
        rtc.byte_index = 2;
        assert_eq!(rtc.validate_state(0), Ok(()));
        rtc.epoch_cycle = 1;
        assert_eq!(rtc.validate_state(0), Err(InvalidSaveState));
        rtc.epoch_cycle = 0;
        // Evenfall-only: an epoch past the host limit would make a date read
        // loop year by year for years of host time.
        rtc.epoch_seconds = Rtc::MAX_EPOCH_SECONDS;
        assert_eq!(rtc.validate_state(0), Ok(()));
        rtc.epoch_seconds = u64::MAX;
        assert_eq!(rtc.validate_state(0), Err(InvalidSaveState));
    }

    #[test]
    fn rtc_date_time_writes_rebase_deterministic_time_and_preserve_the_written_weekday() {
        let mut rtc = Rtc::init(0);
        rtc.payload = [0x04, 0x02, 0x29, 0x00, 0x23, 0x59, 0x58];
        rtc.apply_date_time_write(0);
        rtc.latch_date_time(0);
        assert_eq!(rtc.payload, [0x04, 0x02, 0x29, 0x00, 0x23, 0x59, 0x58]);
        rtc.latch_date_time(3 * MASTER_CLOCK_HZ);
        assert_eq!(rtc.payload, [0x04, 0x03, 0x01, 0x01, 0x00, 0x00, 0x01]);
    }

    #[test]
    fn rtc_time_writes_convert_hour_modes_while_invalid_bcd_remains_atomic() {
        let mut rtc = Rtc::init(0);
        rtc.payload = [0x24, 0x06, 0x15, 0x06, 0x08, 0x09, 0x10];
        rtc.apply_date_time_write(0);
        rtc.control = 0;
        rtc.payload[..3].copy_from_slice(&[0x81, 0x02, 0x03]);
        rtc.apply_time_write(0);
        rtc.latch_time(0);
        assert_eq!(rtc.payload[..3], [0x81, 0x02, 0x03]);
        rtc.control = 0x40;
        rtc.latch_time(0);
        assert_eq!(rtc.payload[..3], [0x13, 0x02, 0x03]);
        rtc.payload = [0x25, 0x1a, 0x01, 0x02, 0x01, 0x02, 0x03];
        rtc.apply_date_time_write(0);
        rtc.latch_date_time(0);
        assert_eq!(rtc.payload, [0x24, 0x06, 0x15, 0x06, 0x13, 0x02, 0x03]);
    }

    #[test]
    fn boktai_solar_sensor_counts_clock_pulses_and_shares_gpio_with_rtc() {
        let mut gpio = Gpio::init_rtc(981_205_445);
        gpio.attach_solar_sensor(252);
        gpio.write_half(0xc8, 1, 0);
        gpio.write_half(0xc6, 7, 0);
        gpio.write_half(0xc4, 2, 0);
        gpio.write_half(0xc4, 0, 0);
        for _ in 0..3 {
            gpio.write_half(0xc4, 1, 0);
            gpio.write_half(0xc4, 0, 0);
            assert_eq!(gpio.read_byte(0xc4).unwrap() & 8, 0);
        }
        gpio.write_half(0xc4, 1, 0);
        gpio.write_half(0xc4, 0, 0);
        assert_ne!(gpio.read_byte(0xc4).unwrap() & 8, 0);
        gpio.set_solar_level(0);
        assert_eq!(gpio.read_byte(0xc4).unwrap() & 8, 0);
        gpio.set_solar_level(252);
        assert_ne!(gpio.read_byte(0xc4).unwrap() & 8, 0);
    }
}
