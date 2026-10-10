//! Registering the app to start when the user logs in.
//!
//! macOS reads a LaunchAgent property list from the user's home directory, so the whole
//! implementation is writing and deleting one file - which means it can be tested against
//! a temporary directory rather than the real one. Windows needs a registry value, so that
//! half lives in [`crate::os`].
//!
//! Both file contents come from `eye_saver_core::autostart_spec`, where the escaping and
//! quoting are already tested.

#[cfg(target_os = "macos")]
mod launch_agent {
    use std::fs;
    use std::path::{Path, PathBuf};

    use eye_saver_core::autostart_spec::{launch_agent_file_name, launch_agent_plist};
    use eye_saver_core::ports::{Autostart, HostResult};

    /// A LaunchAgent plist in a given directory.
    #[derive(Clone, Debug)]
    pub struct LaunchAgent {
        dir: PathBuf,
        program: PathBuf,
    }

    impl LaunchAgent {
        /// An agent that would run `program`, with its plist kept in `dir`.
        #[must_use]
        pub fn new(dir: impl Into<PathBuf>, program: impl Into<PathBuf>) -> Self {
            Self {
                dir: dir.into(),
                program: program.into(),
            }
        }

        /// The agent for this installation: `~/Library/LaunchAgents`, running this binary.
        #[must_use]
        pub fn for_host() -> Self {
            Self::new(crate::os::launch_agents_dir(), crate::os::executable_path())
        }

        /// Where the plist is, or would be.
        #[must_use]
        pub fn plist_path(&self) -> PathBuf {
            self.dir.join(launch_agent_file_name())
        }

        /// The program the plist points at.
        #[must_use]
        pub fn program(&self) -> &Path {
            &self.program
        }
    }

    impl Autostart for LaunchAgent {
        fn is_enabled(&self) -> bool {
            self.plist_path().is_file()
        }

        fn set_enabled(&self, enabled: bool) -> HostResult {
            let path = self.plist_path();
            if !enabled {
                return match fs::remove_file(&path) {
                    Ok(()) => Ok(()),
                    // Already absent is the state that was asked for.
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(e) => Err(format!("removing {}: {e}", path.display())),
                };
            }
            fs::create_dir_all(&self.dir)
                .map_err(|e| format!("creating {}: {e}", self.dir.display()))?;
            let plist = launch_agent_plist(&self.program.to_string_lossy());
            fs::write(&path, plist).map_err(|e| format!("writing {}: {e}", path.display()))
        }
    }
}

#[cfg(target_os = "macos")]
pub use launch_agent::LaunchAgent;

/// The login-item registration for this host.
#[must_use]
pub fn for_host() -> Box<dyn eye_saver_core::ports::Autostart> {
    #[cfg(target_os = "macos")]
    {
        Box::new(LaunchAgent::for_host())
    }
    #[cfg(windows)]
    {
        Box::new(crate::os::RunKey::for_host())
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        Box::new(crate::os::UnsupportedAutostart)
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use crate::test_support::TempDir;
    use eye_saver_core::ports::Autostart;

    #[test]
    fn enabling_writes_a_plist_naming_this_binary() {
        let dir = TempDir::new("autostart-enable");
        let agent = LaunchAgent::new(
            dir.path().join("LaunchAgents"),
            "/Applications/EyeSaver.app/Contents/MacOS/eye-saver",
        );
        assert!(!agent.is_enabled());

        agent.set_enabled(true).unwrap();
        assert!(agent.is_enabled());

        let plist = std::fs::read_to_string(agent.plist_path()).unwrap();
        assert!(plist.contains("com.eyesaver.app"));
        assert!(plist.contains("/Applications/EyeSaver.app/Contents/MacOS/eye-saver"));
        assert!(plist.contains("<key>RunAtLoad</key>"));
        assert_eq!(
            agent.plist_path().file_name().unwrap(),
            "com.eyesaver.app.plist"
        );
    }

    #[test]
    fn disabling_removes_the_plist_and_is_idempotent() {
        let dir = TempDir::new("autostart-disable");
        let agent = LaunchAgent::new(dir.path(), "/bin/true");
        agent.set_enabled(true).unwrap();
        agent.set_enabled(false).unwrap();
        assert!(!agent.is_enabled());
        agent.set_enabled(false).unwrap();
        assert!(!agent.plist_path().exists());
    }

    #[test]
    fn enabling_twice_rewrites_rather_than_failing() {
        let dir = TempDir::new("autostart-twice");
        let agent = LaunchAgent::new(dir.path(), "/bin/true");
        agent.set_enabled(true).unwrap();
        agent.set_enabled(true).unwrap();
        assert!(agent.is_enabled());
    }

    #[test]
    fn an_unwritable_directory_is_reported() {
        let dir = TempDir::new("autostart-unwritable");
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, "not a directory").unwrap();
        let agent = LaunchAgent::new(blocker.join("LaunchAgents"), "/bin/true");
        assert!(
            agent
                .set_enabled(true)
                .unwrap_err()
                .contains("LaunchAgents")
        );
    }

    #[test]
    fn a_plist_that_cannot_be_removed_is_reported() {
        let dir = TempDir::new("autostart-rm");
        let agent = LaunchAgent::new(dir.path().join("sub"), "/bin/true");
        std::fs::create_dir_all(agent.plist_path()).unwrap();
        assert!(
            agent.set_enabled(false).is_err(),
            "a directory in the way is reported"
        );
        assert!(
            !agent.is_enabled(),
            "and is not mistaken for a registration"
        );
    }

    #[test]
    fn the_host_agent_points_into_the_users_library() {
        let agent = LaunchAgent::for_host();
        assert!(agent.plist_path().is_absolute());
        assert!(
            agent
                .plist_path()
                .to_string_lossy()
                .contains("LaunchAgents")
        );
        assert!(agent.program().is_absolute());
    }

    #[test]
    fn the_host_registration_can_be_queried_without_changing_anything() {
        let before = for_host().is_enabled();
        assert_eq!(for_host().is_enabled(), before);
    }
}
