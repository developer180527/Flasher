//! Flasher's terminal half: `flasher list | inspect | write | verify | restore`.
//!
//! The `flasher` binary opens its window when started with no arguments and
//! hands any command to [`run`]. The same libflasher underneath, no GUI: for
//! scripts, servers and anyone who prefers a terminal.

use std::io::{self, BufRead, Write};
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;

use libflasher::platform::human_size;
use libflasher::{image, DeviceInfo, FlashOptions, ImageInfo, ImageKind};

const USAGE: &str = "\
usage: flasher                  open the window
       flasher list
       flasher inspect <image>
       flasher write <image> <device> [--sha256 <hash>] [--no-verify] [--allow-sleep] [--yes]
       flasher verify <image> <device>
       flasher restore <device> [--label <name>] [--allow-sleep] [--yes]

write checks the image against --sha256, or a SHA256SUMS / <image>.sha256
file found next to it, before touching the drive. restore erases a drive back
to one exFAT volume (default name \"USB DRIVE\").";

/// Positional arguments, and `--flag` / `--flag value` options.
struct Args {
    pos: Vec<String>,
    flags: Vec<String>,
    values: Vec<(String, String)>,
}

impl Args {
    const TAKES_VALUE: &[&str] = &["--sha256", "--label"];
    const FLAGS: &[&str] = &["--no-verify", "--allow-sleep", "--yes"];

    fn parse(raw: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut a = Args {
            pos: Vec::new(),
            flags: Vec::new(),
            values: Vec::new(),
        };
        let mut raw = raw.peekable();
        while let Some(arg) = raw.next() {
            if Self::TAKES_VALUE.contains(&arg.as_str()) {
                let v = raw.next().ok_or_else(|| format!("{arg} needs a value"))?;
                a.values.push((arg, v));
            } else if Self::FLAGS.contains(&arg.as_str()) {
                a.flags.push(arg);
            } else if arg.starts_with("--") {
                return Err(format!("unknown option {arg}"));
            } else {
                a.pos.push(arg);
            }
        }
        Ok(a)
    }

    fn flag(&self, f: &str) -> bool {
        self.flags.iter().any(|x| x == f)
    }

    fn value(&self, f: &str) -> Option<&str> {
        self.values
            .iter()
            .find(|(k, _)| k == f)
            .map(|(_, v)| v.as_str())
    }
}

/// Run one terminal command; `args` excludes the program name.
pub fn run(args: impl IntoIterator<Item = String>) -> ExitCode {
    let args: Vec<String> = args.into_iter().collect();
    if matches!(
        args.first().map(String::as_str),
        Some("help" | "--help" | "-h")
    ) {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if matches!(args.first().map(String::as_str), Some("--version" | "-V")) {
        println!("flasher {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    let args = match Args::parse(args.into_iter()) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let pos: Vec<&str> = args.pos.iter().map(String::as_str).collect();
    let awake = !args.flag("--allow-sleep");
    let yes = args.flag("--yes");

    let result = match pos.as_slice() {
        ["list"] => list(),
        ["inspect", img] => inspect(img),
        ["write", img, dev] => write(
            img,
            dev,
            args.value("--sha256"),
            !args.flag("--no-verify"),
            awake,
            yes,
        ),
        ["verify", img, dev] => verify(img, dev),
        ["restore", dev] => restore(
            dev,
            args.value("--label").unwrap_or("USB DRIVE"),
            awake,
            yes,
        ),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("\r\x1b[2Kerror: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Drives larger than this get a second look before they are erased: sticks
/// and cards are smaller; backup and data disks are not.
pub const LARGE_DRIVE: u64 = 64_000_000_000;

/// What to say before erasing `device` (for `image`, when flashing), beyond
/// "everything on it will be erased": its volumes, if it is a large drive,
/// and if the image may not be a disk image at all. The window and the
/// terminal say the same.
pub fn erase_warnings(device: &DeviceInfo, image: Option<&ImageInfo>) -> Vec<String> {
    let mut out = Vec::new();
    if !device.mountpoints.is_empty() {
        out.push(format!("Volumes on it: {}.", device.mountpoints.join(", ")));
    }
    if device.size > LARGE_DRIVE {
        out.push(format!(
            "This is a large drive ({}). Make sure it is not a backup or data disk.",
            human_size(device.size)
        ));
    }
    if image.is_some_and(|i| i.kind == ImageKind::Unknown) {
        out.push(
            "This file has no partition table or ISO header. It may not be a disk image, \
             and the drive may not start from it."
                .into(),
        );
    }
    out
}

/// What a matching checksum shows, by where it came from: `None` for one the
/// user gave, `Some(file)` for one found beside the image. A file from the
/// same place as the image shows the download is intact, not that it is
/// genuine (see `libflasher::checksum`).
pub fn checksum_matched(found_in: Option<&str>) -> String {
    match found_in {
        None => "the image matched the SHA-256 you gave".into(),
        Some(file) => format!("the image matched {file} beside it, so the download is intact"),
    }
}

/// Ask before erasing; `--yes` skips the question, for scripts, but not the
/// warnings.
fn confirm(
    device: &DeviceInfo,
    what: &str,
    warnings: &[String],
    yes: bool,
) -> libflasher::Result<()> {
    for w in warnings {
        eprintln!("warning: {w}");
    }
    if yes {
        return Ok(());
    }
    eprintln!(
        "A failing drive can freeze your computer while it is written. Save your work first."
    );
    print!(
        "ERASE {} ({}) and {what}? Type 'yes': ",
        device.display_name(),
        device.path
    );
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().lock().read_line(&mut answer)?;
    if answer.trim() != "yes" {
        return Err(libflasher::Error::Cancelled);
    }
    Ok(())
}

fn restore(dev: &str, label: &str, awake: bool, yes: bool) -> libflasher::Result<()> {
    let platform = libflasher::current_platform();
    let label =
        libflasher::platform::volume_label(label).map_err(|reason| libflasher::Error::Refused {
            device: dev.into(),
            reason,
        })?;
    let device = listed(platform.as_ref(), dev)?;
    confirm(
        &device,
        &format!("make it an ordinary exFAT drive named \"{label}\""),
        &erase_warnings(&device, None),
        yes,
    )?;
    let _awake = awake
        .then(|| platform.keep_awake("Erasing a drive"))
        .flatten();
    eprint!("erasing and formatting…");
    platform.restore_listed(&device, &label)?;
    eprintln!(
        "\r\x1b[2Kdone: {} is an ordinary drive named \"{label}\" again",
        device.path
    );
    Ok(())
}

fn list() -> libflasher::Result<()> {
    let platform = libflasher::current_platform();
    let devices = platform.list_devices()?;
    if devices.is_empty() {
        println!("no removable drives found ({})", platform.name());
    }
    for d in devices {
        println!("{}  {}", d.path, d.display_name());
        for m in &d.mountpoints {
            println!("    mounted at {m}");
        }
    }
    Ok(())
}

fn inspect(path: &str) -> libflasher::Result<()> {
    let i = image::inspect(path)?;
    println!("{}", i.path.display());
    println!("  kind:        {}", i.kind.describe());
    println!("  compression: {:?}", i.compression);
    println!("  file size:   {}", human_size(i.file_size));
    match i.disk_size {
        Some(s) => println!("  disk size:   {} ({s} bytes)", human_size(s)),
        None => println!(
            "  disk size:   unknown until decompressed ({:?} does not record it)",
            i.compression
        ),
    }
    if i.kind.needs_extract() {
        match libflasher::extract::plan(&i) {
            Ok(p) => {
                println!(
                    "  extract:     {} files, {}, needs {} on the drive, volume \"{}\"",
                    p.files,
                    human_size(p.bytes),
                    human_size(p.needs),
                    p.label
                );
                if let Some(n) = p.wim_parts {
                    println!("  install.wim: over FAT32's 4 GB limit; split into {n} .swm parts");
                }
            }
            Err(e) => println!("  extract:     not possible: {e}"),
        }
    }
    Ok(())
}

fn write(
    img: &str,
    dev: &str,
    sha256: Option<&str>,
    verify: bool,
    awake: bool,
    yes: bool,
) -> libflasher::Result<()> {
    let platform = libflasher::current_platform();
    let info = image::inspect(img)?;
    if info.kind.needs_extract() {
        // Fails here, with the reason, before anything is erased.
        let plan = libflasher::extract::plan(&info)?;
        eprintln!(
            "{img} is an ISO that is not a disk image: its {} files ({}) will be copied onto a FAT32 drive named \"{}\"",
            plan.files,
            human_size(plan.bytes),
            plan.label
        );
    } else if !info.kind.raw_writable() {
        return Err(libflasher::Error::Unsupported(format!(
            "{}: {}",
            img,
            info.kind.describe()
        )));
    }
    let device = listed(platform.as_ref(), dev)?;
    let expected = match sha256 {
        Some(h) => Some((h.to_string(), "--sha256".to_string())),
        None => libflasher::checksum::find_published(&info.path).map(|p| (p.sha256, p.source)),
    };
    match &expected {
        Some((_, source)) if sha256.is_none() => eprintln!(
            "checking the image against {source} beside it: this shows the download is intact, \
             not that it is genuine; for that, pass --sha256 with the hash from the publisher's website"
        ),
        Some(_) => eprintln!("checking the image against the SHA-256 given"),
        None => eprintln!(
            "no SHA-256 given or found next to the image: a damaged download cannot be detected"
        ),
    }
    confirm(
        &device,
        &format!("write {img}"),
        &erase_warnings(&device, Some(&info)),
        yes,
    )?;
    let _awake = awake
        .then(|| platform.keep_awake("Writing a disk image"))
        .flatten();

    if let Some((hash, source)) = &expected {
        let mut status = libflasher::rate::StatusLine::new();
        libflasher::checksum::verify_image(&info, hash, &AtomicBool::new(false), &mut |p| {
            status.update(p);
            let pct = status
                .fraction()
                .map(|f| format!("{:5.1}%  ", f * 100.0))
                .unwrap_or_default();
            eprint!("\r\x1b[2K{pct}{}", status.text());
        })?;
        let found_in = sha256.is_none().then_some(source.as_str());
        eprintln!("\r\x1b[2K{}", checksum_matched(found_in));
    }

    let mut raw = platform.open_listed(&device)?;
    let cancel = AtomicBool::new(false);
    // A write blocked in the kernel never returns to report, so a second
    // thread watches the clock and says so instead of looking frozen.
    let watch = QuietWatch::start();
    let mut status = libflasher::rate::StatusLine::new();
    let result = libflasher::write_image(
        &info,
        raw.as_mut(),
        &FlashOptions::default().with_verify(verify),
        &cancel,
        &mut |p| {
            watch.heard();
            status.update(p);
            let pct = status
                .fraction()
                .map(|f| format!("{:5.1}%  ", f * 100.0))
                .unwrap_or_default();
            eprint!("\r\x1b[2K{pct}{}", status.text());
        },
    );
    watch.stop();
    let written = result?;
    drop(raw);
    eprintln!(
        "\r\x1b[2Kdone: {} written{}",
        human_size(written),
        if verify { " and verified" } else { "" }
    );
    // The image is on the drive: a failed eject is not a failed write.
    match platform.eject(&device) {
        Ok(()) => eprintln!("drive ejected; it is safe to remove"),
        Err(e) => eprintln!(
            "warning: could not eject the drive ({e}); eject it from the system before unplugging it"
        ),
    }
    Ok(())
}

/// Only ever touch a disk the platform itself lists as removable.
fn listed(
    platform: &dyn libflasher::Platform,
    dev: &str,
) -> libflasher::Result<libflasher::DeviceInfo> {
    platform
        .list_devices()?
        .into_iter()
        .find(|d| d.path == dev)
        .ok_or_else(|| libflasher::Error::Refused {
            device: dev.into(),
            reason: "not a listed removable drive".into(),
        })
}

fn verify(img: &str, dev: &str) -> libflasher::Result<()> {
    let platform = libflasher::current_platform();
    let info = image::inspect(img)?;
    let device = listed(platform.as_ref(), dev)?;
    let mut raw = platform.open_listed(&device)?;
    let cancel = AtomicBool::new(false);
    let mut status = libflasher::rate::StatusLine::new();
    let n = libflasher::verify_device(&info, raw.as_mut(), &cancel, &mut |p| {
        status.update(p);
        let pct = status
            .fraction()
            .map(|f| format!("{:5.1}%  ", f * 100.0))
            .unwrap_or_default();
        eprint!("\r\x1b[2K{pct}{}", status.text());
    })?;
    eprintln!(
        "\r\x1b[2Kverified: {} on {} match {}",
        human_size(n),
        device.path,
        img
    );
    Ok(())
}

/// Prints a warning while the drive is silent for too long.
struct QuietWatch {
    /// `None` until the first progress report: before it, silence is the password prompt.
    last: std::sync::Arc<std::sync::Mutex<Option<std::time::Instant>>>,
    done: std::sync::Arc<AtomicBool>,
}

impl QuietWatch {
    const AFTER: std::time::Duration = std::time::Duration::from_secs(10);

    fn start() -> Self {
        use std::sync::atomic::Ordering;
        let last = std::sync::Arc::new(std::sync::Mutex::new(None::<std::time::Instant>));
        let done = std::sync::Arc::new(AtomicBool::new(false));
        let (l, d) = (last.clone(), done.clone());
        std::thread::spawn(move || {
            while !d.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_secs(1));
                let quiet = l.lock().ok().and_then(|t| t.map(|t| t.elapsed()));
                if let Some(quiet) =
                    quiet.filter(|q| *q > Self::AFTER && !d.load(Ordering::Relaxed))
                {
                    eprint!(
                        "\rthe drive has not responded for {} s; it may be failing. If this keeps growing, unplug it.   ",
                        quiet.as_secs()
                    );
                }
            }
        });
        Self { last, done }
    }

    fn heard(&self) {
        if let Ok(mut t) = self.last.lock() {
            *t = Some(std::time::Instant::now());
        }
    }

    fn stop(&self) {
        self.done.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drive(size: u64, mounts: &[&str]) -> DeviceInfo {
        DeviceInfo::new(
            "/dev/disk4",
            "Stick",
            size,
            "USB",
            mounts.iter().map(|m| m.to_string()).collect(),
        )
    }

    #[test]
    fn a_plain_stick_needs_no_warning() {
        assert!(erase_warnings(&drive(32_000_000_000, &[]), None).is_empty());
    }

    #[test]
    fn names_volumes_and_large_drives() {
        let w = erase_warnings(&drive(2_000_000_000_000, &["/Volumes/Backup"]), None);
        assert_eq!(w.len(), 2, "{w:?}");
        assert!(w[0].contains("/Volumes/Backup"), "{w:?}");
        assert!(w[1].contains("large drive (2.0 TB)"), "{w:?}");
        assert!(
            erase_warnings(&drive(LARGE_DRIVE, &[]), None).is_empty(),
            "64 GB is a stick"
        );
    }

    #[test]
    fn says_where_a_checksum_came_from() {
        assert!(checksum_matched(None).contains("you gave"));
        let m = checksum_matched(Some("SHA256SUMS"));
        assert!(
            m.contains("SHA256SUMS beside it") && m.contains("intact"),
            "{m}"
        );
    }
}
