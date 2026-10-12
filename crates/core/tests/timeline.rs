//! End-to-end tests that drive the whole application against fake hardware.
//!
//! [`Sim`] is deliberately a faithful copy of the real event loop in the `eye-saver`
//! binary: sleep until the engine's deadline, sample the clocks and the idle time, feed
//! the events in, apply the effects. Anything these tests prove therefore holds for the
//! shipped app too, and they run in milliseconds with no desktop in sight.

use std::time::Duration;

use eye_saver_core::clock::Clock;
use eye_saver_core::engine::{Badge, Effect, Engine, Event, SuspendReason, TrayCaps};
use eye_saver_core::ports::{Autostart, IdleSource, SettingsStore};
use eye_saver_core::runner::{self, Host, Outcome};
use eye_saver_core::settings::{Settings, TrayDisplay};
use eye_saver_core::testing::{
    FakeAutostart, FakeClock, FakeIdle, FakeNotifier, FakeStore, FakeTray, FakeUi,
};
use eye_saver_core::{Mono, Phase, Status};

const MINUTE: Duration = Duration::from_secs(60);

struct Sim {
    engine: Engine,
    clock: FakeClock,
    idle: FakeIdle,
    tray: FakeTray,
    notifier: FakeNotifier,
    autostart: FakeAutostart,
    store: FakeStore,
    ui: FakeUi,
    wake_at: Option<Mono>,
    /// How many times the event loop woke up: the battery cost of the whole run.
    wakeups: usize,
    exited: bool,
    errors: Vec<String>,
}

impl Sim {
    fn new(settings: Settings, caps: TrayCaps) -> Self {
        let clock = FakeClock::at_zero();
        let (engine, effects) = Engine::new(settings, caps, clock.now());
        let mut sim = Self {
            engine,
            clock,
            idle: FakeIdle::default(),
            tray: FakeTray::default(),
            notifier: FakeNotifier::default(),
            autostart: FakeAutostart::default(),
            store: FakeStore::with(settings),
            ui: FakeUi::default(),
            wake_at: None,
            wakeups: 0,
            exited: false,
            errors: Vec::new(),
        };
        sim.apply(&effects);
        sim
    }

    fn with_defaults() -> Self {
        Self::new(Settings::default(), TrayCaps::WITH_TEXT)
    }

    fn apply(&mut self, effects: &[Effect]) {
        let Outcome {
            wake_at,
            exit,
            mut errors,
        } = runner::apply(
            effects,
            &mut Host {
                tray: &mut self.tray,
                notifier: &self.notifier,
                autostart: &self.autostart,
                store: &self.store,
                ui: &mut self.ui,
            },
        );
        self.wake_at = wake_at;
        self.exited |= exit;
        self.errors.append(&mut errors);
    }

    fn send(&mut self, event: Event) {
        let effects = self.engine.handle(event, self.clock.now());
        self.apply(&effects);
    }

    /// One iteration of the real loop: report idle time, then tick. The engine reads both
    /// clocks on every call and notices an unannounced suspend by itself.
    fn wake(&mut self) {
        self.wakeups += 1;
        let idle = self.idle.idle_time();
        self.send(Event::Idle(idle));
        self.send(Event::Tick);
    }

    /// Runs the loop for `total` of real elapsed time.
    fn run_for(&mut self, total: Duration) {
        let deadline = self.clock.monotonic().add(total);
        loop {
            let Some(next) = self.wake_at else {
                // Nothing scheduled: the process is parked. Let the rest of the time pass
                // without a single wake-up, which is the whole point of an absent
                // schedule.
                let now = self.clock.monotonic();
                if now < deadline {
                    self.clock.advance(deadline.since(now));
                }
                return;
            };
            if next > deadline {
                let now = self.clock.monotonic();
                if now < deadline {
                    self.clock.advance(deadline.since(now));
                }
                return;
            }
            let now = self.clock.monotonic();
            if next > now {
                self.clock.advance(next.since(now));
            }
            self.wake();
        }
    }

    /// The machine suspends for `away`, then wakes and the user unlocks it.
    fn suspend_machine(&mut self, away: Duration) {
        self.send(Event::Suspend(SuspendReason::Sleep));
        self.send(Event::Suspend(SuspendReason::DisplayOff));
        self.clock.suspend(away);
        self.send(Event::Resume(SuspendReason::Sleep));
        self.send(Event::Resume(SuspendReason::DisplayOff));
    }

