//! What the app remembers between runs: the Options tab and the last image
//! folder, in a small TOML file in the OS's usual place for settings.
//!
//! A missing, unreadable or half-written file means defaults, never a failed
//! start: settings are a convenience. Writes go to a temporary file renamed
//! over the old one, so a crash mid-save leaves the previous settings intact.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::app::Settings;

/// Everything remembered. Unknown keys are ignored and missing ones take
/// their defaults, so files from older and newer versions both load.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// The folder the last image came from, where Browse… starts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_image_dir: Option<PathBuf>,
    pub settings: Settings,
}

/// Where prefs live on disk.
#[derive(Clone, Debug)]
pub struct PrefsStore {
    path: PathBuf,
}

impl PrefsStore {
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The OS's usual place: `~/Library/Application Support/Flasher`
    /// on macOS, `%APPDATA%\Flasher` on Windows, and
    /// `$XDG_CONFIG_HOME/flasher` (or `~/.config/flasher`) elsewhere.
    pub fn default_location() -> Option<Self> {
        let env = |k: &str| {
            std::env::var_os(k)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        let dir = if cfg!(target_os = "macos") {
            env("HOME")?.join("Library/Application Support/Flasher")
        } else if cfg!(windows) {
            env("APPDATA")?.join("Flasher")
        } else {
            env("XDG_CONFIG_HOME")
                .or_else(|| env("HOME").map(|h| h.join(".config")))?
                .join("flasher")
        };
        Some(Self::at(dir.join("settings.toml")))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The saved prefs, or the defaults if there are none or they cannot be read.
    pub fn load(&self) -> Prefs {
        fs::read_to_string(&self.path)
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, prefs: &Prefs) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let text = toml::to_string_pretty(prefs).map_err(io::Error::other)?;
        let tmp = self.path.with_extension("toml.tmp");
        fs::write(
            &tmp,
            format!("# Flasher settings. Safe to delete: defaults come back.\n{text}"),
        )?;
        fs::rename(&tmp, &self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ThemeChoice;

    fn temp(name: &str) -> PrefsStore {
        let dir = std::env::temp_dir().join(format!("flasher_prefs_{}_{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        PrefsStore::at(dir.join("nested/settings.toml"))
    }

    #[test]
    fn round_trips() {
        let store = temp("roundtrip");
        let mut p = Prefs::default();
        p.settings.verify = false;
        p.settings.theme = ThemeChoice::Light;
        p.last_image_dir = Some(PathBuf::from("/Users/someone/Downloads"));
        store.save(&p).unwrap();
        assert_eq!(store.load(), p);
        fs::remove_dir_all(store.path().parent().unwrap().parent().unwrap()).ok();
    }

    #[test]
    fn bad_or_partial_files_fall_back_to_defaults() {
        let store = temp("bad");
        assert_eq!(store.load(), Prefs::default(), "no file");
        fs::create_dir_all(store.path().parent().unwrap()).unwrap();
        fs::write(store.path(), "this is [not toml").unwrap();
        assert_eq!(store.load(), Prefs::default(), "garbage");
        // A file from another version: unknown keys ignored, missing ones defaulted.
        fs::write(
            store.path(),
            "future_option = 3\n[settings]\neject = false\n",
        )
        .unwrap();
        let p = store.load();
        assert!(!p.settings.eject);
        assert!(p.settings.verify, "missing keys take their defaults");
        fs::remove_dir_all(store.path().parent().unwrap().parent().unwrap()).ok();
    }
}
