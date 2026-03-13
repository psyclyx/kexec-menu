#!/bin/sh
# mkkernel.sh — build a minimal kernel for kexec-menu from source
#
# Builds from tinyconfig + the project's config fragments (common.config +
# arch-specific). Optionally embeds an initramfs, command line, and boot logo.
#
# Required:
#   KERNEL_SRC  — path to kernel source directory (extracted tarball)
#   ARCH        — "x86_64" or "aarch64"
#
# Optional:
#   CONFIG_DIR  — path to config fragments dir (default: uki/kernel/ in repo)
#   EXTRA_CONFIG — path to additional config fragment to merge last
#   INITRAMFS   — path to CPIO archive to embed (CONFIG_INITRAMFS_SOURCE)
#   CMDLINE     — kernel command line string to embed (CONFIG_CMDLINE)
#   LOGO        — path to 80x80 PPM file (replaces default boot logo)
#   OUTPUT      — output file path (default: vmlinuz or Image next to source)
#   JOBS        — parallel make jobs (default: nproc)
#
# Usage:
#   KERNEL_SRC=~/linux-6.12 ARCH=x86_64 ./scripts/mkkernel.sh
#   KERNEL_SRC=~/linux-6.12 ARCH=aarch64 INITRAMFS=initrd.cpio \
#     CMDLINE="console=ttyAMA0" ./scripts/mkkernel.sh
#
# Dependencies: make, gcc (or cross toolchain), flex, bison, bc, perl,
#               standard coreutils. For aarch64 cross-build: aarch64-linux-gnu-gcc.

set -eu

die() { echo "mkkernel: error: $1" >&2; exit 1; }

# --- Validate required inputs ---

[ -n "${KERNEL_SRC:-}" ] || die "KERNEL_SRC not set"
[ -d "$KERNEL_SRC" ]     || die "KERNEL_SRC not a directory: $KERNEL_SRC"
[ -n "${ARCH:-}" ]       || die "ARCH not set (x86_64 or aarch64)"

case "$ARCH" in
    x86_64)  KARCH=x86;  IMAGE_NAME=bzImage; IMAGE_PATH=arch/x86/boot/bzImage ;;
    aarch64) KARCH=arm64; IMAGE_NAME=Image;   IMAGE_PATH=arch/arm64/boot/Image ;;
    *) die "unsupported ARCH: $ARCH (expected x86_64 or aarch64)" ;;
esac

# --- Locate config fragments ---

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
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

JOBS="${JOBS:-$(nproc 2>/dev/null || echo 1)}"

# --- Set up cross-compilation for aarch64 ---

MAKE_ARGS="ARCH=$KARCH -j$JOBS"
if [ "$ARCH" = "aarch64" ]; then
    host_arch="$(uname -m)"
    if [ "$host_arch" != "aarch64" ]; then
        CROSS_COMPILE="${CROSS_COMPILE:-aarch64-linux-gnu-}"
        MAKE_ARGS="$MAKE_ARGS CROSS_COMPILE=$CROSS_COMPILE"
    fi
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

if [ -n "${OUTPUT:-}" ]; then
    cp "$BUILT" "$OUTPUT"
    echo "mkkernel: wrote $OUTPUT" >&2
else
    echo "mkkernel: built $BUILT" >&2
fi