    fn reminders(&self) -> usize {
        self.notifier.count()
    }

    fn remaining(&self) -> Duration {
        self.engine.remaining(self.clock.now())
    }
}

#[test]
fn an_undisturbed_morning_reminds_once_per_cycle() {
    let mut sim = Sim::with_defaults();
    sim.run_for(Duration::from_secs(2 * 60 * 60));

    // One cycle is 20 minutes of work plus a 20 second break, so five fit in two hours.
    assert_eq!(sim.reminders(), 5);
    assert_eq!(sim.notifier.titles()[0], "Look away");
    assert_eq!(sim.engine.status(), Status::Running);
    assert!(sim.errors.is_empty());
    assert!(!sim.exited);
}

#[test]
fn the_tray_shows_a_live_countdown() {
    let mut sim = Sim::with_defaults();
    assert_eq!(sim.tray.label, "20:00");
    sim.run_for(Duration::from_secs(90));
    assert_eq!(sim.tray.label, "18:30");
    assert_eq!(sim.tray.menu.as_ref().unwrap().header, "Next break in 19m");
}

#[test]
fn the_break_counts_down_in_the_tray_and_then_the_cycle_restarts() {
    let mut sim = Sim::with_defaults();
    sim.run_for(Duration::from_secs(20 * 60 + 5));
    assert_eq!(sim.engine.phase(), Phase::Break);
    assert_eq!(sim.tray.label, "15s");
    assert_eq!(sim.reminders(), 1);

    sim.run_for(Duration::from_secs(15));
    assert_eq!(sim.engine.phase(), Phase::Work);
    assert_eq!(sim.tray.label, "20:00");
    assert_eq!(sim.reminders(), 1, "the end of a break is not announced");
}

#[test]
fn sleeping_through_a_reminder_does_not_queue_it_up_for_the_wake() {
    let mut sim = Sim::with_defaults();
    sim.run_for(Duration::from_secs(10 * 60));
    assert_eq!(sim.remaining(), Duration::from_secs(10 * 60));

    // Lid closed for half an hour, which spans the moment the reminder was due.
    sim.suspend_machine(Duration::from_secs(30 * 60));

    assert_eq!(
        sim.reminders(),
        0,
        "no notification may fire at a closed laptop"
    );
    assert_eq!(sim.engine.status(), Status::Running);
    assert_eq!(
        sim.remaining(),
        Duration::from_secs(20 * 60),
        "a fresh interval on waking"
    );
}

#[test]
fn a_short_sleep_resumes_the_interval_rather_than_restarting_it() {
    let mut sim = Sim::with_defaults();
    sim.run_for(Duration::from_secs(10 * 60));
    sim.suspend_machine(Duration::from_secs(5));
    assert_eq!(sim.remaining(), Duration::from_secs(10 * 60));
}

#[test]
fn nothing_is_counted_or_scheduled_while_the_machine_is_asleep() {
    let mut sim = Sim::with_defaults();
    sim.run_for(MINUTE);
    sim.send(Event::Suspend(SuspendReason::Sleep));

    let before = sim.wakeups;
    sim.run_for(Duration::from_secs(8 * 60 * 60));
    assert_eq!(sim.wakeups, before, "a suspended app must not wake at all");
    assert_eq!(sim.reminders(), 0);
    assert_eq!(sim.tray.label, "");
}

#[test]
fn the_timer_waits_for_the_screen_even_after_the_machine_has_woken() {
    let mut sim = Sim::with_defaults();
    sim.run_for(MINUTE);

    sim.send(Event::Suspend(SuspendReason::Sleep));
    sim.send(Event::Suspend(SuspendReason::DisplayOff));
    sim.send(Event::Suspend(SuspendReason::ScreenLock));
    sim.clock.suspend(Duration::from_secs(60 * 60));

    // Power comes back first, then the display, and the user is still at the login screen.
    sim.send(Event::Resume(SuspendReason::Sleep));
    assert_eq!(sim.engine.status(), Status::Suspended);
    sim.send(Event::Resume(SuspendReason::DisplayOff));
    assert_eq!(sim.engine.status(), Status::Suspended);
    sim.run_for(Duration::from_secs(10 * 60));
    assert_eq!(sim.reminders(), 0);

    sim.send(Event::Resume(SuspendReason::ScreenLock));
    assert_eq!(sim.engine.status(), Status::Running);
    assert_eq!(sim.remaining(), Duration::from_secs(20 * 60));
}

