//! The flasher's UI, as a library: `main.rs` puts it in a window, and
//! [`headless::Headless`] drives it with no window, GPU or drive at all.

mod app;
pub mod headless;
pub mod prefs;
pub mod widgets;

pub use app::{theme, App, Settings, ThemeChoice, DEVICE_POLL};
