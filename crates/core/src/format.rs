//! Turning a remaining duration into the strings the user sees, and working out when each
//! of those strings will next change.
//!
//! The "when does it next change" half is what keeps the app off the CPU: the event loop
//! sleeps until a label actually becomes stale instead of polling. Because it lives here
//! as pure arithmetic, the wake-up schedule is unit tested rather than observed.
//!
//! Countdowns round *up*, so `0:01` is shown for the whole of the last second and `0:00`
//! appears only at expiry.

use std::time::Duration;

use crate::clock::duration_millis;
use crate::engine::{Badge, Phase, ReasonSet, Status, SuspendReason};
use crate::settings::TrayDisplay;

/// Below this much remaining, a minutes-granularity display switches to `m:ss`.
pub const SECONDS_WINDOW: Duration = Duration::from_secs(60);

/// Whole seconds remaining, rounded up.
#[must_use]
pub fn ceil_secs(d: Duration) -> u64 {
    duration_millis(d).div_ceil(1000)
}

/// Whole minutes remaining, rounded up.
#[must_use]
pub fn ceil_minutes(d: Duration) -> u64 {
    duration_millis(d).div_ceil(60_000)
}

/// `19:59` style countdown. Minutes are not wrapped, so a three hour interval reads
/// `180:00`.
#[must_use]
pub fn mm_ss(d: Duration) -> String {
    let secs = ceil_secs(d);
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// `20s` style countdown, used for the short look-away break.
#[must_use]
pub fn secs_label(d: Duration) -> String {
    format!("{}s", ceil_secs(d))
}

/// `19m` style countdown.
#[must_use]
pub fn mins_label(d: Duration) -> String {
    format!("{}m", ceil_minutes(d))
}

/// Time until [`mm_ss`] or [`secs_label`] would render differently, or `None` at expiry.
#[must_use]
pub fn until_next_second(remaining: Duration) -> Option<Duration> {
    let millis = duration_millis(remaining);
    if millis == 0 {
        return None;
    }
    Some(Duration::from_millis(
        millis - (ceil_secs(remaining) - 1) * 1000,
    ))
}

/// Time until [`mins_label`] would render differently, or `None` at expiry.
#[must_use]
pub fn until_next_minute(remaining: Duration) -> Option<Duration> {
    let millis = duration_millis(remaining);
    if millis == 0 {
        return None;
    }
    Some(Duration::from_millis(
        millis - (ceil_minutes(remaining) - 1) * 60_000,
    ))
}

/// The text shown beside the menu bar icon.
///
/// Returns an empty string when nothing should be shown: either the user asked for an
/// icon-only display, or the timer is not counting because the screen is away.
#[must_use]
pub fn tray_label(
    remaining: Duration,
    display: TrayDisplay,
    phase: Phase,
    status: Status,
) -> String {
    match status {
        // The screen is off, locked or the machine just woke: there is nothing to count,
        // and a stale number in the menu bar would be a lie.
        Status::Suspended => String::new(),
        Status::PausedByUser => match display {
            TrayDisplay::IconOnly => String::new(),
            _ => "--".to_owned(),
        },
        Status::Running => match (display, phase) {
            (TrayDisplay::IconOnly, _) => String::new(),
            (_, Phase::Break) => secs_label(remaining),
            (TrayDisplay::MmSs, Phase::Work) => mm_ss(remaining),
            (TrayDisplay::Minutes, Phase::Work) => {
                if remaining > SECONDS_WINDOW {
                    mins_label(remaining)
                } else {
                    mm_ss(remaining)
                }
            }
        },
    }
}

/// Time until [`tray_label`] would render differently, or `None` if it never will.
#[must_use]
pub fn until_tray_label_changes(
    remaining: Duration,
    display: TrayDisplay,
    phase: Phase,
    status: Status,
) -> Option<Duration> {
    if status != Status::Running {
        return None;
    }
    match (display, phase) {
        (TrayDisplay::IconOnly, _) => None,
        (_, Phase::Break) | (TrayDisplay::MmSs, Phase::Work) => until_next_second(remaining),
        (TrayDisplay::Minutes, Phase::Work) => {
            if remaining > SECONDS_WINDOW {
                until_next_minute(remaining)
            } else {
                until_next_second(remaining)
            }
        }
    }
}

/// The first line of the tray menu, and the body of the Windows tooltip.
///
/// Deliberately coarse while working: it is only visible when the menu is open or the
/// cursor is hovering, so refreshing it every second would buy nothing and cost a wake-up
/// every second on hosts whose tray has no text label.
#[must_use]
pub fn menu_header(
    remaining: Duration,
    phase: Phase,
    status: Status,
    reasons: ReasonSet,
) -> String {
    match status {
        Status::PausedByUser => "Paused".to_owned(),
        Status::Suspended => match reasons.primary() {
            Some(SuspendReason::Sleep) => "Paused - machine was asleep".to_owned(),
            Some(SuspendReason::DisplayOff) => "Paused - display is off".to_owned(),
            Some(SuspendReason::ScreenLock) => "Paused - screen is locked".to_owned(),
            Some(SuspendReason::SessionInactive) => "Paused - session is inactive".to_owned(),
            None => "Paused".to_owned(),
        },
        // Minutes even in the final minute: the menu is a transient popup, and refreshing
        // it per second would cost a wake-up per second on hosts where nothing else needs
        // one. The break countdown is the exception - watching it tick is the point.
        Status::Running => match phase {
            Phase::Break => format!("Look away: {}", secs_label(remaining)),
            Phase::Work => format!("Next break in {}", mins_label(remaining)),
        },
    }
}

/// Time until [`menu_header`] would render differently, or `None` if it never will.
#[must_use]
pub fn until_menu_header_changes(
    remaining: Duration,
    phase: Phase,
    status: Status,
) -> Option<Duration> {
    if status != Status::Running {
        return None;
    }
    match phase {
        Phase::Break => until_next_second(remaining),
        Phase::Work => until_next_minute(remaining),
    }
}

/// The countdown to draw into the tray icon on hosts whose tray cannot show text.
///
/// The break is always its seconds, and a minutes display is its minutes. A `19:59`
/// display does not fit in a square side by side, so it is stacked: minutes above seconds.
/// Every number is clamped to two digits. `None` means "draw the plain glyph": nothing is
/// being counted, or the user asked for no countdown.
#[must_use]
pub fn icon_badge(
    remaining: Duration,
    display: TrayDisplay,
    phase: Phase,
    status: Status,
) -> Option<Badge> {
    if status != Status::Running {
        return None;
    }
    let two_digits = |value: u64| u8::try_from(value.min(99)).unwrap_or(99);
    match (display, phase) {
        (TrayDisplay::IconOnly, _) => None,
        (_, Phase::Break) => Some(Badge::Number(u16::from(two_digits(ceil_secs(remaining))))),
        (TrayDisplay::Minutes, Phase::Work) => Some(Badge::Number(u16::from(two_digits(
            ceil_minutes(remaining),
        )))),
        (TrayDisplay::MmSs, Phase::Work) => {
            let secs = ceil_secs(remaining);
            Some(Badge::Stacked {
                minutes: two_digits(secs / 60),
                seconds: two_digits(secs % 60),
            })
        }
    }
}

/// Time until [`icon_badge`] would render differently, or `None` if it never will.
#[must_use]
pub fn until_icon_badge_changes(
    remaining: Duration,
    display: TrayDisplay,
    phase: Phase,
    status: Status,
) -> Option<Duration> {
    if status != Status::Running {
        return None;
    }
    match (display, phase) {
        (TrayDisplay::IconOnly, _) => None,
        (_, Phase::Break) | (TrayDisplay::MmSs, Phase::Work) => until_next_second(remaining),
        (TrayDisplay::Minutes, Phase::Work) => until_next_minute(remaining),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: fn(u64) -> Duration = Duration::from_secs;
    const MS: fn(u64) -> Duration = Duration::from_millis;

    #[test]
    fn rounding_is_upwards_so_the_last_second_is_visible() {
        assert_eq!(ceil_secs(Duration::ZERO), 0);
        assert_eq!(ceil_secs(MS(1)), 1);
        assert_eq!(ceil_secs(MS(1000)), 1);
        assert_eq!(ceil_secs(MS(1001)), 2);
        assert_eq!(ceil_minutes(MS(1)), 1);
        assert_eq!(ceil_minutes(S(60)), 1);
        assert_eq!(ceil_minutes(S(61)), 2);
        assert_eq!(ceil_minutes(Duration::ZERO), 0);
    }

    #[test]
    fn mm_ss_formats_pad_and_do_not_wrap_at_an_hour() {
        assert_eq!(mm_ss(S(1200)), "20:00");
        assert_eq!(
            mm_ss(MS(1_199_500)),
            "20:00",
            "ceiling keeps 20:00 until a full second elapses"
        );
        assert_eq!(mm_ss(MS(1_199_000)), "19:59");
        assert_eq!(mm_ss(S(59)), "0:59");
        assert_eq!(mm_ss(S(5)), "0:05");
        assert_eq!(mm_ss(Duration::ZERO), "0:00");
        assert_eq!(mm_ss(S(180 * 60)), "180:00");
    }

    #[test]
    fn short_labels() {
        assert_eq!(secs_label(S(20)), "20s");
        assert_eq!(secs_label(MS(1)), "1s");
        assert_eq!(mins_label(S(1200)), "20m");
        assert_eq!(mins_label(S(61)), "2m");
    }

    #[test]
    fn until_next_second_lands_exactly_on_label_changes() {
        assert_eq!(until_next_second(S(1200)), Some(S(1)));
        assert_eq!(until_next_second(MS(1500)), Some(MS(500)));
        assert_eq!(until_next_second(MS(1000)), Some(MS(1000)));
        assert_eq!(until_next_second(MS(1)), Some(MS(1)));
        assert_eq!(until_next_second(Duration::ZERO), None);
    }

    #[test]
    fn until_next_minute_lands_exactly_on_label_changes() {
        assert_eq!(until_next_minute(S(1200)), Some(S(60)));
        assert_eq!(until_next_minute(S(61)), Some(S(1)));
        assert_eq!(until_next_minute(S(60)), Some(S(60)));
        assert_eq!(until_next_minute(MS(500)), Some(MS(500)));
        assert_eq!(until_next_minute(Duration::ZERO), None);
    }

    #[test]
    fn label_changes_are_consistent_with_the_label_itself() {
        // Stepping forward by exactly the reported delay must change the rendered text,
        // and stepping one millisecond less must not.
        for display in TrayDisplay::ALL {
            for remaining in [S(1200), S(61), S(60), S(59), MS(1500), MS(1)] {
                let before = tray_label(remaining, display, Phase::Work, Status::Running);
                let Some(delay) =
                    until_tray_label_changes(remaining, display, Phase::Work, Status::Running)
                else {
                    assert_eq!(display, TrayDisplay::IconOnly);
                    continue;
                };
                let at = tray_label(
                    remaining.saturating_sub(delay),
                    display,
                    Phase::Work,
                    Status::Running,
                );
                assert_ne!(
                    before, at,
                    "{display:?} {remaining:?} should change after {delay:?}"
                );
                if delay > MS(1) {
                    let just_before = tray_label(
                        remaining.saturating_sub(delay.saturating_sub(MS(1))),
                        display,
                        Phase::Work,
                        Status::Running,
                    );
                    assert_eq!(
                        before, just_before,
                        "{display:?} {remaining:?} changed too early"
                    );
                }
            }
        }
    }

    #[test]
    fn tray_label_per_display_mode_while_working() {
        assert_eq!(
            tray_label(S(1200), TrayDisplay::MmSs, Phase::Work, Status::Running),
            "20:00"
        );
        assert_eq!(
            tray_label(S(1200), TrayDisplay::Minutes, Phase::Work, Status::Running),
            "20m"
        );
        assert_eq!(
            tray_label(S(45), TrayDisplay::Minutes, Phase::Work, Status::Running),
            "0:45"
        );
        assert_eq!(
            tray_label(S(60), TrayDisplay::Minutes, Phase::Work, Status::Running),
            "1:00"
        );
        assert_eq!(
            tray_label(S(1200), TrayDisplay::IconOnly, Phase::Work, Status::Running),
            ""
        );
    }

    #[test]
    fn tray_label_during_a_break_counts_seconds_in_every_text_mode() {
        assert_eq!(
            tray_label(S(20), TrayDisplay::MmSs, Phase::Break, Status::Running),
            "20s"
        );
        assert_eq!(
            tray_label(S(20), TrayDisplay::Minutes, Phase::Break, Status::Running),
            "20s"
        );
        assert_eq!(
            tray_label(S(20), TrayDisplay::IconOnly, Phase::Break, Status::Running),
            ""
        );
    }

    #[test]
    fn tray_label_hides_a_stale_countdown_when_not_counting() {
        for display in TrayDisplay::ALL {
            assert_eq!(
                tray_label(S(600), display, Phase::Work, Status::Suspended),
                ""
            );
            assert_eq!(
                until_tray_label_changes(S(600), display, Phase::Work, Status::Suspended),
                None
            );
        }
        assert_eq!(
            tray_label(S(600), TrayDisplay::MmSs, Phase::Work, Status::PausedByUser),
            "--"
        );
        assert_eq!(
            tray_label(
                S(600),
                TrayDisplay::IconOnly,
                Phase::Work,
                Status::PausedByUser
            ),
            ""
        );
    }

    #[test]
    fn menu_header_explains_why_the_timer_is_not_counting() {
        let mut reasons = ReasonSet::default();
        assert_eq!(
            menu_header(S(600), Phase::Work, Status::Running, reasons),
            "Next break in 10m"
        );
        assert_eq!(
            menu_header(S(30), Phase::Work, Status::Running, reasons),
            "Next break in 1m"
        );
        assert_eq!(
            menu_header(S(12), Phase::Break, Status::Running, reasons),
            "Look away: 12s"
        );
        assert_eq!(
            menu_header(S(600), Phase::Work, Status::PausedByUser, reasons),
            "Paused"
        );

        reasons.insert(SuspendReason::ScreenLock);
        assert_eq!(
            menu_header(S(600), Phase::Work, Status::Suspended, reasons),
            "Paused - screen is locked"
        );
        reasons.insert(SuspendReason::Sleep);
        assert_eq!(
            menu_header(S(600), Phase::Work, Status::Suspended, reasons),
            "Paused - machine was asleep",
            "sleep outranks the lock that always accompanies it"
        );

        let mut only = ReasonSet::default();
        only.insert(SuspendReason::DisplayOff);
        assert_eq!(
            menu_header(S(600), Phase::Work, Status::Suspended, only),
            "Paused - display is off"
        );
        let mut only = ReasonSet::default();
        only.insert(SuspendReason::SessionInactive);
        assert_eq!(
            menu_header(S(600), Phase::Work, Status::Suspended, only),
            "Paused - session is inactive"
        );
        assert_eq!(
            menu_header(S(600), Phase::Work, Status::Suspended, ReasonSet::default()),
            "Paused"
        );
    }

    #[test]
    fn menu_header_refresh_is_coarse_while_working_and_fine_during_a_break() {
        assert_eq!(
            menu_header(S(1), Phase::Work, Status::Running, ReasonSet::default()),
            "Next break in 1m"
        );
        assert_eq!(
            until_menu_header_changes(S(1200), Phase::Work, Status::Running),
            Some(S(60))
        );
        assert_eq!(
            until_menu_header_changes(S(30), Phase::Work, Status::Running),
            Some(S(30))
        );
        assert_eq!(
            until_menu_header_changes(S(20), Phase::Break, Status::Running),
            Some(S(1))
        );
        assert_eq!(
            until_menu_header_changes(S(20), Phase::Break, Status::PausedByUser),
            None
        );
    }

    #[test]
    fn a_minutes_icon_badge_shows_minutes_then_break_seconds() {
        let badge = |d, phase, status| icon_badge(d, TrayDisplay::Minutes, phase, status);
        assert_eq!(
            badge(S(1200), Phase::Work, Status::Running),
            Some(Badge::Number(20))
        );
        assert_eq!(
            badge(S(1), Phase::Work, Status::Running),
            Some(Badge::Number(1))
        );
        assert_eq!(
            badge(S(20), Phase::Break, Status::Running),
            Some(Badge::Number(20))
        );
        assert_eq!(
            badge(S(180 * 60), Phase::Work, Status::Running),
            Some(Badge::Number(99)),
            "clamped to 2 digits"
        );
        assert_eq!(badge(S(600), Phase::Work, Status::Suspended), None);
        assert_eq!(badge(S(600), Phase::Work, Status::PausedByUser), None);
    }

    #[test]
    fn a_countdown_icon_badge_stacks_minutes_over_seconds() {
        let badge = |d, phase, status| icon_badge(d, TrayDisplay::MmSs, phase, status);
        let stacked = |minutes, seconds| Some(Badge::Stacked { minutes, seconds });
        assert_eq!(badge(S(1200), Phase::Work, Status::Running), stacked(20, 0));
        assert_eq!(
            badge(S(1182), Phase::Work, Status::Running),
            stacked(19, 42)
        );
        assert_eq!(
            badge(MS(59_001), Phase::Work, Status::Running),
            stacked(1, 0),
            "rounded up, like every other countdown"
        );
        assert_eq!(badge(S(7), Phase::Work, Status::Running), stacked(0, 7));
        assert_eq!(
            badge(S(180 * 60), Phase::Work, Status::Running),
            stacked(99, 0),
            "the minutes are clamped to 2 digits"
        );
        assert_eq!(
            badge(S(20), Phase::Break, Status::Running),
            Some(Badge::Number(20)),
            "the break is short enough to need no minutes"
        );
        assert_eq!(badge(S(600), Phase::Work, Status::Suspended), None);
    }

    #[test]
    fn an_icon_only_display_has_no_badge_and_never_wakes_for_one() {
        for phase in [Phase::Work, Phase::Break] {
            assert_eq!(
                icon_badge(S(15), TrayDisplay::IconOnly, phase, Status::Running),
                None
            );
            assert_eq!(
                until_icon_badge_changes(S(15), TrayDisplay::IconOnly, phase, Status::Running),
                None
            );
        }
    }

    #[test]
    fn the_icon_badge_is_refreshed_exactly_as_often_as_it_changes() {
        let until = until_icon_badge_changes;
        assert_eq!(
            until(MS(90_500), TrayDisplay::MmSs, Phase::Work, Status::Running),
            Some(MS(500)),
            "a stacked countdown changes every second"
        );
        assert_eq!(
            until(
                MS(90_500),
                TrayDisplay::Minutes,
                Phase::Work,
                Status::Running
            ),
            Some(MS(30_500)),
            "minutes change once a minute"
        );
        assert_eq!(
            until(
                MS(9_500),
                TrayDisplay::Minutes,
                Phase::Break,
                Status::Running
            ),
            Some(MS(500)),
            "the break counts seconds whatever the display"
        );
        assert_eq!(
            until(S(90), TrayDisplay::MmSs, Phase::Work, Status::PausedByUser),
            None
        );
    }
}
