//! Noticing a suspend that the operating system never told us about.
//!
//! The monotonic clock stops while a machine is suspended; the wall clock does not. If the
//! wall clock has advanced noticeably further than the monotonic clock between two
//! samples, the machine was asleep for the difference. This is a portable backstop for the
//! platform power notifications: if an event is missed, dropped, or simply not emitted by
//! some future OS version, the timer still behaves correctly.
//!
//! It deliberately reports nothing when the wall clock moves *backwards* - that is an NTP
//! correction or a user changing the date, neither of which rested anyone's eyes.

use std::time::Duration;

use crate::clock::{Mono, Wall};

/// How far the two clocks must diverge before it counts as a suspend rather than
/// scheduling noise.
pub const DEFAULT_TOLERANCE: Duration = Duration::from_secs(2);

/// Compares the wall and monotonic clocks across samples.
#[derive(Clone, Copy, Debug)]
pub struct DriftTracker {
    mono: Mono,
    wall: Wall,
    tolerance: Duration,
}

impl DriftTracker {
    /// Starts tracking from the current pair of clock readings.
    #[must_use]
    pub const fn new(mono: Mono, wall: Wall) -> Self {
        Self {
            mono,
            wall,
            tolerance: DEFAULT_TOLERANCE,
        }
    }

    /// Starts tracking with a custom tolerance.
    #[must_use]
    pub const fn with_tolerance(mono: Mono, wall: Wall, tolerance: Duration) -> Self {
        Self {
            mono,
            wall,
            tolerance,
        }
    }

    /// Records a new pair of readings and reports how long the machine was suspended, if
    /// the divergence is large enough to say.
    pub fn sample(&mut self, mono: Mono, wall: Wall) -> Option<Duration> {
        let mono_delta = mono.since(self.mono);
        let wall_delta = wall.since(self.wall);
        let went_backwards = wall < self.wall;
        self.mono = mono;
        self.wall = wall;

        if went_backwards {
            return None;
        }
        let away = wall_delta.saturating_sub(mono_delta);
        (away >= self.tolerance).then_some(away)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracker() -> DriftTracker {
        DriftTracker::new(Mono::from_secs(100), Wall::from_secs(1_000))
    }

    #[test]
    fn clocks_advancing_together_is_not_a_suspend() {
        let mut t = tracker();
        assert_eq!(t.sample(Mono::from_secs(101), Wall::from_secs(1_001)), None);
        assert_eq!(t.sample(Mono::from_secs(161), Wall::from_secs(1_061)), None);
    }

    #[test]
    fn small_divergence_is_scheduling_noise() {
        let mut t = tracker();
        assert_eq!(
            t.sample(Mono::from_millis(100_500), Wall::from_millis(1_002_000)),
            None,
            "1.5s of drift is below the 2s tolerance"
        );
    }

    #[test]
    fn divergence_exactly_at_the_tolerance_counts() {
        let mut t = tracker();
        assert_eq!(
            t.sample(Mono::from_secs(100), Wall::from_secs(1_002)),
            Some(Duration::from_secs(2))
        );
    }

    #[test]
    fn a_frozen_monotonic_clock_reports_the_whole_gap() {
        let mut t = tracker();
        // Thirty minutes of wall time, no monotonic time: a classic laptop-lid suspend.
        assert_eq!(
            t.sample(Mono::from_secs(100), Wall::from_secs(2_800)),
            Some(Duration::from_secs(1_800))
        );
    }

    #[test]
    fn a_monotonic_clock_that_also_ticks_through_sleep_reports_nothing() {
        // Some platforms keep counting across suspend. Then there is no divergence to see
        // and the engine relies on its other signals instead.
        let mut t = tracker();
        assert_eq!(
            t.sample(Mono::from_secs(1_900), Wall::from_secs(2_800)),
            None
        );
    }

    #[test]
    fn a_backwards_wall_clock_is_ignored() {
        let mut t = tracker();
        assert_eq!(t.sample(Mono::from_secs(101), Wall::from_secs(100)), None);
        // The baseline still moved, so the next ordinary sample is judged against it.
        assert_eq!(t.sample(Mono::from_secs(102), Wall::from_secs(101)), None);
    }

    #[test]
    fn the_baseline_advances_so_one_suspend_is_reported_once() {
        let mut t = tracker();
        assert!(
            t.sample(Mono::from_secs(100), Wall::from_secs(1_600))
                .is_some()
        );
        assert_eq!(t.sample(Mono::from_secs(101), Wall::from_secs(1_601)), None);
    }

    #[test]
    fn tolerance_is_configurable() {
        let mut t =
            DriftTracker::with_tolerance(Mono::ZERO, Wall::ZERO, Duration::from_millis(100));
        assert_eq!(
            t.sample(Mono::ZERO, Wall::from_millis(150)),
            Some(Duration::from_millis(150))
        );
    }
}
