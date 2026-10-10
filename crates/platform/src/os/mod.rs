//! The irreducible platform layer.
//!
//! Everything here needs a real, logged-in desktop session to do anything at all: a menu
//! bar, a notification centre, a window server, a registry. None of it can be exercised
//! by `cargo test` on a build machine, so it is kept deliberately thin - each item does
//! one call and converts the result - and excluded from the coverage gate. The decisions
//! these adapters feed are all tested in `eye-saver-core`.

pub mod notify;
pub mod tray;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::{Idle, Screen, data_base_dir, executable_path, launch_agents_dir, tray_icon_size};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{
    Idle, RunKey, Screen, data_base_dir, executable_path, listen_for_signals,
    signal_running_instance, tray_icon_size,
};

#[cfg(not(any(target_os = "macos", windows)))]
mod unsupported;
#[cfg(not(any(target_os = "macos", windows)))]
pub use unsupported::{
    Idle, Screen, UnsupportedAutostart, data_base_dir, executable_path, tray_icon_size,
};

pub use notify::DesktopNotifier;
pub use tray::{MenuChoice, TrayAdapter};
