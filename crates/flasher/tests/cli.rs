//! The `flasher` binary, run as a script or CI job would, against mock drives.

use std::path::PathBuf;
use std::process::Command;

fn setup(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("flasher_cli_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("stick.disk"), vec![0u8; 4 << 20]).unwrap();
    dir
}

fn flasher(dir: &PathBuf, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_flasher"))
        .args(args)
        .env("FLASHER_MOCK_DIR", dir)
        .output()
        .unwrap()
}

#[test]
fn lists_writes_and_verifies() {
    let dir = setup("write");
    let mut img: Vec<u8> = (0..(1 << 20) + 99).map(|i| (i % 249) as u8).collect();
    img[510] = 0x55;
    img[511] = 0xAA;
    let img_path = dir.join("disk.img");
    std::fs::write(&img_path, &img).unwrap();
    let drive = dir.join("stick.disk");

    let out = flasher(&dir, &["list"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("stick.disk"));

    let out = flasher(
        &dir,
        &[
            "write",
            img_path.to_str().unwrap(),
            drive.to_str().unwrap(),
            "--yes",
        ],
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert!(stderr.contains("verified"), "{stderr}");
    assert_eq!(&std::fs::read(&drive).unwrap()[..img.len()], &img[..]);

    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn refuses_a_path_that_is_not_a_listed_drive() {
    let dir = setup("refuse");
    let img_path = dir.join("disk.img");
    let mut img = vec![0u8; 4096];
    img[510] = 0x55;
    img[511] = 0xAA;
    std::fs::write(&img_path, &img).unwrap();
    let not_a_drive = dir.join("notes.txt");
    std::fs::write(&not_a_drive, vec![1u8; 8192]).unwrap();

    let out = flasher(
        &dir,
        &[
            "write",
            img_path.to_str().unwrap(),
            not_a_drive.to_str().unwrap(),
            "--yes",
        ],
    );
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("not a listed removable drive"));
    assert_eq!(
        std::fs::read(&not_a_drive).unwrap(),
        vec![1u8; 8192],
        "an unlisted file was written"
    );

    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn checks_the_published_checksum_before_writing() {
    let dir = setup("sha");
    let img_path = dir.join("disk.img");
    let mut img = vec![7u8; 8192];
    img[510] = 0x55;
    img[511] = 0xAA;
    std::fs::write(&img_path, &img).unwrap();
    let drive = dir.join("stick.disk");
    let (img_s, drive_s) = (img_path.to_str().unwrap(), drive.to_str().unwrap());

    // Wrong hash on the command line: refused, drive untouched.
    let wrong = "ab".repeat(32);
    let out = flasher(
        &dir,
        &["write", img_s, drive_s, "--sha256", &wrong, "--yes"],
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success() && stderr.contains("does not match"),
        "{stderr}"
    );
    assert!(std::fs::read(&drive).unwrap().iter().all(|&b| b == 0));

    // The right one, found in a SHA256SUMS next to the image.
    let hash = flasher_core_hash(&img_path);
    std::fs::write(dir.join("SHA256SUMS"), format!("{hash}  disk.img\n")).unwrap();
    let out = flasher(&dir, &["write", img_s, drive_s, "--yes"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert!(
        stderr.contains("against SHA256SUMS beside it")
            && stderr.contains("not that it is genuine")
            && stderr.contains("matched SHA256SUMS beside it, so the download is intact"),
        "{stderr}"
    );
    std::fs::remove_dir_all(dir).ok();
}

fn flasher_core_hash(path: &std::path::Path) -> String {
    use std::sync::atomic::AtomicBool;
    libflasher::checksum::sha256_file(path, 0, &AtomicBool::new(false), &mut |_| {}).unwrap()
}

#[test]
fn restores_a_drive() {
    let dir = setup("restore");
    let drive = dir.join("stick.disk");
    let out = flasher(
        &dir,
        &[
            "restore",
            drive.to_str().unwrap(),
            "--label",
            "BACKUP",
            "--yes",
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(std::fs::read(&drive).unwrap().starts_with(b"MOCKFS BACKUP"));

    let out = flasher(
        &dir,
        &[
            "restore",
            drive.to_str().unwrap(),
            "--label",
            "no/slashes",
            "--yes",
        ],
    );
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot contain"));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn rejects_unknown_options() {
    let dir = setup("badopt");
    let out = flasher(&dir, &["list", "--frobnicate"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown option --frobnicate"));
    std::fs::remove_dir_all(dir).ok();
}
