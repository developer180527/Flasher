//! macOS: Quit goes through the same close request as the window's close
//! button.
//!
//! Quit — ⌘Q, the Dock's Quit, logging out — reaches an app as
//! `-[NSApplication terminate:]`, which winit lets through without telling
//! the app: the process would end mid-write. AppKit first asks the
//! application delegate `applicationShouldTerminate:`. winit's delegate does
//! not answer that, so we add the method to its class: while a job runs it
//! says "cancel" and leaves a note for the event loop, which treats it as a
//! close request (see `App::request_close`). When nothing runs, quitting
//! goes ahead as before.

use std::sync::atomic::{AtomicBool, Ordering};

use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
use objc2::{ffi, sel};

/// winit's `NSApplicationDelegate` class (winit 0.30).
const DELEGATE_CLASS: &std::ffi::CStr = c"WinitApplicationDelegate";

/// `NSApplicationTerminateReply`.
const TERMINATE_CANCEL: usize = 0;
const TERMINATE_NOW: usize = 1;

/// Whether a job is running, as the event loop last saw it.
static BUSY: AtomicBool = AtomicBool::new(false);
/// Quit was asked for while busy, and not yet passed to the app.
static QUIT_ASKED: AtomicBool = AtomicBool::new(false);

extern "C-unwind" fn should_terminate(
    _this: &AnyObject,
    _cmd: Sel,
    _sender: *mut AnyObject,
) -> usize {
    if BUSY.load(Ordering::SeqCst) {
        QUIT_ASKED.store(true, Ordering::SeqCst);
        TERMINATE_CANCEL
    } else {
        TERMINATE_NOW
    }
}

/// Add `applicationShouldTerminate:` to winit's delegate. Call after the
/// event loop is created. False if winit's class is not there or already
/// answers it (a newer winit): Quit then works as winit makes it.
pub fn guard_quit() -> bool {
    let Some(class) = AnyClass::get(DELEGATE_CLASS) else {
        return false;
    };
    type ShouldTerminate = extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject) -> usize;
    // SAFETY: `should_terminate` has the selector's signature, which the
    // type encoding states: NSUInteger (`Q`), self, _cmd, sender. The
    // runtime calls it through the generic `Imp` type, as for any method.
    unsafe {
        let imp: Imp = std::mem::transmute::<ShouldTerminate, Imp>(should_terminate);
        ffi::class_addMethod(
            (class as *const AnyClass).cast_mut(),
            sel!(applicationShouldTerminate:),
            imp,
            c"Q@:@".as_ptr(),
        )
        .as_bool()
    }
}

/// Tell the quit guard whether a job is running.
pub fn set_busy(busy: bool) {
    BUSY.store(busy, Ordering::SeqCst);
}

/// Whether Quit was asked for while busy since the last call.
pub fn take_quit_request() -> bool {
    QUIT_ASKED.swap(false, Ordering::SeqCst)
}
