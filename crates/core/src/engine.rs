//! The timer state machine.
//!
//! [`Engine`] is the only place where the application decides anything. It is driven by
//! [`Event`]s and answers with [`Effect`]s; it never reads a clock, touches a file or
//! calls an OS API, so every rule below is exercised by unit tests rather than by using
//! the app and hoping.
//!
//! # Why a break can be credited without the user doing anything
//!
//! The point of the app is to rest the user's eyes, not to make them acknowledge a
//! notification. If the machine slept, the screen locked or the display went dark, their
//! eyes already rested, so the work interval restarts instead of a reminder firing at an
//! empty desk. Three independent signals feed that logic, in order of reliability:
//!
//! 1. [`Event::Suspend`] / [`Event::Resume`] - real OS power, lock and display events.
//! 2. The two clocks disagreeing - the wall clock advancing further than the monotonic
//!    clock means the machine was suspended even if no event said so. The engine watches
//!    for this itself on every call, so the host does not have to.
//! 3. A [`Event::Tick`] that arrives so late that a whole break could have fitted in the
//!    gap, which means the process was not scheduled and the break window has passed.
//!
//! What is deliberately *not* a signal is the keyboard and mouse going quiet. Reading,
//! watching and listening all leave them untouched while the eyes stay on the screen, so
//! "no input for a minute" restarted the countdown on people who had not looked away at
//! all - and not asking about input means the app has no reason to look at it.
//!
//! # Why counting stops on the *set* of reasons
//!
//! Waking a laptop raises several events in a row - power resume, display on, session
//! unlock - and they do not arrive in a fixed order across platforms. A single "suspended"
//! flag would start counting at the first of them, while the screen is still off. Instead
//! each reason is tracked in a [`ReasonSet`] and counting resumes only once the set is
//! empty, which is the literal reading of "start counting again when the screen is back".

use std::time::Duration;

use crate::clock::{Mono, Now};
use crate::format;
use crate::settings::Settings;
use crate::sleep_detect::DriftTracker;

/// Which half of the cycle the timer is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Phase {
    /// Counting down to the next reminder.
    Work,
    /// The short look-away break is in progress.
    Break,
}

/// Whether the timer is counting, and if not, why not.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Status {
    /// Counting down.
    Running,
    /// The user pressed Pause.
    PausedByUser,
    /// The machine or the screen is away, so there is nothing to count.
    Suspended,
}

/// A reason the machine cannot be counted as "user is looking at the screen".
///
/// Ordered by how much they explain: a wake from sleep outranks the screen unlock that
/// always follows it, so the menu shows the most informative one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SuspendReason {
    /// The machine suspended.
    Sleep,
    /// The display powered off.
    DisplayOff,
    /// The session locked.
    ScreenLock,
    /// Another user's session is in the foreground.
    SessionInactive,
}

impl SuspendReason {
    /// Every reason, most explanatory first.
    pub const ALL: [Self; 4] = [
        Self::Sleep,
        Self::DisplayOff,
        Self::ScreenLock,
        Self::SessionInactive,
    ];

    const fn bit(self) -> u8 {
        match self {
            Self::Sleep => 1,
            Self::DisplayOff => 2,
            Self::ScreenLock => 4,
            Self::SessionInactive => 8,
        }
    }
}

/// The set of currently active [`SuspendReason`]s.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ReasonSet(u8);

impl ReasonSet {
    /// Adds a reason. Returns whether the set changed.
    pub fn insert(&mut self, reason: SuspendReason) -> bool {
        let before = self.0;
        self.0 |= reason.bit();
        before != self.0
    }

    /// Removes a reason. Returns whether the set changed.
    pub fn remove(&mut self, reason: SuspendReason) -> bool {
        let before = self.0;
        self.0 &= !reason.bit();
        before != self.0
    }

    /// Whether `reason` is active.
    #[must_use]
    pub const fn contains(self, reason: SuspendReason) -> bool {
        self.0 & reason.bit() != 0
    }

    /// Whether nothing is holding the timer back.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// How many reasons are active.
    #[must_use]
    pub const fn len(self) -> u32 {
        self.0.count_ones()
    }

    /// Drops every reason. Used when a resume event arrives that implies all of them.
    pub const fn clear(&mut self) {
        self.0 = 0;
    }

    /// The most explanatory active reason.
    #[must_use]
    pub fn primary(self) -> Option<SuspendReason> {
        SuspendReason::ALL.into_iter().find(|r| self.contains(*r))
    }
}

/// Which artwork the tray should show.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IconKind {
    /// Counting down to the next break.
    Work,
    /// A break is in progress.
    Break,
    /// Not counting.
    Paused,
}

/// What to draw into the tray icon in place of the artwork, on a host whose tray cannot
/// show text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Badge {
    /// One number of up to two digits, as large as will fit.
    Number(u16),
    /// Minutes above seconds: the only way to fit `19:42` into a square.
    Stacked {
        /// The top row.
        minutes: u8,
        /// The bottom row.
        seconds: u8,
    },
}

/// What the host's tray is able to display.
///
/// macOS can put text next to the menu bar icon; the Windows notification area cannot, and
/// offers only an icon and a tooltip. The countdown therefore has to be drawn into the
/// icon there, and - because the icon only shows whole minutes - the wake-up schedule is
/// coarser too. Making this an explicit input keeps that difference testable on any host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrayCaps {
    /// Whether text can be shown beside the tray icon.
    pub text_label: bool,
}

impl TrayCaps {
    /// A tray that can show text beside the icon, as on macOS.
    pub const WITH_TEXT: Self = Self { text_label: true };
    /// A tray that shows only an icon and a tooltip, as on Windows.
    pub const ICON_AND_TOOLTIP: Self = Self { text_label: false };

    /// The capabilities of the host this binary was built for.
    #[must_use]
    pub fn for_host() -> Self {
        if cfg!(target_os = "macos") {
            Self::WITH_TEXT
        } else {
            Self::ICON_AND_TOOLTIP
        }
    }
}

/// Everything the tray should be showing, as a value that can be diffed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayView {
    /// Text beside the icon; empty when the host cannot show text or nothing should show.
    pub label: String,
    /// Hover text.
    pub tooltip: String,
    /// Which artwork to draw.
    pub icon: IconKind,
    /// The countdown to draw in place of the artwork, when the host has nowhere else to
    /// put it.
    pub badge: Option<Badge>,
}

/// The state the tray menu needs to reflect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuModel {
    /// Non-interactive first line, e.g. `Next break in 19m`.
    pub header: String,
    /// Label for the pause item, which doubles as the resume item.
    pub pause_label: &'static str,
    /// Whether the user has paused the timer.
    pub paused: bool,
}

