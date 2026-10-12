//! Windows adapters.
//!
//! Three things are asked of the system: whether the session is locked, whether the
//! display is on, and whether another instance is already running. The keyboard and mouse
//! are not asked about at all. Neither is machine suspend - the engine infers it from the
//! wall clock having run ahead of the monotonic clock.
//!
//! Windows offers no way to *query* the display's power state, only to subscribe to it, so
//! a callback keeps the latest answer in a flag for the event loop to read when it looks.

use std::ffi::c_void;
use std::io;
use std::path::PathBuf;
use std::ptr;
use std::sync::Once;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use eye_saver_core::autostart_spec::{RUN_KEY_NAME, RUN_KEY_PATH, run_key_command};
use eye_saver_core::ports::{Autostart, HostResult, ScreenSource};
use eye_saver_core::screen::ScreenStatus;
use windows::Win32::Foundation::{
    CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_PIPE_CONNECTED, ERROR_SUCCESS, GetLastError, HANDLE,
    INVALID_HANDLE_VALUE,
};
use windows::Win32::Storage::FileSystem::{PIPE_ACCESS_INBOUND, ReadFile};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE,
    PIPE_WAIT,
};
use windows::Win32::System::Power::{
    DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS, POWERBROADCAST_SETTING, PowerSettingRegisterNotification,
};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::Win32::System::StationsAndDesktops::{
    CloseDesktop, DESKTOP_CONTROL_FLAGS, DESKTOP_SWITCHDESKTOP, OpenInputDesktop,
};
use windows::Win32::System::SystemServices::GUID_CONSOLE_DISPLAY_STATE;
use windows::Win32::UI::WindowsAndMessaging::{
    DEVICE_NOTIFY_CALLBACK, GetSystemMetrics, PBT_POWERSETTINGCHANGE, SM_CXSMICON,
};
use windows::core::{HSTRING, w};

/// The pipe a second launch writes to, to ask the running instance for its window.
const PIPE_PATH: &str = r"\\.\pipe\EyeSaver.instance";

/// Whether Windows last reported the display as off. Written by [`on_display_state`], on
/// a thread the system owns.
static DISPLAY_OFF: AtomicBool = AtomicBool::new(false);
/// Subscribes to the display state the first time anyone asks about the screen.
static WATCH_DISPLAY: Once = Once::new();

/// Reads the display and lock state.
#[derive(Clone, Copy, Debug, Default)]
pub struct Screen;

impl ScreenSource for Screen {
    fn screen_status(&self) -> ScreenStatus {
        WATCH_DISPLAY.call_once(watch_display);
        ScreenStatus {
            display_asleep: DISPLAY_OFF.load(Ordering::Relaxed),
            locked: session_is_locked(),
        }
    }
}

/// Asks to be told whenever the display turns off or on, for the life of the process.
///
/// Windows calls back once straight away with the current state, so the flag is right
/// from the start. If the subscription is refused the display is simply assumed to be on,
/// which leaves the lock screen and the clocks to notice an absence.
fn watch_display() {
    let parameters = DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
        Callback: Some(on_display_state),
        Context: ptr::null_mut(),
    };
    let setting = GUID_CONSOLE_DISPLAY_STATE;
    let mut registration = ptr::null_mut();
    // SAFETY: with DEVICE_NOTIFY_CALLBACK the "recipient" is a pointer to the parameters,
    // which are read during the call and outlive it; the GUID and the out-pointer are
    // valid for the call. The registration is never released: it is wanted until exit.
    let result = unsafe {
        PowerSettingRegisterNotification(
            &raw const setting,
            DEVICE_NOTIFY_CALLBACK,
            HANDLE((&raw const parameters).cast_mut().cast()),
            &raw mut registration,
        )
    };
    if result != ERROR_SUCCESS {
        log::warn!("the display turning off will not be noticed: {result:?}");
    }
}

/// Records the display state Windows reports: 0 is off, 1 is on, 2 is dimmed.
///
/// Dimmed counts as on - there is still something to look at.
unsafe extern "system" fn on_display_state(
    _context: *const c_void,
    kind: u32,
    setting: *const c_void,
) -> u32 {
    if kind == PBT_POWERSETTINGCHANGE && !setting.is_null() {
        // SAFETY: for this notification type Windows passes a valid, aligned
        // POWERBROADCAST_SETTING that lives for the duration of the callback, and the
        // length is checked before the payload is read.
        let setting = unsafe { &*setting.cast::<POWERBROADCAST_SETTING>() };
        if setting.PowerSetting == GUID_CONSOLE_DISPLAY_STATE && setting.DataLength >= 1 {
            DISPLAY_OFF.store(setting.Data[0] == 0, Ordering::Relaxed);
        }
    }
    0
}

/// Whether the interactive desktop is unavailable to us.
///
/// The lock screen, the secure desktop behind a UAC prompt and a running screen saver all
/// live on a different desktop, which this process is not allowed to open. All three mean
/// the same thing here: the user is not looking at their work.
fn session_is_locked() -> bool {
    // SAFETY: the handle is closed on every path where it was obtained, and no pointers
    // are involved.
    unsafe {
        match OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, DESKTOP_SWITCHDESKTOP) {
            Ok(desktop) => {
                let _ = CloseDesktop(desktop);
                false
            }
            Err(_) => true,
        }
    }
}

