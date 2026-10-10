//! A disposable directory for tests, so the crate needs no development dependencies.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::{env, fs};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A uniquely named directory that deletes itself when dropped.
pub struct TempDir(PathBuf);

impl TempDir {
    /// Creates a directory named after `label`, the process and a counter.
    pub fn new(label: &str) -> Self {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            env::temp_dir().join(format!("eye-saver-{label}-{}-{unique}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("could not create a temporary directory");
        Self(path)
    }

    /// The directory itself.
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