/// A desktop notification to post.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notification {
    /// Notification title.
    pub title: String,
    /// Notification body.
    pub body: String,
    /// Whether to play the system sound.
    pub sound: bool,
}

/// Something that happened, from the OS or the user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// The scheduled wake-up arrived. Safe to send at any time.
    Tick,
    /// The machine or screen became unavailable.
    Suspend(SuspendReason),
    /// One reason for unavailability went away.
    Resume(SuspendReason),
    /// Every reason went away at once, e.g. a resume event that implies the rest.
    ResumeAll,
    /// The user chose Pause, or Resume while paused.
    TogglePause,
    /// The user chose "Take a break now".
    TakeBreakNow,
    /// The user chose "Reset timer", declaring the current interval finished.
    ResetCycle,
    /// The settings window applied new settings.
    SettingsChanged(Settings),
    /// The user asked for the settings window, from the menu or a second launch.
    OpenSettingsRequested,
    /// The user chose Quit.
    QuitRequested,
}

/// Something the host should do. The engine never acts, it only asks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Post a desktop notification.
    Notify(Notification),
    /// Set the text beside the tray icon. Hosts without text labels ignore this.
    SetTrayLabel(String),
    /// Set the tray hover text.
    SetTrayTooltip(String),
    /// Redraw the tray icon.
    SetTrayIcon {
        /// Which artwork.
        kind: IconKind,
        /// The countdown to draw into it, if any.
        badge: Option<Badge>,
    },
    /// Update the tray menu.
    SetMenu(MenuModel),
    /// Sleep until this instant, then look at the screen and send [`Event::Tick`].
    ///
    /// The absence of this effect means "nothing is pending": the event loop should block
    /// indefinitely, which is what keeps a paused or suspended app completely off the CPU.
    WakeAt(Mono),
    /// Persist these settings.
    SaveSettings(Settings),
    /// Register or deregister the app as a login item.
    SetAutostart(bool),
    /// Show the settings window.
    OpenSettings,
    /// Quit the process.
    Exit,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Credit {
    /// Resuming after this gap may count as a break already taken.
    IfLongEnough,
    /// The caller is setting the phase explicitly; do not second-guess it.
    No,
}

/// The timer.
#[derive(Clone, Debug)]
pub struct Engine {
    settings: Settings,
    caps: TrayCaps,
    phase: Phase,
    paused_by_user: bool,
    reasons: ReasonSet,
    /// When the current phase ends. Meaningful only while counting.
    ends_at: Mono,
    /// What is left of the current phase. Authoritative while not counting.
    remaining: Duration,
    /// Both clocks as of when counting last stopped. An absence has to be measured on
    /// the wall clock: the monotonic clock does not advance while the machine is asleep,
    /// so a suspend would otherwise look like no time passing at all.
    stopped_at: Now,
    /// Watches the two clocks for a suspend nobody announced.
    drift: DriftTracker,
    last_view: Option<TrayView>,
    last_menu: Option<MenuModel>,
}

impl Engine {
    /// Starts a fresh work interval.
    ///
    /// Returns the effects needed to bring the tray from nothing to its initial state.
    #[must_use]
    pub fn new(settings: Settings, caps: TrayCaps, now: Now) -> (Self, Vec<Effect>) {
        let settings = settings.clamped();
        let mut engine = Self {
            settings,
            caps,
            phase: Phase::Work,
            paused_by_user: false,
            reasons: ReasonSet::default(),
            ends_at: now.mono.add(settings.work_interval()),
            remaining: settings.work_interval(),
            stopped_at: now,
            drift: DriftTracker::new(now.mono, now.wall),
            last_view: None,
            last_menu: None,
        };
        let effects = engine.render(now);
        (engine, effects)
    }

    /// The settings currently in force.
    #[must_use]
    pub const fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Which half of the cycle the timer is in.
    #[must_use]
    pub const fn phase(&self) -> Phase {
        self.phase
    }

    /// The reasons the machine is currently considered away.
    #[must_use]
    pub const fn reasons(&self) -> ReasonSet {
        self.reasons
    }

    /// Whether the timer is counting, and if not, why not.
    #[must_use]
    pub const fn status(&self) -> Status {
        if self.paused_by_user {
            Status::PausedByUser
        } else if self.reasons.is_empty() {
            Status::Running
        } else {
            Status::Suspended
        }
    }

    /// Whether time is currently passing for the timer.
    #[must_use]
    pub const fn is_counting(&self) -> bool {
        matches!(self.status(), Status::Running)
    }

    /// How much of the current phase is left.
    #[must_use]
    pub fn remaining(&self, now: Now) -> Duration {
        if self.is_counting() {
            self.ends_at.since(now.mono)
        } else {
            self.remaining
        }
    }

    /// Everything the tray should be showing right now.
    #[must_use]
    pub fn view(&self, now: Now) -> TrayView {
        let remaining = self.remaining(now);
        let status = self.status();
        let label = if self.caps.text_label {
            format::tray_label(remaining, self.settings.tray_display, self.phase, status)
        } else {
            String::new()
        };
        let badge = if self.caps.text_label {
            None
        } else {
            format::icon_badge(remaining, self.settings.tray_display, self.phase, status)
        };
        TrayView {
            label,
            tooltip: format!(
                "{} - {}",
                crate::APP_NAME,
                format::menu_header(remaining, self.phase, status, self.reasons)
            ),
            icon: self.icon_kind(),
            badge,
        }
    }

    /// The state the tray menu should reflect right now.
    #[must_use]
    pub fn menu(&self, now: Now) -> MenuModel {
        MenuModel {
            header: format::menu_header(
                self.remaining(now),
                self.phase,
                self.status(),
                self.reasons,
            ),
            pause_label: if self.paused_by_user {
                "Resume"
            } else {
                "Pause"
            },
            paused: self.paused_by_user,
        }
    }

    /// When the host should wake the engine next, or `None` to sleep indefinitely.
    #[must_use]
    pub fn next_wake(&self, now: Now) -> Option<Mono> {
        if !self.is_counting() {
            return None;
        }
        let remaining = self.remaining(now);
        let status = self.status();
        let now = now.mono;

        // Never schedule a wake-up before the end of the phase; never later than the next
        // moment a visible string goes stale.
        let mut at = self.ends_at.max(now);
        let mut deadlines = Vec::with_capacity(3);
        if self.caps.text_label {
            deadlines.extend(format::until_tray_label_changes(
                remaining,
                self.settings.tray_display,
                self.phase,
                status,
            ));
        } else {
            deadlines.extend(format::until_icon_badge_changes(
                remaining,
                self.settings.tray_display,
                self.phase,
                status,
            ));
        }
        // The menu header is also the tooltip, so it is relevant on every host.
        deadlines.extend(format::until_menu_header_changes(
            remaining, self.phase, status,
        ));
        for d in deadlines {
            at = at.min(now.add(d));
        }

        // The host looks at the screen on every wake, and is told when it changes; this
        // bounds how long a missed notification can go unnoticed while a coarse display
        // would otherwise sleep for a minute at a time.
        Some(at.min(now.add(crate::screen::SAMPLE_INTERVAL)))
    }

