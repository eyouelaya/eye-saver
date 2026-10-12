//! macOS adapters.
//!
//! Two questions are asked of the system, both through long-stable C entry points rather
//! than the Objective-C runtime: is the display asleep, and is the session locked. The
//! keyboard and mouse are not asked about at all. Neither is machine suspend - a suspended
//! process cannot poll, so the engine infers it from the wall clock having run ahead of
//! the monotonic clock.

use std::ffi::{CString, c_char, c_void};
use std::path::PathBuf;
use std::ptr;

use eye_saver_core::ports::ScreenSource;
use eye_saver_core::screen::ScreenStatus;

/// Key in the session dictionary that is present and true while the screen is locked.
const SCREEN_IS_LOCKED: &str = "CGSSessionScreenIsLocked";
/// `kCFStringEncodingUTF8`.
const UTF8: u32 = 0x0800_0100;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGMainDisplayID() -> u32;
    fn CGDisplayIsAsleep(display: u32) -> i32;
    fn CGSessionCopyCurrentDictionary() -> *const c_void;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: *const c_void);
    fn CFDictionaryGetValue(dictionary: *const c_void, key: *const c_void) -> *const c_void;
    fn CFStringCreateWithCString(
        allocator: *const c_void,
        bytes: *const c_char,
        encoding: u32,
    ) -> *const c_void;
    fn CFBooleanGetValue(boolean: *const c_void) -> u8;
}

/// Reads the display and lock state.
#[derive(Clone, Copy, Debug, Default)]
pub struct Screen;

impl ScreenSource for Screen {
    fn screen_status(&self) -> ScreenStatus {
        ScreenStatus {
            display_asleep: display_is_asleep(),
            locked: session_is_locked(),
        }
    }
}

fn display_is_asleep() -> bool {
    // SAFETY: both calls take and return plain integers and are safe to make from any
    // thread; `CGMainDisplayID` always returns a valid display id, and an invalid one
    // would simply yield zero here.
    unsafe { CGDisplayIsAsleep(CGMainDisplayID()) != 0 }
}

/// Whether the login session is locked.
///
/// Returns false when there is no window session to ask - running from a terminal over
/// ssh, for instance - which is the right answer: nothing is hiding the screen.
fn session_is_locked() -> bool {
    let Ok(key_name) = CString::new(SCREEN_IS_LOCKED) else {
        return false;
    };

    // SAFETY: the dictionary is owned by us (a Copy function), the key string is created
    // and released in this scope, and `CFDictionaryGetValue` returns a borrowed value that
    // is only read before the dictionary is released. Every pointer is null-checked.
    unsafe {
        let session = CGSessionCopyCurrentDictionary();
        if session.is_null() {
            return false;
        }
        let key = CFStringCreateWithCString(ptr::null(), key_name.as_ptr(), UTF8);
        let locked = if key.is_null() {
            false
        } else {
            let value = CFDictionaryGetValue(session, key);
            let locked = !value.is_null() && CFBooleanGetValue(value) != 0;
            CFRelease(key);
            locked
        };
        CFRelease(session);
        locked
    }
}

/// `~/Library/Application Support`.
#[must_use]
pub fn data_base_dir() -> PathBuf {
    home().join("Library").join("Application Support")
}

/// `~/Library/LaunchAgents`, where a per-user login item lives.
#[must_use]
pub fn launch_agents_dir() -> PathBuf {
    home().join("Library").join("LaunchAgents")
}

/// This binary, inside its bundle.
#[must_use]
pub fn executable_path() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("eye-saver"))
}

/// Points, doubled for a Retina menu bar. macOS scales the image to the bar height.
#[must_use]
pub fn tray_icon_size() -> u32 {
    44
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map_or_else(std::env::temp_dir, PathBuf::from)
}
