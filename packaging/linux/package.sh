#!/bin/sh
# Build Flasher's Linux packages for this machine's architecture:
#   target/dist/flasher_<version>_<amd64|arm64>.deb          Debian, Ubuntu, …
#   target/dist/Flasher-<version>-linux-<arch>.AppImage      any distribution
#   target/dist/flasher-<version>-linux-<arch>.tar.gz        the files, /usr layout
#
#   packaging/linux/package.sh [deb] [appimage] [tar]   (default: all three)
#
# The binaries need the glibc they were built against or newer: build on
# the oldest system to support (CI uses Ubuntu 22.04, glibc 2.35, which
# covers Debian 12 and Ubuntu 22.04 onwards).
#
# The .deb needs dpkg-deb. The AppImage downloads appimagetool and the
# AppImage runtime once, at pinned versions checked against their SHA-256,
# into target/appimage-tools (or set APPIMAGETOOL to one already here).
# Extra cargo flags (CI uses --locked) go in CARGO_FLAGS; the .deb's
# maintainer in DEB_MAINTAINER.
set -eu
cd "$(dirname "$0")/../.."

want=${*:-deb appimage tar}
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
app_id=io.github.developer180527.flasher
maintainer=${DEB_MAINTAINER:-Venu Gopal <developer180527@users.noreply.github.com>}
case "$(uname -m)" in
  x86_64) arch=x86_64 deb_arch=amd64 ;;
  aarch64 | arm64) arch=aarch64 deb_arch=arm64 ;;
  *) echo "unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac
work=target/linux-package
dist=target/dist
rm -rf "$work"
mkdir -p "$work" "$dist"

cargo build --release ${CARGO_FLAGS:-} -p flasher --bins
bin=target/release

# ---- the files, laid out as installed under /usr ----------------------------

root=$work/root
install -Dm755 "$bin/flasher" "$root/usr/bin/flasher"
install -Dm755 "$bin/libflasher-helper" "$root/usr/libexec/libflasher-helper"
install -Dm644 packaging/linux/$app_id.policy "$root/usr/share/polkit-1/actions/$app_id.policy"
install -Dm644 packaging/linux/$app_id.desktop "$root/usr/share/applications/$app_id.desktop"
install -Dm644 packaging/linux/$app_id.metainfo.xml "$root/usr/share/metainfo/$app_id.metainfo.xml"
for dir in assets/icons/hicolor/*; do
  install -Dm644 "$dir/apps/$app_id.png" "$root/usr/share/icons/hicolor/$(basename "$dir")/apps/$app_id.png"
done
install -Dm644 assets/Inter-OFL.txt "$root/usr/share/doc/flasher/Inter-OFL.txt"
cat >"$root/usr/share/doc/flasher/copyright" <<EOF
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: Flasher
Source: https://github.com/developer180527/Flasher

Files: *
Copyright: Venu Gopal
License: no licence chosen yet; see the source repository

Files: usr/bin/flasher
Comment: embeds the Inter typeface
Copyright: The Inter Project Authors
License: OFL-1.1
 See /usr/share/doc/flasher/Inter-OFL.txt
EOF

# ---- .deb ---------------------------------------------------------------------

build_deb() {
  if ! command -v dpkg-deb >/dev/null; then
    echo "skipping the .deb: dpkg-deb not found" >&2
    return
  fi
  deb_root=$work/deb
  cp -R "$root" "$deb_root"
  # The newest glibc symbol version the binaries use is the oldest glibc
  # they run on.
  glibc=$(objdump -T "$bin/flasher" "$bin/libflasher-helper" 2>/dev/null |
    sed -n 's/.*GLIBC_\([0-9][0-9.]*\).*/\1/p' | sort -t. -k1,1n -k2,2n -u | tail -n 1)
  glibc=${glibc:-$(ldd --version | sed -n '1s/.* \([0-9][0-9.]*\)$/\1/p')}
  mkdir -p "$deb_root/DEBIAN"
  cat >"$deb_root/DEBIAN/control" <<EOF
