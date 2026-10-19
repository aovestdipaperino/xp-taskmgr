#!/bin/bash
# Builds "Task Manager.app" and a drag-to-install DMG in dist/.
#
#   scripts/bundle.sh            # release build, ad-hoc signed
#   SIGN_ID="Developer ID Application: ..." scripts/bundle.sh
set -euo pipefail

cd "$(dirname "$0")/.."

APP_NAME="Task Manager"
BIN="xp-taskmgr"
BUNDLE_ID="com.aovestdipaperino.xp-taskmgr"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
SIGN_ID="${SIGN_ID:--}"

DIST="dist"
APP="$DIST/$APP_NAME.app"
DMG="$DIST/XP-Task-Manager-$VERSION.dmg"

echo "==> Building $BIN $VERSION (release)"
cargo build --release

if [[ ! -f assets/AppIcon.icns ]]; then
    echo "==> Rendering icon"
    ICONSET="$(mktemp -d)/AppIcon.iconset"
    python3 scripts/make_icon.py "$ICONSET"
    iconutil -c icns "$ICONSET" -o assets/AppIcon.icns
fi

echo "==> Assembling $APP"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "target/release/$BIN" "$APP/Contents/MacOS/$BIN"
cp assets/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>en</string>
    <key>CFBundleDisplayName</key>
    <string>$APP_NAME</string>
    <key>CFBundleExecutable</key>
    <string>$BIN</string>
    <key>CFBundleIconFile</key>
    <string>AppIcon</string>
    <key>CFBundleIdentifier</key>
    <string>$BUNDLE_ID</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>$APP_NAME</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>$VERSION</string>
    <key>CFBundleVersion</key>
    <string>$VERSION</string>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.utilities</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSHumanReadableCopyright</key>
    <string>Copyright © $(date +%Y) Enzo Lombardi</string>
</dict>
</plist>
PLIST

echo "==> Signing ($SIGN_ID)"
codesign --force --deep --options runtime --sign "$SIGN_ID" "$APP"
codesign --verify --strict "$APP"

echo "==> Creating $DMG"
STAGE="$(mktemp -d)"
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
rm -f "$DMG"
hdiutil create -volname "XP Task Manager" -srcfolder "$STAGE" -fs HFS+ -format UDZO -ov "$DMG" >/dev/null
rm -rf "$STAGE"

echo "==> Done"
ls -lh "$DMG"
