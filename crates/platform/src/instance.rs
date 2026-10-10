//! Making sure only one EyeSaver runs at a time.
//!
//! Two instances would mean two countdowns, two notifications and two tray icons, so the
//! second launch has to stand down. It does so by failing to take an exclusive lock on a
//! file in the data directory - a lock the operating system releases on its own if the
//! holder crashes, which a plain "is the pid file there?" check cannot claim.
//!
//! Standing down is not quite enough to be friendly: someone who launches the app again
//! usually wants its window. So the second launch also pokes the first one, which responds
//! by opening the settings window. On Unix that poke is a connection to a socket beside
//! the lock file; on Windows it is a message to the hidden window the power watcher
//! already owns.

use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};

/// The outcome of trying to become the running instance.
#[derive(Debug)]
pub enum Instance {
    /// This process holds the lock and should run normally.
    First(InstanceLock),
    /// Another process already holds the lock; this one should signal it and exit.
    AlreadyRunning,
}

/// Proof that this process is the running instance.
///
/// The lock lives as long as this value: dropping it, or the process exiting for any
/// reason, releases it.
#[derive(Debug)]
pub struct InstanceLock {
    _file: File,
    path: PathBuf,
}

impl InstanceLock {
    /// The lock file's path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Tries to become the running instance by locking `path`.
///
/// # Errors
///
/// If the lock file cannot be created or the lock cannot be evaluated at all - a read-only
/// data directory, for instance. A lock simply held by someone else is not an error, it is
/// [`Instance::AlreadyRunning`].
pub fn acquire(path: &Path) -> io::Result<Instance> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path)?;
    match file.try_lock() {
        Ok(()) => Ok(Instance::First(InstanceLock {
            _file: file,
            path: path.to_path_buf(),
        })),
        Err(TryLockError::WouldBlock) => Ok(Instance::AlreadyRunning),
        Err(TryLockError::Error(e)) => Err(e),
    }
}

#[cfg(unix)]
mod signalling {
    use std::io::{Read as _, Write as _};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::Path;
    use std::{fs, io, thread};

    /// What a second launch sends. Read back only to confirm a real peer.
    const POKE: &[u8] = b"show-settings\n";

    /// Listens for second launches, calling `on_signal` for each one.
    ///
    /// A socket left behind by a crashed instance is removed first: we already hold the
    /// lock, so nobody else can be listening on it.
    ///
    /// # Errors
    ///
    /// If the socket cannot be created, or the listener thread cannot be spawned.
    pub fn listen(socket: &Path, on_signal: impl Fn() + Send + 'static) -> io::Result<()> {
        if let Some(dir) = socket.parent() {
            fs::create_dir_all(dir)?;
        }
        let _ = fs::remove_file(socket);
        let listener = UnixListener::bind(socket)?;
        thread::Builder::new()
            .name("eye-saver-instance".into())
            .spawn(move || {
                for stream in listener.incoming() {
                    match stream {
                        Ok(mut stream) => {
                            let mut buffer = [0u8; POKE.len()];
                            let _ = stream.read(&mut buffer);
                            on_signal();
                        }
                        Err(e) => log::warn!("instance socket: {e}"),
                    }
                }
            })?;
        Ok(())
    }

    /// Asks the running instance to show its settings window.
    ///
    /// # Errors
    ///
    /// If nothing is listening, which is also how a caller learns there is none.
    pub fn signal(socket: &Path) -> io::Result<()> {
        let mut stream = UnixStream::connect(socket)?;
        stream.write_all(POKE)
    }
}

#[cfg(windows)]
mod signalling {
    use std::io;
    use std::path::Path;

    /// Listens on a named pipe. The socket path is unused: a pipe name is global, not a
    /// file, so there is nothing to put in the data directory.
    ///
    /// # Errors
    ///
    /// If the listener thread cannot be spawned.
    pub fn listen(_socket: &Path, on_signal: impl Fn() + Send + 'static) -> io::Result<()> {
        crate::os::listen_for_signals(on_signal)
    }

    /// Asks the running instance to show its settings window.
    ///
    /// # Errors
    ///
    /// If nothing is listening, which is also how a caller learns there is none.
    pub fn signal(_socket: &Path) -> io::Result<()> {
        crate::os::signal_running_instance()
    }
}

pub use signalling::{listen, signal};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    #[test]
    fn the_first_caller_gets_the_lock_and_the_second_is_told_to_stand_down() {
        let dir = TempDir::new("instance-lock");
        let path = dir.path().join("instance.lock");

        let first = acquire(&path).unwrap();
        let Instance::First(lock) = first else {
            panic!("the first caller must win")
        };
        assert_eq!(lock.path(), path);

        assert!(
            matches!(acquire(&path).unwrap(), Instance::AlreadyRunning),
            "a second caller must not start a rival timer"
        );

        // Releasing the lock lets the next launch take over, as it would after a crash.
        drop(lock);
        assert!(matches!(acquire(&path).unwrap(), Instance::First(_)));
    }

    #[test]
    fn the_lock_directory_is_created_on_demand() {
        let dir = TempDir::new("instance-mkdir");
        let path = dir
            .path()
            .join("nested")
            .join("deeper")
            .join("instance.lock");
        assert!(matches!(acquire(&path).unwrap(), Instance::First(_)));
        assert!(path.exists());
    }

    #[test]
    fn an_unusable_lock_path_is_an_error_not_a_second_instance() {
        let dir = TempDir::new("instance-bad-path");
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, "not a directory").unwrap();
        assert!(acquire(&blocker.join("instance.lock")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_second_launch_pokes_the_running_instance() {
        use std::sync::mpsc;
        use std::time::Duration;

        let dir = TempDir::new("instance-signal");
        let socket = dir.path().join("instance.sock");

        let (tx, rx) = mpsc::channel();
        listen(&socket, move || {
            let _ = tx.send(());
        })
        .unwrap();

        signal(&socket).unwrap();
        rx.recv_timeout(Duration::from_secs(5))
            .expect("the running instance was not poked");

        // And again, because a user may well launch it a third time.
        signal(&socket).unwrap();
        rx.recv_timeout(Duration::from_secs(5))
            .expect("the second poke was missed");
    }

    #[cfg(unix)]
    #[test]
    fn poking_nothing_is_an_error_rather_than_a_hang() {
        let dir = TempDir::new("instance-no-listener");
        assert!(signal(&dir.path().join("absent.sock")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_socket_left_behind_by_a_crash_is_replaced() {
        use std::sync::mpsc;
        use std::time::Duration;

        let dir = TempDir::new("instance-stale");
        let socket = dir.path().join("instance.sock");
        std::fs::write(&socket, "stale file where a socket should be").unwrap();

        let (tx, rx) = mpsc::channel();
        listen(&socket, move || {
            let _ = tx.send(());
        })
        .unwrap();
        signal(&socket).unwrap();
        rx.recv_timeout(Duration::from_secs(5))
            .expect("a stale socket must not be fatal");
    }
}
