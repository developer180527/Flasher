#!/bin/sh
# Regenerate Flasher's icons and package artwork from make-artwork.swift.
# macOS only (Swift, iconutil, sips, tiffutil). The outputs are committed,
# so packaging on any OS uses them as they are; run this only after
# changing the drawing.
#
#   assets/icons/flasher.icns           macOS app icon
#   assets/icons/flasher.ico            Windows exe and installer icon
#   assets/icons/flasher-256.png        the window icon on Windows and Linux
#   assets/icons/hicolor/…              Linux icon theme, 16–512 px
#   packaging/macos/dmg-background.tiff disk image background, 1x + 2x
#   packaging/windows/wizard-*.bmp      installer side images, 100 % + 200 %
set -e
cd "$(dirname "$0")/../.."
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

swift packaging/artwork/make-artwork.swift "$work/png" assets/Inter.ttf
png=$work/png
mkdir -p assets/icons

# macOS: an iconset of each size and its @2x, then iconutil.
set_=$work/flasher.iconset
mkdir -p "$set_"
for s in 16 32 128 256 512; do
  cp "$png/icon-mac-$s.png" "$set_/icon_${s}x${s}.png"
  cp "$png/icon-mac-$((s * 2)).png" "$set_/icon_${s}x${s}@2x.png"
done
iconutil -c icns "$set_" -o assets/icons/flasher.icns

# Windows: one .ico holding every size Explorer and the taskbar ask for,
# each a PNG (Windows Vista and later read those).
python3 - "$png" assets/icons/flasher.ico <<'EOF'
import struct, sys
src, out = sys.argv[1], sys.argv[2]
sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256]
images = [open(f"{src}/icon-flat-{s}.png", "rb").read() for s in sizes]
header = struct.pack("<HHH", 0, 1, len(sizes))
offset = 6 + 16 * len(sizes)
entries = b""
for s, data in zip(sizes, images):
    # Width and height of 256 are written as 0.
    entries += struct.pack("<BBBBHHII", s % 256, s % 256, 0, 0, 1, 32, len(data), offset)
    offset += len(data)
open(out, "wb").write(header + entries + b"".join(images))
EOF

cp "$png/icon-flat-256.png" assets/icons/flasher-256.png

# Linux: the hicolor icon theme, named after the app id.
for s in 16 24 32 48 64 128 256 512; do
  d=assets/icons/hicolor/${s}x${s}/apps
  mkdir -p "$d"
  cp "$png/icon-flat-$s.png" "$d/io.github.developer180527.flasher.png"
done

# The disk image background: one TIFF holding 1x and 2x, so Finder picks
# the sharp one on Retina screens.
tiffutil -cathidpicheck "$png/dmg-background.png" "$png/dmg-background@2x.png" \
  -out packaging/macos/dmg-background.tiff >/dev/null

# Inno Setup takes 24-bit BMPs (written by the Swift script), one per
# display scaling.
cp "$png"/wizard-*.bmp packaging/windows/

echo "artwork regenerated"
