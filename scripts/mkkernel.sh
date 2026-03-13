#!/bin/sh
# mkkernel.sh — download and build a minimal kernel for kexec-menu
#
# Builds from tinyconfig + the project's config fragments (common.config +
# arch-specific). Optionally embeds an initramfs, command line, and boot logo.
#
# Optional:
#   KERNEL_SRC     — path to existing kernel source dir (skips download)
#   KERNEL_VERSION — version to download (default: 6.12.6)
#   ARCH           — "x86_64" or "aarch64" (default: x86_64)
#   CONFIG_DIR     — path to config fragments dir (default: uki/kernel/ in repo)
#   EXTRA_CONFIG   — path to additional config fragment to merge last
#   INITRAMFS      — path to CPIO archive to embed (CONFIG_INITRAMFS_SOURCE)
#   CMDLINE        — kernel command line string to embed (CONFIG_CMDLINE)
#   LOGO           — path to 80x80 PPM file (replaces default boot logo)
#   OUTPUT         — output file path (default: build/vmlinuz or build/Image)
#   JOBS           — parallel make jobs (default: nproc)
#
# Usage:
#   ./scripts/mkkernel.sh                             # download + build x86_64
#   ARCH=aarch64 ./scripts/mkkernel.sh                # download + build aarch64
#   KERNEL_SRC=~/linux-6.12.6 ./scripts/mkkernel.sh   # use existing source
#   KERNEL_VERSION=6.13.1 ./scripts/mkkernel.sh       # specific version
#
# Dependencies: make, gcc (or cross toolchain), flex, bison, bc, perl,
#               wget/curl (for download), tar, standard coreutils.
#               For aarch64 cross-build: aarch64-linux-gnu-gcc.

set -eu

die() { echo "mkkernel: error: $1" >&2; exit 1; }

KERNEL_VERSION="${KERNEL_VERSION:-6.12.76}"
ARCH="${ARCH:-x86_64}"
JOBS="${JOBS:-$(nproc 2>/dev/null || echo 1)}"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
BUILD_DIR="${BUILD_DIR:-$REPO_DIR/build}"

mkdir -p "$BUILD_DIR"

# --- Architecture setup ---

case "$ARCH" in
    x86_64)  KARCH=x86;  IMAGE_NAME=bzImage; IMAGE_PATH=arch/x86/boot/bzImage ;;
    aarch64) KARCH=arm64; IMAGE_NAME=Image;   IMAGE_PATH=arch/arm64/boot/Image ;;
    *) die "unsupported ARCH: $ARCH (expected x86_64 or aarch64)" ;;
esac

# --- Download source if not provided ---

if [ -z "${KERNEL_SRC:-}" ]; then
    KERNEL_SRC="$BUILD_DIR/linux-${KERNEL_VERSION}"

    if [ ! -d "$KERNEL_SRC" ]; then
        TARBALL="linux-${KERNEL_VERSION}.tar.xz"
        TARBALL_PATH="$BUILD_DIR/$TARBALL"

        if [ ! -f "$TARBALL_PATH" ]; then
            # kernel.org major version directory
            MAJOR_VERSION="${KERNEL_VERSION%%.*}"
            URL="https://cdn.kernel.org/pub/linux/kernel/v${MAJOR_VERSION}.x/$TARBALL"
            echo "mkkernel: downloading linux $KERNEL_VERSION" >&2
            if command -v wget >/dev/null 2>&1; then
                wget -q -O "$TARBALL_PATH" "$URL" || die "download failed: $URL"
            elif command -v curl >/dev/null 2>&1; then
                curl -fsSL -o "$TARBALL_PATH" "$URL" || die "download failed: $URL"
            else
                die "neither wget nor curl found"
            fi
        fi
        echo "mkkernel: extracting $TARBALL" >&2
        tar -xf "$TARBALL_PATH" -C "$BUILD_DIR"
    fi
fi

[ -d "$KERNEL_SRC" ] || die "source directory not found: $KERNEL_SRC"

# --- Locate config fragments ---

CONFIG_DIR="${CONFIG_DIR:-$SCRIPT_DIR/../uki/kernel}"

[ -f "$CONFIG_DIR/common.config" ]     || die "common.config not found in $CONFIG_DIR"
[ -f "$CONFIG_DIR/${ARCH}.config" ]    || die "${ARCH}.config not found in $CONFIG_DIR"

# --- Validate optional inputs ---

if [ -n "${INITRAMFS:-}" ]; then
    [ -f "$INITRAMFS" ] || die "INITRAMFS not found: $INITRAMFS"
    INITRAMFS="$(cd "$(dirname "$INITRAMFS")" && pwd)/$(basename "$INITRAMFS")"
