//! The irreducible platform layer.
//!
//! Everything here needs a real, logged-in desktop session to do anything at all: a menu
//! bar, a notification centre, a window server, a registry. None of it can be exercised
//! by `cargo test` on a build machine, so it is kept deliberately thin - each item does
//! one call and converts the result - and excluded from the coverage gate. The decisions
//! these adapters feed are all tested in `eye-saver-core`.

pub mod notify;
pub mod tray;

use std::sync::OnceLock;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos::subscribe_to_screen_changes;
#[cfg(target_os = "macos")]
pub use macos::{Screen, data_base_dir, executable_path, launch_agents_dir, tray_icon_size};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows::subscribe_to_screen_changes;
#[cfg(windows)]
pub use windows::{
    RunKey, Screen, data_base_dir, executable_path, listen_for_signals, signal_running_instance,
    tray_icon_size,
};

#[cfg(not(any(target_os = "macos", windows)))]
mod unsupported;
#[cfg(not(any(target_os = "macos", windows)))]
pub use unsupported::{
    Screen, UnsupportedAutostart, data_base_dir, executable_path, tray_icon_size,
};

/// What to call when the system says the screen locked, unlocked, went dark or came back.
static ON_SCREEN_CHANGE: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

/// Asks the system to say when the screen locks, unlocks, turns off or turns back on, and
/// calls `on_change` each time, for the life of the process.
///
/// The call carries no state: it means "look now", and the answer is still read from
/// [`Screen`]. That keeps a notification that arrives a moment before the state it
/// announces from being believed over the state itself, and means the periodic look that
/// backs this up needs no separate path.
///
/// Call it once, from the main thread, after the event loop exists. A second call does
/// nothing. On a platform with no such notifications this is a no-op and the periodic look
/// is all there is.
pub fn watch_screen(on_change: impl Fn() + Send + Sync + 'static) {
    if ON_SCREEN_CHANGE.set(Box::new(on_change)).is_ok() {
        #[cfg(any(target_os = "macos", windows))]
        subscribe_to_screen_changes();
    }
}

/// Passes a system notification on to whoever asked to hear about it.
#[cfg(any(target_os = "macos", windows))]
fn screen_changed() {
    if let Some(on_change) = ON_SCREEN_CHANGE.get() {
        on_change();
    }
}

pub use notify::DesktopNotifier;
pub use tray::{MenuChoice, TrayAdapter};
