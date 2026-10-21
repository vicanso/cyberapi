#!/usr/bin/env bash
# The app's icons, made from icons/cyberapi.png: 1024 × 1024, the rounded
# square transparent around it. Called by `make icons`; needs macOS (sips,
# iconutil) and python3.
#
#   resources/icons/icon.icns   macOS (cargo-bundle copies it into the .app):
#                               the square on Apple's icon grid, 824 of the
#                               1024 and centred, as large in the Dock as the
#                               other apps
#   resources/icons/*.png       Linux, the .deb's icons (32, 128, 256), and a
#                               512 one
#   resources/icons/icon.ico    Windows, compiled into the executable by
#                               build.rs (16 to 256)
#   cyberapi-web/www/icon.png   the page's
#
# All but the macOS one have the square nearly fill the canvas, as icons on
# those platforms do. BODY is the square's side in the source, centred in it;
# change it with the picture.
set -euo pipefail
cd "$(dirname "$0")/.."

SOURCE=icons/cyberapi.png
BODY=842
ICONS=resources/icons

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

resize() {
  sips -z "$1" "$1" "$2" --out "$3" >/dev/null
}

# macOS: 1024 × 824 / BODY, padded back to 1024 with transparency.
SCALED=$(( (1024 * 824 + BODY / 2) / BODY ))
resize "$SCALED" "$SOURCE" "$WORK/scaled.png"
sips -p 1024 1024 "$WORK/scaled.png" --out "$WORK/mac.png" >/dev/null
SET="$WORK/icon.iconset"
mkdir -p "$SET"
for size in 16 32 128 256 512; do
  resize "$size" "$WORK/mac.png" "$SET/icon_${size}x${size}.png"
  resize $(( size * 2 )) "$WORK/mac.png" "$SET/icon_${size}x${size}@2x.png"
done
iconutil -c icns "$SET" -o "$ICONS/icon.icns"

# The rest: the square alone, with a 2% margin.
OFFSET=$(( (1024 - BODY) / 2 ))
MARGIN=$(( BODY / 50 ))
sips -c "$BODY" "$BODY" --cropOffset "$OFFSET" "$OFFSET" "$SOURCE" --out "$WORK/body.png" >/dev/null
sips -p $(( BODY + 2 * MARGIN )) $(( BODY + 2 * MARGIN )) "$WORK/body.png" --out "$WORK/full.png" >/dev/null

resize 32 "$WORK/full.png" "$ICONS/32x32.png"
resize 128 "$WORK/full.png" "$ICONS/128x128.png"
resize 256 "$WORK/full.png" "$ICONS/128x128@2x.png"
resize 512 "$WORK/full.png" "$ICONS/icon.png"
resize 128 "$WORK/full.png" cyberapi-web/www/icon.png

# Windows: an .ico of PNG images, which Windows reads since Vista.
for size in 16 24 32 48 64 128 256; do
  resize "$size" "$WORK/full.png" "$WORK/ico-$size.png"
done
python3 - "$ICONS/icon.ico" "$WORK"/ico-*.png <<'EOF'
import struct
import sys

out, paths = sys.argv[1], sys.argv[2:]
images = []
for path in paths:
    with open(path, "rb") as file:
        data = file.read()
    # The PNG's IHDR: width and height, big-endian, after the signature.
    width, height = struct.unpack(">II", data[16:24])
    images.append((width, height, data))
images.sort()

offset = 6 + 16 * len(images)
entries, blobs = b"", b""
for width, height, data in images:
    # A side of 256 is written as 0.
    entries += struct.pack("<BBBBHHII", width % 256, height % 256, 0, 0, 1, 32, len(data), offset)
    offset += len(data)
    blobs += data
with open(out, "wb") as file:
    file.write(struct.pack("<HHH", 0, 1, len(images)) + entries + blobs)
EOF

ls -1 "$ICONS"/icon.icns "$ICONS"/*.png "$ICONS"/icon.ico cyberapi-web/www/icon.png
