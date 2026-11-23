#!/bin/sh
# Renders every icon size from the one vector source, editors/vscode/media/icon.svg.
# Needs Google Chrome (headless SVG render, keeps transparency) and ImageMagick.
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
SVG="$ROOT/editors/vscode/media/icon.svg"
CHROME=${CHROME:-"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"}
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

cat >"$TMP/r.html" <<EOF
<html><body style="margin:0;background:transparent"><img src="file://$SVG" style="width:1024px;height:1024px;display:block"></body></html>
EOF
"$CHROME" --headless=new --disable-gpu --hide-scrollbars --default-background-color=00000000 \
  --allow-file-access-from-files --window-size=1024,1024 --screenshot="$TMP/full.png" "file://$TMP/r.html" 2>/dev/null

# VS Code extension: full bleed, 256 px so the marketplace shows it sharp on HiDPI.
magick "$TMP/full.png" -filter Lanczos -resize 256x256 -strip "$ROOT/editors/vscode/media/icon.png"

# macOS app: Apple's icon grid puts the 824 px tile in a 1024 px canvas.
magick "$TMP/full.png" -filter Lanczos -resize 824x824 -background none -gravity center -extent 1024x1024 -strip \
  "$ROOT/apps/ghit-gui/icon.png"

echo "rendered: editors/vscode/media/icon.png (256), apps/ghit-gui/icon.png (1024)"
