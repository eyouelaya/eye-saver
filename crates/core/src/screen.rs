//! Turning "what is the screen doing right now" into suspend and resume events.
//!
//! The host answers one question - is the display asleep, is the session locked - and this
//! module works out what changed since last time. Keeping the comparison here rather than
//! in the platform code means the interesting half is tested: a lock that arrives twice, a
//! display that wakes before the session unlocks, a state that was missed entirely while
//! the app was paused.
//!
//! Machine suspend is deliberately *not* part of this. It is detected from the two clocks
//! disagreeing (see [`crate::sleep_detect`]), because a suspended process cannot poll.

use std::time::Duration;

use crate::engine::{Event, SuspendReason};

/// How long to wait before looking again while the timer is suspended.
///
/// Only used while something is holding the timer back: the app needs to notice the screen
/// coming back, and nothing else will tell it. While the timer is running or the user has
/// paused it, no polling happens at all.
pub const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// What the host sees when it looks at the screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScreenStatus {
    /// The display has powered off.
    pub display_asleep: bool,
    /// The session is locked, or another user's session is in the foreground.
    pub locked: bool,
}

impl ScreenStatus {
    /// A screen that is on and unlocked: someone is looking at it.
    pub const AWAKE: Self = Self {
        display_asleep: false,
        locked: false,
    };

    /// Whether anything is keeping the user from looking at the screen.
    #[must_use]
    pub const fn is_away(self) -> bool {
        self.display_asleep || self.locked
    }
}

/// Compares successive readings of the screen and reports what changed.
#[derive(Clone, Copy, Debug, Default)]
pub struct ScreenWatch {
    last: ScreenStatus,
}

impl ScreenWatch {
    /// Starts watching from a known state.
    #[must_use]
    pub const fn new(initial: ScreenStatus) -> Self {
        Self { last: initial }
    }

    /// The most recent reading.
    #[must_use]
    pub const fn last(self) -> ScreenStatus {
        self.last
    }

    /// Records a reading and returns the events it implies.
    ///
    /// Nothing is emitted when nothing changed, which is the common case on every poll.
    pub fn observe(&mut self, status: ScreenStatus) -> Vec<Event> {
        let mut events = Vec::new();
        if status.display_asleep != self.last.display_asleep {
            events.push(if status.display_asleep {
                Event::Suspend(SuspendReason::DisplayOff)
            } else {
                Event::Resume(SuspendReason::DisplayOff)
            });
        }
        if status.locked != self.last.locked {
            events.push(if status.locked {
                Event::Suspend(SuspendReason::ScreenLock)
            } else {
                Event::Resume(SuspendReason::ScreenLock)
            });
        }
        self.last = status;
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCKED: ScreenStatus = ScreenStatus {
        display_asleep: false,
        locked: true,
    };
    const DARK: ScreenStatus = ScreenStatus {
        display_asleep: true,
        locked: false,
    };
    const DARK_AND_LOCKED: ScreenStatus = ScreenStatus {
        display_asleep: true,
        locked: true,
    };

    #[test]
    fn an_unchanged_screen_produces_nothing() {
        let mut watch = ScreenWatch::new(ScreenStatus::AWAKE);
        assert!(watch.observe(ScreenStatus::AWAKE).is_empty());
        assert!(
            watch.observe(ScreenStatus::AWAKE).is_empty(),
            "polling must be free"
        );
        assert_eq!(watch.last(), ScreenStatus::AWAKE);
    }

    #[test]
    fn locking_and_unlocking_are_reported_once_each() {
        let mut watch = ScreenWatch::default();
        assert_eq!(
            watch.observe(LOCKED),
            [Event::Suspend(SuspendReason::ScreenLock)]
        );
        assert!(watch.observe(LOCKED).is_empty(), "still locked is not news");
        assert_eq!(
            watch.observe(ScreenStatus::AWAKE),
            [Event::Resume(SuspendReason::ScreenLock)]
        );
    }

    #[test]
    fn the_display_sleeping_and_waking_are_reported_once_each() {
        let mut watch = ScreenWatch::default();
        assert_eq!(
            watch.observe(DARK),
            [Event::Suspend(SuspendReason::DisplayOff)]
        );
        assert!(watch.observe(DARK).is_empty());
        assert_eq!(
            watch.observe(ScreenStatus::AWAKE),
            [Event::Resume(SuspendReason::DisplayOff)]
        );
    }

    #[test]
    fn both_changing_at_once_reports_both() {
        let mut watch = ScreenWatch::default();
        assert_eq!(
            watch.observe(DARK_AND_LOCKED),
            [
                Event::Suspend(SuspendReason::DisplayOff),
                Event::Suspend(SuspendReason::ScreenLock)
            ]
        );
        assert_eq!(
            watch.observe(ScreenStatus::AWAKE),
            [
                Event::Resume(SuspendReason::DisplayOff),
                Event::Resume(SuspendReason::ScreenLock)
            ]
        );
    }

    #[test]
    fn a_display_that_wakes_before_the_session_unlocks_resumes_in_two_steps() {
        // This is the ordinary wake-from-sleep sequence, and the reason the engine counts
        // reasons rather than keeping a single flag.
        let mut watch = ScreenWatch::new(DARK_AND_LOCKED);
        assert_eq!(
            watch.observe(LOCKED),
            [Event::Resume(SuspendReason::DisplayOff)]
        );
        assert_eq!(
            watch.observe(ScreenStatus::AWAKE),
            [Event::Resume(SuspendReason::ScreenLock)]
        );
    }

    #[test]
    fn a_change_missed_while_the_app_was_not_polling_still_reconciles() {
        // The app was paused and stopped looking. By the time it looks again the screen
        // has locked and unlocked; nothing changed overall, so nothing is reported.
        let mut watch = ScreenWatch::new(ScreenStatus::AWAKE);
        assert!(watch.observe(ScreenStatus::AWAKE).is_empty());

        // But if it is still locked when the app looks again, that is reported late rather
        // than not at all.
        let mut watch = ScreenWatch::new(ScreenStatus::AWAKE);
        assert_eq!(
            watch.observe(LOCKED),
            [Event::Suspend(SuspendReason::ScreenLock)]
        );
    }

    #[test]
    fn away_means_either_reason() {
        assert!(!ScreenStatus::AWAKE.is_away());
        assert!(LOCKED.is_away());
        assert!(DARK.is_away());
        assert!(DARK_AND_LOCKED.is_away());
        assert_eq!(ScreenStatus::default(), ScreenStatus::AWAKE);
    }
}
