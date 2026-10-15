#!/usr/bin/env bash
# Builds Openbar5000 as a Flatpak, which runs on any Linux distribution with
# Flatpak installed (Fedora, Arch, openSUSE, Debian/Ubuntu, Mint, ...).
# This is an addition to build-linux.sh (raw binary + .deb), not a
# replacement for it.
#
# Usage:
#   ./compile/build-flatpak.sh            # build the .flatpak bundle
#   ./compile/build-flatpak.sh --install  # ...and install it for this user
#
# Produces:
#   dist/flatpak/Openbar5000-<version>-<arch>.flatpak   (single-file bundle)
#
# Install the bundle on any machine with:
#   flatpak install --user Openbar5000-<version>-<arch>.flatpak
# and run it with:
#   flatpak run io.github.rustingtoast.Openbar5000
#
# Requires flatpak, flatpak-builder (or the org.flatpak.Builder Flatpak) and
# cargo on the build machine. The freedesktop runtime, SDK and Rust
# extension are installed from Flathub automatically on first run.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
APP_ID="io.github.rustingtoast.Openbar5000"
MANIFEST="$SCRIPT_DIR/flatpak/$APP_ID.yml"
METAINFO="$SCRIPT_DIR/flatpak/$APP_ID.metainfo.xml"
DIST_DIR="$REPO_ROOT/dist/flatpak"
VERSION="$(grep -m1 '^version' "$REPO_ROOT/Cargo.toml" | sed -E 's/version = "(.*)"/\1/')"
FLATHUB_REPO="https://dl.flathub.org/repo/flathub.flatpakrepo"

INSTALL=0
for arg in "$@"; do
    case "$arg" in
        --install) INSTALL=1 ;;
        -h|--help) sed -n '2,22p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "error: unknown option '$arg' (try --help)" >&2; exit 1 ;;
    esac
done

if [[ "$(uname -s)" != "Linux" ]]; then
    echo "error: Flatpaks can only be built on Linux." >&2
    exit 1
fi
if ! command -v flatpak >/dev/null 2>&1; then
    echo "error: flatpak not found. Install it from your distribution: https://flatpak.org/setup/" >&2
    exit 1
fi
if command -v flatpak-builder >/dev/null 2>&1; then
    BUILDER=(flatpak-builder)
elif flatpak info org.flatpak.Builder >/dev/null 2>&1; then
    BUILDER=(flatpak run org.flatpak.Builder)
else
    echo "error: flatpak-builder not found. Install your distribution's flatpak-builder package, or:" >&2
    echo "       flatpak install --user flathub org.flatpak.Builder" >&2
    exit 1
fi
if ! command -v cargo >/dev/null 2>&1; then
    echo "error: cargo not found (needed to vendor the Rust crates). Install Rust via https://rustup.rs" >&2
    exit 1
fi

# The AppStream release list should name the version being built.
if ! grep -q "<release version=\"$VERSION\"" "$METAINFO"; then
    echo "warning: $METAINFO has no <release version=\"$VERSION\"> entry; software centers will show an older version." >&2
fi

ARCH="$(flatpak --default-arch)"
BUNDLE="$DIST_DIR/Openbar5000-$VERSION-$ARCH.flatpak"
mkdir -p "$DIST_DIR"

echo "==> Making sure Flathub is available (for the runtime and Rust SDK)..."
flatpak remote-add --user --if-not-exists flathub "$FLATHUB_REPO"

# Flatpak builds have no network: vendor every crate up front, and point
# cargo at the copies.
echo "==> Vendoring Rust crates..."
rm -rf "$DIST_DIR/vendor"
(cd "$REPO_ROOT" && cargo vendor --locked --quiet "$DIST_DIR/vendor" >/dev/null)
cat > "$DIST_DIR/cargo-config.toml" <<'CONFIG'
[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "vendor"
CONFIG

echo "==> Building the Flatpak ($VERSION, $ARCH)..."
"${BUILDER[@]}" \
    --user \
    --force-clean \
    --install-deps-from=flathub \
    --state-dir="$DIST_DIR/.flatpak-builder" \
    --repo="$DIST_DIR/repo" \
    "$DIST_DIR/build-dir" \
    "$MANIFEST"

echo "==> Bundling..."
flatpak build-bundle \
    --runtime-repo="$FLATHUB_REPO" \
    "$DIST_DIR/repo" \
    "$BUNDLE" \
    "$APP_ID"
echo "==> Bundle: $BUNDLE"

if [[ "$INSTALL" == 1 ]]; then
    echo "==> Installing for this user..."
    flatpak install --user --reinstall --noninteractive -y "$BUNDLE"
    echo "==> Installed. Run it with: flatpak run $APP_ID"
fi
