//! The GUI, clicked through end to end with no window, GPU or USB drive.
//!
//! Each test makes mock drives (files) and images in a temp directory, then
//! drives the real `App` through pointer events. Snapshots of each stage land
//! in `target/snapshots/` for looking at.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use flasher::headless::Headless;
use libflasher::mock::MockPlatform;
use libgui::Rect;

const MB: usize = 1 << 20;

struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("flasher_headless_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self { dir }
    }

    fn drive(&self, name: &str, size: usize) -> PathBuf {
        let p = self.dir.join(format!("{name}.disk"));
        std::fs::write(&p, vec![0xEE; size]).unwrap();
        p
    }

    /// A hybrid ISO: an ISO 9660 header and an MBR signature.
    fn image(&self, name: &str, size: usize, hybrid: bool) -> (PathBuf, Vec<u8>) {
        let mut b: Vec<u8> = (0..size).map(|i| (i * 7 % 253) as u8).collect();
        b[0x8001..0x8006].copy_from_slice(b"CD001");
        if hybrid {
            b[510] = 0x55;
            b[511] = 0xAA;
        } else {
            b[510] = 0;
        }
        let p = self.dir.join(name);
        std::fs::write(&p, &b).unwrap();
        (p, b)
    }

    fn app(&self) -> Headless {
        Headless::new(Arc::new(MockPlatform::new(&self.dir)))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn snap(h: &mut Headless, name: &str) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/snapshots");
    std::fs::create_dir_all(&dir).unwrap();
    h.snapshot(&dir.join(format!("{name}.png"))).unwrap();
}

fn flash_through_ui(h: &mut Headless) {
    h.click("flash").unwrap();
    h.click("confirm").unwrap();
    h.wait_until_idle(Duration::from_secs(30)).unwrap();
}

#[test]
fn flashes_and_verifies_end_to_end() {
    let f = Fixture::new("ok");
    let drive = f.drive("stick", 8 * MB);
    let (img, bytes) = f.image("distro.iso", 3 * MB + 1234, true);

    let mut h = f.app();
    assert_eq!(h.app.devices().len(), 1);
    snap(&mut h, "1-empty");

    h.drop_file(&img);
    snap(&mut h, "2-ready");

    h.click("flash").unwrap();
    snap(&mut h, "3-confirm");
    h.click("confirm").unwrap();
    h.wait_until_idle(Duration::from_secs(30)).unwrap();
    snap(&mut h, "4-done");

    let outcome = h.app.outcome().expect("finished").clone();
    assert!(
        outcome.as_ref().is_ok_and(|m| m.contains("verified")),
        "{outcome:?}"
    );
    let written = std::fs::read(&drive).unwrap();
    assert_eq!(&written[..bytes.len()], &bytes[..]);

    h.click("ok").unwrap();
    assert!(h.control("flash").is_some(), "back to the start");
}

#[test]
fn flash_needs_an_image() {
    let f = Fixture::new("noimage");
    f.drive("stick", 8 * MB);
    let mut h = f.app();
    h.click("flash").unwrap();
    assert!(
        h.control("confirm").is_none(),
        "a disabled Flash button was clickable"
    );
}

#[test]
fn flash_needs_a_drive() {
    let f = Fixture::new("nodrive");
    let (img, _) = f.image("distro.iso", MB, true);
    let mut h = f.app();
    h.drop_file(&img);
    snap(&mut h, "no-drive");
    h.click("flash").unwrap();
    assert!(h.control("confirm").is_none());
}

/// The fixture's plain ISO has no readable file tree, so extract mode
/// refuses it; a real plain ISO is extracted (see libflasher's tests).
#[test]
fn refuses_non_hybrid_iso() {
    let f = Fixture::new("plainiso");
    f.drive("stick", 8 * MB);
    let (img, _) = f.image("windows.iso", MB, false);
    let mut h = f.app();
    h.drop_file(&img);
    snap(&mut h, "plain-iso");
    h.click("flash").unwrap();
    assert!(
        h.control("confirm").is_none(),
        "extract-mode images must not be raw-written"
    );
}

