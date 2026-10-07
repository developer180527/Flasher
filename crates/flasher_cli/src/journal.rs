//! Flasher's log: what happened, in order, so a failure on someone else's
//! machine can be explained after the fact.
//!
//! One file in the OS's usual place for logs — `~/Library/Logs/Flasher` on
//! macOS, `%LOCALAPPDATA%\Flasher\Logs` on Windows, `$XDG_STATE_HOME/flasher`
//! (or `~/.local/state/flasher`) elsewhere — shared by the window and the
//! terminal commands. Past 1 MB it is renamed `flasher.1.log` and a new one
//! started, so at most about 2 MB is kept.
//!
//! Logging must never be why a flash fails: if the file cannot be opened or
//! written, lines are dropped silently. It records drive names, image paths
//! and errors; nothing leaves the computer unless the user copies it.

use std::fmt::Display;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const ROTATE_AT: u64 = 1 << 20;
const FILE: &str = "flasher.log";
const OLD: &str = "flasher.1.log";

struct Log {
    path: PathBuf,
    file: File,
}

static LOG: Mutex<Option<Log>> = Mutex::new(None);

/// Where the log goes on this OS, if a home or app-data folder is known.
/// `FLASHER_LOG_DIR` overrides it (tests use it to stay out of the real log).
pub fn default_dir() -> Option<PathBuf> {
    let env = |k: &str| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    if let Some(dir) = env("FLASHER_LOG_DIR") {
        return Some(dir);
    }
    if cfg!(target_os = "macos") {
        Some(env("HOME")?.join("Library/Logs/Flasher"))
    } else if cfg!(windows) {
        Some(env("LOCALAPPDATA")?.join("Flasher").join("Logs"))
    } else {
        Some(
            env("XDG_STATE_HOME")
                .or_else(|| env("HOME").map(|h| h.join(".local/state")))?
                .join("flasher"),
        )
    }
}

/// Start logging into `dir/flasher.log`, rotating first if it is large, and
/// record what is running. Calling it again switches to the new folder.
pub fn open(dir: &Path) {
    let _ = fs::create_dir_all(dir);
    let path = dir.join(FILE);
    if fs::metadata(&path).is_ok_and(|m| m.len() > ROTATE_AT) {
        let _ = fs::rename(&path, dir.join(OLD));
    }
    let Ok(file) = OpenOptions::new().create(true).append(true).open(&path) else {
        return;
    };
    if let Ok(mut log) = LOG.lock() {
        *log = Some(Log { path, file });
    }
    line(format_args!(
        "---- Flasher {} on {} {} ----",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    ));
}

/// Open the log in [`default_dir`], if there is one.
pub fn open_default() {
    if let Some(dir) = default_dir() {
        open(&dir);
    }
}

/// Append one line, timestamped (UTC). Does nothing until [`open`].
pub fn line(msg: impl Display) {
    let Ok(mut log) = LOG.lock() else { return };
    if let Some(l) = log.as_mut() {
        // One write per line, so lines from different threads never interleave.
        let text = format!("{} {msg}\n", timestamp(SystemTime::now()));
        let _ = l.file.write_all(text.as_bytes());
    }
}

/// The log file now in use.
pub fn path() -> Option<PathBuf> {
    LOG.lock().ok()?.as_ref().map(|l| l.path.clone())
}

/// The last `n` lines of the log (fewer if it is shorter).
pub fn tail(n: usize) -> Vec<String> {
    let Some(path) = path() else {
        return Vec::new();
    };
    let Ok(mut f) = File::open(&path) else {
        return Vec::new();
    };
    // The last 64 KiB is plenty for any `n` asked for here.
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let _ = f.seek(SeekFrom::Start(len.saturating_sub(64 << 10)));
    let mut text = String::new();
    let _ = f.read_to_string(&mut text);
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// A report to paste into a bug: what failed, the facts given, and the log
/// leading up to it.
pub fn report(what: &str, facts: &[(&str, String)]) -> String {
    let mut out = format!(
        "Flasher {} on {} {}\n{what}\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    for (k, v) in facts {
        out += &format!("{k}: {v}\n");
    }
    let recent = tail(40);
    if !recent.is_empty() {
        out += "\nRecent log:\n";
        for l in recent {
            out += &l;
            out.push('\n');
        }
    }
    if let Some(p) = path() {
        out += &format!("\nFull log: {}\n", p.display());
    }
    out
}

/// `2026-10-07T09:42:20Z`, from the clock, with no date library.
fn timestamp(t: SystemTime) -> String {
    let secs = t
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::time::Duration;

    /// The log is global: tests that use it take this lock.
    pub(crate) static SERIAL: Mutex<()> = Mutex::new(());

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("flasher_journal_{}_{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn timestamps_are_utc_dates() {
        assert_eq!(timestamp(UNIX_EPOCH), "1970-01-01T00:00:00Z");
        assert_eq!(
            timestamp(UNIX_EPOCH + Duration::from_secs(951_782_400)),
            "2000-02-29T00:00:00Z"
        );
        assert_eq!(
            timestamp(UNIX_EPOCH + Duration::from_secs(1_791_365_540)),
            "2026-10-07T09:32:20Z"
        );
    }

    #[test]
    fn logs_lines_reports_and_rotates() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let dir = temp("rotate");
        open(&dir);
        line("drive /dev/disk4 SanDisk Ultra");
        line(format_args!("error: {}", "the drive was disconnected"));
        let text = fs::read_to_string(dir.join(FILE)).unwrap();
        assert!(text.contains("---- Flasher"), "{text}");
        assert!(text
            .lines()
            .last()
            .unwrap()
            .ends_with("error: the drive was disconnected"));
        assert_eq!(tail(1).len(), 1);

        let r = report(
            "Flash failed: the drive was disconnected",
            &[("Drive", "/dev/disk4".into())],
        );
        assert!(
            r.contains("Drive: /dev/disk4")
                && r.contains("Recent log:")
                && r.contains("SanDisk Ultra"),
            "{r}"
        );

        // Over the limit: the next open starts a fresh file and keeps one old.
        fs::write(dir.join(FILE), vec![b'x'; ROTATE_AT as usize + 1]).unwrap();
        open(&dir);
        assert!(dir.join(OLD).exists());
        assert!(fs::metadata(dir.join(FILE)).unwrap().len() < 1000);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_log_that_cannot_open_is_silent() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        // A file where the folder should be: opening fails, logging is a no-op.
        let blocker = temp("blocked");
        fs::write(&blocker, b"not a folder").unwrap();
        if let Ok(mut l) = LOG.lock() {
            *l = None;
        }
        open(&blocker.join("sub"));
        line("goes nowhere");
        assert_eq!(path(), None);
        let _ = fs::remove_file(blocker);
    }
}
