#!/bin/sh
# mkbcachefs.sh — download and build a static bcachefs binary for kexec-menu
#
# Produces a single static bcachefs binary (mount + key unlock) for the
# kexec-menu initrd.
#
# Optional:
#   BCACHEFS_SRC     — path to existing bcachefs-tools source dir (skips download)
#   BCACHEFS_VERSION — git tag to download (default: v1.11.0)
#   ARCH             — "x86_64" or "aarch64" (default: x86_64)
#   OUTPUT           — output binary path (default: build/bcachefs)
#   JOBS             — parallel make jobs (default: nproc)
#   PKG_CONFIG_PATH  — override pkg-config search path for static libs
#
# Usage:
#   ./scripts/mkbcachefs.sh                                    # download + build x86_64
#   ARCH=aarch64 ./scripts/mkbcachefs.sh                       # cross-build aarch64
#   BCACHEFS_SRC=~/bcachefs-tools ./scripts/mkbcachefs.sh      # use existing source
#
# Dependencies:
#   Build tools: make, pkg-config, gcc (or musl cross toolchain), cargo
#   Static libraries (must be available via pkg-config or system paths):
#     - libuuid (util-linux)
#     - libblkid (util-linux)
#     - libkeyutils (keyutils)
#     - libsodium
#     - liburcu (userspace-rcu)
#     - libzstd
#     - liblz4
#     - libz (zlib)
#     - libaio (optional, for io_uring fallback)
#   For static musl builds, all libraries must be built against musl.

set -eu

die() { echo "mkbcachefs: error: $1" >&2; exit 1; }

BCACHEFS_VERSION="${BCACHEFS_VERSION:-v1.11.0}"
ARCH="${ARCH:-x86_64}"
JOBS="${JOBS:-$(nproc 2>/dev/null || echo 1)}"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
BUILD_DIR="${BUILD_DIR:-$REPO_DIR/build}"
OUTPUT="${OUTPUT:-$BUILD_DIR/bcachefs}"

mkdir -p "$BUILD_DIR"

# --- Validate build tools ---

for tool in make pkg-config cargo; do
    command -v "$tool" >/dev/null 2>&1 || die "$tool not found"
done

# --- Download source if not provided ---

if [ -z "${BCACHEFS_SRC:-}" ]; then
    BCACHEFS_SRC="$BUILD_DIR/bcachefs-tools-${BCACHEFS_VERSION#v}"

    if [ ! -d "$BCACHEFS_SRC" ]; then
        TARBALL="bcachefs-tools-${BCACHEFS_VERSION}.tar.gz"
        TARBALL_PATH="$BUILD_DIR/$TARBALL"

        if [ ! -f "$TARBALL_PATH" ]; then
            URL="https://evilpiepirate.org/git/bcachefs-tools.git/snapshot/bcachefs-tools-${BCACHEFS_VERSION}.tar.gz"
            echo "mkbcachefs: downloading bcachefs-tools $BCACHEFS_VERSION" >&2
            if command -v wget >/dev/null 2>&1; then
                wget -q -O "$TARBALL_PATH" "$URL" || die "download failed: $URL"
            elif command -v curl >/dev/null 2>&1; then
                curl -fsSL -o "$TARBALL_PATH" "$URL" || die "download failed: $URL"
            else
                die "neither wget nor curl found"
            fi
        fi
        echo "mkbcachefs: extracting $TARBALL" >&2
        mkdir -p "$BCACHEFS_SRC"
        tar -xf "$TARBALL_PATH" -C "$BCACHEFS_SRC" --strip-components=1
    fi
fi

[ -d "$BCACHEFS_SRC" ] || die "source directory not found: $BCACHEFS_SRC"

# --- Set up cross-compilation ---

CC=""
RUST_TARGET=""
case "$ARCH" in
    x86_64)
        RUST_TARGET="x86_64-unknown-linux-musl"
        host_arch="$(uname -m)"
        if [ "$host_arch" != "x86_64" ]; then
            CC="${CROSS_COMPILE:-x86_64-linux-musl-}gcc"
        else
            CC="${CC:-gcc}"
        fi
        ;;
    aarch64)
        RUST_TARGET="aarch64-unknown-linux-musl"
        host_arch="$(uname -m)"
        if [ "$host_arch" != "aarch64" ]; then
            CC="${CROSS_COMPILE:-aarch64-linux-musl-}gcc"
        else
            CC="${CC:-gcc}"
        fi
        ;;
    *) die "unsupported ARCH: $ARCH (expected x86_64 or aarch64)" ;;
esac

# --- Build ---

echo "mkbcachefs: building ($JOBS jobs, target: $RUST_TARGET)" >&2

MAKE_ARGS="CC=$CC"
MAKE_ARGS="$MAKE_ARGS CARGO_ARGS=--target=$RUST_TARGET"
MAKE_ARGS="$MAKE_ARGS LDFLAGS=-static"
MAKE_ARGS="$MAKE_ARGS NO_SYSTEMD=1"

# Only build the bcachefs binary (not mount.bcachefs.sh, fsck wrapper, etc.)
make -C "$BCACHEFS_SRC" $MAKE_ARGS -j"$JOBS" bcachefs

# Find the built binary — may be at root or in target dir
BUILT=""
if [ -f "$BCACHEFS_SRC/target/$RUST_TARGET/release/bcachefs" ]; then
    BUILT="$BCACHEFS_SRC/target/$RUST_TARGET/release/bcachefs"
elif [ -f "$BCACHEFS_SRC/bcachefs" ]; then
    BUILT="$BCACHEFS_SRC/bcachefs"
else
    die "build succeeded but bcachefs binary not found"
fi

# --- Output ---

cp "$BUILT" "$OUTPUT"
chmod +x "$OUTPUT"
echo "mkbcachefs: wrote $OUTPUT" >&2
