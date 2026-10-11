//! EyeSaver: a minimalist reminder to rest your eyes.
//!
//! The binary is deliberately thin. It resolves where its files live, makes sure only one
//! instance runs, builds the engine from the saved settings, and hands control to the
//! event loop in [`app`]. Every rule about when to remind, when a break has already been
//! taken and what the tray should say lives in `eye-saver-core`, where it is unit tested.

// No console window should flash up on Windows: this is a tray application.
#![cfg_attr(windows, windows_subsystem = "windows")]
#![warn(clippy::pedantic)]
// Product and platform names are prose, not code.
#![allow(clippy::doc_markdown)]

mod app;
mod ui;
mod window;

use std::process::ExitCode;

use app::{App, UserEvent};
use eye_saver_core::Clock as _;
use eye_saver_core::engine::{Engine, TrayCaps};
use eye_saver_core::ports::SettingsStore as _;
use eye_saver_platform::instance::{self, Instance};
use eye_saver_platform::logger::{self, DEFAULT_MAX_BYTES, Sink};
use eye_saver_platform::{FileStore, Layout, SystemClock};
use log::LevelFilter;
use winit::event_loop::EventLoop;

const USAGE: &str = "\
EyeSaver - look away from the screen every 20 minutes.

Usage: eye-saver [options]

Options:
      --settings   Open the settings window on start, even with --autostart.
      --autostart  Started by the system at login; do not open any window.
  -v, --verbose    Log at debug level.
  -V, --version    Print the version and exit.
  -h, --help       Print this help and exit.

With no options EyeSaver starts in the menu bar or notification area and opens its
settings window, so that launching it visibly does something. Launching it again while it
is running opens that window too, rather than starting a second timer.";

/// What the command line asked for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Args {
    open_settings: bool,
    autostart: bool,
    verbose: bool,
}

/// The reason the process is stopping before the event loop starts.
#[derive(Clone, Debug, PartialEq, Eq)]
enum EarlyExit {
    Help,
    Version,
    Unknown(String),
}

impl Args {
    /// Whether to open the settings window once the tray is up.
    ///
    /// Someone who launches the app by hand gets the window: a tray icon appearing in a
    /// corner is easy to miss, most of all straight after installing. Only a start at
    /// login stays silent, unless the window was asked for by name.
    fn opens_settings(self) -> bool {
        self.open_settings || !self.autostart
    }
}

fn parse_args(raw: impl IntoIterator<Item = String>) -> Result<Args, EarlyExit> {
    let mut args = Args::default();
    for argument in raw {
        match argument.as_str() {
            "--settings" => args.open_settings = true,
            "--autostart" => args.autostart = true,
            "-v" | "--verbose" => args.verbose = true,
            "-h" | "--help" => return Err(EarlyExit::Help),
            "-V" | "--version" => return Err(EarlyExit::Version),
            other => return Err(EarlyExit::Unknown(other.to_owned())),
        }
    }
    Ok(args)
}

