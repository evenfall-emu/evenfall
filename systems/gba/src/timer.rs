//! Cycle-domain state machine for one GBA hardware timer.

use crate::types::Cycle;
use evenfall_state::Codec;

#[derive(Clone, Copy, Default, Codec)]
pub struct Timer {
    pub reload: u16,
    pub counter: u16,
    pub control: u16,
    pub prescaler_phase: Cycle,
    /// Whether the timer is in the cycle after the write that enabled it.
    /// The counter holds the reload value through that cycle: a prescaler
    /// edge inside it does not count. The mGBA suite's timer IRQ table,
    /// recorded on hardware, reads the reload value two cycles after the
    /// enabling write lands.
    pub starting: bool,
}

impl Timer {
    pub fn read_counter(&self) -> u16 {
        self.counter
    }
    pub fn read_control(&self) -> u16 {
        self.control
    }
    pub fn write_reload(&mut self, value: u16) {
        self.reload = value;
    }
    pub fn write_control(&mut self, value: u16, prescaler: u16) {
        let was_enabled = self.enabled();
        self.control = value & 0x00c7;
        if !was_enabled && self.enabled() {
            self.counter = self.reload;
            self.starting = true;
        }
        if self.enabled() {
            self.prescaler_phase = self.phase_at(prescaler);
        }
    }
    /// Inlined into the device step, as it was before the start delay made
    /// it big enough for LLVM to hesitate.
    #[inline(always)]
    pub fn elapse(&mut self, mut cycles: Cycle) -> Cycle {
        if !self.enabled() {
            return 0;
        }
        if self.starting && cycles != 0 {
            self.skip_starting_cycle();
            cycles -= 1;
        }
        let shift = self.prescaler_shift();
        let accumulated = self.prescaler_phase + cycles;
        let ticks = accumulated >> shift;
        self.prescaler_phase = accumulated & ((1 << shift) - 1);
        self.advance_ticks(ticks)
    }
    pub fn cascade(&mut self, ticks: Cycle) -> Cycle {
        if !self.enabled() || !self.count_up_enabled() {
            return 0;
        }
        self.advance_ticks(ticks)
    }
    pub fn elapse_before_overflow(&mut self, mut cycles: Cycle) {
        if !self.enabled() {
            return;
        }
        debug_assert!(cycles < self.cycles_until_overflow().unwrap());
        if self.starting && cycles != 0 {
            self.skip_starting_cycle();
            cycles -= 1;
        }
        let accumulated = self.prescaler_phase + cycles;
        self.prescaler_phase = accumulated & ((1 << self.prescaler_shift()) - 1);
        self.counter += (accumulated >> self.prescaler_shift()) as u16;
    }
    pub fn count_up_enabled(&self) -> bool {
        self.control & (1 << 2) != 0
    }
    pub fn cycles_until_overflow(&self) -> Option<Cycle> {
        if !self.enabled() {
            return None;
        }
        let mut timer = *self;
        let start = Cycle::from(timer.starting);
        if timer.starting {
            timer.skip_starting_cycle();
        }
        Some(
            start + (0x1_0000 - Cycle::from(timer.counter)) * timer.prescaler_divisor()
                - timer.prescaler_phase,
        )
    }
    /// The cycle after the enabling write: the shared prescaler moves on,
    /// but an edge it reaches here does not step the counter.
    fn skip_starting_cycle(&mut self) {
        self.starting = false;
        self.prescaler_phase = (self.prescaler_phase + 1) & (self.prescaler_divisor() - 1);
    }
    pub fn validate_state(&self, prescaler: u16) -> Result<(), InvalidSaveState> {
        if self.enabled()
            && !self.count_up_enabled()
            && self.prescaler_phase != self.phase_at(prescaler)
        {
            return Err(InvalidSaveState);
        }
        if self.starting && (!self.enabled() || self.count_up_enabled()) {
            return Err(InvalidSaveState);
        }
        Ok(())
    }
    fn advance_ticks(&mut self, initial_ticks: Cycle) -> Cycle {
        let mut ticks = initial_ticks;
        let mut overflows = 0;
        while ticks > 0 {
            let until_overflow = 0x1_0000 - Cycle::from(self.counter);
            if ticks < until_overflow {
                self.counter = self.counter.wrapping_add(ticks as u16);
                return overflows;
            }
            ticks -= until_overflow;
            self.counter = self.reload;
            overflows += 1;
        }
        overflows
    }
    pub fn irq_enabled(&self) -> bool {
        self.control & (1 << 6) != 0
    }
    pub fn enabled(&self) -> bool {
        self.control & (1 << 7) != 0
    }
    pub fn overflow_period(&self) -> Cycle {
        (0x1_0000 - Cycle::from(self.reload)) << self.prescaler_shift()
    }
    fn phase_at(&self, prescaler: u16) -> Cycle {
        Cycle::from(prescaler & 0x03ff) & (self.prescaler_divisor() - 1)
    }
    fn prescaler_divisor(&self) -> Cycle {
        1 << self.prescaler_shift()
    }
    fn prescaler_shift(&self) -> u32 {
        match self.control & 3 {
            0 => 0,
            1 => 6,
            2 => 8,
            3 => 10,
            _ => unreachable!(),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct InvalidSaveState;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timer_state_rejects_an_active_phase_off_the_shared_prescaler_grid() {
        let mut timer = Timer::default();
        assert_eq!(timer.validate_state(700), Ok(()));
        timer.control = 1 << 7;
        assert_eq!(timer.validate_state(700), Ok(()));
        timer.prescaler_phase = 1;
        assert_eq!(timer.validate_state(700), Err(InvalidSaveState));
        timer.control = (1 << 7) | 1;
        timer.prescaler_phase = 60;
        assert_eq!(timer.validate_state(700), Ok(()));
        assert_eq!(timer.validate_state(701), Err(InvalidSaveState));
        timer.prescaler_phase = 59;
        assert_eq!(timer.validate_state(700), Err(InvalidSaveState));
        timer.prescaler_phase = 64 + 60;
        assert_eq!(timer.validate_state(700), Err(InvalidSaveState));
        timer.control |= 1 << 2;
        assert_eq!(timer.validate_state(700), Ok(()));
    }

    #[test]
    fn timer_prescalers_preserve_phase_and_overflow_counts_across_chunk_boundaries() {
        for (select, divisor) in [1_u64, 64, 256, 1024].into_iter().enumerate() {
            for start in [0_u16, 700] {
                for reload in [0_u16, 0xff00, 0xffff] {
                    let mut timer = Timer::default();
                    timer.write_reload(reload);
                    timer.write_control(0x80 | select as u16, start);
                    let offset = u64::from(start) % divisor;
                    assert_eq!(offset, timer.prescaler_phase);
                    let mut total = 0;
                    let mut overflows = 0;
                    for chunk in [1, 3, 63, 64, 255, 256, 1023, 1024, 65536, 123456] {
                        total += chunk;
                        overflows += timer.elapse(chunk);
                        let period = 0x10000 - u64::from(reload);
                        // Prescaler edges crossed since the enable, less one
                        // crossed in the cycle after it, which does not count.
                        let ticks = (offset + total) / divisor - (offset + 1) / divisor;
                        let phase = (offset + total) % divisor;
                        assert_eq!(ticks / period, overflows);
                        assert_eq!(
                            (u64::from(reload) + ticks % period) as u16,
                            timer.read_counter()
                        );
                        assert_eq!(phase, timer.prescaler_phase);
                        assert_eq!(
                            Some((period - ticks % period) * divisor - phase),
                            timer.cycles_until_overflow()
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn timer_control_writes_take_the_phase_from_the_shared_prescaler_counter() {
        let mut timer = Timer { reload: 0xffff, ..Timer::default() };
        timer.write_control(0x81, 1000);
        assert_eq!(timer.prescaler_phase, 40);
        assert_eq!(timer.cycles_until_overflow(), Some(24));
        _ = timer.elapse(10);
        timer.write_control(0xc1, 1010);
        assert_eq!(timer.prescaler_phase, 50);
        timer.write_control(0xc3, 1010);
        assert_eq!(timer.cycles_until_overflow(), Some(14));
        timer.write_control(0xc2, 1010);
        assert_eq!(timer.prescaler_phase, 242);
        timer.write_control(0, 1010);
        timer.write_control(0x80, 1010);
        assert_eq!(timer.prescaler_phase, 0);
    }
}
