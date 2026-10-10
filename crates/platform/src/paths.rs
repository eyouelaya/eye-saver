//! Where EyeSaver keeps its files.
//!
//! The layout is derived from a base directory by a pure function, so the names and nesting
//! are testable; only the question of *which* base directory the host uses is left to
//! [`crate::os`].

use std::path::{Path, PathBuf};

/// Directory name used inside the platform's per-user application data directory.
pub const APP_DIR: &str = "EyeSaver";

/// Every file the application owns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    /// The directory holding all of the below.
    pub dir: PathBuf,
    /// The TOML settings file.
    pub settings: PathBuf,
    /// The log file.
    pub log: PathBuf,
    /// The lock file whose exclusive lock marks the running instance.
    pub lock: PathBuf,
    /// The socket a second launch uses to tell the running instance to show its window.
    pub socket: PathBuf,
}

impl Layout {
    /// The layout rooted at `base`, which is expected to be a per-user data directory.
    #[must_use]
    pub fn under(base: impl AsRef<Path>) -> Self {
        let dir = base.as_ref().join(APP_DIR);
        Self {
            settings: dir.join("settings.toml"),
            log: dir.join("eye-saver.log"),
            lock: dir.join("instance.lock"),
            socket: dir.join("instance.sock"),
            dir,
        }
    }

    /// The layout for the current user on this host.
    #[must_use]
    pub fn for_host() -> Self {
        Self::under(crate::os::data_base_dir())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_file_lives_in_one_named_directory() {
        let layout = Layout::under("/tmp/base");
        assert_eq!(layout.dir, Path::new("/tmp/base/EyeSaver"));
        assert_eq!(
            layout.settings,
            Path::new("/tmp/base/EyeSaver/settings.toml")
        );
        assert_eq!(layout.log, Path::new("/tmp/base/EyeSaver/eye-saver.log"));
        assert_eq!(layout.lock, Path::new("/tmp/base/EyeSaver/instance.lock"));
        assert_eq!(layout.socket, Path::new("/tmp/base/EyeSaver/instance.sock"));
        for path in [&layout.settings, &layout.log, &layout.lock, &layout.socket] {
            assert_eq!(path.parent(), Some(layout.dir.as_path()));
        }
    }

    #[test]
    fn the_layout_accepts_any_base_and_stays_distinct() {
        let a = Layout::under("/one");
        let b = Layout::under("/two");
        assert_ne!(a, b);
        assert_eq!(a, Layout::under(PathBuf::from("/one")));
    }

    #[test]
    fn the_host_layout_is_an_absolute_path_under_the_users_own_directory() {
        let layout = Layout::for_host();
        assert!(layout.dir.is_absolute(), "{:?}", layout.dir);
        assert!(layout.dir.ends_with(APP_DIR));
    }
}
