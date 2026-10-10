//! Reading and writing the settings file.
//!
//! Saving is atomic: the new contents go to a temporary file in the same directory and are
//! then renamed over the old one, so a crash or a full disk cannot leave a half-written
//! config that fails to parse on the next launch.

use std::fs;
use std::path::{Path, PathBuf};

use eye_saver_core::ports::{HostResult, SettingsStore};
use eye_saver_core::settings::Settings;

/// A settings file on disk.
#[derive(Clone, Debug)]
pub struct FileStore {
    path: PathBuf,
}

impl FileStore {
    /// A store backed by `path`. The file need not exist yet.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The file this store reads and writes.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl SettingsStore for FileStore {
    /// Loads the settings, falling back to the defaults for anything missing or unreadable.
    ///
    /// A config file the user has broken by hand is a reason to warn, never a reason to
    /// refuse to start: the whole app is a background reminder.
    fn load(&self) -> Settings {
        let text = match fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Settings::default(),
            Err(e) => {
                log::warn!("could not read {}: {e}", self.path.display());
                return Settings::default();
            }
        };
        let (settings, problem) = Settings::from_toml_lossy(&text);
        if let Some(problem) = problem {
            log::warn!("{}: {problem}; using defaults", self.path.display());
        }
        settings
    }

    fn save(&self, settings: &Settings) -> HostResult {
        let dir = self
            .path
            .parent()
            .ok_or_else(|| "settings path has no parent".to_owned())?;
        fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;

        let temporary = self.path.with_extension("toml.new");
        fs::write(&temporary, settings.to_toml())
            .map_err(|e| format!("writing {}: {e}", temporary.display()))?;
        fs::rename(&temporary, &self.path).map_err(|e| {
            let _ = fs::remove_file(&temporary);
            format!("replacing {}: {e}", self.path.display())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;
    use eye_saver_core::settings::{TrayDisplay, WORK_SECS};

    #[test]
    fn a_missing_file_loads_the_defaults_without_creating_anything() {
        let dir = TempDir::new("store-missing");
        let store = FileStore::new(dir.path().join("settings.toml"));
        assert_eq!(store.load(), Settings::default());
        assert!(!store.path().exists(), "loading must not write");
    }

    #[test]
    fn settings_survive_a_round_trip_through_the_file() {
        let dir = TempDir::new("store-round-trip");
        let store = FileStore::new(dir.path().join("settings.toml"));
        let settings = Settings {
            work_interval_secs: 900,
            break_duration_secs: 25,
            idle_credit_secs: 120,
            tray_display: TrayDisplay::IconOnly,
            start_at_login: true,
            notification_sound: false,
        };
        store.save(&settings).unwrap();
        assert!(store.path().exists());
        assert_eq!(store.load(), settings);
    }

    #[test]
    fn saving_creates_the_directory_and_leaves_no_temporary_behind() {
        let dir = TempDir::new("store-mkdir");
        let nested = dir.path().join("deeply").join("nested");
        let store = FileStore::new(nested.join("settings.toml"));
        store.save(&Settings::default()).unwrap();
        assert!(store.path().exists());
        let leftovers: Vec<_> = fs::read_dir(&nested)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| {
                std::path::Path::new(name)
                    .extension()
                    .is_some_and(|e| e == "new")
            })
            .collect();
        assert!(
            leftovers.is_empty(),
            "temporary files left behind: {leftovers:?}"
        );
    }

    #[test]
    fn saving_twice_replaces_rather_than_appends() {
        let dir = TempDir::new("store-replace");
        let store = FileStore::new(dir.path().join("settings.toml"));
        store.save(&Settings::default()).unwrap();
        let changed = Settings {
            work_interval_secs: 600,
            ..Settings::default()
        };
        store.save(&changed).unwrap();
        assert_eq!(store.load(), changed);
        assert_eq!(
            fs::read_to_string(store.path())
                .unwrap()
                .matches("[[")
                .count(),
            0
        );
    }

    #[test]
    fn a_corrupt_file_loads_the_defaults_instead_of_failing() {
        let dir = TempDir::new("store-corrupt");
        let path = dir.path().join("settings.toml");
        fs::write(&path, "this is not = = toml").unwrap();
        assert_eq!(FileStore::new(&path).load(), Settings::default());
    }

    #[test]
    fn a_hand_edited_file_with_absurd_values_is_clamped_on_load() {
        let dir = TempDir::new("store-clamp");
        let path = dir.path().join("settings.toml");
        fs::write(&path, "work-interval-secs = 99999999").unwrap();
        assert_eq!(FileStore::new(&path).load().work_interval_secs, WORK_SECS.1);
    }

    #[test]
    fn a_directory_where_the_file_should_be_is_reported_not_panicked() {
        let dir = TempDir::new("store-isdir");
        let path = dir.path().join("settings.toml");
        fs::create_dir(&path).unwrap();
        let store = FileStore::new(&path);
        assert_eq!(
            store.load(),
            Settings::default(),
            "unreadable means defaults"
        );
        assert!(
            store.save(&Settings::default()).is_err(),
            "and saving says why"
        );
    }

    #[test]
    fn a_path_with_no_parent_is_refused() {
        let store = FileStore::new("/");
        assert_eq!(
            store.save(&Settings::default()).unwrap_err(),
            "settings path has no parent"
        );
    }
}
