//! Desktop notifications.
//!
//! Both platforms attribute a notification to an application identity rather than to a
//! process: macOS reads it from the surrounding `.app` bundle, and Windows from the
//! `AppUserModelID` on the Start Menu shortcut the installer creates. Neither is available
//! to a bare `cargo run`, which is why the README asks for `make dev-bundle` when testing
//! notifications by hand.
//!
//! macOS uses the modern `UNUserNotificationCenter` API (via notify-rust's
//! `preview-macos-un` feature) rather than the older `NSUserNotificationCenter` path:
//! that one has been deprecated since macOS 11, and on current macOS versions it no
//! longer delivers anything - notifications were posted without error, but nothing
//! appeared. `UNUserNotificationCenter` needs two things the old API did not: the binary
//! has to be in a code-signed bundle (an ad-hoc signature, which `make dev-bundle` and
//! both installers produce, is enough), and the user has to grant permission, asked for
//! once at startup below.

use eye_saver_core::engine::Notification;
use eye_saver_core::ports::{HostResult, Notifier};

/// Posts notifications through the platform's notification centre.
#[derive(Clone, Debug, Default)]
pub struct DesktopNotifier;

impl DesktopNotifier {
    /// Prepares the notification backend and, on macOS, asks the user for permission.
    ///
    /// Notifications there show nothing at all until permission is granted, and nothing
    /// ever asks for it except this call - so it has to happen before the first
    /// reminder. Calling it again after the user has already answered does not show a
    /// second prompt, so it is safe to do on every launch rather than only the first.
    #[must_use]
    pub fn new() -> Self {
        #[cfg(target_os = "macos")]
        match notify_rust::check_bundle() {
            Err(e) => log::warn!(
                "notifications are unavailable: {e}. \
                 Run the bundled app (make dev-bundle) to see them."
            ),
            Ok(()) => match notify_rust::request_auth_blocking() {
                Ok(true) => log::debug!("notification permission is granted"),
                Ok(false) => {
                    // `request_auth_blocking` only ever returns a bool, so the log is made
                    // actionable with a second call that can tell "macOS never actually
                    // asked" apart from "the user said no" - the fix is different for each.
                    let status = notify_rust::get_notification_settings_blocking()
                        .map(|s| format!("{:?}", s.authorization_status));
                    log::warn!(
                        "EyeSaver does not have permission to show notifications                          ({status:?}). Open System Settings > Notifications > EyeSaver                          and turn notifications on."
                    );
                }
                Err(e) => log::warn!("could not request notification permission: {e}"),
            },
        }
        Self
    }
}

impl Notifier for DesktopNotifier {
    fn post(&self, notification: &Notification) -> HostResult {
        let mut toast = notify_rust::Notification::new();
        toast.summary(&notification.title).body(&notification.body);

        #[cfg(windows)]
        toast.app_id(eye_saver_core::APP_ID);

        if notification.sound {
            #[cfg(target_os = "macos")]
            toast.sound_name("Ping");
            #[cfg(not(target_os = "macos"))]
            toast.sound_name("Default");
        }

        toast
            .show()
            .map(|_| ())
            .map_err(|e| format!("posting a notification: {e}"))
    }
}
