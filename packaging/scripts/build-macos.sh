#!/usr/bin/env bash
# Build a macOS DMG package for wellfeather.
#
# Usage:
#   ./build-macos.sh
#   ./build-macos.sh --sign "Developer ID Application: Name (TEAMID)"
#   ./build-macos.sh --sign "..." --notarize --apple-id you@example.com --team-id TEAMID
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SIGN=0
CERT_NAME=""
NOTARIZE=0
APPLE_ID=""
TEAM_ID=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --sign)       SIGN=1; CERT_NAME="$2"; shift 2 ;;
        --notarize)   NOTARIZE=1; shift ;;
        --apple-id)   APPLE_ID="$2"; shift 2 ;;
        --team-id)    TEAM_ID="$2"; shift 2 ;;
        *) echo "Unknown option: $1" >&2; exit 1 ;;
    esac
done

if [[ $NOTARIZE -eq 1 ]]; then
    [[ $SIGN -eq 1 ]] || { echo "error: --notarize requires --sign" >&2; exit 1; }
    [[ -n "$APPLE_ID" ]] || { echo "error: --notarize requires --apple-id" >&2; exit 1; }
    [[ -n "$TEAM_ID" ]] || { echo "error: --notarize requires --team-id" >&2; exit 1; }
fi

echo "=== macOS DMG Package ==="

cd "$REPO_ROOT"

# Build release binary
echo "Building release binary..."
cargo build --release

# Read version from Cargo.toml
VERSION=$(grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)".*/\1/')
echo "Version: $VERSION"

TEMP="$REPO_ROOT/packaging/temp/macos"
APP_DIR="$TEMP/wellfeather.app"
CONTENTS="$APP_DIR/Contents"
MACOS_DIR="$CONTENTS/MacOS"
RESOURCES_DIR="$CONTENTS/Resources"
STAGING="$TEMP/staging"
OUTPUT="$REPO_ROOT/packaging/output"

mkdir -p "$MACOS_DIR" "$RESOURCES_DIR" "$STAGING" "$OUTPUT"

# Inject version into Info.plist
sed "s/{VERSION}/$VERSION/g" "$REPO_ROOT/packaging/macos/Info.plist" \
    > "$CONTENTS/Info.plist"

# Copy binary
cp "$REPO_ROOT/target/release/wellfeather" "$MACOS_DIR/wellfeather"
chmod +x "$MACOS_DIR/wellfeather"

# Convert icon.png → AppIcon.icns
ICON_SRC="$REPO_ROOT/app/assets/icon.png"
if [[ ! -f "$ICON_SRC" ]]; then
    echo "error: icon not found at $ICON_SRC" >&2
    echo "Place a 1024×1024 PNG at app/assets/icon.png before packaging." >&2
    exit 1
fi

ICONSET="$RESOURCES_DIR/AppIcon.iconset"
mkdir -p "$ICONSET"
sips -z 16   16   "$ICON_SRC" --out "$ICONSET/icon_16x16.png"
sips -z 32   32   "$ICON_SRC" --out "$ICONSET/icon_16x16@2x.png"
sips -z 32   32   "$ICON_SRC" --out "$ICONSET/icon_32x32.png"
sips -z 64   64   "$ICON_SRC" --out "$ICONSET/icon_32x32@2x.png"
sips -z 128  128  "$ICON_SRC" --out "$ICONSET/icon_128x128.png"
sips -z 256  256  "$ICON_SRC" --out "$ICONSET/icon_128x128@2x.png"
sips -z 256  256  "$ICON_SRC" --out "$ICONSET/icon_256x256.png"
sips -z 512  512  "$ICON_SRC" --out "$ICONSET/icon_256x256@2x.png"
sips -z 512  512  "$ICON_SRC" --out "$ICONSET/icon_512x512.png"
sips -z 1024 1024 "$ICON_SRC" --out "$ICONSET/icon_512x512@2x.png"
iconutil --convert icns "$ICONSET" --output "$RESOURCES_DIR/AppIcon.icns"

# Optional code signing for .app
if [[ $SIGN -eq 1 ]]; then
    echo "Signing .app bundle..."
    codesign --deep --force --options runtime --sign "$CERT_NAME" "$APP_DIR"
fi

# Build staging dir with .app + /Applications symlink
cp -r "$APP_DIR" "$STAGING/wellfeather.app"
ln -sfn /Applications "$STAGING/Applications"

# Create DMG
DMG_PATH="$OUTPUT/wellfeather-$VERSION-macos.dmg"
rm -f "$DMG_PATH"
echo "Creating DMG with hdiutil..."
hdiutil create -size 200m -fs HFS+ -volname wellfeather \
    -srcfolder "$STAGING" -ov -format UDZO "$DMG_PATH"

# Optional DMG signing
if [[ $SIGN -eq 1 ]]; then
    echo "Signing DMG..."
    codesign --sign "$CERT_NAME" "$DMG_PATH"
fi

# Optional notarization
if [[ $NOTARIZE -eq 1 ]]; then
    echo "Submitting to Apple Notary Service..."
    xcrun notarytool submit "$DMG_PATH" \
        --apple-id "$APPLE_ID" --team-id "$TEAM_ID" --wait
    echo "Stapling notarization ticket..."
    xcrun stapler staple "$DMG_PATH"
fi

SIZE_MB=$(du -m "$DMG_PATH" | cut -f1)
echo ""
echo "✓ Package ready: $DMG_PATH (${SIZE_MB} MB)"
