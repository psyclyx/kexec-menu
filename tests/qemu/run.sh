#!/usr/bin/env bash
#
# Boot kexec-menu in QEMU for manual/integration testing.
#
# Usage:
#   nix-shell --run ./tests/qemu/run.sh
#   ./tests/qemu/run.sh --no-build   # skip cargo build, reuse last binary
#
# Must be run inside nix-shell (provides kernel, modules, and tools).
# Needs: qemu-system-x86_64, busybox (static), mke2fs, cpio, and a Rust
# toolchain targeting x86_64-unknown-linux-musl.
#
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BUILD_DIR="$REPO_ROOT/target/qemu-test"
TARGET=x86_64-unknown-linux-musl

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
    echo "run: nix-shell --run ./tests/qemu/run.sh" >&2
    exit 1
fi

# --- Resolve kernel and modules (from shell.nix, never the host) ---
if [[ -z "${QEMU_KERNEL:-}" || ! -f "$QEMU_KERNEL" ]]; then
    echo "error: QEMU_KERNEL not set or not found" >&2
    echo "run: nix-shell --run ./tests/qemu/run.sh" >&2
    exit 1
fi
if [[ -z "${QEMU_KERNEL_MODULES:-}" || ! -d "$QEMU_KERNEL_MODULES" ]]; then
    echo "error: QEMU_KERNEL_MODULES not set or not found" >&2
    echo "run: nix-shell --run ./tests/qemu/run.sh" >&2
    exit 1
fi

KERNEL_PATH="$QEMU_KERNEL"
MODULES_DIR="$QEMU_KERNEL_MODULES"
echo "kernel: $KERNEL_PATH"
echo "modules: $MODULES_DIR"

# --- Build kexec-menu ---
if ! $SKIP_BUILD; then
    echo "building kexec-menu for $TARGET..."
    cargo build --manifest-path "$REPO_ROOT/Cargo.toml" \
        --target "$TARGET" --release -p kexec-menu 2>&1
fi

BINARY="$REPO_ROOT/target/$TARGET/release/kexec-menu"
if [[ ! -f "$BINARY" ]]; then
    echo "error: binary not found at $BINARY" >&2
    echo "build with: nix-shell --run './tests/qemu/run.sh'" >&2
    exit 1
fi
echo "binary: $BINARY ($(stat -c%s "$BINARY") bytes)"

# Verify it's statically linked
if ldd "$BINARY" &>/dev/null 2>&1; then
    if ! ldd "$BINARY" 2>&1 | grep -q "not a dynamic executable"; then
        echo "warning: binary is dynamically linked, may not work in QEMU" >&2
    fi
fi

# --- Create test disks (tmpfiles, cleaned up on exit) ---
mkdir -p "$BUILD_DIR"
DISK="$(mktemp "$BUILD_DIR/test-disk.XXXXXX.ext4")"
BTRFS_DISK="$(mktemp "$BUILD_DIR/test-disk.XXXXXX.raw")"
cleanup_disks() { rm -f "$DISK" "$BTRFS_DISK"; }
trap cleanup_disks EXIT
"$REPO_ROOT/tests/qemu/create-test-disk.sh" "$DISK"
# Empty 64MB disk — formatted as btrfs inside QEMU by init.sh (if mkfs.btrfs present)
truncate -s 64M "$BTRFS_DISK"

# --- Create initrd ---
INITRD="$BUILD_DIR/initrd.cpio"
INITRD_DIR="$BUILD_DIR/initrd-root"
rm -rf "$INITRD_DIR"
mkdir -p "$INITRD_DIR"/{bin,dev,proc,sys,mnt,run,etc}

if [[ -n "${BUSYBOX_STATIC:-}" && -f "$BUSYBOX_STATIC" ]]; then
    BUSYBOX="$BUSYBOX_STATIC"
else
    BUSYBOX="$(command -v busybox)"
fi
cp "$BUSYBOX" "$INITRD_DIR/bin/busybox"
for cmd in sh mount umount mkdir ls cat sleep poweroff reboot insmod modprobe; do
    ln -sf busybox "$INITRD_DIR/bin/$cmd"
done

cp "$BINARY" "$INITRD_DIR/bin/kexec-menu"
cp "$REPO_ROOT/tests/qemu/init.sh" "$INITRD_DIR/init"
chmod +x "$INITRD_DIR/init"

# --- Include kernel modules in initrd ---
# Modules needed for virtio block device + ext4
NEEDED_MODULES=(
    # virtio core (ring before virtio — virtio.ko depends on virtio_ring)
    "drivers/virtio/virtio_ring.ko"
    "drivers/virtio/virtio.ko"
    "drivers/virtio/virtio_pci_modern_dev.ko"
    "drivers/virtio/virtio_pci_legacy_dev.ko"
    "drivers/virtio/virtio_pci.ko"
    # virtio block
    "drivers/block/virtio_blk.ko"
    # ext4 dependencies (crc16 path varies: lib/crc16.ko or lib/crc/crc16.ko)
    "lib/crc16.ko"
    "lib/crc/crc16.ko"
    "crypto/crc32c-cryptoapi.ko"
    "fs/mbcache.ko"
    "fs/jbd2/jbd2.ko"
    # ext4
    "fs/ext4/ext4.ko"
    # btrfs
    "crypto/xor.ko"
    "lib/raid6/raid6_pq.ko"
    "fs/btrfs/btrfs.ko"
)

KMOD_DIR="$INITRD_DIR/lib/modules"
mkdir -p "$KMOD_DIR"
mod_count=0

for mod in "${NEEDED_MODULES[@]}"; do
    src="$MODULES_DIR/kernel/$mod"
    # Try with .xz, .zst, .gz compression
    for ext in "" ".xz" ".zst" ".gz"; do
        if [[ -f "${src}${ext}" ]]; then
            dst="$KMOD_DIR/$(basename "$mod")"
            case "$ext" in
                .xz)   xz -d -c "${src}${ext}" > "$dst" ;;
                .zst)  zstd -d -q "${src}${ext}" -o "$dst" ;;
                .gz)   gzip -d -c "${src}${ext}" > "$dst" ;;
                "")    cp "${src}" "$dst" ;;
            esac
            mod_count=$((mod_count + 1))
            break
        fi
    done
done

echo "modules: $mod_count included in initrd"

(cd "$INITRD_DIR" && find . | cpio -o -H newc --quiet) > "$INITRD"
echo "initrd: $INITRD ($(stat -c%s "$INITRD") bytes)"

# --- Run QEMU ---
echo ""
echo "=== Starting QEMU ==="
echo "  Ctrl-A X to exit"
echo ""

qemu-system-x86_64 \
    -kernel "$KERNEL_PATH" \
    -initrd "$INITRD" \
    -append "console=ttyS0 panic=-1" \
    -drive "file=$DISK,format=raw,if=virtio,readonly=on" \
    -drive "file=$BTRFS_DISK,format=raw,if=virtio" \
    -m 256M \
    -nographic \
    -no-reboot
