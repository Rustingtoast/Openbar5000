#!/usr/bin/env bash
# Builds a release binary of Openbar5000 for Linux, and -- on Debian/Ubuntu
# based systems where dpkg-deb is available -- packages it as a .deb.
#
# Usage:
#   ./compile/build-linux.sh
#
# Produces:
#   dist/linux/openbar5000                        (raw binary, any distro)
#   dist/linux/openbar5000_<version>_<arch>.deb    (Debian/Ubuntu package)
#
# Requires the usual native dev headers for audio (ALSA) and the GUI
# backend (X11/Wayland + fontconfig) to *build*. On Debian/Ubuntu:
#   sudo apt-get install -y build-essential pkg-config libasound2-dev \
#       libxkbcommon-dev libxcb1-dev libx11-dev libxcursor-dev \
#       libxrandr-dev libxi-dev libgl1-mesa-dev libfontconfig1-dev
# Package names vary on other distros (Fedora/Arch/etc.) -- look for the
# "-dev"/"-devel" packages matching the libraries above.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
DIST_DIR="$REPO_ROOT/dist/linux"
APP_NAME="openbar5000"
VERSION="$(grep -m1 '^version' "$REPO_ROOT/Cargo.toml" | sed -E 's/version = "(.*)"/\1/')"

if ! command -v cargo >/dev/null 2>&1; then
    echo "error: cargo not found. Install Rust via https://rustup.rs first." >&2
    exit 1
fi

echo "==> Building Openbar5000 (release) for Linux..."
cd "$REPO_ROOT"
cargo build --release -p app

mkdir -p "$DIST_DIR"
cp "target/release/$APP_NAME" "$DIST_DIR/$APP_NAME"
echo "==> Binary: $DIST_DIR/$APP_NAME"

if ! command -v dpkg-deb >/dev/null 2>&1; then
    echo "==> Skipping .deb packaging: dpkg-deb not found (this step only works on Debian/Ubuntu-based systems)." >&2
    exit 0
fi

ARCH="$(dpkg --print-architecture 2>/dev/null || echo amd64)"
PKG_DIR="$DIST_DIR/pkgroot"
DEB_PATH="$DIST_DIR/${APP_NAME}_${VERSION}_${ARCH}.deb"

echo "==> Packaging .deb (version $VERSION, arch $ARCH)..."
rm -rf "$PKG_DIR"
mkdir -p "$PKG_DIR/DEBIAN" \
         "$PKG_DIR/usr/bin" \
         "$PKG_DIR/usr/share/applications" \
         "$PKG_DIR/usr/share/doc/$APP_NAME" \
         "$PKG_DIR/usr/share/icons/hicolor/scalable/apps" \
         "$PKG_DIR/usr/share/icons/hicolor/256x256/apps"

install -m 755 "$DIST_DIR/$APP_NAME" "$PKG_DIR/usr/bin/$APP_NAME"
cp "$REPO_ROOT/LICENSE" "$PKG_DIR/usr/share/doc/$APP_NAME/copyright"
cp "$REPO_ROOT/crates/ui/ui/assets/icons/app-icon.svg" \
   "$PKG_DIR/usr/share/icons/hicolor/scalable/apps/${APP_NAME}.svg"
cp "$SCRIPT_DIR/assets/linux/openbar5000-256.png" \
   "$PKG_DIR/usr/share/icons/hicolor/256x256/apps/${APP_NAME}.png"

cat > "$PKG_DIR/usr/share/applications/${APP_NAME}.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Openbar5000
Comment=Open-source, cross-platform alternative to foobar2000
Exec=$APP_NAME
Icon=$APP_NAME
Terminal=false
Categories=AudioVideo;Audio;Player;
EOF

# Runtime (not -dev) packages for the libraries this binary links against.
# Exact package names can drift between distro releases (e.g. Ubuntu's
# libasound2 -> libasound2t64 transition) -- if `dpkg -i` complains about
# missing dependencies, `sudo apt-get install -f` will resolve them for the
# system's actual package names. Vulkan is only recommended: Skia draws
# through it when it's there, and through OpenGL (libgl1) otherwise.
cat > "$PKG_DIR/DEBIAN/control" <<EOF
Package: $APP_NAME
Version: $VERSION
Section: sound
Priority: optional
Architecture: $ARCH
Depends: libasound2, libfontconfig1, libx11-6, libxkbcommon0, libxcursor1, libxrandr2, libxi6, libgl1
Recommends: libvulkan1
Maintainer: Openbar5000 Project <noreply@github.com>
Homepage: https://github.com/Rustingtoast/Openbar5000
Description: Open-source, cross-platform alternative to foobar2000
 Openbar5000 is a cross-platform audio player built in Rust with a Slint UI,
 aiming to look and feel as close to foobar2000 as practical.
EOF

# --root-owner-group avoids packaging files owned by the building user's
# UID/GID -- needs dpkg >= 1.19.0.5 (2019), present on any current distro.
dpkg-deb --build --root-owner-group "$PKG_DIR" "$DEB_PATH"
rm -rf "$PKG_DIR"

echo "==> Done: $DEB_PATH"