fi

if [ -n "${LOGO:-}" ]; then
    [ -f "$LOGO" ] || die "LOGO not found: $LOGO"
fi

if [ -n "${EXTRA_CONFIG:-}" ]; then
    [ -f "$EXTRA_CONFIG" ] || die "EXTRA_CONFIG not found: $EXTRA_CONFIG"
fi

# --- Default output path ---

if [ -z "${OUTPUT:-}" ]; then
    case "$ARCH" in
        x86_64)  OUTPUT="$BUILD_DIR/vmlinuz" ;;
        aarch64) OUTPUT="$BUILD_DIR/Image" ;;
    esac
fi

# --- Set up cross-compilation for aarch64 ---

MAKE_ARGS="ARCH=$KARCH -j$JOBS"
host_arch="$(uname -m)"
if [ -n "${CROSS_COMPILE:-}" ]; then
    MAKE_ARGS="$MAKE_ARGS CROSS_COMPILE=$CROSS_COMPILE"
elif [ "$ARCH" = "aarch64" ] && [ "$host_arch" != "aarch64" ]; then
    CROSS_COMPILE="aarch64-linux-gnu-"
    MAKE_ARGS="$MAKE_ARGS CROSS_COMPILE=$CROSS_COMPILE"
elif [ "$ARCH" = "x86_64" ] && [ "$host_arch" != "x86_64" ]; then
    CROSS_COMPILE="x86_64-linux-gnu-"
    MAKE_ARGS="$MAKE_ARGS CROSS_COMPILE=$CROSS_COMPILE"
fi

# --- Build ---

kmake() {
    make -C "$KERNEL_SRC" $MAKE_ARGS "$@"
}

echo "mkkernel: starting $ARCH kernel build (source: $KERNEL_SRC)" >&2

# Start from tinyconfig
echo "mkkernel: generating tinyconfig" >&2
kmake tinyconfig >/dev/null 2>&1

# Merge config fragments using kernel's merge_config.sh
MERGE_SCRIPT="$KERNEL_SRC/scripts/kconfig/merge_config.sh"
[ -f "$MERGE_SCRIPT" ] || die "merge_config.sh not found in kernel source"

FRAGMENTS="$CONFIG_DIR/common.config $CONFIG_DIR/${ARCH}.config"

# Build dynamic config fragment for embedded initramfs / cmdline
DYNAMIC_CONFIG=""
if [ -n "${INITRAMFS:-}" ] || [ -n "${CMDLINE:-}" ]; then
    DYNAMIC_CONFIG="$(mktemp)"
    trap 'rm -f "$DYNAMIC_CONFIG"' EXIT
    if [ -n "${INITRAMFS:-}" ]; then
        echo "CONFIG_BLK_DEV_INITRD=y"       >> "$DYNAMIC_CONFIG"
        echo "CONFIG_INITRAMFS_SOURCE=\"$INITRAMFS\"" >> "$DYNAMIC_CONFIG"
    fi
    if [ -n "${CMDLINE:-}" ]; then
        echo "CONFIG_CMDLINE_BOOL=y"          >> "$DYNAMIC_CONFIG"
        echo "CONFIG_CMDLINE=\"$CMDLINE\""    >> "$DYNAMIC_CONFIG"
    fi
    FRAGMENTS="$FRAGMENTS $DYNAMIC_CONFIG"
fi

if [ -n "${EXTRA_CONFIG:-}" ]; then
    FRAGMENTS="$FRAGMENTS $EXTRA_CONFIG"
fi

echo "mkkernel: merging config fragments" >&2
# merge_config.sh needs to run from the kernel source dir
# -m: merge into existing .config (our tinyconfig base)
(cd "$KERNEL_SRC" && ARCH=$KARCH "$MERGE_SCRIPT" -m .config $FRAGMENTS) >/dev/null 2>&1

# Resolve any new symbols to defaults
kmake olddefconfig >/dev/null 2>&1

# Replace boot logo if requested
if [ -n "${LOGO:-}" ]; then
    echo "mkkernel: replacing boot logo" >&2
    cp "$LOGO" "$KERNEL_SRC/drivers/video/logo/logo_linux_clut224.ppm"
fi

# Build
echo "mkkernel: building $IMAGE_NAME ($JOBS jobs)" >&2
kmake "$IMAGE_NAME"

# --- Output ---

BUILT="$KERNEL_SRC/$IMAGE_PATH"
[ -f "$BUILT" ] || die "build succeeded but $IMAGE_PATH not found"

cp "$BUILT" "$OUTPUT"
echo "mkkernel: wrote $OUTPUT" >&2
