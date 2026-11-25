#!/usr/bin/env bash
# Build the Courly installer disk image: a Finder window with the branded
# background, the app on the left and a link to Applications on the right.
#
# Usage: scripts/make-dmg.sh <path/to/Courly.app> <path/to/Courly.dmg>
#
# The image is neither signed nor notarized here; scripts/release-macos.sh
# does both. Needs create-dmg and rsvg-convert (brew install create-dmg
# librsvg).
set -euo pipefail

cd "$(dirname "$0")/.."

die() {
  echo "error: $*" >&2
  exit 1
}

[[ $# -eq 2 ]] || die "usage: scripts/make-dmg.sh <app> <dmg>"
APP="$1"
DMG="$2"
[[ -d "$APP" ]] || die "no app bundle at $APP"
command -v create-dmg >/dev/null || die "create-dmg is missing: brew install create-dmg"
command -v rsvg-convert >/dev/null || die "rsvg-convert is missing: brew install librsvg"

APP_NAME="$(basename "$APP")"
WORK="$(dirname "$DMG")/dmg-work"
rm -rf "$WORK" "$DMG"
mkdir -p "$WORK/stage"

# One TIFF holding the 1x and 2x renderings, so Retina screens get the
# sharp one. The SVG is 660 x 440 pt.
rsvg-convert -w 660 -h 440 assets/macos/dmg-background.svg -o "$WORK/background.png"
rsvg-convert -w 1320 -h 880 assets/macos/dmg-background.svg -o "$WORK/background@2x.png"
tiffutil -cathidpicheck "$WORK/background.png" "$WORK/background@2x.png" \
  -out "$WORK/background.tiff" >/dev/null 2>&1

ditto "$APP" "$WORK/stage/$APP_NAME"

# The window is 20 pt taller than the background's visible part needs, to
# leave room for the title bar and, when the user shows it, the path bar.
# Icon positions match the ones the background is drawn around.
create-dmg \
  --volname "${APP_NAME%.app}" \
  --volicon "$APP/Contents/Resources/AppIcon.icns" \
  --background "$WORK/background.tiff" \
  --window-pos 240 160 \
  --window-size 660 460 \
  --icon-size 128 \
  --text-size 13 \
  --icon "$APP_NAME" 165 205 \
  --hide-extension "$APP_NAME" \
  --app-drop-link 495 205 \
  --no-internet-enable \
  "$DMG" "$WORK/stage" >"$WORK/create-dmg.log" 2>&1 || {
  cat "$WORK/create-dmg.log" >&2
  die "create-dmg failed"
}

rm -rf "$WORK"
echo "Built $DMG"
