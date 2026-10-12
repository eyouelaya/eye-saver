//! The traits through which the pure core reaches the outside world.
//!
//! Each one is implemented twice: against the real OS in `eye-saver-platform`, and by a
//! recording fake in [`crate::testing`]. Nothing else in this crate knows that an
//! operating system exists.

use crate::engine::{Badge, IconKind, MenuModel, Notification};
use crate::settings::Settings;

/// Errors from the host are reported as text: the core never branches on them, it only
/// passes them to the log, so a richer type would buy nothing.
pub type HostResult = Result<(), String>;

/// The menu bar / notification area item.
pub trait Tray {
    /// Sets the text beside the icon. Hosts without text labels may ignore this.
    ///
    /// # Errors
    ///
    /// If the host refuses or is unavailable; the message is for the log.
    fn set_label(&mut self, label: &str) -> HostResult;
    /// Sets the hover text.
    ///
    /// # Errors
    ///
    /// If the host refuses or is unavailable; the message is for the log.
    fn set_tooltip(&mut self, tooltip: &str) -> HostResult;
    /// Redraws the icon, optionally with the countdown drawn in place of the artwork.
    ///
    /// # Errors
    ///
    /// If the host refuses or is unavailable; the message is for the log.
    fn set_icon(&mut self, kind: IconKind, badge: Option<Badge>) -> HostResult;
    /// Updates the menu contents.
    ///
    /// # Errors
    ///
    /// If the host refuses or is unavailable; the message is for the log.
    fn set_menu(&mut self, menu: &MenuModel) -> HostResult;
}

/// Desktop notifications.
pub trait Notifier {
    /// Posts a notification.
    ///
    /// # Errors
    ///
    /// If the host refuses or is unavailable; the message is for the log.
    fn post(&self, notification: &Notification) -> HostResult;
}

/// How long the user has been away from the keyboard and mouse.
pub trait IdleSource {
    /// Seconds since the last input event, as a duration.
    fn idle_time(&self) -> std::time::Duration;
}

/// What the screen is doing.
pub trait ScreenSource {
    /// Reads the current display and lock state.
    fn screen_status(&self) -> crate::screen::ScreenStatus;
}

/// Registration as a login item.
pub trait Autostart {
    /// Whether the app is currently registered to start at login.
    fn is_enabled(&self) -> bool;
    /// Registers or deregisters the app.
    ///
    /// # Errors
    ///
    /// If the host refuses or is unavailable; the message is for the log.
    fn set_enabled(&self, enabled: bool) -> HostResult;
}

/// Where settings live between runs.
pub trait SettingsStore {
    /// Loads settings, falling back to defaults if there is nothing valid to load.
    fn load(&self) -> Settings;
    /// Writes settings.
    ///
    /// # Errors
    ///
    /// If the host refuses or is unavailable; the message is for the log.
    fn save(&self, settings: &Settings) -> HostResult;
}

/// Opening and closing the settings window.
pub trait Ui {
    /// Shows the settings window, creating it if necessary, and focuses it.
    ///
    /// # Errors
    ///
    /// If the host refuses or is unavailable; the message is for the log.
    fn open_settings(&mut self) -> HostResult;
}
