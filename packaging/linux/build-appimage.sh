#!/bin/bash
# Builds a single-file AppImage from a release binary. Run on Linux, ideally on an old
# distribution (the workflow uses Ubuntu 22.04) so it starts on newer ones too.
#   packaging/linux/build-appimage.sh VERSION
# Expects target/release/opennote. Writes out/OpenNote-linux-x86_64.AppImage.
set -euo pipefail
version="${1:?version}"
root="$(cd "$(dirname "$0")/../.." && pwd)"
appdir="$root/dist/OpenNote.AppDir"
rm -rf "$root/dist" "$root/out"
mkdir -p "$appdir/usr/bin" "$appdir/usr/share/opennote" "$root/out"

cp "$root/target/release/opennote" "$appdir/usr/bin/opennote"
cp -R "$root/assets" "$appdir/usr/share/opennote/assets"
cp "$root/LICENSE" "$root/LICENSES.md" "$root/NOTICE.md" "$appdir/usr/share/opennote/"
cp "$root/packaging/icon.png" "$appdir/opennote.png"

cat > "$appdir/opennote.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=OpenNote
Comment=See piano fingerings played by 3D hands
Exec=opennote %f
Icon=opennote
Categories=AudioVideo;Audio;Music;Education;
MimeType=application/vnd.recordare.musicxml+xml;application/vnd.recordare.musicxml;audio/midi;audio/x-midi;
Terminal=false
DESKTOP

cat > "$appdir/AppRun" <<'RUN'
#!/bin/sh
here="$(dirname "$(readlink -f "$0")")"
exec "$here/usr/bin/opennote" "$@"
RUN
chmod +x "$appdir/AppRun"

tool="$root/dist/appimagetool"
curl -fsSL -o "$tool" \
  "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage"
chmod +x "$tool"
# The runner has no FUSE, so the tool unpacks itself instead of mounting.
ARCH=x86_64 "$tool" --appimage-extract-and-run "$appdir" \
  "$root/out/OpenNote-linux-x86_64.AppImage"
