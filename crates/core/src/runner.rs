//! Carrying out effects.
//!
//! The engine decides, this module performs - through the [`crate::ports`] traits, so it
//! stays testable. What is left for the real application binary is to own the event loop
//! and the OS objects; it contains no decisions of its own.

use crate::clock::Mono;
use crate::engine::Effect;
use crate::ports::{Autostart, Notifier, SettingsStore, Tray, Ui};

/// The collection of host objects effects are applied to.
pub struct Host<'a> {
    /// The menu bar / notification area item.
    pub tray: &'a mut dyn Tray,
    /// Desktop notifications.
    pub notifier: &'a dyn Notifier,
    /// Login item registration.
    pub autostart: &'a dyn Autostart,
    /// Settings persistence.
    pub store: &'a dyn SettingsStore,
    /// The settings window.
    pub ui: &'a mut dyn Ui,
}

/// What the event loop needs to know after a batch of effects has been applied.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// When to wake next. `None` means block until something external happens.
    pub wake_at: Option<Mono>,
    /// Whether the process should exit.
    pub exit: bool,
    /// Host failures, for the log. A tray that will not update or a notification the user
    /// has denied is not a reason to stop reminding them.
    pub errors: Vec<String>,
}

/// Applies every effect in order and summarises the result.
///
/// Errors are collected rather than returned early: the remaining effects still need to
/// run, or a single denied notification would leave the tray showing a stale countdown.
#[must_use]
pub fn apply(effects: &[Effect], host: &mut Host<'_>) -> Outcome {
    let mut outcome = Outcome::default();
    for effect in effects {
        let result = match effect {
            Effect::Notify(n) => host.notifier.post(n),
            Effect::SetTrayLabel(label) => host.tray.set_label(label),
            Effect::SetTrayTooltip(tooltip) => host.tray.set_tooltip(tooltip),
            Effect::SetTrayIcon { kind, badge } => host.tray.set_icon(*kind, *badge),
            Effect::SetMenu(menu) => host.tray.set_menu(menu),
            Effect::SaveSettings(settings) => host.store.save(settings),
            Effect::SetAutostart(enabled) => host.autostart.set_enabled(*enabled),
            Effect::OpenSettings => host.ui.open_settings(),
            Effect::WakeAt(at) => {
                outcome.wake_at = Some(*at);
                Ok(())
            }
            Effect::Exit => {
                outcome.exit = true;
                Ok(())
            }
        };
        if let Err(message) = result {
            outcome
                .errors
                .push(format!("{}: {message}", describe(effect)));
        }
    }
    outcome
}

