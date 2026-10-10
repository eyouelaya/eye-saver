//! Property tests: invariants that must hold for *every* sequence of events, however
//! implausible.
//!
//! The hand-written tests cover the timelines a person can imagine. These cover the ones
//! they cannot - a lid closing twice, a resume with no suspend, the date jumping backwards
//! mid-break - which is exactly where a timer state machine tends to break.

use std::time::Duration;

use eye_saver_core::clock::{Mono, Now, Wall};
use eye_saver_core::engine::{Effect, Engine, Event, Phase, Status, SuspendReason, TrayCaps};
use eye_saver_core::settings::{Settings, TrayDisplay};
use proptest::prelude::*;

/// How the clocks move before an event is delivered.
#[derive(Clone, Debug)]
enum Step {
    /// Ordinary elapsed time.
    Advance(u64),
    /// Wall time only: the machine was suspended.
    SuspendClock(u64),
    /// Wall time backwards: an NTP correction or a date change.
    Rewind(u64),
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        8 => (0u64..3_600_000).prop_map(Step::Advance),
        2 => (0u64..7_200_000).prop_map(Step::SuspendClock),
        1 => (0u64..7_200_000).prop_map(Step::Rewind),
    ]
}

fn reason() -> impl Strategy<Value = SuspendReason> {
    prop_oneof![
        Just(SuspendReason::Sleep),
        Just(SuspendReason::DisplayOff),
        Just(SuspendReason::ScreenLock),
        Just(SuspendReason::SessionInactive),
    ]
}

fn settings() -> impl Strategy<Value = Settings> {
    (
        0u64..40_000,
        0u64..2_000,
        0u64..4_000,
        prop_oneof![
            Just(TrayDisplay::MmSs),
            Just(TrayDisplay::Minutes),
            Just(TrayDisplay::IconOnly)
        ],
        any::<bool>(),
        any::<bool>(),
    )
        .prop_map(|(work, brk, idle, display, login, sound)| Settings {
            work_interval_secs: work,
            break_duration_secs: brk,
            idle_credit_secs: idle,
            tray_display: display,
            start_at_login: login,
            notification_sound: sound,
        })
}

fn event() -> impl Strategy<Value = Event> {
    prop_oneof![
        10 => Just(Event::Tick),
        3 => reason().prop_map(Event::Suspend),
        3 => reason().prop_map(Event::Resume),
        1 => Just(Event::ResumeAll),
        5 => (0u64..3_600).prop_map(|s| Event::Idle(Duration::from_secs(s))),
        2 => Just(Event::TogglePause),
        1 => Just(Event::TakeBreakNow),
        1 => Just(Event::ResetCycle),
        1 => Just(Event::OpenSettingsRequested),
        2 => settings().prop_map(Event::SettingsChanged),
    ]
}

const START: Now = Now::new(Mono::from_secs(1_000), Wall::from_secs(1_700_000_000));

fn apply_step(now: Now, step: &Step) -> Now {
    match *step {
        Step::Advance(ms) => now.add(Duration::from_millis(ms)),
        Step::SuspendClock(ms) => now.suspend(Duration::from_millis(ms)),
        Step::Rewind(ms) => now.rewind_wall(Duration::from_millis(ms)),
    }
}

fn wake_ups(effects: &[Effect]) -> Vec<Mono> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::WakeAt(at) => Some(*at),
            _ => None,
        })
        .collect()
}