#[test]
fn a_suspend_the_os_forgot_to_mention_is_caught_by_the_clocks() {
    let mut sim = Sim::with_defaults();
    sim.run_for(Duration::from_secs(10 * 60));

    // No power event at all: only the wall clock moves. This is the backstop.
    sim.clock.suspend(Duration::from_secs(45 * 60));
    sim.run_for(Duration::from_secs(1));

    assert_eq!(sim.reminders(), 0);
    assert_eq!(sim.remaining(), Duration::from_secs(20 * 60));
}

#[test]
fn an_ntp_correction_is_not_mistaken_for_a_nap() {
    let mut sim = Sim::with_defaults();
    sim.run_for(Duration::from_secs(10 * 60));
    sim.clock.rewind_wall(Duration::from_secs(3 * 60 * 60));
    sim.run_for(Duration::from_secs(60));
    assert_eq!(
        sim.remaining(),
        Duration::from_secs(9 * 60),
        "the countdown just kept going"
    );
}

#[test]
fn walking_away_from_the_desk_is_a_break() {
    let mut sim = Sim::with_defaults();
    sim.run_for(Duration::from_secs(15 * 60));

    // An hour-long meeting. Idle time keeps growing; the screen never sleeps.
    for minute in 1..=60 {
        sim.idle.set(Duration::from_secs(minute * 60));
        sim.run_for(MINUTE);
    }
    assert_eq!(sim.reminders(), 0, "no reminders to an empty chair");

    // Back at the desk: essentially a whole interval still to go.
    sim.idle.set(Duration::ZERO);
    sim.run_for(Duration::from_secs(1));
    assert!(
        sim.remaining() > Duration::from_secs(19 * 60),
        "{:?}",
        sim.remaining()
    );
}

#[test]
fn a_quick_coffee_does_not_reset_anything() {
    let mut sim = Sim::with_defaults();
    sim.run_for(Duration::from_secs(10 * 60));
    sim.idle.set(Duration::from_secs(30));
    sim.run_for(Duration::from_secs(30));
    sim.idle.set(Duration::ZERO);
    assert_eq!(sim.remaining(), Duration::from_secs(9 * 60 + 30));
}

#[test]
fn pausing_parks_the_process_and_resuming_brings_it_back() {
    let mut sim = Sim::with_defaults();
    sim.run_for(Duration::from_secs(5 * 60));
    sim.send(Event::TogglePause);

    let before = sim.wakeups;
    sim.run_for(Duration::from_secs(4 * 60 * 60));
    assert_eq!(sim.wakeups, before, "a paused app must not wake at all");
    assert_eq!(sim.reminders(), 0);

    sim.send(Event::TogglePause);
    assert_eq!(sim.engine.status(), Status::Running);
    sim.run_for(Duration::from_secs(21 * 60));
    assert_eq!(
        sim.reminders(),
        1,
        "and the cycle starts afresh after a long pause"
    );
}

#[test]
fn the_settings_window_round_trips_through_the_store_and_the_login_item() {
    let mut sim = Sim::with_defaults();
    let changed = Settings {
        work_interval_secs: 15 * 60,
        tray_display: TrayDisplay::Minutes,
        start_at_login: true,
        ..Settings::default()
    };
    sim.send(Event::SettingsChanged(changed));

    assert_eq!(sim.store.load(), changed, "written through to disk");
    assert!(sim.autostart.is_enabled(), "registered as a login item");
    assert_eq!(sim.tray.label, "15m");

    sim.run_for(Duration::from_secs(15 * 60));
    assert_eq!(sim.reminders(), 1, "the new interval is the one that fires");
}

#[test]
fn a_second_launch_only_opens_the_settings_window() {
    let mut sim = Sim::with_defaults();
    sim.send(Event::OpenSettingsRequested);
    assert_eq!(sim.ui.opens, 1);
    assert!(!sim.exited);
    sim.run_for(Duration::from_secs(21 * 60));
    assert_eq!(
        sim.reminders(),
        1,
        "the timer kept running behind the window"
    );
}

#[test]
fn quitting_exits() {
    let mut sim = Sim::with_defaults();
    sim.send(Event::QuitRequested);
    assert!(sim.exited);
}