fn main() -> ExitCode {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(EarlyExit::Help) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(EarlyExit::Version) => {
            println!("{} {}", eye_saver_core::APP_NAME, env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Err(EarlyExit::Unknown(argument)) => {
            eprintln!("eye-saver: unrecognised option '{argument}'\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    let layout = Layout::for_host();
    let level = if args.verbose {
        LevelFilter::Debug
    } else {
        LevelFilter::Info
    };
    let _ = logger::init(Sink::new(&layout.log, DEFAULT_MAX_BYTES), level);

    // One timer, one tray icon, one set of notifications.
    match instance::acquire(&layout.lock) {
        Ok(Instance::First(lock)) => run(&layout, lock, args),
        Ok(Instance::AlreadyRunning) => {
            log::info!("another EyeSaver is already running; asking it for its window");
            if let Err(e) = instance::signal(&layout.socket) {
                log::warn!("could not reach the running instance: {e}");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            log::error!("could not use {}: {e}", layout.lock.display());
            eprintln!("eye-saver: could not use {}: {e}", layout.lock.display());
            ExitCode::FAILURE
        }
    }
}

fn run(layout: &Layout, lock: instance::InstanceLock, args: Args) -> ExitCode {
    log::info!("EyeSaver {} starting", env!("CARGO_PKG_VERSION"));

    let store = FileStore::new(&layout.settings);
    let settings = store.load();
    let clock = SystemClock::new();
    let (engine, pending) = Engine::new(settings, TrayCaps::for_host(), clock.now());

    let mut builder = EventLoop::<UserEvent>::with_user_event();
    // A menu bar extra, not an application: no Dock icon and no entry in the app switcher.
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS as _};
        builder.with_activation_policy(ActivationPolicy::Accessory);
    }
    let event_loop = match builder.build() {
        Ok(event_loop) => event_loop,
        Err(e) => {
            log::error!("could not start the event loop: {e}");
            eprintln!("eye-saver: could not start the event loop: {e}");
            return ExitCode::FAILURE;
        }
    };

    // Tray and menu activity arrives on channels of their own; forward both into the loop
    // so that there is a single place where things happen.
    let proxy = event_loop.create_proxy();
    let menu_proxy = proxy.clone();
    tray_icon::menu::MenuEvent::set_event_handler(Some(
        move |event: tray_icon::menu::MenuEvent| {
            let _ = menu_proxy.send_event(UserEvent::Menu(event.id));
        },
    ));

    // A second launch pokes us instead of starting a rival timer.
    let instance_proxy = proxy.clone();
    if let Err(e) = instance::listen(&layout.socket, move || {
        let _ = instance_proxy.send_event(UserEvent::ShowSettings);
    }) {
        log::warn!("second launches will not open the settings window: {e}");
    }

    let mut app = App::new(engine, pending, clock, store, proxy, args.opens_settings());
    let result = event_loop.run_app(&mut app);
    drop(lock);

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log::error!("the event loop stopped: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Args, EarlyExit> {
        parse_args(args.iter().map(|s| (*s).to_owned()))
    }

    #[test]
    fn no_arguments_means_run_normally() {
        assert_eq!(parse(&[]), Ok(Args::default()));
    }

    #[test]
    fn each_flag_is_recognised() {
        assert!(parse(&["--settings"]).unwrap().open_settings);
        assert!(parse(&["--autostart"]).unwrap().autostart);
        assert!(parse(&["-v"]).unwrap().verbose);
        assert!(parse(&["--verbose"]).unwrap().verbose);
        assert_eq!(
            parse(&["--settings", "--verbose"]).unwrap(),
            Args {
                open_settings: true,
                autostart: false,
                verbose: true
            }
        );
    }

    #[test]
    fn a_launch_by_hand_opens_settings_and_a_start_at_login_does_not() {
        assert!(parse(&[]).unwrap().opens_settings());
        assert!(parse(&["--verbose"]).unwrap().opens_settings());
        assert!(!parse(&["--autostart"]).unwrap().opens_settings());
        assert!(parse(&["--settings"]).unwrap().opens_settings());
        assert!(
            parse(&["--autostart", "--settings"])
                .unwrap()
                .opens_settings(),
            "asking for the window by name wins"
        );
    }

    #[test]
    fn help_and_version_stop_before_anything_happens() {
        assert_eq!(parse(&["-h"]), Err(EarlyExit::Help));
        assert_eq!(parse(&["--help"]), Err(EarlyExit::Help));
        assert_eq!(parse(&["-V"]), Err(EarlyExit::Version));
        assert_eq!(parse(&["--version"]), Err(EarlyExit::Version));
    }

    #[test]
    fn an_unknown_option_is_reported_with_its_name() {
        assert_eq!(
            parse(&["--wat"]),
            Err(EarlyExit::Unknown("--wat".to_owned()))
        );
    }

    #[test]
    fn the_usage_text_lists_every_flag() {
        for flag in [
            "--settings",
            "--autostart",
            "--verbose",
            "--version",
            "--help",
        ] {
            assert!(USAGE.contains(flag), "{flag} is undocumented");
        }
    }
}