    /// When the host should wake next, including the poll that notices the screen coming
    /// back.
    ///
    /// [`Self::next_wake`] answers "when does the timer need me", which is `None` while
    /// nothing is being counted. That is the right answer for a paused timer - the user
    /// will click something - but not for a suspended one: the host should be told when
    /// the screen comes back, and this is the fallback look in case it is not.
    #[must_use]
    pub fn host_deadline(&self, now: Now) -> Option<Mono> {
        match self.status() {
            Status::Suspended => Some(
                self.next_wake(now)
                    .unwrap_or_else(|| now.mono.add(crate::screen::SAMPLE_INTERVAL)),
            ),
            Status::Running | Status::PausedByUser => self.next_wake(now),
        }
    }

    /// Feeds one event to the state machine and returns what the host should do.
    pub fn handle(&mut self, event: Event, now: Now) -> Vec<Effect> {
        // Before anything else: did the machine sleep without saying so? A wall clock that
        // has run ahead of the monotonic clock is the only evidence left of a suspend that
        // raised no power event, and it costs two subtractions to check.
        if let Some(away) = self.drift.sample(now.mono, now.wall)
            && self.is_counting()
            && away >= self.settings.break_duration()
        {
            self.begin(Phase::Work, now);
        }
        match event {
            Event::Tick => self.on_tick(now),
            Event::Suspend(reason) => self.transition(now, Credit::No, |s| {
                s.reasons.insert(reason);
            }),
            Event::Resume(reason) => self.transition(now, Credit::IfLongEnough, |s| {
                s.reasons.remove(reason);
            }),
            Event::ResumeAll => self.transition(now, Credit::IfLongEnough, |s| {
                s.reasons.clear();
            }),
            Event::TogglePause => {
                let credit = if self.paused_by_user {
                    Credit::IfLongEnough
                } else {
                    Credit::No
                };
                self.transition(now, credit, |s| s.paused_by_user = !s.paused_by_user)
            }
            Event::TakeBreakNow => self.on_take_break_now(now),
            Event::ResetCycle => self.transition(now, Credit::No, |s| {
                s.phase = Phase::Work;
                s.remaining = s.settings.work_interval();
            }),
            Event::SettingsChanged(settings) => self.on_settings(settings, now),
            Event::OpenSettingsRequested => {
                // Still re-render: the host treats a batch without a WakeAt as "sleep
                // until something happens", so dropping it here would stall the countdown
                // for as long as the settings window stayed open.
                let mut effects = vec![Effect::OpenSettings];
                effects.extend(self.render(now));
                effects
            }
            Event::QuitRequested => vec![Effect::Exit],
        }
    }

    fn on_tick(&mut self, now: Now) -> Vec<Effect> {
        if !self.is_counting() || now.mono < self.ends_at {
            return self.render(now);
        }
        let overshoot = now.mono.since(self.ends_at);
        let mut effects = Vec::new();
        match self.phase {
            Phase::Work if overshoot >= self.settings.break_duration() => {
                // This tick is so late that the whole break could have happened inside the
                // gap - the process was descheduled, or the machine was suspended without
                // telling us. Reminding the user now would be reminding them about a break
                // whose window has already closed, so start a fresh interval silently.
                self.begin(Phase::Work, now);
            }
            Phase::Work => {
                self.begin(Phase::Break, now);
                effects.push(Effect::Notify(self.look_away_notification()));
            }
            Phase::Break => self.begin(Phase::Work, now),
        }
        effects.extend(self.render(now));
        effects
    }

    fn on_take_break_now(&mut self, now: Now) -> Vec<Effect> {
        let mut effects = vec![Effect::Notify(self.look_away_notification())];
        effects.extend(self.transition(now, Credit::No, |s| {
            s.paused_by_user = false;
            s.phase = Phase::Break;
            s.remaining = s.settings.break_duration();
        }));
        effects
    }

    fn on_settings(&mut self, incoming: Settings, now: Now) -> Vec<Effect> {
        let next = incoming.clamped();
        if next == self.settings {
            return self.render(now);
        }
        let autostart_changed = next.start_at_login != self.settings.start_at_login;
        let timings_changed = next.work_interval_secs != self.settings.work_interval_secs
            || next.break_duration_secs != self.settings.break_duration_secs;
        self.settings = next;

        let mut effects = vec![Effect::SaveSettings(next)];
        if autostart_changed {
            effects.push(Effect::SetAutostart(next.start_at_login));
        }
        if timings_changed {
            // "The interval is now 15 minutes" should mean the next break is 15 minutes
            // away, not some scaled fraction of the old interval.
            self.begin(self.phase, now);
        }
        effects.extend(self.render(now));
        effects
    }

    /// Starts `phase` from the beginning, as of `now`.
    fn begin(&mut self, phase: Phase, now: Now) {
        self.phase = phase;
        self.remaining = match phase {
            Phase::Work => self.settings.work_interval(),
            Phase::Break => self.settings.break_duration(),
        };
        self.ends_at = now.mono.add(self.remaining);
    }

    /// Applies a state change, converting between a deadline and a remaining duration
    /// whenever that change starts or stops the clock.
    ///
    /// Centralising this is what keeps pause, lock, sleep and display-off from each having
    /// their own arithmetic - and their own off-by-one bugs.
    fn transition(
        &mut self,
        now: Now,
        credit: Credit,
        change: impl FnOnce(&mut Self),
    ) -> Vec<Effect> {
        let was_counting = self.is_counting();
        if was_counting {
            self.remaining = self.ends_at.since(now.mono);
        }

        change(self);

        let counting = self.is_counting();
        if was_counting && !counting {
            self.stopped_at = now;
        }
        if !was_counting && counting && credit == Credit::IfLongEnough {
            // The longer of the two readings. The wall clock is the one that moves while
            // the machine is suspended; the monotonic clock is the one that survives the
            // user changing the date.
            let away = now
                .wall
                .since(self.stopped_at.wall)
                .max(now.mono.since(self.stopped_at.mono));
            if away >= self.settings.break_duration() {
                self.phase = Phase::Work;
                self.remaining = self.settings.work_interval();
            }
        }
        if counting {
            self.ends_at = now.mono.add(self.remaining);
        }
        self.render(now)
    }

