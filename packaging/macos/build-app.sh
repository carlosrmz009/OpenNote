#!/bin/bash
# Builds OpenNote.app and a disk image from a release binary. Run on macOS.
#   packaging/macos/build-app.sh VERSION ARCH   (ARCH: arm64 or intel; names the dmg)
# Expects target/release/opennote. Writes out/OpenNote-VERSION-macos-ARCH.dmg.
set -euo pipefail
version="${1:?version}"
arch="${2:?arch}"
root="$(cd "$(dirname "$0")/../.." && pwd)"
app="$root/dist/OpenNote.app"
rm -rf "$root/dist" "$root/out"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$root/out"

cp "$root/target/release/opennote" "$app/Contents/MacOS/opennote"
cp -R "$root/assets" "$app/Contents/Resources/assets"
cp "$root/LICENSE" "$root/LICENSES.md" "$root/NOTICE.md" "$app/Contents/Resources/"

# The icon, at every size macOS asks for.
icons="$root/dist/OpenNote.iconset"
mkdir -p "$icons"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$root/packaging/icon.png" --out "$icons/icon_${size}x${size}.png" >/dev/null
  sips -z "$((size * 2))" "$((size * 2))" "$root/packaging/icon.png" --out "$icons/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$icons" -o "$app/Contents/Resources/OpenNote.icns"
rm -rf "$icons"

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>OpenNote</string>
  <key>CFBundleDisplayName</key><string>OpenNote</string>
  <key>CFBundleIdentifier</key><string>dev.opennote.app</string>
  <key>CFBundleExecutable</key><string>opennote</string>
  <key>CFBundleIconFile</key><string>OpenNote</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${version}</string>
  <key>CFBundleVersion</key><string>${version}</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>CFBundleDocumentTypes</key>
  <array>
    <dict>
      <key>CFBundleTypeName</key><string>Score</string>
      <key>CFBundleTypeRole</key><string>Viewer</string>
      <key>LSHandlerRank</key><string>Alternate</string>
      <key>CFBundleTypeExtensions</key>
      <array><string>musicxml</string><string>mxl</string><string>xml</string><string>mid</string><string>midi</string></array>
    </dict>
  </array>
</dict>
</plist>
PLIST

# Apple Silicon refuses to start code that is not signed at all. This signs it for the
# machine it runs on only; it is not a developer signature, so Gatekeeper still asks the
# first time (see the README).
codesign --force --deep --sign - "$app"

staging="$root/dist/dmg"
mkdir -p "$staging"
cp -R "$app" "$staging/"
ln -s /Applications "$staging/Applications"
hdiutil create -volname "OpenNote" -srcfolder "$staging" -ov -format UDZO \
  "$root/out/OpenNote-${version}-macos-${arch}.dmg"
