#!/usr/bin/env bash
# Build, sign, notarize and publish Courly for macOS (Apple silicon).
#
# Installed copies update themselves from the GitHub release this script
# creates (crates/app/src/updater.rs reads latest.json from it).
#
# Usage: scripts/release-macos.sh ["one-line release notes"]
#
# Bump `version` in the workspace Cargo.toml and push that commit first:
# the script refuses to publish a dirty tree or an existing tag.
#
# Needs: gh (write access to the repo), cargo-about, the Developer ID
# certificate in the login keychain, the App Store Connect API key used for
# notarization, and the updater signing key (backup in 1Password, vault
# Soflution).
set -euo pipefail

cd "$(dirname "$0")/.."

REPO="Soflutionltd/courly"
APP_NAME="Courly"
BUNDLE_ID="com.soflution.mail"
TOOLCHAIN="+1.99.0"
SIGN_IDENTITY="${SIGN_IDENTITY:-Developer ID Application: Soflution ltd (TB8CC687M3)}"
UPDATER_KEY="${UPDATER_KEY:-$HOME/.tauri/courly_updater.key}"
NOTARY_KEY_ID="${NOTARY_KEY_ID:-77ZLGAGK7W}"
NOTARY_ISSUER="${NOTARY_ISSUER:-ab5aeb0f-f6b3-43e8-bf3f-5ed0dc325471}"
NOTARY_KEY="${NOTARY_KEY:-$HOME/.appstoreconnect/private_keys/AuthKey_${NOTARY_KEY_ID}.p8}"
NOTES="${1:-}"

die() {
  echo "error: $*" >&2
  exit 1
}

[[ "$(uname -s)" == "Darwin" && "$(uname -m)" == "arm64" ]] || die "run this on an Apple silicon Mac"
[[ -f "$UPDATER_KEY" ]] || die "updater key not found at $UPDATER_KEY"
[[ -f "$NOTARY_KEY" ]] || die "notarization key not found at $NOTARY_KEY"
gh auth status >/dev/null 2>&1 || die "gh is not authenticated"
git diff --quiet HEAD || die "uncommitted changes; commit and push first"

VERSION="$(cargo "$TOOLCHAIN" metadata --no-deps --format-version 1 |
  python3 -c 'import json, sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "runner"))')"
TAG="v${VERSION}"
COMMIT="$(git rev-parse HEAD)"
git fetch -q origin
git merge-base --is-ancestor "$COMMIT" origin/main || die "HEAD is not pushed to origin/main"
if gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1; then
  die "$TAG is already published; bump the version in Cargo.toml"
fi
[[ -n "$NOTES" ]] || NOTES="${APP_NAME} ${VERSION}"

OUT="target/release-macos"
APP="${OUT}/${APP_NAME}.app"
ARCHIVE="${OUT}/${APP_NAME}.app.tar.gz"
MANIFEST="${OUT}/latest.json"
rm -rf "$OUT"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

echo "==> Building ${APP_NAME} ${VERSION}"
cargo "$TOOLCHAIN" build --release --locked -p runner
cp target/release/courly "$APP/Contents/MacOS/courly"

echo "==> Bundling"
ICONSET="${OUT}/AppIcon.iconset"
mkdir -p "$ICONSET"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" assets/icon.png --out "${ICONSET}/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z "$double" "$double" assets/icon.png --out "${ICONSET}/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"
rm -rf "$ICONSET"
for notice in LICENSE LICENSE-APACHE NOTICE; do cp "$notice" "$APP/Contents/Resources/$notice"; done
cargo about generate --manifest-path crates/runner/Cargo.toml --locked about.hbs \
  -o "$APP/Contents/Resources/THIRD-PARTY-LICENSES.html"