    fn icon_kind(&self) -> IconKind {
        match self.status() {
            Status::Running => match self.phase {
                Phase::Work => IconKind::Work,
                Phase::Break => IconKind::Break,
            },
            Status::PausedByUser | Status::Suspended => IconKind::Paused,
        }
    }

    fn look_away_notification(&self) -> Notification {
        Notification {
            title: "Look away".to_owned(),
            body: format!(
                "Focus on something about 20 feet away for {}.",
                plural_secs(self.settings.break_duration_secs)
            ),
            sound: self.settings.notification_sound,
        }
    }

    /// Emits only the parts of the display that actually changed, plus the next wake-up.
    ///
    /// The filtering matters: with a one-second countdown this runs 60 times a minute, and
    /// every suppressed effect is an OS call not made.
    fn render(&mut self, now: Now) -> Vec<Effect> {
        let view = self.view(now);
        let mut out = Vec::new();
        let previous = self.last_view.as_ref();
        if previous.map(|v| v.label.as_str()) != Some(view.label.as_str()) {
            out.push(Effect::SetTrayLabel(view.label.clone()));
        }
        if previous.map(|v| v.tooltip.as_str()) != Some(view.tooltip.as_str()) {
            out.push(Effect::SetTrayTooltip(view.tooltip.clone()));
        }
        if previous.map(|v| (v.icon, v.badge)) != Some((view.icon, view.badge)) {
            out.push(Effect::SetTrayIcon {
                kind: view.icon,
                badge: view.badge,
            });
        }
        self.last_view = Some(view);

        let menu = self.menu(now);
        if self.last_menu.as_ref() != Some(&menu) {
            out.push(Effect::SetMenu(menu.clone()));
            self.last_menu = Some(menu);
        }

        if let Some(at) = self.next_wake(now) {
            out.push(Effect::WakeAt(at));
        }
        out
    }
}

