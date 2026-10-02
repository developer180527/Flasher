#!/bin/sh
# Build Flasher.app and the disk image it is installed from:
#   target/dist/Flasher-<version>-macos-<universal|arm64|x86_64>.dmg
#
# A universal app (Apple silicon + Intel) when both Rust targets are
# installed (`rustup target add x86_64-apple-darwin aarch64-apple-darwin`),
# otherwise this Mac's architecture alone.
#
# Signing: ad hoc by default, which runs on the Mac that built it and, after
# a right-click → Open, on others. For a release, set
#   APPLE_SIGN_IDENTITY  "Developer ID Application: Name (TEAMID)"
#   NOTARY_PROFILE       a `xcrun notarytool store-credentials` profile
# and the app is signed with the hardened runtime, and the disk image signed,
# notarized and stapled.
#
# Laying out the disk image window drives Finder through AppleScript; the
# first run asks permission for the terminal to control Finder.
set -eu
cd "$(dirname "$0")/../.."

version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
work=target/macos-package
dist=target/dist
volume=Flasher
rm -rf "$work"
mkdir -p "$work" "$dist"

# ---- the binary --------------------------------------------------------------

installed=$(rustup target list --installed 2>/dev/null || true)
bins=""
archs=""
for target in aarch64-apple-darwin x86_64-apple-darwin; do
  if echo "$installed" | grep -qx "$target"; then
    cargo build --release ${CARGO_FLAGS:-} -p flasher --bin flasher --target "$target"
    bins="$bins target/$target/release/flasher"
    archs="$archs ${target%%-*}"
  fi
done
if [ -z "$bins" ]; then
  cargo build --release ${CARGO_FLAGS:-} -p flasher --bin flasher
  bins=target/release/flasher
  archs=$(uname -m)
fi
case "$archs" in
  *aarch64*x86_64*) suffix=universal ;;
  *aarch64* | *arm64*) suffix=arm64 ;;
  *) suffix=x86_64 ;;
esac

# ---- Flasher.app -------------------------------------------------------------

app=$work/Flasher.app
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
# Named "Flasher": macOS titles the app menu (About, Hide, Quit) after the
# executable.
# shellcheck disable=SC2086 # one word per binary
lipo -create $bins -output "$app/Contents/MacOS/Flasher"
sed "s/@VERSION@/$version/g" packaging/macos/Info.plist >"$app/Contents/Info.plist"
printf 'APPL????' >"$app/Contents/PkgInfo"
cp assets/icons/flasher.icns "$app/Contents/Resources/"
cp assets/Inter-OFL.txt "$app/Contents/Resources/"

if [ -n "${APPLE_SIGN_IDENTITY:-}" ]; then
  codesign --force --options runtime --timestamp --sign "$APPLE_SIGN_IDENTITY" "$app"
else
  codesign --force --sign - "$app"
fi
codesign --verify --strict "$app"

# ---- the disk image ----------------------------------------------------------

if [ -e "/Volumes/$volume" ]; then
  echo "/Volumes/$volume is already mounted; eject it first" >&2
  exit 1
fi

stage=$work/stage
mkdir -p "$stage/.background"
cp -R "$app" "$stage/"
ln -s /Applications "$stage/Applications"
cp packaging/macos/dmg-background.tiff "$stage/.background/background.tiff"

# Writable first, so Finder can store the window's layout in it; room to
# spare for that.
size_mb=$(($(du -sm "$stage" | cut -f1) + 20))
hdiutil create -quiet -srcfolder "$stage" -volname "$volume" -fs HFS+ \
  -format UDRW -size "${size_mb}m" "$work/rw.dmg"
device=$(hdiutil attach -readwrite -noverify -noautoopen "$work/rw.dmg" |
  awk '/^\/dev\// { print $1; exit }')
detach() { hdiutil detach "$device" -quiet || hdiutil detach "$device" -force -quiet; }
trap detach EXIT

# The window: icon view, no toolbar, the background, the app on the left and
# Applications on the right, where the background's arrow points. Positions
# are icon centres, matching make-artwork.swift.
layout() {
  osascript <<EOF
tell application "Finder"
  tell disk "$volume"
    open
    set current view of container window to icon view
    set toolbar visible of container window to false
    set statusbar visible of container window to false
    -- 660 × 420 of content under a 32-point title bar.
    set the bounds of container window to {200, 120, 860, 572}
    set opts to the icon view options of container window
    set arrangement of opts to not arranged
    set icon size of opts to 128
    set text size of opts to 13
    set background picture of opts to file ".background:background.tiff"
    set position of item "Flasher.app" of container window to {165, 215}
    set position of item "Applications" of container window to {495, 215}
    close
    open
    update without registering applications
    delay 2
    close
  end tell
end tell
EOF
}
# Finder is sometimes slow to see a newly attached volume.
layout || { sleep 3; layout; }

# The volume's own icon, shown in Finder's sidebar and on the desktop.
# Added only now: `hdiutil create -srcfolder` leaves .VolumeIcon.icns out,
# and Finder deletes it while laying the window out.
cp assets/icons/flasher.icns "/Volumes/$volume/.VolumeIcon.icns"
if setfile=$(xcrun -f SetFile 2>/dev/null); then
  "$setfile" -a C "/Volumes/$volume"
fi

# Wait for Finder to write the layout, then seal the image.
i=0
while [ ! -f "/Volumes/$volume/.DS_Store" ] && [ $i -lt 20 ]; do
  sleep 0.5
  i=$((i + 1))
done
rm -rf "/Volumes/$volume/.fseventsd"
chmod -Rf go-w "/Volumes/$volume" || true
sync
detach
trap - EXIT

dmg=$dist/Flasher-$version-macos-$suffix.dmg
rm -f "$dmg"
hdiutil convert -quiet "$work/rw.dmg" -format ULFO -o "$dmg"

if [ -n "${APPLE_SIGN_IDENTITY:-}" ]; then
  codesign --force --timestamp --sign "$APPLE_SIGN_IDENTITY" "$dmg"
  if [ -n "${NOTARY_PROFILE:-}" ]; then
    xcrun notarytool submit "$dmg" --keychain-profile "$NOTARY_PROFILE" --wait
    xcrun stapler staple "$dmg"
  fi
fi

echo "$dmg"
