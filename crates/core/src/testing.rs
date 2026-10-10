//! Recording fakes for every port, plus a controllable clock.
//!
//! These live in the library rather than behind `#[cfg(test)]` so that the integration
//! tests - and the platform crate's own tests - can drive the whole application without an
//! operating system. They are small, allocation-light and dead code in a release build.

use std::cell::{Cell, RefCell};
use std::time::Duration;

use crate::clock::{Clock, Mono, Wall};
use crate::engine::{IconKind, MenuModel, Notification};
use crate::ports::{Autostart, HostResult, IdleSource, Notifier, SettingsStore, Tray, Ui};
use crate::settings::Settings;

/// A clock the test moves by hand.
///
/// [`Self::advance`] moves both clocks, as time passing normally does. [`Self::suspend`]
/// moves only the wall clock, which is what a machine going to sleep looks like.
#[derive(Debug)]
pub struct FakeClock {
    mono: Cell<Mono>,
    wall: Cell<Wall>,
}

impl FakeClock {
    /// A clock starting at the given readings.
    #[must_use]
    pub const fn new(mono: Mono, wall: Wall) -> Self {
        Self {
            mono: Cell::new(mono),
            wall: Cell::new(wall),
        }
    }

    /// A clock starting at zero monotonic and an arbitrary wall time.
    #[must_use]
    pub const fn at_zero() -> Self {
        Self::new(Mono::ZERO, Wall::from_millis(1_700_000_000_000))
    }

    /// Moves both clocks forward: ordinary elapsed time.
    pub fn advance(&self, delta: Duration) {
        self.mono.set(self.mono.get().add(delta));
        self.wall.set(self.wall.get().add(delta));
    }

    /// Moves only the wall clock forward: the machine was suspended.
    pub fn suspend(&self, delta: Duration) {
        self.wall.set(self.wall.get().add(delta));
    }

    /// Moves the wall clock backwards: an NTP correction.
    pub fn rewind_wall(&self, delta: Duration) {
        self.wall.set(self.wall.get().sub(delta));
    }

    /// The current monotonic reading.
    #[must_use]
    pub fn mono(&self) -> Mono {
        self.mono.get()
    }
}

impl Clock for FakeClock {
    fn monotonic(&self) -> Mono {
        self.mono.get()
    }
    fn wall(&self) -> Wall {
        self.wall.get()
    }
}

/// Records what the tray was told to show.
#[derive(Debug, Default)]
pub struct FakeTray {
    /// Latest label.
    pub label: String,
    /// Latest tooltip.
    pub tooltip: String,
    /// Latest icon request.
    pub icon: Option<(IconKind, Option<u16>)>,
    /// Latest menu.
    pub menu: Option<MenuModel>,
    /// How many calls of any kind were made, so tests can assert on churn.
    pub calls: usize,
    /// When set, every call fails with this message.
    pub fail: Option<String>,
}

impl FakeTray {
    fn check(&mut self) -> HostResult {
        self.calls += 1;
        match &self.fail {
            Some(e) => Err(e.clone()),
            None => Ok(()),
        }
    }
}

impl Tray for FakeTray {
    fn set_label(&mut self, label: &str) -> HostResult {
        label.clone_into(&mut self.label);
        self.check()
    }
    fn set_tooltip(&mut self, tooltip: &str) -> HostResult {
        tooltip.clone_into(&mut self.tooltip);
        self.check()
    }
    fn set_icon(&mut self, kind: IconKind, badge: Option<u16>) -> HostResult {
        self.icon = Some((kind, badge));
        self.check()
    }
    fn set_menu(&mut self, menu: &MenuModel) -> HostResult {
        self.menu = Some(menu.clone());
        self.check()
    }
}

/// Records posted notifications.
#[derive(Debug, Default)]
pub struct FakeNotifier {
    /// Everything posted, in order.
    pub posted: RefCell<Vec<Notification>>,
    /// When set, every post fails with this message.
    pub fail: Option<String>,
}

impl FakeNotifier {
    /// How many notifications were posted.
    #[must_use]
    pub fn count(&self) -> usize {
        self.posted.borrow().len()
    }

