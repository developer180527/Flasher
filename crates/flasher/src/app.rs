//! The flasher's state and UI. Knows nothing about windows or GPUs; the host
//! in `main.rs` drives it once per frame.
//!
//! Three tabs — Flash, Restore drive, Options — over one shared area for what
//! is happening now (confirm, progress, result), which stays in view whichever
//! tab is open.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use libflasher::platform::{human_size, volume_label};
use libflasher::rate::{human_duration, StatusLine};
use libflasher::{
    checksum, image, Compression, DeviceInfo, FlashOptions, ImageInfo, ImageKind, Platform,
    Progress,
};
use libgui::*;

use serde::{Deserialize, Serialize};

use crate::prefs::{Prefs, PrefsStore};
use crate::widgets::{group_box, label_middle_ellipsis, tab_bar, tab_page};

const TABS: [&str; 3] = ["Flash", "Restore drive", "Options"];
const TAB_FLASH: usize = 0;
const TAB_RESTORE: usize = 1;
const TAB_OPTIONS: usize = 2;
/// Names of the tabs for `controls`, in order.
const TAB_CONTROLS: [&str; 3] = ["tab:flash", "tab:restore", "tab:options"];

/// How often the drive list is re-read where the platform cannot notify us
/// of drives coming and going.
pub const DEVICE_POLL: Duration = Duration::from_secs(2);

/// Where it can, the list is re-read when notified, and also this often in
/// case a notification is ever missed.
const SAFETY_POLL: Duration = Duration::from_secs(30);

/// One plug-in raises several notifications (the disk, then each partition);
/// wait this long for the burst to end and list once.
const SETTLE: Duration = Duration::from_millis(150);

/// Silence after which the UI says the drive may have stopped responding.
const QUIET_WARNING: Duration = Duration::from_secs(10);

/// The choices on the Options tab.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub verify: bool,
    pub eject: bool,
    pub keep_awake: bool,
    /// Compare the image with its published SHA-256 before writing, when one
    /// was given or found.
    pub check_sha256: bool,
    pub auto_refresh: bool,
    pub theme: ThemeChoice,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            verify: true,
            eject: true,
            keep_awake: true,
            check_sha256: true,
            auto_refresh: true,
            theme: ThemeChoice::System,
        }
    }
}

/// Light, dark, or whatever the OS is set to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeChoice {
    const ALL: [ThemeChoice; 3] = [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark];
    const LABELS: [&'static str; 3] = ["System", "Light", "Dark"];

    fn is_dark(self, system_dark: bool) -> bool {
        match self {
            ThemeChoice::System => system_dark,
            ThemeChoice::Light => false,
            ThemeChoice::Dark => true,
        }
    }
}

/// The app's two themes: libgui's presets plus the few colours in
/// `themes/*.toml`, which hold only what differs.
pub fn theme(dark: bool) -> Result<Theme, String> {
    let src = if dark {
        include_str!("../themes/dark.toml")
    } else {
        include_str!("../themes/light.toml")
    };
    Theme::from_toml(src).map_err(|e| e.to_string())
}

/// Green that reads well on each theme's background (libgui's palette has no
/// "success" colour of its own).
fn success_color(dark: bool) -> Color {
    if dark {
        Color::rgba(0.29, 0.87, 0.50, 1.0) // #4ade80
    } else {
        Color::rgba(0.08, 0.50, 0.24, 1.0) // #15803d
    }
}

pub struct App {
    platform: Arc<dyn Platform>,
    devices: Vec<DeviceInfo>,
    device_names: Vec<String>,
    selected: usize,
    list_error: Option<String>,
    /// New drive lists from the polling thread; `None` when auto-refresh is off.
    watcher: Option<Receiver<Result<Vec<DeviceInfo>, String>>>,
    poll_every: Duration,

    tab: usize,
    image_path: String,
    image: Option<Result<ImageInfo, String>>,
    /// For an ISO that is extracted rather than written byte for byte: what
    /// that will take, or why it cannot be done.
    extract_plan: Option<Result<libflasher::extract::Plan, String>>,
    sha256: String,
    /// The file the checksum was read from, when it was found rather than typed.
    sha256_source: Option<String>,
    /// An image being read in the background (`inspect`, the extract plan,
    /// a checksum file beside it): a Windows ISO's file list can take
    /// seconds to read, and the window must not freeze meanwhile. A newer
    /// choice replaces the receiver, so a stale answer is never applied.
    loading: Option<Receiver<Loaded>>,
    /// A drive listing running in the background (Refresh, after a job).
    listing: Option<Receiver<Result<Vec<DeviceInfo>, String>>>,
    label: String,
    pub settings: Settings,
    stage: Stage,
    /// The OS's light/dark appearance, as last reported by the host.
    system_dark: bool,
    /// Whether the theme now on the `Ui` is the dark one; `None` before the
    /// first frame.
    applied_dark: Option<bool>,