#[test]
fn cancel_at_confirmation_writes_nothing() {
    let f = Fixture::new("cancel");
    let drive = f.drive("stick", 8 * MB);
    let (img, _) = f.image("distro.iso", MB, true);
    let mut h = f.app();
    h.drop_file(&img);
    h.click("flash").unwrap();
    h.click("cancel").unwrap();
    assert!(h.control("flash").is_some());
    assert!(
        std::fs::read(&drive).unwrap().iter().all(|&b| b == 0xEE),
        "drive was touched"
    );
}

#[test]
fn reports_a_drive_that_fails_verification() {
    let f = Fixture::new("corrupt");
    f.drive("corrupt-stick", 8 * MB);
    let (img, _) = f.image("distro.iso", MB, true);
    let mut h = f.app();
    h.drop_file(&img);
    flash_through_ui(&mut h);
    snap(&mut h, "verify-failed");
    let outcome = h.app.outcome().unwrap();
    assert!(
        outcome
            .as_ref()
            .is_err_and(|e| e.contains("verification failed at byte 0") || e.contains("byte 0")),
        "{outcome:?}"
    );
}

#[test]
fn reports_denied_permission() {
    let f = Fixture::new("denied");
    let drive = f.drive("denied-stick", 8 * MB);
    let (img, _) = f.image("distro.iso", MB, true);
    let mut h = f.app();
    h.drop_file(&img);
    flash_through_ui(&mut h);
    let outcome = h.app.outcome().unwrap();
    assert!(
        outcome.as_ref().is_err_and(|e| e.contains("permission")),
        "{outcome:?}"
    );
    assert!(std::fs::read(&drive).unwrap().iter().all(|&b| b == 0xEE));
}

#[test]
fn reports_an_image_too_large_for_the_drive() {
    let f = Fixture::new("toolarge");
    f.drive("tiny", 2 * MB);
    let (img, _) = f.image("distro.iso", 3 * MB, true);
    let mut h = f.app();
    h.drop_file(&img);
    flash_through_ui(&mut h);
    let outcome = h.app.outcome().unwrap();
    assert!(
        outcome.as_ref().is_err_and(|e| e.contains("holds only")),
        "{outcome:?}"
    );
}

#[test]
fn verify_can_be_turned_off() {
    let f = Fixture::new("noverify");
    f.drive("stick", 8 * MB);
    let (img, _) = f.image("distro.iso", MB, true);
    let mut h = f.app();
    h.drop_file(&img);
    h.click("tab:options").unwrap();
    h.click("verify").unwrap();
    h.click("tab:flash").unwrap();
    flash_through_ui(&mut h);
    let outcome = h.app.outcome().unwrap();
    assert!(
        outcome.as_ref().is_ok_and(|m| !m.contains("verified")),
        "{outcome:?}"
    );
}

#[test]
fn survives_the_drive_being_unplugged_mid_write() {
    let f = Fixture::new("unplug");
    f.drive("unplug-stick", 8 * MB);
    let (img, _) = f.image("distro.iso", 4 * MB, true);
    let mut h = f.app();
    h.drop_file(&img);
    flash_through_ui(&mut h);
    snap(&mut h, "unplugged");
    let outcome = h.app.outcome().unwrap().clone();
    assert!(
        outcome.as_ref().is_err_and(|e| e.contains("disconnected")),
        "{outcome:?}"
    );
    // The list was refreshed: the drive is gone, so there is nothing to flash.
    assert!(h.app.devices().is_empty());
    h.click("ok").unwrap();
    h.click("flash").unwrap();
    assert!(
        h.control("confirm").is_none(),
        "could flash with no drive attached"
    );
}

#[test]
fn refuses_a_different_drive_that_took_the_same_path() {
    let f = Fixture::new("reused");
    let drive = f.drive("stick", 8 * MB);
    let (img, _) = f.image("distro.iso", MB, true);
    let mut h = f.app();
    h.drop_file(&img);
    h.click("flash").unwrap();
    // While the confirmation is up, the stick is swapped for another disk
    // that the OS gives the same name.
    std::fs::write(&drive, vec![0x11; 16 * MB]).unwrap();
    h.click("confirm").unwrap();
    h.wait_until_idle(Duration::from_secs(30)).unwrap();
    let outcome = h.app.outcome().unwrap();
    assert!(
        outcome
            .as_ref()
            .is_err_and(|e| e.contains("different drive")),
        "{outcome:?}"
    );
    assert!(
        std::fs::read(&drive).unwrap().iter().all(|&b| b == 0x11),
        "wrote to the wrong drive"
    );
}