    /// The titles posted, in order.
    #[must_use]
    pub fn titles(&self) -> Vec<String> {
        self.posted
            .borrow()
            .iter()
            .map(|n| n.title.clone())
            .collect()
    }
}

impl Notifier for FakeNotifier {
    fn post(&self, notification: &Notification) -> HostResult {
        self.posted.borrow_mut().push(notification.clone());
        match &self.fail {
            Some(e) => Err(e.clone()),
            None => Ok(()),
        }
    }
}

/// A settable idle time.
#[derive(Debug, Default)]
pub struct FakeIdle(Cell<Duration>);

impl FakeIdle {
    /// An idle source reporting `idle`.
    #[must_use]
    pub const fn new(idle: Duration) -> Self {
        Self(Cell::new(idle))
    }
    /// Changes the reported idle time.
    pub fn set(&self, idle: Duration) {
        self.0.set(idle);
    }
}

impl IdleSource for FakeIdle {
    fn idle_time(&self) -> Duration {
        self.0.get()
    }
}

/// Records login-item registration.
#[derive(Debug, Default)]
pub struct FakeAutostart {
    /// Current registration state.
    pub enabled: Cell<bool>,
    /// How many times it was changed.
    pub writes: Cell<usize>,
    /// When set, every change fails with this message.
    pub fail: Option<String>,
}

impl Autostart for FakeAutostart {
    fn is_enabled(&self) -> bool {
        self.enabled.get()
    }
    fn set_enabled(&self, enabled: bool) -> HostResult {
        self.writes.set(self.writes.get() + 1);
        if let Some(e) = &self.fail {
            return Err(e.clone());
        }
        self.enabled.set(enabled);
        Ok(())
    }
}

/// An in-memory settings store.
#[derive(Debug, Default)]
pub struct FakeStore {
    /// What a load returns, and what a save overwrites.
    pub settings: RefCell<Settings>,
    /// How many saves happened, so tests can assert writes are not spammed.
    pub saves: Cell<usize>,
    /// When set, every save fails with this message.
    pub fail: Option<String>,
}

impl FakeStore {
    /// A store preloaded with `settings`.
    #[must_use]
    pub fn with(settings: Settings) -> Self {
        Self {
            settings: RefCell::new(settings),
            ..Self::default()
        }
    }
}

impl SettingsStore for FakeStore {
    fn load(&self) -> Settings {
        *self.settings.borrow()
    }
    fn save(&self, settings: &Settings) -> HostResult {
        self.saves.set(self.saves.get() + 1);
        if let Some(e) = &self.fail {
            return Err(e.clone());
        }
        *self.settings.borrow_mut() = *settings;
        Ok(())
    }
}

/// Records settings-window opens.
#[derive(Debug, Default)]
pub struct FakeUi {
    /// How many times the window was asked to open.
    pub opens: usize,
    /// When set, every open fails with this message.
    pub fail: Option<String>,
}