/// `20 seconds`, `1 second`.
fn plural_secs(secs: u64) -> String {
    if secs == 1 {
        "1 second".to_owned()
    } else {
        format!("{secs} seconds")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::TrayDisplay;

    const T0: Now = Now::new(
        Mono::from_secs(10_000),
        crate::clock::Wall::from_secs(1_700_000_000),
    );
    const WORK: Duration = Duration::from_secs(1200);
    const BREAK: Duration = Duration::from_secs(20);
    const SECOND: Duration = Duration::from_secs(1);

    fn engine() -> Engine {
        Engine::new(Settings::default(), TrayCaps::WITH_TEXT, T0).0
    }

    fn engine_with(settings: Settings) -> Engine {
        Engine::new(settings, TrayCaps::WITH_TEXT, T0).0
    }

    fn labels(effects: &[Effect]) -> Vec<&str> {
        effects
            .iter()
            .filter_map(|e| match e {
                Effect::SetTrayLabel(l) => Some(l.as_str()),
                _ => None,
            })
            .collect()
    }

    fn notes(effects: &[Effect]) -> Vec<&Notification> {
        effects
            .iter()
            .filter_map(|e| match e {
                Effect::Notify(n) => Some(n),
                _ => None,
            })
            .collect()
    }

    fn icons(effects: &[Effect]) -> Vec<(IconKind, Option<Badge>)> {
        effects
            .iter()
            .filter_map(|e| match e {
                Effect::SetTrayIcon { kind, badge } => Some((*kind, *badge)),
                _ => None,
            })
            .collect()
    }

    fn wake(effects: &[Effect]) -> Option<Mono> {
        effects.iter().rev().find_map(|e| match e {
            Effect::WakeAt(at) => Some(*at),
            _ => None,
        })
    }

    // --- construction -------------------------------------------------------------

    #[test]
    fn a_new_engine_starts_a_full_work_interval_and_paints_everything() {
        let (engine, effects) = Engine::new(Settings::default(), TrayCaps::WITH_TEXT, T0);
        assert_eq!(engine.phase(), Phase::Work);
        assert_eq!(engine.status(), Status::Running);
        assert!(engine.is_counting());
        assert_eq!(engine.remaining(T0), WORK);
        assert_eq!(labels(&effects), ["20:00"]);
        assert_eq!(icons(&effects), [(IconKind::Work, None)]);
        assert_eq!(wake(&effects), Some(T0.add(SECOND).mono));
        assert!(effects.iter().any(
            |e| matches!(e, Effect::SetTrayTooltip(t) if t == "EyeSaver - Next break in 20m")
        ));
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::SetMenu(m) if m.header == "Next break in 20m"))
        );
    }

    #[test]
    fn construction_clamps_hostile_settings() {
        let engine = engine_with(Settings {
            work_interval_secs: 1,
            ..Settings::default()
        });
        assert_eq!(
            engine.settings().work_interval_secs,
            crate::settings::WORK_SECS.0
        );
        assert_eq!(engine.remaining(T0), Duration::from_secs(60));
    }

    // --- the ordinary cycle -------------------------------------------------------

    #[test]
    fn ticking_before_the_deadline_only_refreshes_the_countdown() {
        let mut e = engine();
        let effects = e.handle(Event::Tick, T0.add(SECOND));
        assert_eq!(labels(&effects), ["19:59"]);
        assert!(notes(&effects).is_empty());
        assert!(
            icons(&effects).is_empty(),
            "the icon has not changed, so it must not be redrawn"
        );
        assert_eq!(e.phase(), Phase::Work);
    }

    #[test]
    fn ticking_twice_inside_one_second_paints_nothing() {
        let mut e = engine();
        e.handle(Event::Tick, T0.add(Duration::from_millis(1200)));
        let effects = e.handle(Event::Tick, T0.add(Duration::from_millis(1700)));
        assert!(labels(&effects).is_empty(), "same label, no OS call");
        assert_eq!(effects.len(), 1, "only the next wake-up: {effects:?}");
        assert!(matches!(effects[0], Effect::WakeAt(_)));
    }

    #[test]
    fn the_work_interval_ending_notifies_and_starts_the_break() {
        let mut e = engine();
        let effects = e.handle(Event::Tick, T0.add(WORK));
        assert_eq!(notes(&effects).len(), 1);
        assert_eq!(notes(&effects)[0].title, "Look away");
        assert_eq!(
            notes(&effects)[0].body,
            "Focus on something about 20 feet away for 20 seconds."
        );
        assert!(notes(&effects)[0].sound);
        assert_eq!(e.phase(), Phase::Break);
        assert_eq!(e.remaining(T0.add(WORK)), BREAK);
        assert_eq!(labels(&effects), ["20s"]);
        assert_eq!(icons(&effects), [(IconKind::Break, None)]);
    }

    #[test]
    fn the_break_ending_silently_starts_the_next_interval() {
        let mut e = engine();
        e.handle(Event::Tick, T0.add(WORK));
        let at = T0.add(WORK).add(BREAK);
        let effects = e.handle(Event::Tick, at);
        assert!(
            notes(&effects).is_empty(),
            "nothing to announce: they may look back now"
        );
        assert_eq!(e.phase(), Phase::Work);
        assert_eq!(e.remaining(at), WORK);
        assert_eq!(icons(&effects), [(IconKind::Work, None)]);
    }

    #[test]
    fn the_notification_follows_the_configured_break_length_and_sound() {
        let mut e = engine_with(Settings {
            break_duration_secs: 1, // below the minimum, so clamped to five
            notification_sound: false,
            ..Settings::default()
        });
        assert_eq!(
            e.settings().break_duration_secs,
            crate::settings::BREAK_SECS.0
        );
        let effects = e.handle(Event::Tick, T0.add(WORK));
        assert_eq!(
            notes(&effects)[0].body,
            "Focus on something about 20 feet away for 5 seconds."
        );
        assert!(!notes(&effects)[0].sound);
    }

    #[test]
    fn a_tick_that_arrives_after_the_whole_break_window_does_not_nag() {
        // The process was descheduled for an hour: reminding the user to look away now
        // would be reminding them about a break that is already over.
        let mut e = engine();
        let at = T0.add(WORK).add(Duration::from_secs(3600));
        let effects = e.handle(Event::Tick, at);
        assert!(notes(&effects).is_empty());
        assert_eq!(e.phase(), Phase::Work);
        assert_eq!(e.remaining(at), WORK);
    }

    #[test]
    fn a_tick_only_slightly_late_still_notifies() {
        let mut e = engine();
        let at = T0.add(WORK).add(Duration::from_secs(5));
        let effects = e.handle(Event::Tick, at);
        assert_eq!(notes(&effects).len(), 1);
        assert_eq!(e.phase(), Phase::Break);
        assert_eq!(
            e.remaining(at),
            BREAK,
            "the break gets its full length from now"
        );
    }

    // --- suspend and resume -------------------------------------------------------

    #[test]
    fn suspending_stops_the_clock_clears_the_countdown_and_asks_for_no_wake_ups() {
        let mut e = engine();
        e.handle(Event::Tick, T0.add(Duration::from_secs(300)));
        let at = T0.add(Duration::from_secs(300));
        let effects = e.handle(Event::Suspend(SuspendReason::Sleep), at);

        assert_eq!(e.status(), Status::Suspended);
        assert!(!e.is_counting());
        assert_eq!(e.remaining(at), Duration::from_secs(900));
        assert_eq!(labels(&effects), [""], "a frozen countdown would be a lie");
        assert_eq!(icons(&effects), [(IconKind::Paused, None)]);
        assert_eq!(
            wake(&effects),
            None,
            "nothing to count, so nothing to wake for"
        );
    }

    #[test]
    fn a_suspended_engine_ignores_ticks_and_never_fires() {
        let mut e = engine();
        e.handle(Event::Suspend(SuspendReason::Sleep), T0);
        for minutes in 1..=90 {
            let effects = e.handle(Event::Tick, T0.add(Duration::from_secs(minutes * 60)));
            assert!(
                notes(&effects).is_empty(),
                "fired while asleep after {minutes}m"
            );
            assert_eq!(wake(&effects), None);
        }
        assert_eq!(e.remaining(T0.add(Duration::from_secs(5400))), WORK);
    }

    #[test]
    fn resuming_after_long_enough_away_counts_as_a_break_taken() {
        let mut e = engine();
        let suspend_at = T0.add(Duration::from_secs(600));
        e.handle(Event::Suspend(SuspendReason::Sleep), suspend_at);
        let resume_at = suspend_at.add(Duration::from_secs(1800));
        e.handle(Event::Resume(SuspendReason::Sleep), resume_at);

        assert_eq!(e.status(), Status::Running);
        assert_eq!(e.phase(), Phase::Work);
        assert_eq!(
            e.remaining(resume_at),
            WORK,
            "a fresh interval, not the old 10 minutes"
        );
    }

    #[test]
    fn resuming_after_a_blink_keeps_the_countdown_where_it_was() {
        let mut e = engine();
        let suspend_at = T0.add(Duration::from_secs(600));
        e.handle(Event::Suspend(SuspendReason::DisplayOff), suspend_at);
        let resume_at = suspend_at.add(Duration::from_secs(5));
        e.handle(Event::Resume(SuspendReason::DisplayOff), resume_at);

        assert_eq!(e.remaining(resume_at), Duration::from_secs(600));
    }

    #[test]
    fn an_away_gap_exactly_as_long_as_a_break_counts() {
        let mut e = engine();
        e.handle(Event::Suspend(SuspendReason::Sleep), T0);
        let resume_at = T0.add(BREAK);
        e.handle(Event::Resume(SuspendReason::Sleep), resume_at);
        assert_eq!(e.remaining(resume_at), WORK);
    }

    #[test]
    fn counting_resumes_only_once_every_reason_has_cleared() {
        // A laptop waking up raises power, display and lock events in an order that
        // differs between platforms and between wakes. Counting must not restart while
        // the screen is still off.
        let mut e = engine();
        e.handle(Event::Suspend(SuspendReason::Sleep), T0);
        e.handle(Event::Suspend(SuspendReason::DisplayOff), T0);
        e.handle(Event::Suspend(SuspendReason::ScreenLock), T0);
        assert_eq!(e.reasons().len(), 3);

        let mut at = T0.add(Duration::from_secs(3600));
        for reason in [SuspendReason::Sleep, SuspendReason::DisplayOff] {
            at = at.add(SECOND);
            let effects = e.handle(Event::Resume(reason), at);
            assert!(!e.is_counting(), "still waiting after {reason:?}");
            assert_eq!(wake(&effects), None);
        }

        at = at.add(SECOND);
        let effects = e.handle(Event::Resume(SuspendReason::ScreenLock), at);
        assert!(e.is_counting());
        assert!(e.reasons().is_empty());
        assert_eq!(e.remaining(at), WORK);
        assert_eq!(labels(&effects), ["20:00"]);
        assert!(wake(&effects).is_some());
    }

    #[test]
    fn resume_all_clears_every_reason_at_once() {
        let mut e = engine();
        e.handle(Event::Suspend(SuspendReason::Sleep), T0);
        e.handle(Event::Suspend(SuspendReason::ScreenLock), T0);
        let at = T0.add(Duration::from_secs(600));
        e.handle(Event::ResumeAll, at);
        assert!(e.is_counting());
        assert_eq!(e.remaining(at), WORK);
    }

    #[test]
    fn repeated_and_unmatched_power_events_are_harmless() {
        let mut e = engine();
        e.handle(Event::Resume(SuspendReason::Sleep), T0);
        assert!(
            e.is_counting(),
            "a resume with nothing to resume changes nothing"
        );

        e.handle(Event::Suspend(SuspendReason::Sleep), T0);
        e.handle(Event::Suspend(SuspendReason::Sleep), T0.add(SECOND));
        assert_eq!(e.reasons().len(), 1, "duplicate suspends do not stack");

        let at = T0.add(Duration::from_secs(600));
        e.handle(Event::Resume(SuspendReason::Sleep), at);
        e.handle(Event::Resume(SuspendReason::Sleep), at);
        assert!(e.is_counting());
    }

    #[test]
    fn suspending_during_a_break_and_coming_back_later_starts_work_afresh() {
        let mut e = engine();
        e.handle(Event::Tick, T0.add(WORK));
        let at = T0.add(WORK).add(Duration::from_secs(5));
        e.handle(Event::Suspend(SuspendReason::ScreenLock), at);
        assert_eq!(e.phase(), Phase::Break);

        let back = at.add(Duration::from_secs(300));
        e.handle(Event::Resume(SuspendReason::ScreenLock), back);
        assert_eq!(e.phase(), Phase::Work);
        assert_eq!(e.remaining(back), WORK);
    }

    // --- drift ---------------------------------------------------------------------

    #[test]
    fn a_suspend_that_raised_no_event_is_noticed_from_the_clocks_alone() {
        let mut e = engine();
        // Half an hour of wall time with the monotonic clock frozen: the machine was
        // asleep and nothing told us.
        let at = T0
            .add(Duration::from_secs(600))
            .suspend(Duration::from_secs(1800));
        let effects = e.handle(Event::Tick, at);
        assert_eq!(e.remaining(at), WORK);
        assert!(
            notes(&effects).is_empty(),
            "no reminder for a break already taken"
        );
    }

    #[test]
    fn a_gap_too_short_to_rest_anyone_does_not_restart_the_interval() {
        let mut e = engine();
        let at = T0
            .add(Duration::from_secs(600))
            .suspend(Duration::from_secs(3));
        e.handle(Event::Tick, at);
        assert_eq!(e.remaining(at), Duration::from_secs(600));
    }

    #[test]
    fn clock_divergence_while_already_suspended_is_left_to_the_resume() {
        let mut e = engine();
        e.handle(Event::Suspend(SuspendReason::Sleep), T0);
        let at = T0.suspend(Duration::from_secs(1800));
        e.handle(Event::Tick, at);
        assert_eq!(e.phase(), Phase::Work);
        assert!(!e.is_counting());
    }

    #[test]
    fn a_wall_clock_correction_is_not_mistaken_for_a_nap() {
        let mut e = engine();
        let at = T0
            .add(Duration::from_secs(600))
            .rewind_wall(Duration::from_secs(7200));
        e.handle(Event::Tick, at);
        assert_eq!(e.remaining(at), Duration::from_secs(600));
    }

    // --- the screen cadence -----------------------------------------------------------

    #[test]
    fn a_coarse_display_is_still_woken_often_enough_to_watch_the_screen() {
        let mut e = engine_with(Settings {
            tray_display: TrayDisplay::IconOnly,
            ..Settings::default()
        });
        // With no text to refresh, looking at the screen is what sets the wake-up.
        let effects = e.handle(Event::Tick, T0);
        assert_eq!(
            wake(&effects),
            Some(T0.add(crate::screen::SAMPLE_INTERVAL).mono)
        );
        // And it is measured from each wake, so the gap never grows.
        let at = T0.add(Duration::from_secs(7));
        assert_eq!(
            e.next_wake(at),
            Some(at.add(crate::screen::SAMPLE_INTERVAL).mono)
        );
    }

    // --- user actions ---------------------------------------------------------------

    #[test]
    fn pausing_stops_the_clock_and_the_wake_ups() {
        let mut e = engine();
        let at = T0.add(Duration::from_secs(600));
        let effects = e.handle(Event::TogglePause, at);

        assert_eq!(e.status(), Status::PausedByUser);
        assert_eq!(e.remaining(at), Duration::from_secs(600));
        assert_eq!(labels(&effects), ["--"]);
        assert_eq!(icons(&effects), [(IconKind::Paused, None)]);
        assert_eq!(wake(&effects), None);
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::SetMenu(m) if m.pause_label == "Resume" && m.paused))
        );
    }

    #[test]
    fn a_long_pause_counts_as_a_break_and_a_short_one_does_not() {
        let mut e = engine();
        let pause_at = T0.add(Duration::from_secs(600));
        e.handle(Event::TogglePause, pause_at);
        let resume_at = pause_at.add(Duration::from_secs(300));
        e.handle(Event::TogglePause, resume_at);
        assert_eq!(e.remaining(resume_at), WORK);

        let mut e = engine();
        e.handle(Event::TogglePause, pause_at);
        let resume_at = pause_at.add(Duration::from_secs(2));
        e.handle(Event::TogglePause, resume_at);
        assert_eq!(e.remaining(resume_at), Duration::from_secs(600));
    }

    #[test]
    fn pausing_while_suspended_keeps_the_timer_stopped_after_the_screen_returns() {
        let mut e = engine();
        e.handle(Event::Suspend(SuspendReason::ScreenLock), T0);
        e.handle(Event::TogglePause, T0);
        let at = T0.add(Duration::from_secs(600));
        e.handle(Event::Resume(SuspendReason::ScreenLock), at);
        assert_eq!(e.status(), Status::PausedByUser);
        assert!(!e.is_counting());
    }

    #[test]
    fn taking_a_break_now_notifies_starts_the_break_and_cancels_a_pause() {
        let mut e = engine();
        let at = T0.add(Duration::from_secs(300));
        e.handle(Event::TogglePause, at);
        let effects = e.handle(Event::TakeBreakNow, at);

        assert_eq!(notes(&effects).len(), 1);
        assert_eq!(e.phase(), Phase::Break);
        assert_eq!(e.status(), Status::Running);
        assert_eq!(e.remaining(at), BREAK);
        assert!(
            matches!(effects.first(), Some(Effect::Notify(_))),
            "the notification comes first so the user is told before the icon flips"
        );
    }

    #[test]
    fn resetting_the_cycle_starts_a_fresh_interval_without_a_notification() {
        let mut e = engine();
        let at = T0.add(Duration::from_secs(1100));
        let effects = e.handle(Event::ResetCycle, at);
        assert!(notes(&effects).is_empty());
        assert_eq!(e.phase(), Phase::Work);
        assert_eq!(e.remaining(at), WORK);
    }

    #[test]
    fn resetting_the_cycle_while_paused_leaves_it_paused() {
        let mut e = engine();
        e.handle(Event::TogglePause, T0);
        let at = T0.add(Duration::from_secs(60));
        e.handle(Event::ResetCycle, at);
        assert_eq!(e.status(), Status::PausedByUser);
        assert_eq!(e.remaining(at), WORK);
    }

    #[test]
    fn resetting_during_a_break_ends_the_break() {
        let mut e = engine();
        e.handle(Event::Tick, T0.add(WORK));
        let at = T0.add(WORK).add(Duration::from_secs(2));
        e.handle(Event::ResetCycle, at);
        assert_eq!(e.phase(), Phase::Work);
    }

    #[test]
    fn opening_settings_does_not_stall_the_countdown() {
        let mut e = engine();
        let effects = e.handle(Event::OpenSettingsRequested, T0);
        assert!(effects.contains(&Effect::OpenSettings));
        assert!(
            wake(&effects).is_some(),
            "dropping the wake-up would freeze the timer"
        );
    }

    #[test]
    fn quitting_asks_only_to_exit() {
        let mut e = engine();
        assert_eq!(e.handle(Event::QuitRequested, T0), vec![Effect::Exit]);
    }

    // --- settings -------------------------------------------------------------------

    #[test]
    fn unchanged_settings_are_not_written_back() {
        let mut e = engine();
        let effects = e.handle(Event::SettingsChanged(Settings::default()), T0);
        assert!(
            !effects.iter().any(|e| matches!(e, Effect::SaveSettings(_))),
            "the settings window repaints constantly; writing the file each time is waste"
        );
    }

    #[test]
    fn changing_the_interval_restarts_the_current_phase() {
        let mut e = engine();
        let at = T0.add(Duration::from_secs(600));
        let next = Settings {
            work_interval_secs: 900,
            ..Settings::default()
        };
        let effects = e.handle(Event::SettingsChanged(next), at);

        assert!(effects.contains(&Effect::SaveSettings(next)));
        assert_eq!(
            e.remaining(at),
            Duration::from_secs(900),
            "15 minutes from now"
        );
        assert!(!effects.iter().any(|e| matches!(e, Effect::SetAutostart(_))));
    }

    #[test]
    fn changing_the_break_length_while_on_a_break_restarts_the_break() {
        let mut e = engine();
        e.handle(Event::Tick, T0.add(WORK));
        let at = T0.add(WORK).add(Duration::from_secs(5));
        e.handle(
            Event::SettingsChanged(Settings {
                break_duration_secs: 30,
                ..Settings::default()
            }),
            at,
        );
        assert_eq!(e.phase(), Phase::Break);
        assert_eq!(e.remaining(at), Duration::from_secs(30));
    }

    #[test]
    fn toggling_start_at_login_asks_the_host_to_register() {
        let mut e = engine();
        let next = Settings {
            start_at_login: true,
            ..Settings::default()
        };
        let effects = e.handle(Event::SettingsChanged(next), T0);
        assert!(effects.contains(&Effect::SetAutostart(true)));
        assert!(effects.contains(&Effect::SaveSettings(next)));

        let effects = e.handle(Event::SettingsChanged(Settings::default()), T0);
        assert!(effects.contains(&Effect::SetAutostart(false)));
    }

    #[test]
    fn changing_only_the_display_mode_repaints_without_disturbing_the_countdown() {
        let mut e = engine();
        let at = T0.add(Duration::from_secs(600));
        let next = Settings {
            tray_display: TrayDisplay::Minutes,
            ..Settings::default()
        };
        let effects = e.handle(Event::SettingsChanged(next), at);
        assert_eq!(labels(&effects), ["10m"]);
        assert_eq!(
            e.remaining(at),
            Duration::from_secs(600),
            "the timer kept running"
        );
    }

    #[test]
    fn settings_from_the_window_are_clamped_before_being_stored() {
        let mut e = engine();
        let effects = e.handle(
            Event::SettingsChanged(Settings {
                work_interval_secs: 9_999_999,
                ..Settings::default()
            }),
            T0,
        );
        assert_eq!(
            e.settings().work_interval_secs,
            crate::settings::WORK_SECS.1
        );
        assert!(effects.iter().any(
            |e| matches!(e, Effect::SaveSettings(s) if s.work_interval_secs == crate::settings::WORK_SECS.1)
        ));
    }

    #[test]
    fn settings_changes_while_suspended_do_not_restart_the_clock() {
        let mut e = engine();
        e.handle(Event::Suspend(SuspendReason::Sleep), T0);
        e.handle(
            Event::SettingsChanged(Settings {
                work_interval_secs: 600,
                ..Settings::default()
            }),
            T0,
        );
        assert!(!e.is_counting());
        assert_eq!(e.remaining(T0), Duration::from_secs(600));
    }

    // --- scheduling -------------------------------------------------------------------

    #[test]
    fn a_second_by_second_display_wakes_once_a_second() {
        let mut e = engine();
        let effects = e.handle(Event::Tick, T0.add(SECOND));
        assert_eq!(wake(&effects), Some(T0.add(Duration::from_secs(2)).mono));
    }

    #[test]
    fn a_minutes_display_wakes_only_for_the_screen_check_until_the_last_minute() {
        let mut e = engine_with(Settings {
            tray_display: TrayDisplay::Minutes,
            ..Settings::default()
        });
        assert_eq!(
            e.next_wake(T0),
            Some(T0.add(crate::screen::SAMPLE_INTERVAL).mono),
            "the label is good for a minute; the screen is not left that long"
        );
        // Close to the minute mark it is the label that is due first.
        let at = T0.add(Duration::from_secs(55));
        assert_eq!(e.next_wake(at), Some(T0.add(Duration::from_secs(60)).mono));

        // Inside the final minute the display switches to seconds, and so does the clock.
        let late = T0.add(Duration::from_secs(1170));
        e.handle(Event::Tick, late);
        assert_eq!(e.next_wake(late), Some(late.add(SECOND).mono));
    }

    #[test]
    fn a_break_countdown_wakes_every_second_in_every_display_mode() {
        for display in TrayDisplay::ALL {
            let mut e = engine_with(Settings {
                tray_display: display,
                ..Settings::default()
            });
            e.handle(Event::Tick, T0.add(WORK));
            let at = T0.add(WORK);
            assert_eq!(e.next_wake(at), Some(at.add(SECOND).mono), "{display:?}");
        }
    }

    #[test]
    fn a_stopped_engine_asks_for_no_wake_up_at_all() {
        let mut e = engine();
        e.handle(Event::TogglePause, T0);
        assert_eq!(e.next_wake(T0), None);

        let mut e = engine();
        e.handle(Event::Suspend(SuspendReason::DisplayOff), T0);
        assert_eq!(e.next_wake(T0), None);
    }

    #[test]
    fn a_suspended_engine_still_looks_for_the_screen_coming_back() {
        let mut e = engine();
        e.handle(Event::Suspend(SuspendReason::DisplayOff), T0);
        assert_eq!(e.next_wake(T0), None, "the timer itself needs nothing");
        assert_eq!(
            e.host_deadline(T0),
            Some(T0.add(crate::screen::SAMPLE_INTERVAL).mono),
            "but a missed notification must not leave it waiting for ever"
        );
    }

    #[test]
    fn a_paused_engine_asks_for_nothing_at_all() {
        let mut e = engine();
        e.handle(Event::TogglePause, T0);
        assert_eq!(
            e.host_deadline(T0),
            None,
            "the user will click when they are ready"
        );
    }

    #[test]
    fn a_running_engine_uses_its_own_schedule_as_the_host_deadline() {
        let e = engine();
        assert_eq!(e.host_deadline(T0), e.next_wake(T0));
    }

    #[test]
    fn an_overdue_engine_asks_to_wake_immediately() {
        let e = engine();
        let late = T0.add(WORK).add(Duration::from_secs(10));
        assert_eq!(e.next_wake(late), Some(late.mono));
    }

    #[test]
    fn every_event_except_quit_re_arms_the_wake_up_while_counting() {
        let events = [
            Event::Tick,
            Event::ResetCycle,
            Event::TakeBreakNow,
            Event::OpenSettingsRequested,
            Event::SettingsChanged(Settings {
                work_interval_secs: 777,
                ..Settings::default()
            }),
            Event::Resume(SuspendReason::Sleep),
            Event::ResumeAll,
        ];
        for event in events {
            let mut e = engine();
            let effects = e.handle(event, T0.add(SECOND));
            assert!(e.is_counting());
            assert_eq!(
                effects
                    .iter()
                    .filter(|e| matches!(e, Effect::WakeAt(_)))
                    .count(),
                1,
                "{event:?} must leave exactly one wake-up armed"
            );
        }
    }

    // --- hosts without a text label ---------------------------------------------------

    #[test]
    fn a_tray_without_text_stacks_the_countdown_into_the_icon() {
        let (mut e, effects) = Engine::new(Settings::default(), TrayCaps::ICON_AND_TOOLTIP, T0);
        assert_eq!(labels(&effects), [""], "nowhere to put text");
        let stacked = |minutes, seconds| Some(Badge::Stacked { minutes, seconds });
        assert_eq!(icons(&effects), [(IconKind::Work, stacked(20, 0))]);
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::SetTrayTooltip(t) if t.contains("20m")))
        );

        // The seconds are on show, so it has to be woken for every one of them.
        assert_eq!(e.next_wake(T0), Some(T0.add(SECOND).mono));
        let effects = e.handle(Event::Tick, T0.add(SECOND));
        assert_eq!(icons(&effects), [(IconKind::Work, stacked(19, 59))]);
    }

    #[test]
    fn a_tray_without_text_can_show_minutes_alone_and_then_wakes_rarely() {
        let settings = Settings {
            tray_display: TrayDisplay::Minutes,
            ..Settings::default()
        };
        let (mut e, effects) = Engine::new(settings, TrayCaps::ICON_AND_TOOLTIP, T0);
        assert_eq!(icons(&effects), [(IconKind::Work, Some(Badge::Number(20)))]);

        // Minutes, not seconds: so it also only needs waking once a minute.
        assert_eq!(
            e.next_wake(T0),
            Some(T0.add(crate::screen::SAMPLE_INTERVAL).mono),
            "the screen check"
        );
        let at = T0.add(Duration::from_secs(61));
        let effects = e.handle(Event::Tick, at);
        assert_eq!(icons(&effects), [(IconKind::Work, Some(Badge::Number(19)))]);
    }

    #[test]
    fn a_tray_without_text_counts_break_seconds_in_the_icon() {
        let (mut e, _) = Engine::new(Settings::default(), TrayCaps::ICON_AND_TOOLTIP, T0);
        let effects = e.handle(Event::Tick, T0.add(WORK));
        assert_eq!(
            icons(&effects),
            [(IconKind::Break, Some(Badge::Number(20)))]
        );
    }

    #[test]
    fn asking_for_an_icon_only_display_removes_the_badge_too() {
        let (_, effects) = Engine::new(
            Settings {
                tray_display: TrayDisplay::IconOnly,
                ..Settings::default()
            },
            TrayCaps::ICON_AND_TOOLTIP,
            T0,
        );
        assert_eq!(icons(&effects), [(IconKind::Work, None)]);
    }

    #[test]
    fn host_capabilities_are_detected_from_the_build_target() {
        assert_eq!(TrayCaps::for_host().text_label, cfg!(target_os = "macos"));
    }

    // --- small pieces -----------------------------------------------------------------

    #[test]
    fn reason_sets_behave_like_sets() {
        let mut set = ReasonSet::default();
        assert!(set.is_empty() && set.primary().is_none());
        assert_eq!(set.len(), 0);

        assert!(set.insert(SuspendReason::ScreenLock));
        assert!(!set.insert(SuspendReason::ScreenLock), "already present");
        assert!(set.contains(SuspendReason::ScreenLock));
        assert!(!set.contains(SuspendReason::Sleep));
        assert_eq!(set.len(), 1);

        set.insert(SuspendReason::Sleep);
        assert_eq!(
            set.primary(),
            Some(SuspendReason::Sleep),
            "most explanatory first"
        );

        assert!(set.remove(SuspendReason::Sleep));
        assert!(!set.remove(SuspendReason::Sleep));
        assert_eq!(set.primary(), Some(SuspendReason::ScreenLock));

        set.clear();
        assert!(set.is_empty());
    }

    #[test]
    fn every_reason_has_a_distinct_bit() {
        let mut set = ReasonSet::default();
        for reason in SuspendReason::ALL {
            set.insert(reason);
        }
        assert_eq!(set.len(), u32::try_from(SuspendReason::ALL.len()).unwrap());
    }

    #[test]
    fn plural_seconds_reads_naturally() {
        assert_eq!(plural_secs(1), "1 second");
        assert_eq!(plural_secs(20), "20 seconds");
        assert_eq!(plural_secs(0), "0 seconds");
    }
}
