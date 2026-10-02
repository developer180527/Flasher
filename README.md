# Flasher

# # \![ISO Flasher App Interface](Flasher/miscellaneous/img.png)

# Write disk images to USB drives and SD cards, Raspberry Pi OS, Linux
distributions, Windows installers, any `.img`, `.iso`, `.img.xz`, on macOS,
Linux and Windows, in a window or in the terminal.

```
flasher                          open the window
flasher list                     removable drives
flasher inspect image.iso        what it is, and how it would be written
flasher write pi.img.xz /dev/disk4
flasher verify pi.img.xz /dev/disk4
flasher restore /dev/disk4       back to an ordinary exFAT drive
flasher --help
```
Built on [libflasher](https://github.com/developer180527/libflasher) (the
engine, MIT) and [libgui](https://github.com/developer180527/libgui) (the UI).

## Installing

From the [releases](https://github.com/developer180527/Flasher/releases):

- **macOS 11+**: `Flasher-\<version>-macos-universal.dmg`. Open it and drag
  Flasher onto Applications. Builds not yet signed with a Developer ID need a
  right-click → Open the first time. The terminal commands are `
  /Applications/Flasher.app/Contents/MacOS/Flasher list` and so on.
- **Windows 10/11**: `Flasher-\<version>-windows-x64-setup.exe` installs it for
  everyone or just you, with a Start menu entry and, if ticked, the terminal
  commands on `PATH`. The `\-portable.zip` is the same program without
  installing. Flasher asks for administrator rights when its window opens, as
  writing a disk needs them.
- **Debian, Ubuntu** (12 / 22.04 and newer): `sudo apt install
  ./flasher_\<version>_amd64.deb` (or `_arm64`). It installs the disk helper and
  its password prompt properly. On a server, add `\--no-install-recommends` and
  use the terminal commands.
- **Other Linux**: `Flasher-\<version>-linux-x86_64.AppImage` (or `aarch64`): `
  chmod +x` it and run it. Writing a disk asks for a password through a generic
  prompt, and runs the helper from your private runtime directory; the `.deb`
  does both properly. The `.tar.gz` holds the same files as the `.deb`, laid out
  under `usr/`.

Each release lists every file's SHA-256 in `SHA256SUMS`.

## What it does

- **Flash**: pick a drive and an image (Browse… or drop it on the window). The
  SHA-256 fills itself in from a `SHA256SUMS` / `.sha256` next to the image and
  is checked before the drive is touched. Disk images and hybrid ISOs are
  written byte for byte; other ISOs (Windows installers) are extracted onto
  FAT32, with an oversized `install.wim` split. Anything that cannot work is
  explained before anyone is asked to erase a drive.
- **Restore drive**: turn a flashed drive back into one exFAT volume.
- **Options**: verify, eject, checksum checking, keep the computer awake,
  automatic drive list, System / Light / Dark appearance, all remembered.
- **While working**: phase, amounts, speed and time left (labelled when
  estimated), a warning if the drive stops responding, a clear message if it is
  unplugged, green on success.
- **Safety**: the drive list follows drives coming and going; a confirmation is
  withdrawn if its drive changes; every erase names the drive and its path; the
  system disk is never offered.

## Architecture

```
flasher (binary)            no arguments → the window; a command → flasher_cli
├── main.rs                 host: winit window, wgpu surface, frame loop, file dialog,
│                           clipboard, OS light/dark; on Windows, GUI subsystem + UAC relaunch
├── app.rs                  App: all state and UI, with no window or GPU knowledge
│                           Stage: Idle → Confirm → Running(Job) → Finished;
│                           jobs run on worker threads and report through channels
├── widgets.rs              notebook tabs, titled group box, middle-ellipsis label,
│                           built on libgui's public API; candidates to move into libgui
├── prefs.rs                settings as TOML in the OS config folder, saved atomically
├── headless.rs             drives App with no window: clicks by control name, PNG snapshots
├── windows.rs              console attach, elevation check, ShellExecute "runas"
└── bin/libflasher-helper   Linux: the pkexec helper (all of it is libflasher's)
flasher_cli                 list / inspect / write / verify / restore, on the same libflasher calls
build.rs, windows.manifest  Windows: icon, version information, DPI awareness in flasher.exe
packaging/macos             Info.plist, dmg background; package.sh → .app and .dmg
packaging/windows           Inno Setup script and images; build.ps1 → installer and .zip
packaging/linux             desktop entry, AppStream data, polkit policy, AppRun;
                            package.sh → .deb, AppImage, .tar.gz
packaging/artwork           make-artwork.swift draws the icon and package images
assets/icons                the icon as .icns, .ico and PNGs (generated, committed)
scripts/                    pin-libflasher.sh, relock.sh, check-lock.sh
```
**Boundaries.** `App` takes a libgui `Ui` and produces a frame; it never sees a
window or a GPU, so the same code runs in the window and in headless tests. All
disk work is libflasher's: the app decides *when* and shows *what*. The GUI and
CLI describe progress with libflasher's `StatusLine`, so both use the same words.

**Privilege.** macOS: libflasher asks through `authopen`, the system password
prompt. Linux: `pkexec libflasher-helper` opens the disk and hands the open file
back; Flasher itself never runs as root. Windows: the window relaunches itself
elevated through UAC; terminal commands never elevate on their own.

## How it is tested

The GUI is tested with no window, GPU or drive: `Headless` builds real frames,
clicks controls through libgui's own hit-testing, and renders PNGs with
libgui's CPU renderer, against libflasher's mock drives (files with injected
faults). 41 tests cover flashing; every refusal; unplugging mid-write; another
drive taking the same path; stalls; checksums found, wrong and malformed;
hot-plug; keep-awake; restore; extract refusals; appearance; button placement;
and remembered settings. CI runs them on macOS, Linux and Windows and keeps the
screenshots.

The disk backends are tested in libflasher's CI, down to booting extracted ISOs
in QEMU.

## Building

```
git clone https://github.com/developer180527/Flasher
cd Flasher && cargo run --release
```
Packages, each built on its own OS, into `target/dist/`:

```
packaging/macos/package.sh                  .dmg (universal if both Rust targets are installed)
packaging\windows\build.ps1                 installer and .zip (needs Inno Setup 6)
packaging/linux/package.sh [deb appimage tar]
```
The Release workflow runs all three in CI: by hand for packages to try, or on a `
v\<version>` tag to draft a GitHub release. macOS signing and notarization are
optional, through the secrets listed in `.github/workflows/release.yml`. The
icon and package images are drawn by `packaging/artwork/make-artwork.sh`
(macOS); its output is committed.

Flasher builds the libflasher commit pinned in `Cargo.toml`. To work on both,
clone libflasher next to Flasher and copy `.cargo/config.toml.example` to `
.cargo/config.toml`. Run `scripts/relock.sh` before committing so `Cargo.lock`
keeps the pin (CI checks). Move the pin with `scripts/pin-libflasher.sh \[commit]`
; without a commit it takes libflasher's latest `main`.

## State

Done: everything above. About 3,200 lines on top of libflasher's 7,900.

**Not yet**, most important first:

- **Signing**: packages exist for all three systems, but are not yet signed with
  a Developer ID (macOS) or a code-signing certificate (Windows), so both
  systems warn on first launch. The release workflow signs and notarizes for
  macOS once its secrets are set.
- **Real-hardware testing**: a Raspberry Pi image and an Ubuntu ISO written on
  macOS so far. Nothing on Linux or Windows; extract mode never on a real
  stick; the Windows UAC relaunch never run.
- **Accessibility**: libgui has no screen-reader support yet.
- Smaller: a better name for sticks whose model is a placeholder
  ("ProductCode"); Restore shows indeterminate progress; no multi-drive
  flashing or download-from-URL.
- **Licence**: not chosen yet.

## Licence

Not chosen yet. libflasher is MIT; libgui is MIT OR Apache-2.0.