impl Ui for FakeUi {
    fn open_settings(&mut self) -> HostResult {
        self.opens += 1;
        match &self.fail {
            Some(e) => Err(e.clone()),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::TrayDisplay;

    #[test]
    fn advance_moves_both_clocks_and_suspend_moves_only_the_wall_clock() {
        let c = FakeClock::new(Mono::from_secs(10), Wall::from_secs(1_000));
        c.advance(Duration::from_secs(5));
        assert_eq!(c.monotonic(), Mono::from_secs(15));
        assert_eq!(c.wall(), Wall::from_secs(1_005));

        c.suspend(Duration::from_secs(600));
        assert_eq!(
            c.monotonic(),
            Mono::from_secs(15),
            "a suspended machine does not tick"
        );
        assert_eq!(c.wall(), Wall::from_secs(1_605));

        c.rewind_wall(Duration::from_secs(100));
        assert_eq!(c.wall(), Wall::from_secs(1_505));
        assert_eq!(c.mono(), c.monotonic());
        assert_eq!(FakeClock::at_zero().monotonic(), Mono::ZERO);
    }

    #[test]
    fn the_fake_tray_records_and_can_be_made_to_fail() {
        let mut tray = FakeTray::default();
        assert!(tray.set_label("20:00").is_ok());
        assert!(tray.set_tooltip("EyeSaver").is_ok());
        assert!(tray.set_icon(IconKind::Work, Some(20)).is_ok());
        let menu = MenuModel {
            header: "h".into(),
            pause_label: "Pause",
            paused: false,
        };
        assert!(tray.set_menu(&menu).is_ok());
        assert_eq!(tray.label, "20:00");
        assert_eq!(tray.tooltip, "EyeSaver");
        assert_eq!(tray.icon, Some((IconKind::Work, Some(20))));
        assert_eq!(tray.menu, Some(menu));
        assert_eq!(tray.calls, 4);

        tray.fail = Some("no tray".into());
        assert_eq!(tray.set_label("x").unwrap_err(), "no tray");
        assert_eq!(tray.set_tooltip("x").unwrap_err(), "no tray");
        assert_eq!(tray.set_icon(IconKind::Break, None).unwrap_err(), "no tray");
        assert!(
            tray.set_menu(&MenuModel {
                header: String::new(),
                pause_label: "Pause",
                paused: false
            })
            .is_err()
        );
    }

    #[test]
    fn the_fake_notifier_records_and_can_be_made_to_fail() {
        let n = Notification {
            title: "Look away".into(),
            body: "b".into(),
            sound: true,
        };
        let notifier = FakeNotifier::default();
        assert!(notifier.post(&n).is_ok());
        assert_eq!(notifier.count(), 1);
        assert_eq!(notifier.titles(), ["Look away"]);

        let failing = FakeNotifier {
            fail: Some("denied".into()),
            ..FakeNotifier::default()
        };
        assert_eq!(failing.post(&n).unwrap_err(), "denied");
        assert_eq!(
            failing.count(),
            1,
            "a failed post is still recorded as attempted"
        );
    }

    #[test]
    fn the_fake_idle_source_is_settable() {
        let idle = FakeIdle::new(Duration::from_secs(3));
        assert_eq!(idle.idle_time(), Duration::from_secs(3));
        idle.set(Duration::from_secs(90));
        assert_eq!(idle.idle_time(), Duration::from_secs(90));
        assert_eq!(FakeIdle::default().idle_time(), Duration::ZERO);
    }

    #[test]
    fn the_fake_autostart_records_and_can_be_made_to_fail() {
        let a = FakeAutostart::default();
        assert!(!a.is_enabled());
        assert!(a.set_enabled(true).is_ok());
        assert!(a.is_enabled());
        assert_eq!(a.writes.get(), 1);

        let failing = FakeAutostart {
            fail: Some("no registry".into()),
            ..FakeAutostart::default()
        };
        assert_eq!(failing.set_enabled(true).unwrap_err(), "no registry");
        assert!(
            !failing.is_enabled(),
            "a failed write must not claim success"
        );
    }

    #[test]
    fn the_fake_store_round_trips_and_can_be_made_to_fail() {
        let store = FakeStore::default();
        assert_eq!(store.load(), Settings::default());

        let changed = Settings {
            tray_display: TrayDisplay::IconOnly,
            ..Settings::default()
        };
        assert!(store.save(&changed).is_ok());
        assert_eq!(store.load(), changed);
        assert_eq!(store.saves.get(), 1);

        let preloaded = FakeStore::with(changed);
        assert_eq!(preloaded.load(), changed);

        let failing = FakeStore {
            fail: Some("read only".into()),
            ..FakeStore::default()
        };
        assert_eq!(failing.save(&changed).unwrap_err(), "read only");
        assert_eq!(failing.load(), Settings::default());
    }

    #[test]
    fn the_fake_ui_records_and_can_be_made_to_fail() {
        let mut ui = FakeUi::default();
        assert!(ui.open_settings().is_ok());
        assert_eq!(ui.opens, 1);
        ui.fail = Some("no display".into());
        assert_eq!(ui.open_settings().unwrap_err(), "no display");
        assert_eq!(ui.opens, 2);
    }
}
