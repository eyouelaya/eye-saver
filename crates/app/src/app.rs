//! The event loop.
//!
//! This is the whole of the application's "wiring": it owns the operating system objects,
//! pumps events into `eye_saver_core::engine::Engine`, and hands the effects it gets back
//! to `eye_saver_core::runner`. It makes no decisions of its own, which is why the rules
//! can be tested without a desktop.
//!
//! The loop sleeps with `ControlFlow::WaitUntil`, using the deadline the engine asks for.
//! That is the whole of the battery story: a running timer with a second-by-second
//! countdown wakes once a second to format a string, a coarser display wakes once a
//! minute, and a paused timer asks for no deadline at all, so the process blocks
//! indefinitely until the user clicks something.

use std::time::{Duration, Instant};

use eye_saver_core::engine::{Badge, Effect, Engine, Event, IconKind, MenuModel};
use eye_saver_core::ports::{Autostart, HostResult, ScreenSource as _, Tray, Ui};
use eye_saver_core::runner::{self, Host};
use eye_saver_core::screen::ScreenWatch;
use eye_saver_core::{Clock as _, Mono};
use eye_saver_platform::os::{self, DesktopNotifier, MenuChoice, TrayAdapter};
use eye_saver_platform::{FileStore, SystemClock};
use tray_icon::menu::MenuId;
use winit::application::ApplicationHandler;
use winit::event::{StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy};
use winit::window::WindowId;

use crate::window::SettingsWindow;

/// Events that reach the loop from somewhere other than winit.
#[derive(Debug)]
pub enum UserEvent {
    /// A tray menu item was chosen.
    Menu(MenuId),
    /// Another launch asked for the settings window.
    ShowSettings,
    /// egui asked to be repainted after this delay.
    Repaint(Duration),
}

/// The application.
pub struct App {
    engine: Engine,
    pending: Vec<Effect>,
    clock: SystemClock,
    watch: ScreenWatch,
    screen: os::Screen,
    notifier: DesktopNotifier,
    autostart: Box<dyn Autostart>,
    store: FileStore,
    tray: Option<TrayAdapter>,
    window: Option<SettingsWindow>,
    proxy: EventLoopProxy<UserEvent>,
    open_settings_on_start: bool,
    repaint_at: Option<Instant>,
}

impl App {
    /// Assembles the application around an engine that has already been built.
    pub fn new(
        engine: Engine,
        pending: Vec<Effect>,
        clock: SystemClock,
        store: FileStore,
        proxy: EventLoopProxy<UserEvent>,
        open_settings_on_start: bool,
    ) -> Self {
        Self {
            engine,
            pending,
            clock,
            watch: ScreenWatch::default(),
            screen: os::Screen,
            notifier: DesktopNotifier::new(),
            autostart: eye_saver_platform::autostart::for_host(),
            store,
            tray: None,
            window: None,
            proxy,
            open_settings_on_start,
            repaint_at: None,
        }
    }

    /// Feeds events to the engine and carries out whatever it asks for.
    fn pump(&mut self, event_loop: &ActiveEventLoop, events: impl IntoIterator<Item = Event>) {
        for event in events {
            let now = self.clock.now();
            let effects = self.engine.handle(event, now);
            self.dispatch(event_loop, &effects);
        }
    }

    fn dispatch(&mut self, event_loop: &ActiveEventLoop, effects: &[Effect]) {
        let mut open = OpenRequest::default();
        let mut fallback = NullTray;
        let Self {
            tray,
            notifier,
            autostart,
            store,
            ..
        } = self;
        let tray: &mut dyn Tray = match tray.as_mut() {
            Some(tray) => tray,
            None => &mut fallback,
        };
        let outcome = runner::apply(
            effects,
            &mut Host {
                tray,
                notifier,
                autostart: &**autostart,
                store,
                ui: &mut open,
            },
        );

        for error in &outcome.errors {
            log::warn!("{error}");
        }
        if outcome.exit {
            log::info!("quitting");
            event_loop.exit();
            return;
        }
        if open.requested {
            self.show_settings(event_loop);
        }
    }

    /// One iteration of the loop: look at the screen, then tick.
    fn poll_and_tick(&mut self, event_loop: &ActiveEventLoop) {
        let status = self.screen.screen_status();
        let mut events = self.watch.observe(status);
        events.push(Event::Tick);
        self.pump(event_loop, events);
    }

