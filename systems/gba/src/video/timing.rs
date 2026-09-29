//! LCD scanline timing and DISPSTAT/VCOUNT status.

use crate::types::Cycle;
use evenfall_state::Codec;

pub const VISIBLE_LINE_COUNT: usize = 160;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Events {
    pub entered_vblank: bool,
    pub vblank_irq: bool,
    pub hblank_irq: bool,
    pub vcount_irq: bool,
    pub hblank_lines: [bool; VISIBLE_LINE_COUNT],
    pub hblank_any: bool,
    pub frames_completed: Cycle,
}

impl Default for Events {
    fn default() -> Self {
        Self {
            entered_vblank: false,
            vblank_irq: false,
            hblank_irq: false,
            vcount_irq: false,
            hblank_lines: [false; VISIBLE_LINE_COUNT],
            hblank_any: false,
            frames_completed: 0,
        }
    }
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Codec)]
pub struct PpuTiming {
    pub cycle_in_line: u16,
    pub line: u16,
    pub dispstat_control: u16,
}

impl PpuTiming {
    pub fn read_dispstat(&self) -> u16 {
        let mut value = self.dispstat_control;
        if self.line >= 160 {
            value |= 1 << 0;
        }
        if self.cycle_in_line >= 960 {
            value |= 1 << 1;
        }
        if self.line == self.vcount_compare() {
            value |= 1 << 2;
        }
        value
    }
    pub fn write_dispstat(&mut self, value: u16) {
        self.dispstat_control = value & 0xff38;
    }
    pub fn read_vcount(&self) -> u16 {
        self.line
    }
    pub fn cycles_until_next_event(&self) -> Cycle {
        let hblank_irq = self.dispstat_control & (1 << 4) != 0;
        if self.cycle_in_line < 960 && (usize::from(self.line) < VISIBLE_LINE_COUNT || hblank_irq) {
            return 960 - Cycle::from(self.cycle_in_line);
        }
        let line_end = 1232 - Cycle::from(self.cycle_in_line);
        let mut target = if usize::from(self.line) < VISIBLE_LINE_COUNT { 160 } else { 228 };
        if self.dispstat_control & (1 << 5) != 0 {
            let compare = self.vcount_compare();
            if compare > self.line && compare < target {
                target = compare;
            }
        }
        let until_target = line_end + Cycle::from(target - self.line - 1) * 1232;
        if usize::from(self.line + 1) < VISIBLE_LINE_COUNT || hblank_irq {
            return until_target.min(line_end + 960);
        }
        until_target
    }
    pub fn elapse(&mut self, cycles: Cycle) -> Events {
        if self.elapse_without_events(cycles) {
            return Events::default();
        }
        let mut remaining = cycles;
        let mut events = Events::default();
        while remaining > 0 {
            let boundary = if self.cycle_in_line < 960 { 960 } else { 1232 };
            let until_boundary = Cycle::from(boundary - self.cycle_in_line);
            let advanced = remaining.min(until_boundary) as u16;
            self.cycle_in_line += advanced;
            remaining -= Cycle::from(advanced);
            if self.cycle_in_line != boundary {
                continue;
            }
            if boundary == 960 {
                if usize::from(self.line) < VISIBLE_LINE_COUNT {
                    events.hblank_lines[usize::from(self.line)] = true;
                    events.hblank_any = true;
                }
                if self.dispstat_control & (1 << 4) != 0 {
                    events.hblank_irq = true;
                }
                continue;
            }
            self.cycle_in_line = 0;
            self.line += 1;
            if self.line == 228 {
                self.line = 0;
                events.frames_completed += 1;
            }
            if self.line == 160 {
                events.entered_vblank = true;
                if self.dispstat_control & (1 << 3) != 0 {
                    events.vblank_irq = true;
                }
            }
            if self.line == self.vcount_compare() && self.dispstat_control & (1 << 5) != 0 {
                events.vcount_irq = true;
            }
        }
        events
    }
    pub fn elapse_without_events(&mut self, cycles: Cycle) -> bool {
        if cycles >= self.cycles_until_next_event() {
            return false;
        }
        self.elapse_silently(cycles);
        true
    }
    pub fn elapse_before_boundary(&mut self, cycles: Cycle) {
        debug_assert!(cycles < self.cycles_until_next_event());
        self.elapse_silently(cycles);
    }
    fn elapse_silently(&mut self, cycles: Cycle) {
        let position = Cycle::from(self.cycle_in_line) + cycles;
        if position < 1232 {
            self.cycle_in_line = position as u16;
            return;
        }
        self.line += (position / 1232) as u16;
        self.cycle_in_line = (position % 1232) as u16;
        debug_assert!(self.line < 228);
    }
    fn vcount_compare(&self) -> u16 {
        self.dispstat_control >> 8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Zig: an advance ending on a boundary still emits every event
    #[test]
    fn an_advance_ending_on_a_boundary_still_emits_every_event() {
        let mut timing = PpuTiming {
            cycle_in_line: 959,
            line: 159,
            dispstat_control: (160 << 8) | (1 << 3) | (1 << 5),
        };
        let events = timing.elapse(1 + 272);
        assert_eq!(timing.line, 160);
        assert_eq!(timing.cycle_in_line, 0);
        assert!(events.hblank_any);
        assert!(events.hblank_lines[159]);
        assert_eq!(events.hblank_lines.iter().filter(|&&set| set).count(), 1);
        assert!(events.entered_vblank);
        assert!(events.vblank_irq);
        assert!(events.vcount_irq);
        assert!(!events.hblank_irq);
        assert_eq!(events.frames_completed, 0);
    }

    /// Zig: one large advance preserves frame wrapping and later boundaries
    #[test]
    fn one_large_advance_preserves_frame_wrapping_and_later_boundaries() {
        let mut timing =
            PpuTiming { cycle_in_line: 1231, line: 227, dispstat_control: (1 << 4) | (1 << 5) };
        let events = timing.elapse(1 + 1232 * 228 + 960);
        assert_eq!((timing.line, timing.cycle_in_line), (0, 960));
        assert_eq!(events.frames_completed, 2);
        assert!(events.hblank_any);
        assert!(events.hblank_lines[0]);
        assert!(events.hblank_irq);
        assert!(events.vcount_irq);
    }

    // Independent per-boundary oracle from ZiGBA's stepToRawBoundaryForTest.
    fn step_to_raw_boundary_for_test(timing: &mut PpuTiming) -> (Cycle, bool) {
        let hblank = timing.cycle_in_line < 960;
        let cycles = Cycle::from(if hblank { 960 } else { 1232 } - timing.cycle_in_line);
        if hblank {
            timing.cycle_in_line = 960;
            return (
                cycles,
                usize::from(timing.line) < VISIBLE_LINE_COUNT
                    || timing.dispstat_control & (1 << 4) != 0,
            );
        }
        timing.cycle_in_line = 0;
        timing.line = (timing.line + 1) % 228;
        let vcount =
            timing.line == timing.vcount_compare() && timing.dispstat_control & (1 << 5) != 0;
        (cycles, timing.line == 0 || timing.line == 160 || vcount)
    }

    /// Zig: silent boundaries end exactly at the next boundary that delivers an event
    #[test]
    fn silent_boundaries_end_exactly_at_the_next_boundary_that_delivers_an_event() {
        for hblank_irq in [0, 1 << 4] {
            for vcount_irq in [0, 1 << 5] {
                for compare in [0, 1, 100, 159, 160, 161, 200, 227, 228, 255] {
                    let control = hblank_irq | vcount_irq | (compare << 8);
                    for line in 0..228 {
                        for dot in [0, 1, 959, 960, 961, 1231] {
                            let start =
                                PpuTiming { cycle_in_line: dot, line, dispstat_control: control };
                            let mut reference = start;
                            let mut expected = 0;
                            loop {
                                let (cycles, event) = step_to_raw_boundary_for_test(&mut reference);
                                expected += cycles;
                                if event {
                                    break;
                                }
                            }
                            assert_eq!(expected, start.cycles_until_next_event());
                            let mut silent = start;
                            assert!(silent.elapse_without_events(expected - 1));
                            let mut raw = start;
                            let mut remaining = expected - 1;
                            while remaining > 0 {
                                let mut probe = raw;
                                let (cycles, _) = step_to_raw_boundary_for_test(&mut probe);
                                if cycles > remaining {
                                    raw.cycle_in_line += remaining as u16;
                                    break;
                                }
                                raw = probe;
                                remaining -= cycles;
                            }
                            assert_eq!(raw, silent);
                            let mut boundary = silent;
                            assert!(!boundary.elapse_without_events(1));
                            let mut endpoint = silent;
                            endpoint.cycle_in_line += 1;
                            if endpoint.cycle_in_line == 1232 {
                                endpoint.cycle_in_line = 0;
                                endpoint.line = (endpoint.line + 1) % 228;
                            }
                            let events = boundary.elapse(1);
                            assert_eq!(endpoint, boundary);
                            assert_ne!(events, Events::default());
                        }
                    }
                }
            }
        }
    }
}
