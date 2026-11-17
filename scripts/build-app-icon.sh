#!/usr/bin/env bash
# Regenerate every application icon from the brand artwork (default: Logo.png).
#
#   bash scripts/build-app-icon.sh [path/to/Logo.png]
#
# Outputs (committed, so CI needs no ImageMagick/librsvg):
#   src/assets/app_icon_1024.png  macOS master: white rounded tile 824x824 on a 1024 canvas
#                                 (Apple's icon grid, ~100 px transparent margin + soft shadow)
#   src/assets/app_icon.png       512 px copy of the master: window/Dock icon (egui overrides the
#                                 bundle icon at runtime), tray and Linux desktop icon
#   src/assets/app_icon.ico       Windows icon (16..256 px) from a flat variant with a thin margin
#
# Needs: rsvg-convert (librsvg), sips (macOS), python3 (stdlib only).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LOGO="$(cd "$(dirname "${1:-$ROOT/Logo.png}")" && pwd)/$(basename "${1:-$ROOT/Logo.png}")"
OUT="$ROOT/src/assets"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# Logo.png is 2048x2048 on a near-white background; the artwork's bounding box is about
# 1662x1198 px centred on the canvas. Scale it to ~74% of the tile width.
# Embed the (downscaled) logo as a data URI: rsvg-convert refuses external file references.
sips -s format png -z 1024 1024 "$LOGO" --out "$WORK/logo_1024.png" >/dev/null
LOGO_B64="$(base64 < "$WORK/logo_1024.png" | tr -d '\n')"

make_svg() { # $1 tile size  $2 corner radius  $3 shadow(1/0)
  local tile="$1" radius="$2" shadow="$3"
  local off=$(( (1024 - tile) / 2 ))
  local scale
  scale=$(python3 -c "print(round($tile * 0.74 / 1662, 5))")
  local img=$(python3 -c "print(round(2048 * $scale, 3))")
  local pos=$(python3 -c "print(round(512 - 1024 * $scale, 3))")
  cat <<SVG
<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="1024" height="1024" viewBox="0 0 1024 1024">
  <defs>
    <clipPath id="tile"><rect x="$off" y="$off" width="$tile" height="$tile" rx="$radius" ry="$radius"/></clipPath>
    <filter id="shadow" x="-20%" y="-20%" width="140%" height="150%">
      <feGaussianBlur in="SourceAlpha" stdDeviation="14"/>
      <feOffset dy="10" result="b"/>
      <feComponentTransfer><feFuncA type="linear" slope="0.28"/></feComponentTransfer>
    </filter>
    <!-- Push the logo's slightly noisy "white" (about 252/255) to pure white so no box shows. -->
    <filter id="white" color-interpolation-filters="sRGB">
      <feComponentTransfer>
        <feFuncR type="linear" slope="1.04"/><feFuncG type="linear" slope="1.04"/><feFuncB type="linear" slope="1.04"/>
      </feComponentTransfer>
    </filter>
  </defs>
  $( [ "$shadow" = 1 ] && echo "<rect x=\"$off\" y=\"$off\" width=\"$tile\" height=\"$tile\" rx=\"$radius\" ry=\"$radius\" fill=\"#000\" filter=\"url(#shadow)\"/>" )
  <rect x="$off" y="$off" width="$tile" height="$tile" rx="$radius" ry="$radius" fill="#fff"/>
  <g clip-path="url(#tile)">
    <image x="$pos" y="$pos" width="$img" height="$img" filter="url(#white)" xlink:href="data:image/png;base64,$LOGO_B64"/>
  </g>
</svg>
SVG
}

make_svg 824 185 1 > "$WORK/mac.svg"
make_svg 960 215 0 > "$WORK/flat.svg"
rsvg-convert -w 1024 -h 1024 "$WORK/mac.svg" -o "$OUT/app_icon_1024.png"
rsvg-convert -w 1024 -h 1024 "$WORK/flat.svg" -o "$WORK/flat_1024.png"
sips -s format png -z 512 512 "$OUT/app_icon_1024.png" --out "$OUT/app_icon.png" >/dev/null

# Windows .ico: PNG-compressed entries (supported since Vista).
for sz in 16 24 32 48 64 128 256; do
  sips -s format png -z "$sz" "$sz" "$WORK/flat_1024.png" --out "$WORK/ico_$sz.png" >/dev/null
done
python3 - "$WORK" "$OUT/app_icon.ico" <<'PY'
import struct, sys
work, out = sys.argv[1], sys.argv[2]
sizes = [16, 24, 32, 48, 64, 128, 256]
blobs = [open(f"{work}/ico_{s}.png", "rb").read() for s in sizes]
header = struct.pack("<HHH", 0, 1, len(sizes))
offset = 6 + 16 * len(sizes)
entries = b""
for s, b in zip(sizes, blobs):
    entries += struct.pack("<BBBBHHII", s % 256, s % 256, 0, 0, 1, 32, len(b), offset)
    offset += len(b)
open(out, "wb").write(header + entries + b"".join(blobs))
PY
echo "Wrote app_icon_1024.png, app_icon.png (512) and app_icon.ico in $OUT"