proptest! {
    /// The timer never shows more time remaining than the phase it is in actually has,
    /// and never a negative amount - the two ways a countdown can visibly lie.
    #[test]
    fn remaining_time_stays_inside_the_current_phase(
        initial in settings(),
        script in prop::collection::vec((step(), event()), 1..60),
    ) {
        let (mut engine, _) = Engine::new(initial, TrayCaps::WITH_TEXT, START);
        let mut now = START;
        for (step, event) in &script {
            now = apply_step(now, step);
            engine.handle(*event, now);

            let limit = match engine.phase() {
                Phase::Work => engine.settings().work_interval(),
                Phase::Break => engine.settings().break_duration(),
            };
            prop_assert!(
                engine.remaining(now) <= limit,
                "{:?} left in a {:?} phase of {:?}",
                engine.remaining(now),
                engine.phase(),
                limit
            );
        }
    }

    /// Counting and the reasons not to count can never disagree. This is the invariant
    /// that keeps a timer from ticking down while the screen is off.
    #[test]
    fn counting_agrees_with_the_reasons_not_to_count(
        initial in settings(),
        script in prop::collection::vec((step(), event()), 1..60),
    ) {
        let (mut engine, _) = Engine::new(initial, TrayCaps::WITH_TEXT, START);
        let mut now = START;
        for (step, event) in &script {
            now = apply_step(now, step);
            engine.handle(*event, now);

            let running = engine.status() == Status::Running;
            prop_assert_eq!(running, engine.is_counting());
            if running {
                prop_assert!(engine.reasons().is_empty());
            }
            if !engine.reasons().is_empty() {
                prop_assert!(!engine.is_counting(), "counted down with the screen away");
            }
        }
    }

    /// Exactly one wake-up is armed whenever the timer is counting, and none when it is
    /// not. Too many is a busy loop; too few is a stopped clock.
    #[test]
    fn the_schedule_is_armed_exactly_when_there_is_something_to_count(
        initial in settings(),
        script in prop::collection::vec((step(), event()), 1..60),
    ) {
        let (mut engine, _) = Engine::new(initial, TrayCaps::WITH_TEXT, START);
        let mut now = START;
        for (step, event) in &script {
            now = apply_step(now, step);
            let effects = engine.handle(*event, now);
            let wakes = wake_ups(&effects);

            if engine.is_counting() {
                prop_assert_eq!(wakes.len(), 1, "{:?} -> {:?}", event, effects);
                let at = wakes[0];
                prop_assert!(at >= now.mono, "a wake-up in the past spins the loop");
                let ceiling = engine
                    .settings()
                    .work_interval()
                    .max(engine.settings().idle_sample_interval());
                prop_assert!(
                    at <= now.mono.add(ceiling),
                    "sleeping past the end of the phase stops the clock"
                );
            } else {
                prop_assert!(wakes.is_empty(), "woke with nothing to count: {:?}", effects);
            }
        }
    }

    /// Nothing is ever asked of the host that it did not need: no notification arrives
    /// while the screen is away, and the tray is never told to show a countdown then.
    #[test]
    fn a_machine_that_is_away_is_never_nagged(
        initial in settings(),
        script in prop::collection::vec((step(), event()), 1..60),
    ) {
        let (mut engine, _) = Engine::new(initial, TrayCaps::WITH_TEXT, START);
        let mut now = START;
        for (step, event) in &script {
            now = apply_step(now, step);
            let effects = engine.handle(*event, now);

            // "Take a break now" is the user asking for it, so it is allowed to notify
            // whatever the screen is doing.
            if engine.is_counting() || matches!(event, Event::TakeBreakNow) {
                continue;
            }
            for effect in &effects {
                prop_assert!(
                    !matches!(effect, Effect::Notify(_)),
                    "notified while {:?}",
                    engine.status()
                );
                if let Effect::SetTrayLabel(label) = effect {
                    prop_assert!(
                        label.is_empty() || label == "--",
                        "showed the countdown {label:?} while not counting"
                    );
                }
            }
        }
    }

    /// Settings are clamped no matter how they arrive, and whatever is handed to the host
    /// to persist is what the engine is actually using.
    #[test]
    fn persisted_settings_are_always_valid_and_current(
        initial in settings(),
        incoming in prop::collection::vec(settings(), 1..10),
    ) {
        let (mut engine, _) = Engine::new(initial, TrayCaps::WITH_TEXT, START);
        prop_assert!(engine.settings().is_clamped());

        let mut now = START;
        for next in incoming {
            now = now.add(Duration::from_secs(1));
            let effects = engine.handle(Event::SettingsChanged(next), now);
            prop_assert!(engine.settings().is_clamped());
            prop_assert_eq!(*engine.settings(), next.clamped());
            for effect in &effects {
                if let Effect::SaveSettings(saved) = effect {
                    prop_assert_eq!(saved, engine.settings());
                }
            }
        }
    }
}
