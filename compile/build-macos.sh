#!/usr/bin/env bash
# Builds a release binary of Openbar5000 for macOS, and packages it into a
# proper Openbar5000.app bundle (with the app icon) so Finder/Dock/Spotlight
# show it correctly -- a raw binary has no icon of its own outside the Dock
# while it's actually running.
#
# Usage:
#   ./compile/build-macos.sh              # build for the host architecture
#   ./compile/build-macos.sh --universal  # build a universal (x86_64 + arm64) binary
#
# Produces:
#   dist/macos/openbar5000       (raw binary)
#   dist/macos/Openbar5000.app   (app bundle, with icon)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
DIST_DIR="$REPO_ROOT/dist/macos"
APP_NAME="openbar5000"
VERSION="$(grep -m1 '^version' "$REPO_ROOT/Cargo.toml" | sed -E 's/version = "(.*)"/\1/')"
UNIVERSAL=false

for arg in "$@"; do
    case "$arg" in
        --universal) UNIVERSAL=true ;;
        *)
            echo "error: unknown option '$arg' (expected --universal)" >&2
            exit 1
            ;;
    esac
done

if ! command -v cargo >/dev/null 2>&1; then
    echo "error: cargo not found. Install Rust via https://rustup.rs first." >&2
    exit 1
fi

cd "$REPO_ROOT"
mkdir -p "$DIST_DIR"

if [ "$UNIVERSAL" = true ]; then
    echo "==> Building universal (x86_64 + arm64) release binary..."
    rustup target add x86_64-apple-darwin aarch64-apple-darwin >/dev/null
    cargo build --release -p app --target x86_64-apple-darwin
    cargo build --release -p app --target aarch64-apple-darwin
    lipo -create -output "$DIST_DIR/$APP_NAME" \
        "target/x86_64-apple-darwin/release/$APP_NAME" \
        "target/aarch64-apple-darwin/release/$APP_NAME"
else
    echo "==> Building release binary for the host architecture..."
    cargo build --release -p app
    cp "target/release/$APP_NAME" "$DIST_DIR/$APP_NAME"
fi

echo "==> Binary: $DIST_DIR/$APP_NAME"

echo "==> Packaging Openbar5000.app (version $VERSION)..."
BUNDLE_DIR="$DIST_DIR/Openbar5000.app"
rm -rf "$BUNDLE_DIR"
mkdir -p "$BUNDLE_DIR/Contents/MacOS" "$BUNDLE_DIR/Contents/Resources"

cp "$DIST_DIR/$APP_NAME" "$BUNDLE_DIR/Contents/MacOS/$APP_NAME"
cp "$SCRIPT_DIR/assets/macos/AppIcon.icns" "$BUNDLE_DIR/Contents/Resources/AppIcon.icns"

cat > "$BUNDLE_DIR/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>Openbar5000</string>
    <key>CFBundleDisplayName</key>
    <string>Openbar5000</string>
    <key>CFBundleIdentifier</key>
    <string>org.openbar5000.app</string>
    <key>CFBundleVersion</key>
    <string>$VERSION</string>
    <key>CFBundleShortVersionString</key>
    <string>$VERSION</string>
    <key>CFBundleExecutable</key>
    <string>$APP_NAME</string>
    <key>CFBundleIconFile</key>
    <string>AppIcon</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>LSMinimumSystemVersion</key>
    <string>10.15</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSHumanReadableCopyright</key>
    <string>Apache-2.0</string>
</dict>
</plist>
EOF

echo "==> Done: $BUNDLE_DIR"