#[test]
fn refuses_a_drive_removed_before_writing() {
    let f = Fixture::new("removed");
    let drive = f.drive("stick", 8 * MB);
    let (img, _) = f.image("distro.iso", MB, true);
    let mut h = f.app();
    h.drop_file(&img);
    h.click("flash").unwrap();
    std::fs::remove_file(&drive).unwrap();
    h.click("confirm").unwrap();
    h.wait_until_idle(Duration::from_secs(30)).unwrap();
    let outcome = h.app.outcome().unwrap();
    assert!(
        outcome
            .as_ref()
            .is_err_and(|e| e.contains("no longer connected")),
        "{outcome:?}"
    );
}

// ---- tabs, checksum, hot-plug, keep-awake, restore ----------------------

fn wait_for(h: &mut Headless, what: &str, mut done: impl FnMut(&Headless) -> bool) {
    let end = std::time::Instant::now() + Duration::from_secs(5);
    while !done(h) {
        assert!(
            std::time::Instant::now() < end,
            "timed out waiting for {what}"
        );
        h.frame();
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn tabs_switch_pages_and_keep_state() {
    let f = Fixture::new("tabs");
    f.drive("stick", 8 * MB);
    let (img, _) = f.image("distro.iso", MB, true);
    let mut h = f.app();
    h.drop_file(&img);
    snap(&mut h, "tab-flash");
    assert!(h.control("image").is_some() && h.control("verify").is_none());

    h.click("tab:options").unwrap();
    snap(&mut h, "tab-options");
    assert!(h.control("verify").is_some() && h.control("image").is_none());
    assert!(h.control("flash").is_none(), "Options has no action button");

    h.click("tab:restore").unwrap();
    snap(&mut h, "tab-restore");
    assert!(h.control("label").is_some() && h.control("restore").is_some());

    // The image chosen on Flash is still there on return.
    h.click("tab:flash").unwrap();
    h.click("flash").unwrap();
    assert!(h.control("confirm").is_some());
}

#[test]
fn finds_and_checks_a_published_checksum() {
    let f = Fixture::new("sum_ok");
    let drive = f.drive("stick", 8 * MB);
    let (img, bytes) = f.image("distro.iso", MB, true);
    let hash = {
        use std::sync::atomic::AtomicBool;
        libflasher::checksum::sha256_file(&img, 0, &AtomicBool::new(false), &mut |_| {}).unwrap()
    };
    std::fs::write(f.dir.join("SHA256SUMS"), format!("{hash}  distro.iso\n")).unwrap();

    let mut h = f.app();
    h.drop_file(&img);
    assert_eq!(h.app.sha256(), (hash.as_str(), Some("SHA256SUMS")));
    snap(&mut h, "checksum-found");
    flash_through_ui(&mut h);
    let outcome = h.app.outcome().unwrap();
    assert!(
        outcome
            .as_ref()
            .is_ok_and(|m| m.contains("matched its published SHA-256")),
        "{outcome:?}"
    );
    assert_eq!(&std::fs::read(&drive).unwrap()[..bytes.len()], &bytes[..]);
}

#[test]
fn a_wrong_checksum_stops_before_the_drive_is_touched() {
    let f = Fixture::new("sum_bad");
    let drive = f.drive("stick", 8 * MB);
    let (img, _) = f.image("distro.iso", MB, true);
    let mut h = f.app();
    h.drop_file(&img);
    h.app.set_sha256(&"ab".repeat(32));
    flash_through_ui(&mut h);
    snap(&mut h, "checksum-mismatch");
    let outcome = h.app.outcome().unwrap();
    assert!(
        outcome
            .as_ref()
            .is_err_and(|e| e.contains("does not match")),
        "{outcome:?}"
    );
    assert!(
        std::fs::read(&drive).unwrap().iter().all(|&b| b == 0xEE),
        "drive was written"
    );
}

#[test]
fn a_malformed_checksum_blocks_flashing() {
    let f = Fixture::new("sum_malformed");
    f.drive("stick", 8 * MB);
    let (img, _) = f.image("distro.iso", MB, true);
    let mut h = f.app();
    h.drop_file(&img);
    h.app.set_sha256("not-a-hash");
    h.click("flash").unwrap();
    assert!(
        h.control("confirm").is_none(),
        "flashed with an unusable checksum"
    );
}

#[test]
fn drives_appear_and_disappear_without_refresh() {
    let f = Fixture::new("hotplug");
    let mut h = Headless::with_app(flasher::App::with_poll(
        Arc::new(MockPlatform::new(&f.dir)),
        Duration::from_millis(20),
    ));
    assert!(h.app.devices().is_empty());
    let drive = f.drive("stick", 8 * MB);
    wait_for(&mut h, "the drive to appear", |h| {
        h.app.devices().len() == 1
    });
    std::fs::remove_file(drive).unwrap();
    wait_for(&mut h, "the drive to disappear", |h| {
        h.app.devices().is_empty()
    });
}

#[test]
fn a_confirmation_is_withdrawn_when_its_drive_goes() {
    let f = Fixture::new("confirm_gone");
    let drive = f.drive("stick", 8 * MB);
    let (img, _) = f.image("distro.iso", MB, true);
    let mut h = Headless::with_app(flasher::App::with_poll(
        Arc::new(MockPlatform::new(&f.dir)),
        Duration::from_millis(20),
    ));
    h.drop_file(&img);
    h.click("flash").unwrap();
    assert!(h.control("confirm").is_some());
    std::fs::remove_file(drive).unwrap();
    wait_for(&mut h, "the confirmation to close", |h| {
        h.control("confirm").is_none()
    });
}

#[test]
fn keeps_the_computer_awake_only_while_working() {
    let f = Fixture::new("awake");
    f.drive("slow-stick", 8 * MB);
    let (img, _) = f.image("distro.iso", 4 * MB, true);
    let marker = f.dir.join(".awake");
    let mut h = f.app();
    h.drop_file(&img);
    assert!(!marker.exists());
    h.click("flash").unwrap();
    h.click("confirm").unwrap();
    wait_for(&mut h, "keep-awake to start", |_| marker.exists());
    h.wait_until_idle(Duration::from_secs(30)).unwrap();
    assert!(h.app.outcome().unwrap().is_ok());
    assert!(
        !marker.exists(),
        "still keeping the computer awake after finishing"
    );
}

#[test]
fn keep_awake_can_be_turned_off() {
    let f = Fixture::new("noawake");
    f.drive("slow-stick", 8 * MB);
    let (img, _) = f.image("distro.iso", 4 * MB, true);
    let marker = f.dir.join(".awake");
    let mut h = f.app();
    h.drop_file(&img);
    h.click("tab:options").unwrap();
    h.click("keep_awake").unwrap();
    h.click("tab:flash").unwrap();
    h.click("flash").unwrap();
    h.click("confirm").unwrap();
    while h.app.busy() {
        assert!(!marker.exists(), "kept awake although turned off");
        h.frame();
    }
}

#[test]
fn restores_a_drive_through_the_ui() {
    let f = Fixture::new("restore");
    let drive = f.drive("stick", 8 * MB);
    let mut h = f.app();
    h.click("tab:restore").unwrap();
    h.app.set_label("MY STICK");
    h.click("restore").unwrap();
    snap(&mut h, "restore-confirm");
    h.click("confirm").unwrap();
    h.wait_until_idle(Duration::from_secs(30)).unwrap();
    snap(&mut h, "restore-done");
    let outcome = h.app.outcome().unwrap();
    assert!(
        outcome.as_ref().is_ok_and(|m| m.contains("MY STICK")),
        "{outcome:?}"
    );
    let head = std::fs::read(&drive).unwrap();
    assert!(
        head.starts_with(b"MOCKFS MY STICK"),
        "drive was not restored"
    );
}

#[test]
fn restore_refuses_a_bad_name() {
    let f = Fixture::new("restore_badname");
    let drive = f.drive("stick", 8 * MB);
    let mut h = f.app();
    h.click("tab:restore").unwrap();
    h.app.set_label("way/too:long name!");
    snap(&mut h, "restore-bad-name");
    h.click("restore").unwrap();
    assert!(h.control("confirm").is_none());
    assert!(std::fs::read(&drive).unwrap().iter().all(|&b| b == 0xEE));
}

// ---- appearance ---------------------------------------------------------

#[test]
fn both_theme_files_parse() {
    let dark = flasher::theme(true).expect("dark.toml");
    let light = flasher::theme(false).expect("light.toml");
    assert_ne!(dark.palette.bg_app, light.palette.bg_app);
}

#[test]
fn theme_follows_the_choice_and_the_system() {
    let f = Fixture::new("theme");
    let mut h = f.app();
    let light_bg = flasher::theme(false).unwrap().palette.bg_app;
    let dark_bg = flasher::theme(true).unwrap().palette.bg_app;

    // System (the default) follows what the host reports.
    h.app.set_system_dark(false);
    h.frame();
    assert_eq!(h.ui.theme.palette.bg_app, light_bg);
    snap(&mut h, "light-flash");
    h.app.set_system_dark(true);
    h.frame();
    assert_eq!(h.ui.theme.palette.bg_app, dark_bg);

    // An explicit choice wins over the system, via the control itself.
    h.click("tab:options").unwrap();
    let r = h.control("theme").unwrap();
    let light_segment = Rect::new(r.x + r.w / 3.0, r.y, r.w / 3.0, r.h);
    h.click_at(light_segment);
    assert_eq!(h.app.settings.theme, flasher::ThemeChoice::Light);
    h.frame();
    assert_eq!(h.ui.theme.palette.bg_app, light_bg);
    snap(&mut h, "light-options");
}

#[test]
fn action_buttons_sit_on_the_right() {
    let f = Fixture::new("right");
    f.drive("stick", 8 * MB);
    let (img, _) = f.image("distro.iso", MB, true);
    let mut h = f.app();
    h.drop_file(&img);
    let width = h.info.screen_size.x;
    let flash = h.control("flash").unwrap();
    assert!(flash.x > width / 2.0, "Flash is on the left: {flash:?}");
    h.click("flash").unwrap();
    let (cancel, confirm) = (h.control("cancel").unwrap(), h.control("confirm").unwrap());
    assert!(
        confirm.x > cancel.x,
        "the primary button should be rightmost: cancel {cancel:?}, confirm {confirm:?}"
    );
    assert!(
        confirm.x + confirm.w > width - 40.0,
        "not against the right edge: {confirm:?}"
    );
}

#[test]
fn success_is_green_and_progress_is_readable() {
    let f = Fixture::new("green");
    f.drive("slow-stick", 8 * MB);
    let (img, _) = f.image("distro.iso", 4 * MB, true);
    let mut h = f.app();
    h.drop_file(&img);
    h.click("flash").unwrap();
    h.click("confirm").unwrap();
    // Mid-write, for a look at the status line and the percentage.
    while h.app.busy() && !h.app.progress_started() {
        h.frame();
    }
    snap(&mut h, "progress");
    h.wait_until_idle(Duration::from_secs(30)).unwrap();
    snap(&mut h, "done-dark");
    h.app.set_system_dark(false);
    h.frame();
    snap(&mut h, "done-light");
    assert!(h.app.outcome().unwrap().is_ok());
}

#[test]
fn long_paths_are_cut_in_the_middle_at_the_minimum_width() {
    let f = Fixture::new("narrow_window_with_a_rather_long_directory_name");
    f.drive("a-stick-with-a-long-name", 8 * MB);
    let mut h = f.app();
    h.info.screen_size.x = 520.0;
    h.frames(3);
    snap(&mut h, "narrow");
}

// ---- remembered settings ------------------------------------------------

#[test]
fn settings_are_remembered_between_runs() {
    use flasher::prefs::PrefsStore;
    let f = Fixture::new("prefs");
    let (img, _) = f.image("distro.iso", MB, true);
    let store = PrefsStore::at(f.dir.join("config/settings.toml"));

    let mut first = Headless::with_app(flasher::App::with_store(
        Arc::new(MockPlatform::new(&f.dir)),
        store.clone(),
    ));
    first.drop_file(&img);
    first.click("tab:options").unwrap();
    first.click("verify").unwrap();
    first.click("keep_awake").unwrap();
    let r = first.control("theme").unwrap();
    first.click_at(Rect::new(r.x + 2.0 * r.w / 3.0, r.y, r.w / 3.0, r.h)); // Dark
    assert!(store.path().exists(), "nothing was saved");
    drop(first);

    let second = flasher::App::with_store(Arc::new(MockPlatform::new(&f.dir)), store);
    assert!(!second.settings.verify);
    assert!(!second.settings.keep_awake);
    assert!(
        second.settings.eject,
        "untouched settings keep their defaults"
    );
    assert_eq!(second.settings.theme, flasher::ThemeChoice::Dark);
    assert_eq!(second.last_image_dir(), Some(f.dir.as_path()));
}

// ---- extract mode -------------------------------------------------------

#[test]
fn a_plain_iso_without_uefi_is_refused_before_flashing() {
    let f = Fixture::new("extract_no_uefi");
    let drive = f.drive("stick", 96 * MB);
    let (img, _) = f.image("windows.iso", MB, false); // ISO 9660 header, no MBR, no files
    let mut h = f.app();
    h.drop_file(&img);
    snap(&mut h, "extract-refused");
    h.click("flash").unwrap();
    assert!(
        h.control("confirm").is_none(),
        "offered to erase the drive for an ISO it cannot extract"
    );
    assert!(std::fs::read(&drive).unwrap().iter().all(|&b| b == 0xEE));
}

// ---- closing the window, ejecting ---------------------------------------

/// Starts a flash long enough (a slow drive) to act on while it runs.
fn flash_in_progress(f: &Fixture) -> Headless {
    f.drive("slow-stick", 64 * MB);
    let (img, _) = f.image("distro.iso", 32 * MB, true);
    let mut h = f.app();
    h.drop_file(&img);
    h.click("flash").unwrap();
    h.click("confirm").unwrap();
    wait_for(&mut h, "the write to start", |h| h.app.progress_started());
    h
}

#[test]
fn closing_when_idle_closes_at_once() {
    let f = Fixture::new("close_idle");
    f.drive("stick", 8 * MB);
    let mut h = f.app();
    assert!(h.app.request_close());
}

#[test]
fn closing_mid_write_stops_the_write_before_closing() {
    let f = Fixture::new("close_busy");
    let mut h = flash_in_progress(&f);
    assert!(!h.app.request_close(), "closed with a write in progress");
    assert!(!h.app.should_close(), "closing before the write stopped");
    snap(&mut h, "closing");
    h.wait_until_idle(Duration::from_secs(30)).unwrap();
    assert!(h.app.should_close(), "did not close once the write stopped");
    let outcome = h.app.outcome().unwrap();
    assert!(
        outcome.as_ref().is_err_and(|e| e.contains("cancelled")),
        "{outcome:?}"
    );
}

#[test]
fn closing_twice_closes_without_waiting() {
    let f = Fixture::new("close_twice");
    let mut h = flash_in_progress(&f);
    assert!(!h.app.request_close());
    assert!(h.app.request_close(), "a second close should not wait");
    h.wait_until_idle(Duration::from_secs(30)).unwrap();
}

/// Mock drives whose eject always fails, as macOS's does while a volume
/// it has just mounted is busy.
struct EjectFails(MockPlatform);

impl libflasher::Platform for EjectFails {
    fn name(&self) -> &'static str {
        "mock, eject fails"
    }
    fn list_devices(&self) -> libflasher::Result<Vec<libflasher::DeviceInfo>> {
        self.0.list_devices()
    }
    fn open_device(
        &self,
        device: &libflasher::DeviceInfo,
    ) -> libflasher::Result<Box<dyn libflasher::RawDevice>> {
        self.0.open_device(device)
    }
    fn eject(&self, _device: &libflasher::DeviceInfo) -> libflasher::Result<()> {
        Err(std::io::Error::other("the disk is busy").into())
    }
}

#[test]
fn a_failed_eject_is_a_warning_on_a_successful_flash() {
    let f = Fixture::new("eject_fails");
    let drive = f.drive("stick", 8 * MB);
    let (img, bytes) = f.image("distro.iso", MB, true);
    let mut h = Headless::new(Arc::new(EjectFails(MockPlatform::new(&f.dir))));
    h.drop_file(&img);
    flash_through_ui(&mut h);
    snap(&mut h, "eject-failed");
    let outcome = h.app.outcome().unwrap();
    assert!(
        outcome
            .as_ref()
            .is_ok_and(|m| m.contains("verified") && !m.contains("safe to remove")),
        "{outcome:?}"
    );
    let warning = h.app.outcome_warning().expect("a warning about ejecting");
    assert!(warning.contains("the disk is busy"), "{warning}");
    assert_eq!(&std::fs::read(&drive).unwrap()[..bytes.len()], &bytes[..]);
}