#[test]
fn a_host_that_refuses_to_cooperate_does_not_stop_the_reminders() {
    let mut sim = Sim::with_defaults();
    sim.tray.fail = Some("tray unavailable".into());
    sim.notifier.fail = Some("notifications denied".into());

    sim.run_for(Duration::from_secs(21 * 60));
    assert_eq!(sim.reminders(), 1, "still tried to remind the user");
    assert!(
        !sim.errors.is_empty(),
        "and reported the failures for the log"
    );
    assert!(
        sim.errors
            .iter()
            .any(|e| e.contains("notifications denied"))
    );
}

#[test]
fn the_display_mode_decides_how_often_the_process_wakes() {
    let hour = Duration::from_secs(60 * 60);
    let mut counts = Vec::new();
    for display in TrayDisplay::ALL {
        let mut sim = Sim::new(
            Settings {
                tray_display: display,
                ..Settings::default()
            },
            TrayCaps::WITH_TEXT,
        );
        sim.run_for(hour);
        counts.push(sim.wakeups);
    }
    let [mm_ss, minutes, icon_only] = <[usize; 3]>::try_from(counts).unwrap();

    // A live second-by-second countdown is one wake per second, by definition.
    assert!(
        (3500..=3700).contains(&mm_ss),
        "mm:ss woke {mm_ss} times an hour"
    );
    // The coarser modes only wake for the minute marks, the final minute of each cycle,
    // the break, and the idle check.
    assert!(minutes < mm_ss / 5, "minutes woke {minutes} times an hour");
    assert!(
        icon_only < minutes,
        "icon-only is the cheapest of all: {icon_only} vs {minutes} wake-ups an hour"
    );
}

#[test]
fn a_windows_style_tray_stacks_the_live_countdown_into_the_icon() {
    let mut sim = Sim::new(Settings::default(), TrayCaps::ICON_AND_TOOLTIP);
    assert_eq!(
        sim.tray.label, "",
        "the notification area has nowhere to put text"
    );
    assert_eq!(
        sim.tray.icon.unwrap().1,
        Some(Badge::Stacked {
            minutes: 20,
            seconds: 0
        }),
        "so minutes and seconds both go into the icon"
    );

    sim.run_for(Duration::from_secs(90));
    assert_eq!(
        sim.tray.icon.unwrap().1,
        Some(Badge::Stacked {
            minutes: 18,
            seconds: 30
        })
    );

    // Showing seconds costs what it costs on any host: a wake for each of them.
    sim.run_for(Duration::from_secs(60 * 60));
    assert!(
        (3500..=3800).contains(&sim.wakeups),
        "woke {} times in an hour and a half minute",
        sim.wakeups
    );
}

#[test]
fn a_windows_style_tray_showing_minutes_wakes_rarely() {
    let settings = Settings {
        tray_display: TrayDisplay::Minutes,
        ..Settings::default()
    };
    let mut sim = Sim::new(settings, TrayCaps::ICON_AND_TOOLTIP);
    assert_eq!(
        sim.tray.icon.unwrap().1,
        Some(Badge::Number(20)),
        "the minutes go into the icon"
    );
    assert!(sim.tray.tooltip.contains("20m"));

    sim.run_for(Duration::from_secs(60 * 60));
    // One wake per minute for the badge, one per second for each 20 second break, and the
    // idle check every 15 seconds - which is what dominates, at four an hour per minute.
    assert!(sim.wakeups < 400, "woke {} times an hour", sim.wakeups);
    assert_eq!(sim.reminders(), 2);
}

#[test]
fn a_full_day_of_mixed_interruptions_stays_consistent() {
    let mut sim = Sim::with_defaults();
    for _ in 0..4 {
        sim.run_for(Duration::from_secs(50 * 60));
        sim.suspend_machine(Duration::from_secs(10 * 60));
        sim.idle.set(Duration::from_secs(5 * 60));
        sim.run_for(Duration::from_secs(5 * 60));
        sim.idle.set(Duration::ZERO);
        sim.send(Event::TogglePause);
        sim.run_for(Duration::from_secs(10 * 60));
        sim.send(Event::TogglePause);
        sim.send(Event::TakeBreakNow);
        sim.run_for(Duration::from_secs(60));
    }
    assert_eq!(sim.engine.status(), Status::Running);
    assert_eq!(sim.engine.phase(), Phase::Work);
    assert!(sim.remaining() <= Duration::from_secs(20 * 60));
    assert!(sim.errors.is_empty());
    assert!(
        sim.reminders() >= 8,
        "reminders kept coming all day: {}",
        sim.reminders()
    );
}
