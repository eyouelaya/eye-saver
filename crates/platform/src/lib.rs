//! Operating-system adapters for EyeSaver.
//!
//! Every type here implements one of the traits in `eye_saver_core::ports`, so that the
//! pure core stays unaware of which platform it is running on. The split inside this crate
//! matters for testing:
//!
//! * The modules at the top level are ordinary code - file handling, path building, lock
//!   files - and are unit tested like anything else.
//! * [`os`] is the irreducible part: calls into AppKit, Win32, the tray and the
//!   notification centre. It cannot be exercised without a logged-in desktop session, so
//!   it is kept as thin as possible and excluded from the coverage gate.

#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]
// Product and platform names (EyeSaver, macOS, LaunchAgent) are prose, not code.
#![allow(clippy::doc_markdown)]
// `assert!(x.is_empty())` reads better in a test than comparing against a typed empty
// slice, and the test names say what is expected.
#![allow(clippy::assert_is_empty)]

pub mod autostart;
pub mod clock;
pub mod instance;
pub mod logger;
pub mod os;
pub mod paths;
pub mod store;

#[cfg(test)]
mod test_support;

pub use clock::SystemClock;
pub use instance::{Instance, InstanceLock};
pub use paths::Layout;
pub use store::FileStore;
