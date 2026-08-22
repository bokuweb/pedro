#!/usr/bin/env bash
# Builds Pedro.app, the macOS bundle the binary has to live in to be an
# application rather than a process with a window.
#
# A bare executable has no name in the menu bar, cannot be told to come to the
# front, is not in the Dock, and cannot be double-clicked. Everything else here
# works without it — `cargo run -p pedro-app` opens the window — so this is what
# turns that window into something the rest of the system knows about.
#
# pdfium travels inside the bundle, in Contents/Frameworks, which is the second
# place pedro-pdf looks: an application dragged to /Applications has no vendor
# directory above it.
set -euo pipefail

PROFILE="${PROFILE:-release}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET="${CARGO_TARGET_DIR:-$ROOT/target}"
BUNDLE="$TARGET/Pedro.app"

if [ "$(uname -s)" != "Darwin" ]; then
  echo "Pedro.app is a macOS bundle; this is $(uname -s)" >&2
  exit 1
fi

case "$PROFILE" in
  release) cargo build --release -p pedro-app ;;
  debug)   cargo build -p pedro-app ;;
  *)
    echo "PROFILE must be release or debug, not $PROFILE" >&2
    exit 1
    ;;
esac

BINARY="$TARGET/$PROFILE/pedro"
if [ ! -x "$BINARY" ]; then
  echo "no binary at $BINARY" >&2
  exit 1
fi

rm -rf "$BUNDLE"
mkdir -p "$BUNDLE/Contents/MacOS" "$BUNDLE/Contents/Resources" "$BUNDLE/Contents/Frameworks"

cp "$BINARY" "$BUNDLE/Contents/MacOS/Pedro"

# The version the workspace says, so the bundle cannot drift from it. Asked of
# cargo rather than read out of a manifest, because the crate's own manifest
# says `version.workspace = true` and grepping it finds nothing.
VERSION="$(cargo metadata --no-deps --format-version 1 |
  sed -n 's/.*"name":"pedro-app","version":"\([^"]*\)".*/\1/p' | head -1)"
if [ -z "$VERSION" ]; then
  echo "could not read pedro-app's version from cargo metadata" >&2
  exit 1
fi

cat > "$BUNDLE/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>Pedro</string>
    <key>CFBundleDisplayName</key>
    <string>Pedro</string>
    <key>CFBundleExecutable</key>
    <string>Pedro</string>
    <key>CFBundleIdentifier</key>
    <string>com.bokuweb.pedro</string>
    <key>CFBundleVersion</key>
    <string>$VERSION</string>
    <key>CFBundleShortVersionString</key>
    <string>$VERSION</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>LSMinimumSystemVersion</key>
    <string>13.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>CFBundleDocumentTypes</key>
    <array>
        <dict>
            <key>CFBundleTypeName</key>
            <string>PDF document</string>
            <key>CFBundleTypeRole</key>
            <string>Viewer</string>
            <key>LSItemContentTypes</key>
            <array>
                <string>com.adobe.pdf</string>
            </array>
        </dict>
    </array>
</dict>
</plist>
PLIST

if [ -f "$ROOT/vendor/pdfium/lib/libpdfium.dylib" ]; then
  cp "$ROOT/vendor/pdfium/lib/libpdfium.dylib" "$BUNDLE/Contents/Frameworks/"
else
  echo "warning: no vendor/pdfium/lib/libpdfium.dylib to bundle; run scripts/fetch-pdfium.sh" >&2
fi

# The embedding model too, or the reader loses searching by meaning simply by
# moving the application: it is found by looking above the working directory,
# and an app in /Applications has nothing above it. Optional, as it is
# everywhere else — 134MB is a lot to carry for someone who never searches.
if [ -d "$ROOT/vendor/embedding" ] && [ -f "$ROOT/vendor/embedding/model.safetensors" ]; then
  cp -R "$ROOT/vendor/embedding" "$BUNDLE/Contents/Resources/embedding"
else
  echo "note: no vendor/embedding to bundle, so the app will search by words" \
       "alone; run scripts/fetch-embedding.sh to include it" >&2
fi

# Ad-hoc, so the bundle runs on the machine that built it. A copy to give away
# needs a Developer ID and notarisation, which is not something a script can do.
codesign --force --deep --sign - "$BUNDLE" >/dev/null 2>&1 ||
  echo "warning: could not sign $BUNDLE" >&2

echo "$BUNDLE"