mkdir -p "$APP/Contents/Resources/fonts"
for font_dir in crates/mail-html/fonts/*/; do
  cp "${font_dir}OFL.txt" "$APP/Contents/Resources/fonts/$(basename "$font_dir")-OFL.txt"
done

cat >"$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleDisplayName</key>
	<string>${APP_NAME}</string>
	<key>CFBundleExecutable</key>
	<string>courly</string>
	<key>CFBundleIconFile</key>
	<string>AppIcon</string>
	<key>CFBundleIdentifier</key>
	<string>${BUNDLE_ID}</string>
	<key>CFBundleName</key>
	<string>${APP_NAME}</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>${VERSION}</string>
	<key>CFBundleVersion</key>
	<string>${VERSION}</string>
	<key>LSApplicationCategoryType</key>
	<string>public.app-category.productivity</string>
	<key>LSMinimumSystemVersion</key>
	<string>13.0</string>
	<key>NSHighResolutionCapable</key>
	<true/>
</dict>
</plist>
PLIST

echo "==> Signing with ${SIGN_IDENTITY}"
codesign --force --options runtime --timestamp --sign "$SIGN_IDENTITY" "$APP"
codesign --verify --strict --verbose=2 "$APP"

echo "==> Notarizing"
NOTARIZE_ZIP="${OUT}/notarize.zip"
ditto -c -k --keepParent "$APP" "$NOTARIZE_ZIP"
xcrun notarytool submit "$NOTARIZE_ZIP" \
  --key "$NOTARY_KEY" --key-id "$NOTARY_KEY_ID" --issuer "$NOTARY_ISSUER" \
  --wait --timeout 30m >"${OUT}/notarize.log"
grep -q "status: Accepted" "${OUT}/notarize.log" || {
  cat "${OUT}/notarize.log" >&2
  die "notarization was not accepted"
}
rm -f "$NOTARIZE_ZIP"
xcrun stapler staple "$APP"
spctl --assess --type execute --verbose=2 "$APP" 2>&1 | grep -q "Notarized Developer ID" ||
  die "Gatekeeper does not accept the notarized app"

echo "==> Packaging the update"
COPYFILE_DISABLE=1 tar -czf "$ARCHIVE" -C "$OUT" "${APP_NAME}.app"
npx --yes @tauri-apps/cli@2.12.4 signer sign \
  --private-key-path "$UPDATER_KEY" --password "" \
  --app-version "$VERSION" "$ARCHIVE" >/dev/null
[[ -s "${ARCHIVE}.sig" ]] || die "updater signature was not written"

python3 - "$MANIFEST" "$VERSION" "$NOTES" "$(cat "${ARCHIVE}.sig")" \
  "https://github.com/${REPO}/releases/download/${TAG}/${APP_NAME}.app.tar.gz" <<'PY'
import datetime, json, sys
path, version, notes, signature, url = sys.argv[1:]
manifest = {
    "version": version,
    "notes": notes,
    "pub_date": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "platforms": {"darwin-aarch64": {"signature": signature, "url": url}},
}
with open(path, "w") as handle:
    json.dump(manifest, handle, indent=2)
PY

echo "==> Publishing ${TAG}"
gh release create "$TAG" "$ARCHIVE" "${ARCHIVE}.sig" "$MANIFEST" \
  --repo "$REPO" --target "$COMMIT" --title "${APP_NAME} ${VERSION}" --notes "$NOTES" --latest

echo "==> Checking the published manifest"
for attempt in 1 2 3 4 5 6 7 8 9 10; do
  published="$(curl -fsSL -H "Cache-Control: no-cache" \
    "https://github.com/${REPO}/releases/latest/download/latest.json?ts=$(date +%s)" |
    python3 -c 'import json, sys; print(json.load(sys.stdin)["version"])' || true)"
  if [[ "$published" == "$VERSION" ]]; then
    echo "Published ${APP_NAME} ${VERSION}: installed copies will offer the update."
    exit 0
  fi
  echo "latest.json still reports '${published}', retrying (${attempt}/10)"
  sleep 15
done
die "latest.json does not report ${VERSION} after publishing"
