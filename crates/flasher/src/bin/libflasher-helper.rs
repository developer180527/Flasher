//! The small program Flasher starts through pkexec on Linux to open a disk
//! for writing, so the app itself never runs as root. Installed next to
//! `flasher` or in /usr/libexec. All of it is libflasher's; see
//! `libflasher_linux::helper`.

fn main() -> std::process::ExitCode {
    #[cfg(target_os = "linux")]
    return libflasher::linux_helper::main(std::env::args().skip(1));
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("libflasher-helper is only used on Linux");
        std::process::ExitCode::FAILURE
    }
}
