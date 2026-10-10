//! Time types.
//!
//! Two unrelated notions of time are kept deliberately distinct, because the difference
//! between them is how the application notices that the machine was asleep:
//!
//! * [`Mono`] - a monotonic instant. Never goes backwards, unaffected by the user or NTP
//!   changing the wall clock, and (on both macOS and Windows) does not advance while the
//!   machine is suspended.
//! * [`Wall`] - calendar time. Advances across suspend, but can jump in either direction.
//!
//! Both are plain millisecond counters so that tests can construct any timeline they like.

use std::time::Duration;

/// A monotonic point in time, in milliseconds since an arbitrary origin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Mono(u64);

/// A wall-clock point in time, in milliseconds since an arbitrary origin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Wall(u64);

macro_rules! timestamp_impl {
    ($name:ident) => {
        impl $name {
            /// The origin of the time scale.
            pub const ZERO: Self = Self(0);

            /// Builds a timestamp from a raw millisecond count.
            #[must_use]
            pub const fn from_millis(millis: u64) -> Self {
                Self(millis)
            }

            /// Builds a timestamp from a whole number of seconds, saturating on overflow.
            #[must_use]
            pub const fn from_secs(secs: u64) -> Self {
                Self(secs.saturating_mul(1000))
            }

            /// The raw millisecond count.
            #[must_use]
            pub const fn as_millis(self) -> u64 {
                self.0
            }

            /// Time elapsed since `earlier`, or zero if `earlier` is in the future.
            #[must_use]
            pub const fn since(self, earlier: Self) -> Duration {
                Duration::from_millis(self.0.saturating_sub(earlier.0))
            }

            /// This timestamp advanced by `delta`, saturating at the end of the scale.
            ///
            /// Deliberately not `std::ops::Add`: these saturate rather than overflow, and
            /// an operator that silently clamps would be a trap.
            #[must_use]
            #[allow(clippy::should_implement_trait)]
            pub fn add(self, delta: Duration) -> Self {
                Self(self.0.saturating_add(duration_millis(delta)))
            }

            /// This timestamp moved back by `delta`, saturating at [`Self::ZERO`].
            #[must_use]
            #[allow(clippy::should_implement_trait)]
            pub fn sub(self, delta: Duration) -> Self {
                Self(self.0.saturating_sub(duration_millis(delta)))
            }
        }
    };
}

timestamp_impl!(Mono);
timestamp_impl!(Wall);

/// A duration in whole milliseconds, saturating rather than overflowing.
///
/// `Duration::as_millis` returns a `u128`; the timeline only ever needs `u64`, and
/// saturating keeps the arithmetic total so no code path can panic.
#[must_use]
pub fn duration_millis(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// Both clocks, read at the same moment.
///
/// Every decision the engine makes needs both: deadlines are measured on the monotonic
/// clock, which is immune to the user changing the date, while *absences* have to be
/// measured on the wall clock, because the monotonic clock does not advance while the
/// machine is suspended. Reading them as a pair keeps the two consistent with each other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Now {
    /// The monotonic reading.
    pub mono: Mono,
    /// The wall-clock reading.
    pub wall: Wall,
}

impl Now {
    /// Pairs two readings.
    #[must_use]
    pub const fn new(mono: Mono, wall: Wall) -> Self {
        Self { mono, wall }
    }

    /// Both clocks advanced by `delta`: ordinary elapsed time.
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn add(self, delta: Duration) -> Self {
        Self {
            mono: self.mono.add(delta),
            wall: self.wall.add(delta),
        }
    }

    /// Only the wall clock advanced by `delta`: the machine was suspended.
    #[must_use]
    pub fn suspend(self, delta: Duration) -> Self {
        Self {
            mono: self.mono,
            wall: self.wall.add(delta),
        }
    }

    /// The wall clock moved backwards: an NTP correction or a date change.
    #[must_use]
    pub fn rewind_wall(self, delta: Duration) -> Self {
        Self {
            mono: self.mono,
            wall: self.wall.sub(delta),
        }
    }
}

/// Source of the two clocks the application needs.
///
/// Implemented against the operating system in `eye-saver-platform`, and by
/// [`crate::testing::FakeClock`] in tests.
pub trait Clock {
    /// A monotonic instant that does not advance while the machine is suspended.
    fn monotonic(&self) -> Mono;
    /// Calendar time, which does advance while the machine is suspended.
    fn wall(&self) -> Wall;

    /// Reads both clocks together.
    fn now(&self) -> Now {
        Now::new(self.monotonic(), self.wall())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructors_agree() {
        assert_eq!(Mono::from_secs(2), Mono::from_millis(2000));
        assert_eq!(Wall::from_secs(2).as_millis(), 2000);
        assert_eq!(Mono::ZERO, Mono::from_millis(0));
        assert_eq!(Mono::default(), Mono::ZERO);
    }

    #[test]
    fn from_secs_saturates() {
        assert_eq!(Mono::from_secs(u64::MAX).as_millis(), u64::MAX);
    }

    #[test]
    fn since_measures_forward_gaps_only() {
        let a = Mono::from_secs(10);
        let b = Mono::from_secs(25);
        assert_eq!(b.since(a), Duration::from_secs(15));
        assert_eq!(
            a.since(b),
            Duration::ZERO,
            "backwards comparison clamps to zero"
        );
    }

    #[test]
    fn add_and_sub_are_inverses_and_saturate() {
        let t = Mono::from_secs(100);
        assert_eq!(t.add(Duration::from_secs(5)), Mono::from_secs(105));
        assert_eq!(t.sub(Duration::from_secs(5)), Mono::from_secs(95));
        assert_eq!(t.sub(Duration::from_secs(1_000)), Mono::ZERO);
        assert_eq!(
            Mono::from_millis(u64::MAX)
                .add(Duration::from_secs(1))
                .as_millis(),
            u64::MAX
        );
    }

    #[test]
    fn duration_millis_saturates_instead_of_truncating() {
        assert_eq!(duration_millis(Duration::from_millis(1500)), 1500);
        assert_eq!(duration_millis(Duration::MAX), u64::MAX);
    }

    #[test]
    fn a_now_advances_both_clocks_but_a_suspend_only_moves_the_wall_clock() {
        let t = Now::new(Mono::from_secs(10), Wall::from_secs(1_000));
        assert_eq!(
            t.add(Duration::from_secs(5)),
            Now::new(Mono::from_secs(15), Wall::from_secs(1_005))
        );
        assert_eq!(
            t.suspend(Duration::from_secs(600)),
            Now::new(Mono::from_secs(10), Wall::from_secs(1_600))
        );
        assert_eq!(
            t.rewind_wall(Duration::from_secs(100)),
            Now::new(Mono::from_secs(10), Wall::from_secs(900))
        );
    }

    #[test]
    fn a_clock_reads_both_hands_together() {
        struct Fixed;
        impl Clock for Fixed {
            fn monotonic(&self) -> Mono {
                Mono::from_secs(7)
            }
            fn wall(&self) -> Wall {
                Wall::from_secs(99)
            }
        }
        assert_eq!(
            Fixed.now(),
            Now::new(Mono::from_secs(7), Wall::from_secs(99))
        );
    }

    #[test]
    fn timestamps_order_and_hash() {
        use std::collections::HashSet;
        assert!(Mono::from_secs(1) < Mono::from_secs(2));
        assert!(Wall::from_secs(2) > Wall::from_secs(1));
        let set: HashSet<Mono> = [Mono::ZERO, Mono::ZERO, Mono::from_secs(1)]
            .into_iter()
            .collect();
        assert_eq!(set.len(), 2);
    }
}