    fn show_settings(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            window.focus();
            window.request_redraw();
            return;
        }
        match SettingsWindow::open(event_loop, *self.engine.settings(), self.proxy.clone()) {
            Ok(window) => {
                window.focus();
                window.request_redraw();
                self.window = Some(window);
            }
            Err(e) => log::error!("could not open the settings window: {e}"),
        }
    }

    fn close_settings(&mut self) {
        if let Some(window) = self.window.take() {
            window.close();
        }
        self.repaint_at = None;
    }

    /// Paints the settings window and acts on what the user chose.
    fn draw_settings(&mut self, event_loop: &ActiveEventLoop) {
        let saved = *self.engine.settings();
        let Some(window) = self.window.as_mut() else {
            return;
        };

        let action = window.draw(&saved);
        let draft = window.draft();
        if action.revert {
            window.set_draft(saved);
            window.request_redraw();
        }

        if action.save {
            self.pump(event_loop, [Event::SettingsChanged(draft)]);
            // The engine clamps, so show what was actually applied rather than what was
            // asked for.
            let applied = *self.engine.settings();
            if let Some(window) = self.window.as_mut() {
                window.set_draft(applied);
                window.request_redraw();
            }
        }
        if action.close {
            self.close_settings();
        }
    }

    /// The next instant the loop needs to be awake, counting both halves of the app.
    fn deadline(&self) -> Option<Instant> {
        let now = self.clock.now();
        let engine = self
            .engine
            .host_deadline(now)
            .map(|at: Mono| Instant::now() + at.since(now.mono));
        [engine, self.repaint_at].into_iter().flatten().min()
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.tray.is_some() {
            return;
        }
        let now = self.clock.now();
        match TrayAdapter::new(&self.engine.menu(now)) {
            Ok(tray) => self.tray = Some(tray),
            // Without a tray there is no way to reach the app at all, so this is fatal.
            Err(e) => {
                log::error!("could not create the tray icon: {e}");
                event_loop.exit();
                return;
            }
        }

        let pending = std::mem::take(&mut self.pending);
        self.dispatch(event_loop, &pending);
        self.poll_and_tick(event_loop);

        if self.open_settings_on_start {
            self.open_settings_on_start = false;
            self.show_settings(event_loop);
        }
    }

    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        if !matches!(
            cause,
            StartCause::ResumeTimeReached { .. } | StartCause::Poll
        ) {
            return;
        }
        if self.repaint_at.is_some_and(|at| at <= Instant::now()) {
            self.repaint_at = None;
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
        self.poll_and_tick(event_loop);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Menu(id) => {
                let Some(choice) = self.tray.as_ref().and_then(|tray| tray.choice(&id)) else {
                    return;
                };
                let event = match choice {
                    MenuChoice::TakeBreakNow => Event::TakeBreakNow,
                    MenuChoice::ResetCycle => Event::ResetCycle,
                    MenuChoice::TogglePause => Event::TogglePause,
                    MenuChoice::OpenSettings => Event::OpenSettingsRequested,
                    // Nothing for the engine to decide: the same link the settings
                    // window shows, followed without opening the window first.
                    MenuChoice::CheckForUpdates => {
                        if let Err(e) = webbrowser::open(eye_saver_core::RELEASES_URL) {
                            log::warn!("could not open the releases page: {e}");
                        }
                        return;
                    }
                    MenuChoice::Quit => Event::QuitRequested,
                };
                self.pump(event_loop, [event]);
            }
            UserEvent::ShowSettings => self.pump(event_loop, [Event::OpenSettingsRequested]),
            UserEvent::Repaint(delay) => {
                if delay.is_zero() {
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                } else {
                    self.repaint_at = Instant::now().checked_add(delay);
                }
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self
            .window
            .as_ref()
            .is_none_or(|window| window.id() != window_id)
        {
            return;
        }
        match event {
            WindowEvent::CloseRequested | WindowEvent::Destroyed => self.close_settings(),
            WindowEvent::RedrawRequested => self.draw_settings(event_loop),
            other => {
                let repaint = self
                    .window
                    .as_mut()
                    .is_some_and(|window| window.on_window_event(&other));
                if repaint && let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        match self.deadline() {
            Some(at) => event_loop.set_control_flow(ControlFlow::WaitUntil(at)),
            // Nothing is pending: block until the user does something. This is what keeps a
            // paused EyeSaver completely off the CPU.
            None => event_loop.set_control_flow(ControlFlow::Wait),
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.close_settings();
        self.tray = None;
    }
}

/// Records that the engine asked for the settings window, so that the window can be
/// created once the event loop is in hand.
#[derive(Debug, Default)]
struct OpenRequest {
    requested: bool,
}

impl Ui for OpenRequest {
    fn open_settings(&mut self) -> HostResult {
        self.requested = true;
        Ok(())
    }
}

/// Stands in for the tray before it exists, so that a failure to create it cannot take the
/// effect dispatcher with it.
#[derive(Debug)]
struct NullTray;

impl Tray for NullTray {
    fn set_label(&mut self, _label: &str) -> HostResult {
        Ok(())
    }
    fn set_tooltip(&mut self, _tooltip: &str) -> HostResult {
        Ok(())
    }
    fn set_icon(&mut self, _kind: IconKind, _badge: Option<Badge>) -> HostResult {
        Ok(())
    }
    fn set_menu(&mut self, _menu: &MenuModel) -> HostResult {
        Ok(())
    }
}
