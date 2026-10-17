#!/usr/bin/env bash
# Builds a release binary of Openbar5000 for macOS, and packages it into a
# proper Openbar5000.app bundle (with the app icon) so Finder/Dock/Spotlight
# show it correctly -- a raw binary has no icon of its own outside the Dock
# while it's actually running.
#
# It then puts the bundle on a disk image for sharing: opened, it shows the
# app next to a link to Applications, to drag it onto -- the usual way a Mac
# app is installed.
#
# Usage:
#   ./compile/build-macos.sh              # build for the host architecture
#   ./compile/build-macos.sh --universal  # build a universal (x86_64 + arm64) binary
#
# Produces:
#   dist/macos/openbar5000                        (raw binary)
#   dist/macos/Openbar5000.app                    (app bundle, with icon)
#   dist/macos/Openbar5000-<version>-<arch>.dmg   (disk image, to share)

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
    ARCH="universal"
else
    echo "==> Building release binary for the host architecture..."
    cargo build --release -p app
    cp "target/release/$APP_NAME" "$DIST_DIR/$APP_NAME"
    ARCH="$(uname -m)"
fi

echo "==> Binary: $DIST_DIR/$APP_NAME"

echo "==> Packaging Openbar5000.app (version $VERSION)..."
# A bundle version is numbers only: a pre-release ("0.7.1-alpha") is
# bundled as its release number.
BUNDLE_VERSION="${VERSION%%-*}"
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
    <string>$BUNDLE_VERSION</string>
    <key>CFBundleShortVersionString</key>
    <string>$BUNDLE_VERSION</string>
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

# Signed as a whole (ad hoc: no developer certificate needed), so the
# bundle's contents match its signature. Copied from a downloaded disk
# image, an app whose signature doesn't cover its Info.plist is reported as
# "damaged"; signed, macOS instead asks once whether to open it.
codesign --force --deep --sign - "$BUNDLE_DIR"

echo "==> App: $BUNDLE_DIR"

echo "==> Making the disk image..."
VOLUME_NAME="Openbar5000"
DMG="$DIST_DIR/Openbar5000-$VERSION-$ARCH.dmg"
MOUNT_POINT="/Volumes/$VOLUME_NAME"
# Finder finds the volume to lay out by its name: another one by that name
# (an earlier image still open) would be laid out instead.
if [ -e "$MOUNT_POINT" ]; then
    echo "error: $MOUNT_POINT is already in use -- eject it first." >&2
    exit 1
fi
WORK_DIR="$(mktemp -d)"
trap 'hdiutil detach -quiet "$MOUNT_POINT" 2>/dev/null || true; rm -rf "$WORK_DIR"' EXIT

# What the image holds: the app, a link to Applications to drop it on, and
# the window's background, hidden in a dot folder.
STAGE="$WORK_DIR/stage"
mkdir -p "$STAGE/.background"
cp -R "$BUNDLE_DIR" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
cp "$SCRIPT_DIR/assets/macos/dmg-background.tiff" "$STAGE/.background/background.tiff"

# Made writable first, with room for the window layout Finder saves.
STAGE_MB="$(du -sm "$STAGE" | cut -f1)"
hdiutil create -quiet -ov -fs HFS+ -format UDRW -volname "$VOLUME_NAME" \
    -srcfolder "$STAGE" -size "$((STAGE_MB + 20))m" "$WORK_DIR/rw.dmg"
hdiutil attach -quiet -readwrite -noverify -noautoopen -mountpoint "$MOUNT_POINT" "$WORK_DIR/rw.dmg"

# The window's look is saved by Finder itself (in the volume's .DS_Store),
# so Finder is asked to set it up: an icon view of 640x400 points over the
# background, the two icons where the background's arrow points. The first
# time, macOS asks whether the terminal may control Finder, and waits for
# an answer: given LAYOUT_TIMEOUT seconds, then left. Without that (or
# without a desktop session), the image still works, just unstyled.
# The bounds are the whole window, title bar included.
LAYOUT_TIMEOUT=120
echo "    Asking Finder to lay out the window (if macOS asks, allow the terminal to control Finder)..."
osascript >/dev/null <<APPLESCRIPT &
tell application "Finder"
    tell disk "$VOLUME_NAME"
        open
        set current view of container window to icon view
        set toolbar visible of container window to false
        set statusbar visible of container window to false
        set bounds of container window to {200, 120, 840, 548}
        set view_options to icon view options of container window
        set arrangement of view_options to not arranged
        set icon size of view_options to 112
        set text size of view_options to 13
        set background picture of view_options to file ".background:background.tiff"
        set position of item "Openbar5000.app" of container window to {160, 236}
        set position of item "Applications" of container window to {480, 236}
        update without registering applications
        delay 1
        close
    end tell
end tell
APPLESCRIPT
LAYOUT_PID=$!
for _ in $(seq 1 "$LAYOUT_TIMEOUT"); do
    kill -0 "$LAYOUT_PID" 2>/dev/null || break
    sleep 1
done
if kill -0 "$LAYOUT_PID" 2>/dev/null; then
    kill "$LAYOUT_PID" 2>/dev/null || true
    LAID_OUT=false
elif wait "$LAYOUT_PID"; then
    LAID_OUT=true
else
    LAID_OUT=false
fi
if [ "$LAID_OUT" = true ]; then
    # Finder writes the layout out in its own time.
    for _ in $(seq 1 20); do
        [ -f "$MOUNT_POINT/.DS_Store" ] && break
        sleep 0.5
    done
else
    echo "warning: Finder didn't lay out the disk image's window, so it opens as a plain folder." >&2
    echo "         Allow your terminal to control Finder (System Settings > Privacy & Security >" >&2
    echo "         Automation), then build again." >&2
fi

# The volume shows the app's icon: its .VolumeIcon.icns, once the root has
# the "custom icon" Finder flag. Added only now, as laying out the window
# deletes the file and clears the flag. SetFile comes with Xcode's command
# line tools; without them, the flag is written directly.
cp "$SCRIPT_DIR/assets/macos/AppIcon.icns" "$MOUNT_POINT/.VolumeIcon.icns"
if command -v SetFile >/dev/null 2>&1; then
    SetFile -a C "$MOUNT_POINT"
else
    xattr -wx com.apple.FinderInfo 0000000000000000040000000000000000000000000000000000000000000000 "$MOUNT_POINT"
fi
rm -rf "$MOUNT_POINT/.fseventsd"
hdiutil detach -quiet "$MOUNT_POINT"

# Compressed and read-only, as shared.
rm -f "$DMG"
hdiutil convert -quiet "$WORK_DIR/rw.dmg" -format UDZO -imagekey zlib-level=9 -o "$DMG"
# The image file shows the app's icon too. It's kept with the file on a Mac
# (copies, zips, AirDrop); a browser download drops it, as for every disk
# image, and shows the standard one.
osascript -l JavaScript - "$SCRIPT_DIR/assets/macos/AppIcon.icns" "$DMG" >/dev/null <<'JAVASCRIPT' ||
ObjC.import("AppKit");
function run([icon, file]) {
    return $.NSWorkspace.sharedWorkspace.setIconForFileOptions($.NSImage.alloc.initWithContentsOfFile(icon), file, 0);
}
JAVASCRIPT
    echo "warning: couldn't give the disk image file its icon." >&2

echo "==> Done: $DMG"