Package: flasher
Version: $version
Architecture: $deb_arch
Maintainer: $maintainer
Installed-Size: $(du -sk "$deb_root/usr" | cut -f1)
Section: utils
Priority: optional
Homepage: https://github.com/developer180527/Flasher
Depends: libc6 (>= $glibc), libgcc-s1
Recommends: pkexec | policykit-1, exfatprogs, xdg-desktop-portal, libvulkan1 | libegl1, libxkbcommon0, libwayland-client0 | libx11-6
Description: write disk images to USB drives and SD cards
 Flasher writes Raspberry Pi OS, Linux distributions, Windows installers
 and any .img, .iso or compressed image to a USB drive or SD card, and
 reads it back to check it. It checks downloads against their published
 SHA-256, offers only removable drives, and can turn a flashed drive back
 into an ordinary exFAT drive.
 .
 A window, or terminal commands: flasher list, inspect, write, verify,
 restore. Disks are opened through pkexec and a small helper, so Flasher
 itself never runs as root. Without a desktop (a server), install with
 --no-install-recommends and use the terminal commands.
EOF
  out=$dist/flasher_${version}_${deb_arch}.deb
  dpkg-deb --root-owner-group -Zxz --build "$deb_root" "$out" >/dev/null
  echo "$out"
}

# ---- AppImage -----------------------------------------------------------------

# Pinned tools: appimagetool 1.9.1 and the type 2 runtime 20251108.
fetch() { # url sha256 file
  if [ ! -f "$3" ] || ! echo "$2  $3" | sha256sum -c --status; then
    curl -fsSL -o "$3.part" "$1"
    echo "$2  $3.part" | sha256sum -c --status || {
      echo "checksum mismatch: $1" >&2
      exit 1
    }
    mv "$3.part" "$3"
  fi
}

build_appimage() {
  tools=target/appimage-tools
  mkdir -p "$tools"
  case $arch in
    x86_64)
      tool_sum=ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0
      runtime_sum=2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d ;;
    aarch64)
      tool_sum=f0837e7448a0c1e4e650a93bb3e85802546e60654ef287576f46c71c126a9158
      runtime_sum=00cbdfcf917cc6c0ff6d3347d59e0ca1f7f45a6df1a428a0d6d8a78664d87444 ;;
  esac
  runtime=$tools/runtime-$arch
  fetch "https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-$arch" \
    "$runtime_sum" "$runtime"
  tool=${APPIMAGETOOL:-}
  if [ -z "$tool" ]; then
    tool=$tools/appimagetool-$arch.AppImage
    fetch "https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-$arch.AppImage" \
      "$tool_sum" "$tool"
    chmod +x "$tool"
  fi

  appdir=$work/Flasher.AppDir
  cp -R "$root" "$appdir"
  install -m755 packaging/linux/AppRun "$appdir/AppRun"
  cp packaging/linux/$app_id.desktop "$appdir/"
  cp assets/icons/hicolor/256x256/apps/$app_id.png "$appdir/"
  ln -sf $app_id.png "$appdir/.DirIcon"

  out=$dist/Flasher-$version-linux-$arch.AppImage
  # Extract-and-run: no FUSE needed where it is built (containers, CI).
  APPIMAGE_EXTRACT_AND_RUN=1 ARCH=$arch "$tool" --no-appstream \
    --runtime-file "$runtime" "$appdir" "$out" >/dev/null
  echo "$out"
}

# ---- tar.gz -------------------------------------------------------------------

build_tar() {
  name=flasher-$version-linux-$arch
  mkdir -p "$work/$name"
  cp -R "$root/usr" "$work/$name/"
  cat >"$work/$name/INSTALL" <<'EOF'
Flasher, laid out as installed under /usr. To install system-wide:

  sudo cp -R usr/. /usr/

libflasher-helper must end up in /usr/libexec, where the polkit policy
names it, so that the password prompt says what it is for. The window
needs a desktop session; on a server, use the terminal commands:
flasher list, inspect, write, verify, restore (flasher --help).
EOF
  out=$dist/$name.tar.gz
  tar -C "$work" -czf "$out" "$name"
  echo "$out"
}

for what in $want; do
  case $what in
    deb) build_deb ;;
    appimage) build_appimage ;;
    tar) build_tar ;;
    *) echo "unknown package: $what (deb, appimage, tar)" >&2; exit 2 ;;
  esac
done
