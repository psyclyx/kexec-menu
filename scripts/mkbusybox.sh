#!/bin/sh
# mkbusybox.sh — download and build a minimal static busybox for kexec-menu
#
# Produces a single static busybox binary with only the applets needed
# by the kexec-menu initrd. Uses the config fragment at uki/initrd/busybox.config.
#
# Optional:
#   BUSYBOX_SRC     — path to existing busybox source dir (skips download)
#   BUSYBOX_VERSION — version to download (default: 1.36.1)
#   ARCH            — "x86_64" or "aarch64" (default: x86_64)
#   OUTPUT          — output binary path (default: build/busybox)
#   JOBS            — parallel make jobs (default: nproc)
#
# Usage:
#   ./scripts/mkbusybox.sh                             # download + build x86_64
#   ARCH=aarch64 ./scripts/mkbusybox.sh                # cross-build aarch64
#   BUSYBOX_SRC=~/busybox-1.36.1 ./scripts/mkbusybox.sh  # use existing source
#
# Dependencies: make, gcc (or cross toolchain), wget/curl, tar, sed

set -eu

die() { echo "mkbusybox: error: $1" >&2; exit 1; }

BUSYBOX_VERSION="${BUSYBOX_VERSION:-1.36.1}"
ARCH="${ARCH:-x86_64}"
JOBS="${JOBS:-$(nproc 2>/dev/null || echo 1)}"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
BUILD_DIR="${BUILD_DIR:-$REPO_DIR/build}"
OUTPUT="${OUTPUT:-$BUILD_DIR/busybox}"
CONFIG_FRAGMENT="$REPO_DIR/uki/initrd/busybox.config"

[ -f "$CONFIG_FRAGMENT" ] || die "config fragment not found: $CONFIG_FRAGMENT"

mkdir -p "$BUILD_DIR"

# --- Download source if not provided ---

if [ -z "${BUSYBOX_SRC:-}" ]; then
    TARBALL="busybox-${BUSYBOX_VERSION}.tar.bz2"
    TARBALL_PATH="$BUILD_DIR/$TARBALL"
    BUSYBOX_SRC="$BUILD_DIR/busybox-${BUSYBOX_VERSION}"

    if [ ! -d "$BUSYBOX_SRC" ]; then
        if [ ! -f "$TARBALL_PATH" ]; then
            URL="https://busybox.net/downloads/$TARBALL"
            echo "mkbusybox: downloading busybox $BUSYBOX_VERSION" >&2
            if command -v wget >/dev/null 2>&1; then
                wget -q -O "$TARBALL_PATH" "$URL" || die "download failed: $URL"
            elif command -v curl >/dev/null 2>&1; then
                curl -fsSL -o "$TARBALL_PATH" "$URL" || die "download failed: $URL"
            else
                die "neither wget nor curl found"
            fi
        fi
        echo "mkbusybox: extracting $TARBALL" >&2
        tar -xf "$TARBALL_PATH" -C "$BUILD_DIR"
    fi
fi

[ -d "$BUSYBOX_SRC" ] || die "source directory not found: $BUSYBOX_SRC"

# --- Set up cross-compilation ---

MAKE_ARGS=""
case "$ARCH" in
    x86_64)
        host_arch="$(uname -m)"
        if [ "$host_arch" != "x86_64" ]; then
            CROSS="${CROSS_COMPILE:-x86_64-linux-musl-}"
            MAKE_ARGS="CROSS_COMPILE=$CROSS"
        fi
        ;;
    aarch64)
        host_arch="$(uname -m)"
        if [ "$host_arch" != "aarch64" ]; then
            CROSS="${CROSS_COMPILE:-aarch64-linux-musl-}"
            MAKE_ARGS="CROSS_COMPILE=$CROSS"
        fi
        ;;
    *) die "unsupported ARCH: $ARCH (expected x86_64 or aarch64)" ;;
esac

# --- Configure ---

bmake() {
    make -C "$BUSYBOX_SRC" $MAKE_ARGS "$@"
}

echo "mkbusybox: configuring (allnoconfig + fragment)" >&2

bmake allnoconfig >/dev/null 2>&1

# Merge our config fragment: for each CONFIG_FOO=y line, enable it in .config
while IFS= read -r line; do
    case "$line" in
        '#'*|'') continue ;;
        CONFIG_*=y)
            key="${line%%=*}"
            # Enable: replace "# CONFIG_FOO is not set" or add if missing
            if grep -q "# $key is not set" "$BUSYBOX_SRC/.config" 2>/dev/null; then
                sed -i "s/^# $key is not set$/$line/" "$BUSYBOX_SRC/.config"
            elif ! grep -q "^$key=" "$BUSYBOX_SRC/.config" 2>/dev/null; then
                echo "$line" >> "$BUSYBOX_SRC/.config"
            fi
            ;;
        CONFIG_*=*)
            key="${line%%=*}"
            value="${line#*=}"
            if grep -q "# $key is not set" "$BUSYBOX_SRC/.config" 2>/dev/null; then
                sed -i "s|^# $key is not set$|$line|" "$BUSYBOX_SRC/.config"
            elif grep -q "^$key=" "$BUSYBOX_SRC/.config" 2>/dev/null; then
                sed -i "s|^$key=.*|$line|" "$BUSYBOX_SRC/.config"
            else
                echo "$line" >> "$BUSYBOX_SRC/.config"
            fi
            ;;
    esac
done < "$CONFIG_FRAGMENT"

# Handle CROSS_COMPILE prefix in config
if [ -n "${CROSS:-}" ]; then
    sed -i "s|^CONFIG_CROSS_COMPILER_PREFIX=.*|CONFIG_CROSS_COMPILER_PREFIX=\"$CROSS\"|" \
        "$BUSYBOX_SRC/.config"
fi

# Resolve dependencies
yes "" 2>/dev/null | bmake oldconfig >/dev/null 2>&1 || true

# --- Build ---

echo "mkbusybox: building ($JOBS jobs)" >&2
bmake -j"$JOBS" busybox

[ -f "$BUSYBOX_SRC/busybox" ] || die "build succeeded but busybox binary not found"

# --- Output ---

cp "$BUSYBOX_SRC/busybox" "$OUTPUT"
chmod +x "$OUTPUT"
echo "mkbusybox: wrote $OUTPUT" >&2
