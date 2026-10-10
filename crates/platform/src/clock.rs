//! The real clocks.

use std::time::{Instant, SystemTime, UNIX_EPOCH};

use eye_saver_core::clock::{Clock, Mono, Wall};

/// Reads the host's monotonic and wall clocks.
///
/// The monotonic reading is an offset from process start rather than a raw platform
/// counter, which keeps the numbers small and makes a log line readable.
#[derive(Debug)]
pub struct SystemClock {
    started: Instant,
}

impl SystemClock {
    /// Starts the monotonic scale at this moment.
    #[must_use]
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn monotonic(&self) -> Mono {
        Mono::from_millis(u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX))
    }

    fn wall(&self) -> Wall {
        let since_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        Wall::from_millis(u64::try_from(since_epoch.as_millis()).unwrap_or(u64::MAX))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_monotonic_clock_starts_near_zero_and_never_goes_backwards() {
        let clock = SystemClock::new();
        let first = clock.monotonic();
        assert!(
            first.as_millis() < 1_000,
            "the scale starts at process start"
        );
        let second = clock.monotonic();
        assert!(second >= first);
    }

    #[test]
    fn the_wall_clock_reports_a_plausible_calendar_time() {
        let clock = SystemClock::default();
        // Comfortably after 2020 and before 2100: enough to catch a unit mix-up.
        let millis = clock.wall().as_millis();
        assert!(millis > 1_577_836_800_000, "{millis} is before 2020");
        assert!(millis < 4_102_444_800_000, "{millis} is after 2100");
    }

    #[test]
    fn both_hands_can_be_read_together() {
        let clock = SystemClock::new();
        let now = clock.now();
        assert_eq!(
            now.mono.as_millis() / 10,
            clock.monotonic().as_millis() / 10
        );
        assert!(now.wall.as_millis() > 0);
    }
}