/// A short name for an effect, used to prefix host errors in the log.
fn describe(effect: &Effect) -> &'static str {
    match effect {
        Effect::Notify(_) => "notify",
        Effect::SetTrayLabel(_) => "tray label",
        Effect::SetTrayTooltip(_) => "tray tooltip",
        Effect::SetTrayIcon { .. } => "tray icon",
        Effect::SetMenu(_) => "tray menu",
        Effect::WakeAt(_) => "schedule",
        Effect::SaveSettings(_) => "save settings",
        Effect::SetAutostart(_) => "autostart",
        Effect::OpenSettings => "open settings",
        Effect::Exit => "exit",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Badge, IconKind, MenuModel, Notification};
    use crate::settings::Settings;
    use crate::testing::{FakeAutostart, FakeNotifier, FakeStore, FakeTray, FakeUi};

    struct Harness {
        tray: FakeTray,
        notifier: FakeNotifier,
        autostart: FakeAutostart,
        store: FakeStore,
        ui: FakeUi,
    }

    impl Harness {
        fn new() -> Self {
            Self {
                tray: FakeTray::default(),
                notifier: FakeNotifier::default(),
                autostart: FakeAutostart::default(),
                store: FakeStore::default(),
                ui: FakeUi::default(),
            }
        }

        fn run(&mut self, effects: &[Effect]) -> Outcome {
            apply(
                effects,
                &mut Host {
                    tray: &mut self.tray,
                    notifier: &self.notifier,
                    autostart: &self.autostart,
                    store: &self.store,
                    ui: &mut self.ui,
                },
            )
        }
    }

    fn one_of_each() -> Vec<Effect> {
        vec![
            Effect::Notify(Notification {
                title: "Look away".into(),
                body: "b".into(),
                sound: true,
            }),
            Effect::SetTrayLabel("19:59".into()),
            Effect::SetTrayTooltip("EyeSaver - next break in 20m".into()),
            Effect::SetTrayIcon {
                kind: IconKind::Work,
                badge: Some(Badge::Number(20)),
            },
            Effect::SetMenu(MenuModel {
                header: "h".into(),
                pause_label: "Pause",
                paused: false,
            }),
            Effect::SaveSettings(Settings::default()),
            Effect::SetAutostart(true),
            Effect::OpenSettings,
            Effect::WakeAt(Mono::from_secs(42)),
            Effect::Exit,
        ]
    }

    #[test]
    fn every_effect_reaches_its_port() {
        let mut h = Harness::new();
        let outcome = h.run(&one_of_each());

        assert_eq!(h.notifier.titles(), ["Look away"]);
        assert_eq!(h.tray.label, "19:59");
        assert_eq!(h.tray.tooltip, "EyeSaver - next break in 20m");
        assert_eq!(h.tray.icon, Some((IconKind::Work, Some(Badge::Number(20)))));
        assert!(h.tray.menu.is_some());
        assert_eq!(h.store.saves.get(), 1);
        assert!(h.autostart.is_enabled());
        assert_eq!(h.ui.opens, 1);
        assert_eq!(outcome.wake_at, Some(Mono::from_secs(42)));
        assert!(outcome.exit);
        assert!(outcome.errors.is_empty());
    }

    #[test]
    fn an_empty_batch_asks_for_nothing() {
        let mut h = Harness::new();
        assert_eq!(h.run(&[]), Outcome::default());
        assert_eq!(h.tray.calls, 0, "no effects must mean no OS calls");
    }

    #[test]
    fn the_last_schedule_wins() {
        let mut h = Harness::new();
        let outcome = h.run(&[
            Effect::WakeAt(Mono::from_secs(1)),
            Effect::WakeAt(Mono::from_secs(2)),
        ]);
        assert_eq!(outcome.wake_at, Some(Mono::from_secs(2)));
    }

    #[test]
    fn host_failures_are_collected_and_do_not_stop_later_effects() {
        let mut h = Harness::new();
        h.tray.fail = Some("tray gone".into());
        h.notifier.fail = Some("denied".into());
        h.autostart.fail = Some("locked".into());
        h.store.fail = Some("read only".into());
        h.ui.fail = Some("no display".into());

        let outcome = h.run(&one_of_each());

        assert_eq!(
            outcome.wake_at,
            Some(Mono::from_secs(42)),
            "scheduling still happened"
        );
        assert!(outcome.exit);
        assert_eq!(outcome.errors.len(), 8, "every failing port reported once");
        assert!(outcome.errors.iter().any(|e| e == "notify: denied"));
        assert!(outcome.errors.iter().any(|e| e == "tray label: tray gone"));
        assert!(
            outcome
                .errors
                .iter()
                .any(|e| e == "tray tooltip: tray gone")
        );
        assert!(outcome.errors.iter().any(|e| e == "tray icon: tray gone"));
        assert!(outcome.errors.iter().any(|e| e == "tray menu: tray gone"));
        assert!(
            outcome
                .errors
                .iter()
                .any(|e| e == "save settings: read only")
        );
        assert!(outcome.errors.iter().any(|e| e == "autostart: locked"));
        assert!(
            outcome
                .errors
                .iter()
                .any(|e| e == "open settings: no display")
        );
    }

    #[test]
    fn every_effect_has_a_name_for_the_log() {
        for effect in one_of_each() {
            assert!(!describe(&effect).is_empty());
        }
    }
}