/// The `Run` registry value that starts the app at sign-in.
#[derive(Clone, Debug)]
pub struct RunKey {
    exe: PathBuf,
}

impl RunKey {
    /// The registration for this installation.
    #[must_use]
    pub fn for_host() -> Self {
        Self {
            exe: executable_path(),
        }
    }
}

impl Autostart for RunKey {
    fn is_enabled(&self) -> bool {
        let subkey = HSTRING::from(RUN_KEY_PATH);
        let name = HSTRING::from(RUN_KEY_NAME);
        // SAFETY: every out-parameter is optional and passed as `None`; this only asks
        // whether a string value exists.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                &subkey,
                &name,
                RRF_RT_REG_SZ,
                None,
                None,
                None,
            )
        };
        status.is_ok()
    }

    fn set_enabled(&self, enabled: bool) -> HostResult {
        let subkey = HSTRING::from(RUN_KEY_PATH);
        let name = HSTRING::from(RUN_KEY_NAME);
        if !enabled {
            // SAFETY: both strings outlive the call.
            let status = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, &subkey, &name) };
            // Already absent is the state that was asked for.
            if status == ERROR_FILE_NOT_FOUND {
                return Ok(());
            }
            return status
                .ok()
                .map_err(|e| format!("removing the Run value: {e}"));
        }

        let command = run_key_command(&self.exe.to_string_lossy());
        let wide: Vec<u16> = command.encode_utf16().chain(std::iter::once(0)).collect();
        let bytes = wide.len() * size_of::<u16>();
        // SAFETY: `wide` is a NUL-terminated UTF-16 buffer and `bytes` is its exact byte
        // length; both outlive the call, as REG_SZ requires.
        let status = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                &subkey,
                &name,
                REG_SZ.0,
                Some(wide.as_ptr().cast()),
                u32::try_from(bytes).unwrap_or(0),
            )
        };
        status
            .ok()
            .map_err(|e| format!("writing the Run value: {e}"))
    }
}

/// Listens for second launches on a named pipe, calling `on_signal` for each one.
///
/// # Errors
///
/// If the listener thread cannot be spawned.
pub fn listen_for_signals(on_signal: impl Fn() + Send + 'static) -> io::Result<()> {
    thread::Builder::new()
        .name("eye-saver-instance".into())
        .spawn(move || {
            loop {
                // SAFETY: the handle is closed on every path, and the only pointer passed is
                // the static pipe name.
                let handle = unsafe {
                    CreateNamedPipeW(
                        w!(r"\\.\pipe\EyeSaver.instance"),
                        PIPE_ACCESS_INBOUND,
                        PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                        1,
                        0,
                        64,
                        0,
                        None,
                    )
                };
                if handle == INVALID_HANDLE_VALUE {
                    log::warn!(
                        "could not open the instance pipe; a second launch will not be noticed"
                    );
                    return;
                }
                if wait_for_client(handle) {
                    let mut buffer = [0u8; 64];
                    let mut read = 0u32;
                    // SAFETY: the buffer and the count outlive the call.
                    let _ =
                        unsafe { ReadFile(handle, Some(&mut buffer), Some(&raw mut read), None) };
                    on_signal();
                    // SAFETY: the handle came from CreateNamedPipeW and is still open.
                    let _ = unsafe { DisconnectNamedPipe(handle) };
                }
                // SAFETY: as above; the handle is not used again.
                let _ = unsafe { CloseHandle(handle) };
            }
        })?;
    Ok(())
}

/// Whether a client is now attached to `handle`.
fn wait_for_client(handle: HANDLE) -> bool {
    // SAFETY: the handle is a live named pipe server end.
    unsafe {
        if ConnectNamedPipe(handle, None).is_ok() {
            return true;
        }
        // A client that connected between creating and connecting is still a client.
        GetLastError() == ERROR_PIPE_CONNECTED
    }
}

/// Asks the running instance to show its settings window.
///
/// # Errors
///
/// If no instance is listening, which is also how a caller learns there is none.
pub fn signal_running_instance() -> io::Result<()> {
    use std::io::Write as _;
    let mut pipe = std::fs::OpenOptions::new().write(true).open(PIPE_PATH)?;
    pipe.write_all(b"show-settings\n")
}

/// `%APPDATA%`.
#[must_use]
pub fn data_base_dir() -> PathBuf {
    std::env::var_os("APPDATA").map_or_else(std::env::temp_dir, PathBuf::from)
}

/// This binary.
#[must_use]
pub fn executable_path() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("eye-saver.exe"))
}

/// The exact size Windows wants for a notification area icon, so the digits stay crisp
/// instead of being resampled.
#[must_use]
pub fn tray_icon_size() -> u32 {
    // SAFETY: a plain numeric call.
    let size = unsafe { GetSystemMetrics(SM_CXSMICON) };
    u32::try_from(size).unwrap_or(16).max(16)
}
