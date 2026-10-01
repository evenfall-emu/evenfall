use std::time::{Duration, Instant};

/// One GBA frame of wall-clock time: 280,896 cycles at 16,777,216 Hz.
pub const FRAME: Duration = Duration::from_nanos(280_896 * 1_000_000_000 / 16_777_216);

/// How far behind the wall clock emulation may fall before it gives up
/// catching up, as the browser's Worker does.
pub const MAX_LAG: Duration = Duration::from_millis(200);

/// Keeps emulated frames in step with the wall clock.
///
/// A frame is due once the clock reaches `next`. Running one moves `next` a
/// frame later, so a short stall is made up by running frames back to back.
/// A stall longer than [`MAX_LAG`] is dropped instead: emulation slows for a
/// moment rather than racing to catch up.
#[derive(Clone, Copy, Debug)]
pub struct Pacer {
    next: Instant,
}

impl Pacer {
    /// The first frame is due at once.
    pub fn new(now: Instant) -> Self {
        Self { next: now }
    }

    pub fn due(&self, now: Instant) -> bool {
        now >= self.next
    }

    /// When the next frame is due.
    pub fn next(&self) -> Instant {
        self.next
    }

    /// Records a frame that finished at `now`.
    pub fn ran(&mut self, now: Instant) {
        self.next += FRAME;
        if now.saturating_duration_since(self.next) > MAX_LAG {
            self.next = now;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 280,896 / 16,777,216 s is 16.742706 ms, about 59.73 frames a second.
    #[test]
    fn a_frame_lasts_280896_cycles_of_the_master_clock() {
        assert_eq!(FRAME, Duration::from_nanos(16_742_706));
    }

    /// On time, a frame is due exactly one frame after the previous one was.
    #[test]
    fn frames_fall_due_one_frame_apart() {
        let start = Instant::now();
        let mut pacer = Pacer::new(start);
        assert!(pacer.due(start));
        pacer.ran(start);
        assert!(!pacer.due(start + FRAME - Duration::from_nanos(1)));
        assert!(pacer.due(start + FRAME));
    }

    /// 50 ms late, the three frames it covers are due at once, and the
    /// fourth only on schedule, so a short stall is made up exactly.
    #[test]
    fn a_short_stall_is_made_up() {
        let start = Instant::now();
        let mut pacer = Pacer::new(start);
        let late = start + Duration::from_millis(50);
        let mut ran = 0;
        while pacer.due(late) {
            pacer.ran(late);
            ran += 1;
        }
        // Frames due at 0, 16.7 and 33.5 ms; the next at 50.2 ms.
        assert_eq!(ran, 3);
        assert_eq!(pacer.next(), start + 3 * FRAME);
    }

    /// 500 ms late, the schedule restarts at the clock: the late frame and
    /// one more run at once, instead of the thirty the stall covers.
    #[test]
    fn a_long_stall_is_dropped() {
        let start = Instant::now();
        let mut pacer = Pacer::new(start);
        let late = start + Duration::from_millis(500);
        let mut ran = 0;
        while pacer.due(late) {
            pacer.ran(late);
            ran += 1;
        }
        assert_eq!(ran, 2);
        assert_eq!(pacer.next(), late + FRAME);
    }
}
