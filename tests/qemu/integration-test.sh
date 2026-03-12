#!/usr/bin/env bash
#
# QEMU integration test: boot kexec-menu, verify default entry resolution.
#
# Usage:
#   nix-shell tests/qemu/shell.nix --run ./tests/qemu/integration-test.sh
#
# Boots a minimal kernel+initrd in QEMU with a test ext4 disk, runs
# kexec-menu --auto-default --dry-run, and checks for successful output.
#
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BUILD_DIR="$REPO_ROOT/target/qemu-test"
TARGET=x86_64-unknown-linux-musl
TIMEOUT_SECS=30

SKIP_BUILD=false
for arg in "$@"; do
    case "$arg" in
        --no-build) SKIP_BUILD=true ;;
        --help|-h)
            sed -n '2,/^$/{ s/^# //; s/^#//; p }' "$0"
            exit 0
            ;;
    esac
done

# --- Check tools ---
missing=()
for tool in qemu-system-x86_64 busybox mke2fs cpio; do
    command -v "$tool" &>/dev/null || missing+=("$tool")
done
if [[ ${#missing[@]} -gt 0 ]]; then
    echo "error: missing tools: ${missing[*]}" >&2
    echo "run: nix-shell tests/qemu/shell.nix --run ./tests/qemu/integration-test.sh" >&2
    exit 1
fi

# --- Resolve kernel and modules ---
if [[ -z "${QEMU_KERNEL:-}" || ! -f "$QEMU_KERNEL" ]]; then
    echo "error: QEMU_KERNEL not set or not found" >&2
    exit 1
fi
if [[ -z "${QEMU_KERNEL_MODULES:-}" || ! -d "$QEMU_KERNEL_MODULES" ]]; then
    echo "error: QEMU_KERNEL_MODULES not set or not found" >&2
    exit 1
fi

KERNEL_PATH="$QEMU_KERNEL"
MODULES_DIR="$QEMU_KERNEL_MODULES"

# --- Build kexec-menu ---
if ! $SKIP_BUILD; then
    echo "building kexec-menu for $TARGET..."
    cargo build --manifest-path "$REPO_ROOT/Cargo.toml" \
        --target "$TARGET" --release -p kexec-menu 2>&1
fi

BINARY="$REPO_ROOT/target/$TARGET/release/kexec-menu"
if [[ ! -f "$BINARY" ]]; then
    echo "error: binary not found at $BINARY" >&2
    exit 1
fi

# --- Create test disks (tmpfiles, cleaned up on exit) ---
mkdir -p "$BUILD_DIR"
DISK="$(mktemp "$BUILD_DIR/test-disk.XXXXXX.ext4")"
BTRFS_DISK="$(mktemp "$BUILD_DIR/test-disk.XXXXXX.raw")"
cleanup_disks() { rm -f "$DISK" "$BTRFS_DISK"; }
trap cleanup_disks EXIT
"$REPO_ROOT/tests/qemu/create-test-disk.sh" "$DISK"
# Empty 64MB disk — formatted as btrfs inside QEMU by init-test.sh
truncate -s 64M "$BTRFS_DISK"

# --- Create initrd (with test init) ---
INITRD="$BUILD_DIR/initrd-test.cpio"
INITRD_DIR="$BUILD_DIR/initrd-test-root"
rm -rf "$INITRD_DIR"
mkdir -p "$INITRD_DIR"/{bin,dev,proc,sys,mnt,run,etc,tmp}

if [[ -n "${BUSYBOX_STATIC:-}" && -f "$BUSYBOX_STATIC" ]]; then
    BUSYBOX="$BUSYBOX_STATIC"
else
    BUSYBOX="$(command -v busybox)"
fi
cp "$BUSYBOX" "$INITRD_DIR/bin/busybox"
for cmd in sh mount umount mkdir ls cat sleep poweroff insmod grep; do
    ln -sf busybox "$INITRD_DIR/bin/$cmd"
done

cp "$BINARY" "$INITRD_DIR/bin/kexec-menu"
if [[ -n "${MKFS_BTRFS_STATIC:-}" && -f "$MKFS_BTRFS_STATIC" ]]; then
    cp "$MKFS_BTRFS_STATIC" "$INITRD_DIR/bin/mkfs.btrfs"
fi
cp "$REPO_ROOT/tests/qemu/init-test.sh" "$INITRD_DIR/init"
chmod +x "$INITRD_DIR/init"

# --- Include kernel modules ---
NEEDED_MODULES=(
    "drivers/virtio/virtio_ring.ko"
    "drivers/virtio/virtio.ko"
    "drivers/virtio/virtio_pci_modern_dev.ko"
    "drivers/virtio/virtio_pci_legacy_dev.ko"
    "drivers/virtio/virtio_pci.ko"
    "drivers/block/virtio_blk.ko"
    "lib/crc16.ko"
    "lib/crc/crc16.ko"
    "crypto/crc32c-cryptoapi.ko"
    "fs/mbcache.ko"
    "fs/jbd2/jbd2.ko"
    "fs/ext4/ext4.ko"
    # btrfs
    "crypto/xor.ko"
    "lib/raid6/raid6_pq.ko"
    "fs/btrfs/btrfs.ko"
)

KMOD_DIR="$INITRD_DIR/lib/modules"
mkdir -p "$KMOD_DIR"

for mod in "${NEEDED_MODULES[@]}"; do
    src="$MODULES_DIR/kernel/$mod"
    for ext in "" ".xz" ".zst" ".gz"; do
        if [[ -f "${src}${ext}" ]]; then
            dst="$KMOD_DIR/$(basename "$mod")"
            case "$ext" in
                .xz)   xz -d -c "${src}${ext}" > "$dst" ;;
                .zst)  zstd -d -q "${src}${ext}" -o "$dst" ;;
                .gz)   gzip -d -c "${src}${ext}" > "$dst" ;;
                "")    cp "${src}" "$dst" ;;
            esac
            break
        fi
    done
done

(cd "$INITRD_DIR" && find . | cpio -o -H newc --quiet) > "$INITRD"

# --- Run QEMU with timeout, capture output ---
echo "running QEMU integration test (timeout: ${TIMEOUT_SECS}s)..."
OUTPUT_FILE="$BUILD_DIR/test-output.log"

timeout "$TIMEOUT_SECS" \
    qemu-system-x86_64 \
        -kernel "$KERNEL_PATH" \
        -initrd "$INITRD" \
        -append "console=ttyS0 panic=-1" \
        -drive "file=$DISK,format=raw,if=virtio,readonly=on" \
        -drive "file=$BTRFS_DISK,format=raw,if=virtio" \
        -m 256M \
        -nographic \
        -no-reboot \
    > "$OUTPUT_FILE" 2>&1 || true

# --- Check results ---
if grep -q "TEST_RESULT=PASS" "$OUTPUT_FILE"; then
    echo "PASS: integration test succeeded"
    exit 0
elif grep -q "TEST_RESULT=FAIL" "$OUTPUT_FILE"; then
    echo "FAIL: integration test failed"
    echo "--- output ---"
    tail -30 "$OUTPUT_FILE"
    exit 1
else
    echo "FAIL: test did not complete (timeout or crash)"
    echo "--- output ---"
    tail -30 "$OUTPUT_FILE"
    exit 1
fi