    /// Set by the Browse button; the host shows the native file dialog
    /// between frames, since libgui has no file dialogs by design.
    pub want_browse: bool,
    /// Where Browse… starts: the folder of the last image chosen.
    last_image_dir: Option<PathBuf>,
    /// Where settings are remembered, and what was last written there.
    store: Option<(PrefsStore, Prefs)>,
    /// Where each named control was drawn last frame, so a headless driver can
    /// click it exactly as a user would. See `headless.rs`.
    pub controls: Vec<(&'static str, Rect)>,
    /// The window was asked to close while a job ran: it closes when the
    /// job ends (see [`App::request_close`]).
    closing: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Task {
    Flash,
    Restore,
}

enum Stage {
    Idle,
    /// A button was pressed whose action erases a drive; waiting for "yes".
    Confirm(Task),
    Running(Job),
    /// How the job ended, and something to know even though it worked.
    Finished(Result<String, String>, Option<String>),
}

struct Job {
    task: Task,
    rx: Receiver<Msg>,
    cancel: Arc<AtomicBool>,
    /// What to show: phase, amounts, speed, time left — shared with the CLI.
    status: StatusLine,
    /// Whether any progress has arrived. Before it, the worker is waiting on
    /// the password prompt and unmounting, which report nothing.
    heard: bool,
    started: Instant,
    /// When the worker last reported anything. A write blocked in the kernel
    /// reports nothing, and this is how the UI notices.
    last_heard: Instant,
}

/// What reading an image in the background found.
struct Loaded {
    image: Result<ImageInfo, String>,
    plan: Option<Result<libflasher::extract::Plan, String>>,
    found: Option<checksum::Published>,
}

enum Msg {
    Progress(Progress),
    /// The outcome, and a warning to show alongside a success.
    Done(Result<String, String>, Option<String>),
}

impl App {
    pub fn new(platform: Arc<dyn Platform>) -> Self {
        Self::with_poll(platform, DEVICE_POLL)
    }

    /// [`App::new`] with a different drive-list polling interval (tests use a
    /// short one).
    pub fn with_poll(platform: Arc<dyn Platform>, poll_every: Duration) -> Self {
        let mut app = Self {
            platform,
            devices: Vec::new(),
            device_names: Vec::new(),
            selected: 0,
            list_error: None,
            watcher: None,
            poll_every,
            tab: TAB_FLASH,
            image_path: String::new(),
            image: None,
            extract_plan: None,
            sha256: String::new(),
            sha256_source: None,
            loading: None,
            listing: None,
            label: "USB DRIVE".into(),
            settings: Settings::default(),
            system_dark: true,
            applied_dark: None,
            stage: Stage::Idle,
            want_browse: false,
            last_image_dir: None,
            store: None,
            controls: Vec::new(),
            closing: false,
        };
        app.refresh();
        app.set_auto_refresh(true);
        app
    }

    /// [`App::new`], remembering settings in `store` (loaded now, saved
    /// whenever they change).
    pub fn with_store(platform: Arc<dyn Platform>, store: PrefsStore) -> Self {
        let mut app = Self::new(platform);
        let prefs = store.load();
        app.settings = prefs.settings;
        app.last_image_dir = prefs.last_image_dir.clone();
        app.set_auto_refresh(prefs.settings.auto_refresh);
        app.store = Some((store, prefs));
        app
    }

    /// A job is running: the host keeps frames coming to show it.
    pub fn busy(&self) -> bool {
        matches!(self.stage, Stage::Running(_))
    }

    /// Work is running in the background that will change what is shown:
    /// an image being read, or the drive list. The host keeps waking for it.
    pub fn pending(&self) -> bool {
        self.loading.is_some() || self.listing.is_some()
    }

    /// What the last job ended with, once it has ended.
    pub fn outcome(&self) -> Option<&Result<String, String>> {
        match &self.stage {
            Stage::Finished(r, _) => Some(r),
            _ => None,
        }
    }

    /// A warning that came with the last job's outcome: the drive was
    /// written, but could not be ejected.
    pub fn outcome_warning(&self) -> Option<&str> {
        match &self.stage {
            Stage::Finished(_, w) => w.as_deref(),
            _ => None,
        }
    }

    /// The host calls this when the user closes the window. Returns whether
    /// it may close now.
    ///
    /// Closing mid-job would kill the worker part-way through a write and
    /// leave the drive half-written, so the first request cancels a flash
    /// (a restore is one OS command and runs to its end) and the window
    /// closes once the job has stopped: see [`App::should_close`]. A second
    /// request closes at once — for a drive that has stopped responding,
    /// whose job may never end.
    pub fn request_close(&mut self) -> bool {
        let Stage::Running(job) = &self.stage else {
            return true;
        };
        if self.closing {
            return true;
        }
        self.closing = true;
        job.cancel.store(true, Ordering::Relaxed);
        false
    }

    /// A close was put off until the job ended, and it has.
    pub fn should_close(&self) -> bool {
        self.closing && !self.busy()
    }

    pub fn devices(&self) -> &[DeviceInfo] {
        &self.devices
    }

    /// The size of the image chosen, once it has been read (for tests).
    pub fn image_size(&self) -> Option<u64> {
        match &self.image {
            Some(Ok(i)) => i.disk_size,
            _ => None,
        }
    }

    /// The checksum the next flash will be checked against, and where it came from.
    pub fn sha256(&self) -> (&str, Option<&str>) {
        (&self.sha256, self.sha256_source.as_deref())
    }

    pub fn set_image(&mut self, path: &Path) {
        if self.busy() {
            return;
        }
        self.image_path = path.display().to_string();
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            self.last_image_dir = Some(dir.to_path_buf());
        }
        self.load_image();
    }

    pub fn set_sha256(&mut self, text: &str) {
        self.sha256 = text.to_string();
        self.sha256_source = None;
    }

    /// Whether a running job has reported progress yet (for tests that want
    /// to look at the app mid-write).
    pub fn progress_started(&self) -> bool {
        matches!(&self.stage, Stage::Running(j) if j.heard)
    }

    /// The host reports the OS appearance here, at start and when it changes.
    pub fn set_system_dark(&mut self, dark: bool) {
        self.system_dark = dark;
    }

    /// Whether the dark theme is the one in use.
    pub fn dark(&self) -> bool {
        self.settings.theme.is_dark(self.system_dark)
    }

    pub fn set_label(&mut self, text: &str) {
        self.label = text.to_string();
    }

    fn load_image(&mut self) {
        let path = self.image_path.trim().to_string();
        self.image = None;
        self.extract_plan = None;
        self.stage = Stage::Idle;
        if path.is_empty() {
            self.loading = None;
            self.apply_found(None);
            return;
        }
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let image = image::inspect(&path).map_err(|e| e.to_string());
            // Read an ISO's file list now, so a reason it cannot be
            // extracted shows before anyone is asked to erase a drive.
            let plan = match &image {
                Ok(i) if i.kind.needs_extract() => {
                    Some(libflasher::extract::plan(i).map_err(|e| e.to_string()))
                }
                _ => None,
            };
            let found = checksum::find_published(Path::new(&path));
            let _ = tx.send(Loaded { image, plan, found });
        });
        self.loading = Some(rx);
    }

    /// A checksum found beside the image. One found for the previous image
    /// says nothing about this one; one the user typed stays theirs to change.
    fn apply_found(&mut self, found: Option<checksum::Published>) {
        if self.sha256_source.is_some() || self.sha256.is_empty() {
            self.sha256.clear();
            self.sha256_source = None;
            if let Some(found) = found {
                self.sha256 = found.sha256;
                self.sha256_source = Some(found.source);
            }
        }
    }

    // ---- drives ----------------------------------------------------------

    /// List the drives again, off the UI thread: on macOS a listing runs
    /// `diskutil` once per disk. [`App::tick`] applies the result.
    fn refresh(&mut self) {
        let (tx, rx) = mpsc::channel();
        let platform = self.platform.clone();
        thread::spawn(move || {
            let _ = tx.send(platform.list_devices().map_err(|e| e.to_string()));
        });
        self.listing = Some(rx);
    }

    fn apply_devices(&mut self, listed: Result<Vec<DeviceInfo>, String>) {
        let previous = self.devices.get(self.selected).cloned();
        match listed {
            Ok(d) => {
                self.devices = d;
                self.list_error = None;
            }
            Err(e) => {
                self.devices.clear();
                self.list_error = Some(e);
            }
        }
        self.device_names = self.devices.iter().map(DeviceInfo::display_name).collect();
        let kept = previous
            .as_ref()
            .and_then(|p| self.devices.iter().position(|d| d.same_disk(p)));
        self.selected = kept.unwrap_or(0);
        // A confirmation names one drive. If that drive is gone or replaced,
        // the question no longer means what the user read: ask again.
        if previous.is_some() && kept.is_none() && matches!(self.stage, Stage::Confirm(_)) {
            self.stage = Stage::Idle;
        }
    }

    fn set_auto_refresh(&mut self, on: bool) {
        self.settings.auto_refresh = on;
        if !on {
            self.watcher = None; // the thread exits on its next send
            return;
        }
        if self.watcher.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let platform = self.platform.clone();
        // Native notifications (DiskArbitration, uevents, Configuration
        // Manager) where the platform has them; polling where it does not.
        let (ping, pinged) = mpsc::channel::<()>();
        let watch = platform.watch(Arc::new(move || {
            let _ = ping.send(());
        }));
        let every = if watch.is_some() {
            SAFETY_POLL
        } else {
            self.poll_every
        };
        thread::spawn(move || {
            // Owned here, so notifications stop when this thread does.
            let _watch = watch;
            let mut last: Option<Result<Vec<DeviceInfo>, String>> = None;
            loop {
                if pinged.recv_timeout(every).is_ok() {
                    thread::sleep(SETTLE);
                    while pinged.try_recv().is_ok() {}
                }
                let now = platform.list_devices().map_err(|e| e.to_string());
                if last.as_ref() != Some(&now) {
                    if tx.send(now.clone()).is_err() {
                        return;
                    }
                    last = Some(now);
                }
            }
        });
        self.watcher = Some(rx);
    }

    /// Take in whatever arrived from the background: new drive lists and job
    /// progress. Returns true if anything changed and the window should redraw.
    /// The host calls this while idle, so a drive plugged in shows up without
    /// the user touching anything.
    pub fn tick(&mut self) -> bool {
        let mut changed = false;
        let mut latest = None;
        if let Some(rx) = &self.watcher {
            while let Ok(list) = rx.try_recv() {
                latest = Some(list);
            }
        }
        if let Some(list) = latest {
            self.apply_devices(list);
            changed = true;
        }
        if let Some(rx) = &self.listing {
            match rx.try_recv() {
                Ok(list) => {
                    self.listing = None;
                    self.apply_devices(list);
                    changed = true;
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.listing = None,
            }
        }
        if let Some(rx) = &self.loading {
            match rx.try_recv() {
                Ok(loaded) => {
                    self.loading = None;
                    self.image = Some(loaded.image);
                    self.extract_plan = loaded.plan;
                    self.apply_found(loaded.found);
                    changed = true;
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.loading = None;
                    self.image = Some(Err("reading the image stopped unexpectedly".into()));
                    changed = true;
                }
            }
        }
        changed | self.poll_job()
    }

    // ---- jobs ------------------------------------------------------------

    fn ready_image(&self) -> Option<&ImageInfo> {
        match &self.image {
            Some(Ok(i)) if i.kind.raw_writable() => Some(i),
            Some(Ok(i)) if matches!(self.extract_plan, Some(Ok(_))) => Some(i),
            _ => None,
        }
    }

    /// The checksum to check against: `Ok(None)` for none, `Err` for one that
    /// cannot be a SHA-256 (which blocks flashing rather than being ignored).
    fn expected_sha256(&self) -> Result<Option<String>, String> {
        if !self.settings.check_sha256 || self.sha256.trim().is_empty() {
            return Ok(None);
        }
        checksum::normalize(&self.sha256)
            .map(Some)
            .ok_or_else(|| "That is not a SHA-256: it should be 64 hexadecimal digits.".into())
    }

    fn start(&mut self, task: Task) {
        let Some(device) = self.devices.get(self.selected).cloned() else {
            return;
        };
        let platform = self.platform.clone();
        let settings = self.settings;
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let stop = cancel.clone();

        match task {
            Task::Flash => {
                let (Some(image), Ok(expected)) =
                    (self.ready_image().cloned(), self.expected_sha256())
                else {
                    return;
                };
                let found_in = self.sha256_source.clone();
                thread::spawn(move || {
                    let mut send = |p| {
                        let _ = tx.send(Msg::Progress(p));
                    };
                    let result = (|| {
                        let _awake = settings
                            .keep_awake
                            .then(|| platform.keep_awake("Writing a disk image"))
                            .flatten();
                        // Before the password prompt: a bad download should
                        // not cost the user a password, let alone a drive.
                        if let Some(h) = &expected {
                            checksum::verify_image(&image, h, &stop, &mut send)?;
                        }
                        // May show the OS password prompt; fine off the UI thread.
                        let mut raw = platform.open_listed(&device)?;
                        let options = FlashOptions::default().with_verify(settings.verify);
                        let n = libflasher::write_image(
                            &image,
                            raw.as_mut(),
                            &options,
                            &stop,
                            &mut send,
                        )?;
                        drop(raw);
                        // The image is on the drive by now: failing to eject
                        // (macOS reporting a just-mounted volume busy, say)
                        // is a warning, not a failed flash.
                        let ejected = settings.eject.then(|| platform.eject(&device));
                        let mut done = format!("Done: {} written", human_size(n));
                        if settings.verify {
                            done += " and verified";
                        }
                        if expected.is_some() {
                            done += "; ";
                            done += &flasher_cli::checksum_matched(found_in.as_deref());
                        }
                        done += if matches!(ejected, Some(Ok(()))) {
                            ". It is safe to remove the drive."
                        } else {
                            "."
                        };
                        let warning = match ejected {
                            Some(Err(e)) => Some(format!(
                                "The drive could not be ejected ({e}). Eject it from the system before unplugging it."
                            )),
                            _ => None,
                        };
                        Ok::<_, libflasher::Error>((done, warning))
                    })();
                    let (result, warning) = match result {
                        Ok((done, warning)) => (Ok(done), warning),
                        Err(e) => (Err(e.to_string()), None),
                    };
                    let _ = tx.send(Msg::Done(result, warning));
                });
            }
            Task::Restore => {
                let label = self.label.clone();
                thread::spawn(move || {
                    let result = (|| {
                        let _awake = settings
                            .keep_awake
                            .then(|| platform.keep_awake("Erasing a drive"))
                            .flatten();
                        platform.restore_listed(&device, &label)?;
                        Ok::<_, libflasher::Error>(format!(
                            "Done: the drive is an ordinary storage drive again, named \"{}\".",
                            label.trim()
                        ))
                    })();
                    let _ = tx.send(Msg::Done(result.map_err(|e| e.to_string()), None));
                });
            }
        }

        self.stage = Stage::Running(Job {
            task,
            rx,
            cancel,
            status: StatusLine::new(),
            heard: false,
            started: Instant::now(),
            last_heard: Instant::now(),
        });
    }

    /// Drain the worker's messages. Returns true if any arrived.
    fn poll_job(&mut self) -> bool {
        let Stage::Running(job) = &mut self.stage else {
            return false;
        };
        let mut any = false;
        loop {
            match job.rx.try_recv() {
                Ok(Msg::Progress(p)) => {
                    any = true;
                    job.last_heard = Instant::now();
                    job.heard = true;
                    job.status.update(p);
                }
                Ok(Msg::Done(r, warning)) => {
                    self.stage = Stage::Finished(r, warning);
                    self.refresh();
                    return true;
                }
                Err(TryRecvError::Empty) => return any,
                Err(TryRecvError::Disconnected) => {
                    self.stage =
                        Stage::Finished(Err("the worker thread stopped unexpectedly".into()), None);
                    return true;
                }
            }
        }
    }

    // ---- UI --------------------------------------------------------------

    pub fn ui(&mut self, ui: &mut Ui) {
        self.controls.clear();
        self.tick();
        if self.busy() {
            ui.request_repaint();
        }
        let dark = self.dark();
        if self.applied_dark != Some(dark) {
            match theme(dark) {
                Ok(t) => ui.theme = t,
                // Unreachable unless a theme file is broken, which a test
                // catches; keep whatever theme is showing.
                Err(e) => self.list_error = Some(format!("theme: {e}")),
            }
            self.applied_dark = Some(dark);
        }
        let layout = Layout::column()
            .width(Size::Grow(1.0))
            .height(Size::Grow(1.0))
            .padding(Insets::all(16.0))
            .gap(12.0);
        let frame = Frame {
            fill: ui.theme.palette.bg_app,
            ..Frame::none()
        };
        ui.container_id(Id::new("root"), layout, frame, |ui| {
            // No heading: the window's title bar already names the app, and
            // the tabs get the room.
            // The tabs stay usable during a job, to look around; what they
            // contain does not, so nothing changes under a running write.
            ui.container_id(
                Id::new("tabs"),
                Layout::column()
                    .width(Size::Grow(1.0))
                    .height(Size::Grow(1.0)),
                Frame::none(),
                |ui| {
                    let tabs = tab_bar(ui, "main", &mut self.tab, &TABS);
                    for (name, r) in TAB_CONTROLS.iter().zip(&tabs) {
                        self.controls.push((name, r.rect));
                    }
                    let busy = self.busy();
                    tab_page(ui, "main", self.tab, &TABS, |ui| {
                        ui.enabled(!busy, |ui| match self.tab {
                            TAB_FLASH => self.flash_tab(ui),
                            TAB_RESTORE => self.restore_tab(ui),
                            TAB_OPTIONS => self.options_tab(ui),
                            _ => {}
                        })
                    });
                },
            );
            self.actions(ui);
        });
        self.save_prefs();
    }

    /// Where Browse… should open.
    pub fn last_image_dir(&self) -> Option<&Path> {
        self.last_image_dir.as_deref()
    }

    /// Write settings if they changed since last written. Cheap when they
    /// have not: one comparison per frame.
    fn save_prefs(&mut self) {
        let now = Prefs {
            settings: self.settings,
            last_image_dir: self.last_image_dir.clone(),
        };
        if let Some((store, saved)) = &mut self.store {
            if *saved != now {
                // Remember what we tried even if it failed, so a read-only
                // disk is not retried sixty times a second.
                let _ = store.save(&now);
                *saved = now;
            }
        }
    }

    fn drive_picker(&mut self, ui: &mut Ui) {
        ui.section("Drive");
        ui.row(|ui| {
            if self.devices.is_empty() {
                ui.label_muted("No removable drives found. Plug one in.");
            } else {
                let names: Vec<&str> = self.device_names.iter().map(String::as_str).collect();
                let r = ui.combo("drive", &mut self.selected, &names);
                self.controls.push(("drive", r.rect));
            }
            let r = ui.button("Refresh");
            self.controls.push(("refresh", r.rect));
            if r.clicked {
                self.refresh();
            }
        });
        if let Some(e) = &self.list_error {
            error(ui, e);
        } else if let Some(d) = self.devices.get(self.selected) {
            let mut line = d.path.clone();
            if !d.mountpoints.is_empty() {
                line += &format!(
                    ", mounted at {} (will be unmounted)",
                    d.mountpoints.join(", ")
                );
            }
            let muted = ui.theme.palette.text_muted;
            label_middle_ellipsis(ui, "drive_path", &line, muted);
        }
    }

    fn flash_tab(&mut self, ui: &mut Ui) {
        self.drive_picker(ui);

        ui.section("Image");
        ui.row(|ui| {
            let r = ui.text_input(
                "image",
                &mut self.image_path,
                "Path to .img, .iso, .img.xz, …",
            );
            self.controls.push(("image", r.response.rect));
            if r.submitted {
                self.load_image();
            }
            let r = ui.button("Browse…");
            self.controls.push(("browse", r.rect));
            if r.clicked {
                self.want_browse = true;
            }
        });
        match &self.image {
            None if self.loading.is_some() => ui.label_muted("Reading the image…"),
            None => ui.label_muted("Choose a file, or drop one on the window."),
            Some(Err(e)) => error(ui, e),
            Some(Ok(i)) => {
                let size = match i.disk_size {
                    Some(s) => human_size(s),
                    None => format!(
                        "{} file, size unknown until written",
                        human_size(i.file_size)
                    ),
                };
                let line = match i.compression {
                    Compression::None => format!("{} · {size}", i.kind.describe()),
                    c => format!("{} · {size} · {} compressed", i.kind.describe(), c.name()),
                };
                match &self.extract_plan {
                    None if i.kind.raw_writable() => ui.label_muted(&line),
                    None => error(ui, &line),
                    Some(Ok(p)) => {
                        let split = p
                            .wim_parts
                            .map(|n| format!(" · install.wim split into {n} parts for FAT32"))
                            .unwrap_or_default();
                        ui.label_muted(&format!(
                            "{line} · {} files, {} · volume \"{}\"{split}",
                            p.files,
                            human_size(p.bytes),
                            p.label
                        ))
                    }
                    Some(Err(e)) => {
                        ui.label_muted(&line);
                        error(ui, &format!("Cannot extract this ISO: {e}."));
                    }
                }
            }
        }

        ui.section("SHA-256 checksum");
        let r = ui.text_input(
            "sha256",
            &mut self.sha256,
            "Paste the checksum from the download page (optional)",
        );
        self.controls.push(("sha256", r.response.rect));
        if r.changed {
            self.sha256_source = None;
        }
        match (self.expected_sha256(), &self.sha256_source) {
            (Err(e), _) => error(ui, &e),
            (Ok(Some(_)), Some(src)) => ui.label_muted(&format!(
                "Found in {src} next to the image and checked before writing. That shows the \
                 download is intact; to know it is genuine, use the checksum from the \
                 publisher's website instead."
            )),
            (Ok(Some(_)), None) => {
                ui.label_muted("The image is checked against this before anything is written.")
            }
            (Ok(None), _) if !self.settings.check_sha256 => {
                ui.label_muted("Checking is turned off on the Options tab.")
            }
            (Ok(None), _) => {
                ui.label_muted("Without one, a damaged or tampered download cannot be detected.")
            }
        }
    }

    fn restore_tab(&mut self, ui: &mut Ui) {
        ui.paragraph(
            "Turns a drive that was flashed with an image back into an ordinary storage drive: \
             one exFAT volume that macOS, Windows and Linux can all read and write.",
        );
        self.drive_picker(ui);
        ui.section("Name");
        let r = ui.text_input("label", &mut self.label, "USB DRIVE");
        self.controls.push(("label", r.response.rect));
        match volume_label(&self.label) {
            Ok(_) => ui.label_muted("Up to 11 letters, digits, spaces, - or _."),
            Err(e) => error(ui, &format!("{}{}", e[..1].to_uppercase(), &e[1..])),
        }
    }

    fn options_tab(&mut self, ui: &mut Ui) {
        group_box(ui, "writing", "Writing", |ui| {
            let r = ui.checkbox("Verify after writing", &mut self.settings.verify);
            self.controls.push(("verify", r.rect));
            let r = ui.checkbox(
                "Check the image's SHA-256 before writing, when one is known",
                &mut self.settings.check_sha256,
            );
            self.controls.push(("check_sha256", r.rect));
            let r = ui.checkbox("Eject the drive when done", &mut self.settings.eject);
            self.controls.push(("eject", r.rect));
        });
        group_box(ui, "advanced", "Advanced settings", |ui| {
            let r = ui.checkbox(
                "Keep the computer awake while working",
                &mut self.settings.keep_awake,
            );
            self.controls.push(("keep_awake", r.rect));
            let mut auto = self.settings.auto_refresh;
            let r = ui.checkbox("Update the drive list automatically", &mut auto);
            self.controls.push(("auto_refresh", r.rect));
            if auto != self.settings.auto_refresh {
                self.set_auto_refresh(auto);
            }
        });
        group_box(ui, "appearance", "Appearance", |ui| {
            let mut i = ThemeChoice::ALL
                .iter()
                .position(|t| *t == self.settings.theme)
                .unwrap_or(0);
            let r = ui.segmented("theme", &mut i, &ThemeChoice::LABELS);
            self.controls.push(("theme", r.rect));
            self.settings.theme = ThemeChoice::ALL[i];
            ui.label_muted("System follows your computer's light or dark setting.");
        });
        ui.label_muted(&format!(
            "Flasher {} · {} drives",
            env!("CARGO_PKG_VERSION"),
            self.platform.name()
        ));
    }

    /// What the confirmation says beyond "everything will be erased": the
    /// drive's volumes, a large drive, an image that may not be one. The
    /// same words as the terminal's.
    pub fn confirm_warnings(&self) -> Vec<String> {
        let Stage::Confirm(task) = self.stage else {
            return Vec::new();
        };
        let Some(device) = self.devices.get(self.selected) else {
            return Vec::new();
        };
        let image = match task {
            Task::Flash => self.ready_image(),
            Task::Restore => None,
        };
        flasher_cli::erase_warnings(device, image)
    }

    fn can_start(&self, task: Task) -> bool {
        let drive = self.selected < self.devices.len();
        match task {
            Task::Flash => drive && self.ready_image().is_some() && self.expected_sha256().is_ok(),
            Task::Restore => drive && volume_label(&self.label).is_ok(),
        }
    }

    /// The area under the tabs: the button that starts the open tab's task,
    /// and then the confirmation, progress and outcome of whatever runs.
    /// Buttons sit on the right, the primary one rightmost, as dialogs do.
    fn actions(&mut self, ui: &mut Ui) {
        let dark = self.dark();
        match &mut self.stage {
            Stage::Idle => {
                let task = match self.tab {
                    TAB_FLASH => Task::Flash,
                    TAB_RESTORE => Task::Restore,
                    _ => return,
                };
                let ok = self.can_start(task);
                button_row(ui, "idle", |ui| {
                    ui.enabled(ok, |ui| {
                        let (label, name) = match task {
                            Task::Flash => ("Flash", "flash"),
                            Task::Restore => ("Restore drive…", "restore"),
                        };
                        let r = ui.button_primary(label);
                        self.controls.push((name, r.rect));
                        if r.clicked {
                            self.stage = Stage::Confirm(task);
                        }
                    });
                });
            }
            Stage::Confirm(task) => {
                let task = *task;
                let name = &self.device_names[self.selected];
                let path = &self.devices[self.selected].path;
                warning(
                    ui,
                    &format!("Everything on {name} ({path}) will be erased."),
                );
                for w in self.confirm_warnings() {
                    warning(ui, &w);
                }
                ui.label_muted(
                    "A failing drive can freeze your computer while it is written. Save your work first.",
                );
                let unknown = task == Task::Flash
                    && self
                        .ready_image()
                        .is_some_and(|i| i.kind == ImageKind::Unknown);
                button_row(ui, "confirm", |ui| {
                    let r = ui.button("Cancel");
                    self.controls.push(("cancel", r.rect));
                    if r.clicked {
                        self.stage = Stage::Idle;
                    }
                    let label = match task {
                        // Said out loud: the warning above was read and overruled.
                        Task::Flash if unknown => "Erase and flash anyway",
                        Task::Flash => "Erase and flash",
                        Task::Restore => "Erase and restore",
                    };
                    let r = ui.button_primary(label);
                    self.controls.push(("confirm", r.rect));
                    if r.clicked {
                        self.start(task);
                    }
                });
            }
            Stage::Running(job) => {
                let elapsed = human_duration(job.started.elapsed());
                let text = match (job.task, job.heard) {
                    (Task::Restore, _) => format!("Erasing and formatting the drive… {elapsed}"),
                    (Task::Flash, true) => job.status.text(),
                    (Task::Flash, false) => {
                        format!("Waiting for your password, then unmounting the drive… {elapsed}")
                    }
                };
                // The status gets its own wrapping line; the percentage sits
                // beside the bar. libgui's `progress` puts both on one line,
                // where a long status runs into the number.
                let (size, ink) = (ui.theme.metrics.font_size, ui.theme.palette.text);
                ui.paragraph_with(&text, size, ink, Align::Start);
                let fraction = job.status.fraction();
                ui.container_id(
                    Id::new("progress_row"),
                    Layout::row()
                        .width(Size::Grow(1.0))
                        .height(Size::Fit)
                        .gap(10.0)
                        .align(Align::Start, Align::Center),
                    Frame::none(),
                    |ui| {
                        ui.progress("", fraction);
                        if let Some(f) = fraction {
                            ui.label_muted(&format!("{:.0}%", f.clamp(0.0, 1.0) * 100.0));
                        }
                    },
                );
                let quiet = job.last_heard.elapsed();
                if job.heard && quiet > QUIET_WARNING {
                    warning(
                        ui,
                        &format!(
                            "No response from the drive for {} s. It may be failing; unplug it if this keeps growing.",
                            quiet.as_secs()
                        ),
                    );
                }
                if self.closing {
                    warning(
                        ui,
                        match job.task {
                            Task::Flash => "Stopping the write; the window closes when it has stopped. Close it again to quit at once and leave the drive half-written.",
                            Task::Restore => "The window closes when the drive is finished. Close it again to quit at once and leave the drive unusable.",
                        },
                    );
                }
                // Restoring is one OS command; there is nothing safe to stop.
                if job.task == Task::Flash {
                    let stopping = job.cancel.load(Ordering::Relaxed);
                    button_row(ui, "running", |ui| {
                        ui.enabled(!stopping, |ui| {
                            let r = ui.button(if stopping { "Stopping…" } else { "Cancel" });
                            self.controls.push(("cancel", r.rect));
                            if r.clicked {
                                job.cancel.store(true, Ordering::Relaxed);
                            }
                        });
                    });
                }
            }
            Stage::Finished(result, note) => {
                match result {
                    Ok(msg) => {
                        let size = ui.theme.metrics.font_size;
                        ui.paragraph_with(msg, size, success_color(dark), Align::Start);
                    }
                    Err(e) => error(ui, e),
                }
                if let Some(note) = note {
                    warning(ui, note);
                }
                button_row(ui, "finished", |ui| {
                    let r = ui.button_primary("OK");
                    self.controls.push(("ok", r.rect));
                    if r.clicked {
                        self.stage = Stage::Idle;
                    }
                });
            }
        }
    }
}

/// A row of buttons pushed to the right edge.
fn button_row<R>(ui: &mut Ui, key: &str, body: impl FnOnce(&mut Ui) -> R) -> R {
    ui.container_id(
        Id::new(("buttons", key)),
        Layout::row()
            .width(Size::Grow(1.0))
            .height(Size::Fit)
            .gap(8.0)
            .align(Align::End, Align::Center),
        Frame::none(),
        body,
    )
}

fn error(ui: &mut Ui, text: &str) {
    let (size, color) = (ui.theme.metrics.font_size, ui.theme.palette.danger);
    // Wrapped: error messages are sentences, and the window can be narrow.
    ui.paragraph_with(text, size, color, Align::Start);
}

fn warning(ui: &mut Ui, text: &str) {
    let (size, color) = (ui.theme.metrics.font_size, ui.theme.palette.warning);
    ui.paragraph_with(text, size, color, Align::Start);
}
