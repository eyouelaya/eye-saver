//! macOS adapters.
//!
//! Two questions are asked of the system, both through long-stable C entry points rather
//! than the Objective-C runtime: is the display asleep, and is the session locked. The
//! keyboard and mouse are not asked about at all. Neither is machine suspend - a suspended
//! process cannot poll, so the engine infers it from the wall clock having run ahead of
//! the monotonic clock.
//!
//! So that nothing has to keep asking, the system is also asked to say when either answer
//! changes. The display sleeping and waking is announced only through `NSWorkspace`, which
//! is the one place here that does need the Objective-C runtime; the lock is announced on
//! the distributed notification centre, which has a C interface.

use std::ffi::{CString, c_char, c_void};
use std::path::PathBuf;
use std::ptr::{self, NonNull};

use block2::RcBlock;
use objc2_app_kit::{
    NSWorkspace, NSWorkspaceScreensDidSleepNotification, NSWorkspaceScreensDidWakeNotification,
};
use objc2_foundation::NSNotification;

use eye_saver_core::ports::ScreenSource;
use eye_saver_core::screen::ScreenStatus;

/// Key in the session dictionary that is present and true while the screen is locked.
const SCREEN_IS_LOCKED: &str = "CGSSessionScreenIsLocked";
/// `kCFStringEncodingUTF8`.
const UTF8: u32 = 0x0800_0100;
/// Posted on the distributed notification centre as the screen locks and unlocks. Neither
/// name is in a header, but both have been stable since OS X 10.7.
const LOCK_NOTIFICATIONS: [&str; 2] = ["com.apple.screenIsLocked", "com.apple.screenIsUnlocked"];
/// `CFNotificationSuspensionBehaviorDeliverImmediately`: do not hold notifications back
/// while the app is in the background, which a menu bar app always is.
const DELIVER_IMMEDIATELY: isize = 4;

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
    fn CFNotificationCenterGetDistributedCenter() -> *const c_void;
    fn CFNotificationCenterAddObserver(
        center: *const c_void,
        observer: *const c_void,
        callback: extern "C" fn(
            *const c_void,
            *mut c_void,
            *const c_void,
            *const c_void,
            *const c_void,
        ),
        name: *const c_void,
        object: *const c_void,
        suspension_behavior: isize,
    );
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

/// Subscribes to the display sleeping and waking and to the screen locking and unlocking.
///
/// Must be called on the main thread: both centres deliver on the run loop of the thread
/// that registered, and the main run loop is the one the event loop keeps turning.
pub(super) fn subscribe_to_screen_changes() {
    let block = RcBlock::new(|_: NonNull<NSNotification>| super::screen_changed());
    let workspace = NSWorkspace::sharedWorkspace().notificationCenter();
    // SAFETY: both statics are notification names that AppKit defines for as long as it is
    // loaded.
    let names = unsafe {
        [
            NSWorkspaceScreensDidSleepNotification,
            NSWorkspaceScreensDidWakeNotification,
        ]
    };
    for name in names {
        // SAFETY: no sender filter and no queue, so the block runs synchronously on the
        // posting thread, which for this centre is the main thread; the block captures
        // nothing and what it calls is Send + Sync.
        let observer = unsafe {
            workspace.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block)
        };
        // Dropping the token would not unsubscribe, but keeping it is what the API asks
        // for, and this subscription is wanted until the process exits.
        std::mem::forget(observer);
    }

    for name in LOCK_NOTIFICATIONS {
        let Ok(name) = CString::new(name) else {
            continue;
        };
        // SAFETY: the centre is a process-wide singleton that is never released; the name
        // is created and released here, and the centre keeps its own copy; the callback is
        // a plain function that ignores its arguments; null observer and object mean "no
        // context" and "from anyone".
        unsafe {
            let name = CFStringCreateWithCString(ptr::null(), name.as_ptr(), UTF8);
            if name.is_null() {
                continue;
            }
            CFNotificationCenterAddObserver(
                CFNotificationCenterGetDistributedCenter(),
                ptr::null(),
                on_lock_notification,
                name,
                ptr::null(),
                DELIVER_IMMEDIATELY,
            );
            CFRelease(name);
        }
    }
}

/// Called by the distributed notification centre when the screen locks or unlocks.
extern "C" fn on_lock_notification(
    _center: *const c_void,
    _observer: *mut c_void,
    _name: *const c_void,
    _object: *const c_void,
    _user_info: *const c_void,
) {
    super::screen_changed();
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
