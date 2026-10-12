//! Stubs for platforms EyeSaver does not target.
//!
//! EyeSaver ships for macOS and Windows. These stubs exist so that the crate still
//! compiles elsewhere - which keeps `cargo check` useful on a Linux CI runner - while
//! reporting honestly that nothing is wired up.

use std::path::PathBuf;

use eye_saver_core::ports::{Autostart, HostResult, ScreenSource};
use eye_saver_core::screen::ScreenStatus;

/// Always reports an awake, unlocked screen.
#[derive(Clone, Copy, Debug, Default)]
pub struct Screen;

impl ScreenSource for Screen {
    fn screen_status(&self) -> ScreenStatus {
        ScreenStatus::AWAKE
    }
}

/// Reports that starting at login is not available here.
#[derive(Clone, Copy, Debug, Default)]
pub struct UnsupportedAutostart;

impl Autostart for UnsupportedAutostart {
    fn is_enabled(&self) -> bool {
        false
    }
    fn set_enabled(&self, _enabled: bool) -> HostResult {
        Err("starting at login is only supported on macOS and Windows".to_owned())
    }
}

/// The user's data directory, by the XDG convention.
#[must_use]
pub fn data_base_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME").map_or_else(
        || {
            std::env::var_os("HOME")
                .map_or_else(std::env::temp_dir, PathBuf::from)
                .join(".local")
                .join("share")
        },
        PathBuf::from,
    )
}

/// This binary.
#[must_use]
pub fn executable_path() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("eye-saver"))
}

/// A conventional tray icon size.
#[must_use]
pub fn tray_icon_size() -> u32 {
    22
}
