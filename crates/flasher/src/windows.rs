//! Windows start-up: one binary that is both a window and a terminal tool,
//! and that asks for administrator rights only when it opens its window.
//!
//! The binary is built for the Windows GUI subsystem, so double-clicking it
//! opens no console window. Terminal commands attach to the console they
//! were started from, so their output still appears there.
//!
//! Writing a raw disk needs an elevated process. Rather than demand it in
//! the manifest (which would also elevate `flasher list` and send its
//! output to a new console window), the window relaunches itself through
//! the standard UAC prompt when it is not elevated. Declining keeps it
//! running unelevated: it can still list drives, and writing says why not.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::Security::{
    GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
};
use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW};

/// Passed to the relaunched copy, so it never asks again (and cannot loop
/// if elevation silently does not happen).
pub const RELAUNCHED: &str = "--elevated-relaunch";

/// Send terminal output to the console the command was typed in.
pub fn attach_console() {
    // SAFETY: plain Win32 call; failing (no parent console) is harmless.
    unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
}

pub fn is_elevated() -> bool {
    // SAFETY: the token handle is closed below; the out-parameters are
    // valid locals of the right size.
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            (&mut elevation as *mut TOKEN_ELEVATION).cast(),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        );
        CloseHandle(token);
        ok != 0 && elevation.TokenIsElevated != 0
    }
}

fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(Some(0)).collect()
}

/// Start an elevated copy of this program through the UAC prompt. True if
/// it started, and this copy should exit; false if the user declined or it
/// failed, and this copy should carry on.
pub fn relaunch_elevated() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let (verb, file, params) = (
        wide(OsStr::new("runas")),
        wide(exe.as_os_str()),
        wide(OsStr::new(RELAUNCHED)),
    );
    // SAFETY: zeroed is a valid SHELLEXECUTEINFOW; the strings outlive the call.
    unsafe {
        let mut info: SHELLEXECUTEINFOW = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_NOASYNC;
        info.lpVerb = verb.as_ptr();
        info.lpFile = file.as_ptr();
        info.lpParameters = params.as_ptr();
        info.nShow = 1; // SW_SHOWNORMAL
        ShellExecuteExW(&mut info) != 0
    }
}
