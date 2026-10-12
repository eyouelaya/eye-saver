//! Pure, deterministic logic for EyeSaver.
//!
//! This crate contains every decision the application makes and knows nothing about any
//! operating system API: no threads, no file system, no clock reading, no GUI. Time enters
//! through [`clock::Mono`] values supplied by the caller, the outside world is reached only
//! through the traits in [`ports`], and every behaviour is therefore unit testable.
//!
//! The application binary is a thin adapter: it reads the clock, pumps OS events into
//! [`engine::Engine::handle`], and hands the returned [`engine::Effect`]s to
//! [`runner::apply`].

#![deny(unsafe_code)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]
// Product and platform names (EyeSaver, macOS, LaunchAgent) are prose, not code.
#![allow(clippy::doc_markdown)]
// `assert!(x.is_empty())` reads better in a test than comparing against a typed empty
// slice, and the test names say what is expected.
#![allow(clippy::assert_is_empty)]

pub mod autostart_spec;
pub mod clock;
pub mod engine;
pub mod format;
pub mod icon;
pub mod ports;
pub mod runner;
pub mod screen;
pub mod settings;
pub mod sleep_detect;
pub mod testing;

pub use clock::{Clock, Mono, Now, Wall};
pub use engine::{Badge, Effect, Engine, Event, IconKind, MenuModel, Phase, Status, SuspendReason};
pub use settings::{Settings, TrayDisplay};

/// Reverse-DNS identifier used for the macOS bundle, the Windows `AppUserModelID` and the
/// LaunchAgent label. Notifications on both platforms are attributed to this identifier.
pub const APP_ID: &str = "com.eyesaver.app";

/// Human readable application name, used in notifications and installer metadata.
pub const APP_NAME: &str = "EyeSaver";

/// Where new versions are published. The settings window links here; nothing fetches it.
pub const RELEASES_URL: &str = "https://github.com/eyouelaya/eye-saver/releases";
