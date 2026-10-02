# Flasher

Write disk images to USB drives and SD cards: Raspberry Pi OS, Linux
distributions, any `.img`, `.iso`, `.img.xz`. On macOS, Linux and Windows, in a
window or in the terminal.

```
flasher                      open the window
flasher list                 removable drives
flasher write pi.img.xz /dev/disk4
flasher restore /dev/disk4   back to an ordinary exFAT drive
flasher --help
```

Checks the image against its published SHA-256 before writing (found
automatically next to the download), writes and verifies, keeps the computer
awake, notices a stalled or unplugged drive, and remembers its settings.

Built on [libflasher](https://github.com/developer180527/libflasher) (MIT)
and [libgui](https://github.com/developer180527/libgui).

## Building

Flasher expects a libflasher checkout next to it:

```
git clone https://github.com/developer180527/libflasher
git clone https://github.com/developer180527/Flasher
cd Flasher && cargo run --release
```
