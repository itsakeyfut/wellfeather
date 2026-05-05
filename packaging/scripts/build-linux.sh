#!/usr/bin/env bash
# Build a Linux AppImage for wellfeather.
#
# Prerequisites:
#   - appimagetool in PATH (https://github.com/AppImage/AppImageKit/releases)
#   - imagemagick (optional — used for icon resizing; falls back to copy)
#
# Usage:
#   ./build-linux.sh
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

echo "=== Linux AppImage Package ==="

# Build release binary
echo "Building release binary..."
cargo build --release

# Read version from Cargo.toml
VERSION=$(grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)".*/\1/')
echo "Version: $VERSION"

TEMP="$REPO_ROOT/packaging/temp/linux"
APPDIR="$TEMP/AppDir"
OUTPUT="$REPO_ROOT/packaging/output"

mkdir -p \
    "$APPDIR/usr/share/applications" \
    "$APPDIR/usr/share/icons/hicolor" \
    "$OUTPUT"

# Copy binary
cp "$REPO_ROOT/target/release/wellfeather" "$APPDIR/wellfeather"
chmod +x "$APPDIR/wellfeather"

# Desktop entry
cp "$REPO_ROOT/packaging/linux/wellfeather.desktop" "$APPDIR/wellfeather.desktop"
cp "$REPO_ROOT/packaging/linux/wellfeather.desktop" \
    "$APPDIR/usr/share/applications/wellfeather.desktop"

# Icon assets
ICON_SRC="$REPO_ROOT/app/assets/icon.png"
if [[ ! -f "$ICON_SRC" ]]; then
    echo "error: icon not found at $ICON_SRC" >&2
    echo "Place a 1024×1024 PNG at app/assets/icon.png before packaging." >&2
    exit 1
fi

HAS_MAGICK=0
command -v magick &>/dev/null && HAS_MAGICK=1

for SIZE in 16 32 48 128 256 512; do
    ICON_DIR="$APPDIR/usr/share/icons/hicolor/${SIZE}x${SIZE}/apps"
    mkdir -p "$ICON_DIR"
    DEST="$ICON_DIR/wellfeather.png"
    if [[ $HAS_MAGICK -eq 1 ]]; then
        magick "$ICON_SRC" -resize "${SIZE}x${SIZE}" "$DEST"
    else
        echo "warning: ImageMagick not found; copying source PNG for ${SIZE}x${SIZE}"
        cp "$ICON_SRC" "$DEST"
    fi
    # appimagetool requires a root-level icon — use the 256px version
    if [[ $SIZE -eq 256 ]]; then
        cp "$DEST" "$APPDIR/wellfeather.png"
    fi
done

# Run appimagetool
if ! command -v appimagetool &>/dev/null; then
    echo "error: appimagetool not found in PATH." >&2
    echo "Download from https://github.com/AppImage/AppImageKit/releases" >&2
    exit 1
fi

APPIMAGE_PATH="$OUTPUT/wellfeather-$VERSION-x86_64.AppImage"
rm -f "$APPIMAGE_PATH"
echo "Running appimagetool..."
ARCH=x86_64 APPIMAGE_EXTRACT_AND_RUN=1 appimagetool "$APPDIR" "$APPIMAGE_PATH"

SIZE_MB=$(du -m "$APPIMAGE_PATH" | cut -f1)
echo ""
echo "✓ Package ready: $APPIMAGE_PATH (${SIZE_MB} MB)"
