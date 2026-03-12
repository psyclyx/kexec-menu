#!/usr/bin/env bash
#
# Create a test ext4 disk image with a boot tree for QEMU testing.
#
# Usage: create-test-disk.sh <output-path>
#
# Creates a ~32MB ext4 image with:
#   boot/nixos/generation-1/  (older, one entry)
#   boot/nixos/generation-2/  (newer, two entries)
#
set -euo pipefail

OUTPUT="${1:?usage: create-test-disk.sh <output-path>}"
SIZE_MB=32

# --- Helper functions ---

populate_tree() {
    local root="$1"
    mkdir -p "$root/boot/nixos/generation-1"
    mkdir -p "$root/boot/nixos/generation-2"

    # generation-1: older
    echo "DUMMY KERNEL" > "$root/boot/nixos/generation-1/vmlinuz"
    echo "DUMMY INITRD" > "$root/boot/nixos/generation-1/initrd"
    cat > "$root/boot/nixos/generation-1/entries.json" <<'JSON'
[
  {"name": "default", "kernel": "vmlinuz", "initrd": "initrd", "cmdline": "console=ttyS0 root=/dev/vda1"}
]
JSON
    touch -d "2025-01-01" "$root/boot/nixos/generation-1"

    # generation-2: newer, two entries
    echo "DUMMY KERNEL" > "$root/boot/nixos/generation-2/vmlinuz"
    echo "DUMMY INITRD" > "$root/boot/nixos/generation-2/initrd"
    cat > "$root/boot/nixos/generation-2/entries.json" <<'JSON'
[
  {"name": "default", "kernel": "vmlinuz", "initrd": "initrd", "cmdline": "console=ttyS0 root=/dev/vda1 quiet"},
  {"name": "gaming", "kernel": "vmlinuz", "initrd": "initrd", "cmdline": "console=ttyS0 root=/dev/vda1 preempt=full"}
]
JSON
}

populate_with_debugfs() {
    local img="$1"
    local tmpdir
    tmpdir="$(mktemp -d)"
    trap "rm -rf $tmpdir" RETURN

    echo "DUMMY KERNEL" > "$tmpdir/vmlinuz"
    echo "DUMMY INITRD" > "$tmpdir/initrd"

    cat > "$tmpdir/entries1.json" <<'JSON'
[
  {"name": "default", "kernel": "vmlinuz", "initrd": "initrd", "cmdline": "console=ttyS0 root=/dev/vda1"}
]
JSON

    cat > "$tmpdir/entries2.json" <<'JSON'
[
  {"name": "default", "kernel": "vmlinuz", "initrd": "initrd", "cmdline": "console=ttyS0 root=/dev/vda1 quiet"},
  {"name": "gaming", "kernel": "vmlinuz", "initrd": "initrd", "cmdline": "console=ttyS0 root=/dev/vda1 preempt=full"}
]
JSON

    debugfs -w "$img" -f /dev/stdin <<CMDS
mkdir boot
mkdir boot/nixos
mkdir boot/nixos/generation-1
mkdir boot/nixos/generation-2
cd boot/nixos/generation-1
write $tmpdir/vmlinuz vmlinuz
write $tmpdir/initrd initrd
write $tmpdir/entries1.json entries.json
cd /boot/nixos/generation-2
write $tmpdir/vmlinuz vmlinuz
write $tmpdir/initrd initrd
write $tmpdir/entries2.json entries.json
CMDS
}

# --- Main ---

# Create sparse image and format
truncate -s "${SIZE_MB}M" "$OUTPUT"
mke2fs -t ext4 -L "test-boot" -q "$OUTPUT"

# Try fuse2fs (no root needed), then root mount, then debugfs
MOUNT_DIR="$(mktemp -d)"
cleanup_mount() {
    if mountpoint -q "$MOUNT_DIR" 2>/dev/null; then
        if command -v fusermount &>/dev/null; then
            fusermount -u "$MOUNT_DIR" 2>/dev/null || umount "$MOUNT_DIR" 2>/dev/null || true
        else
            umount "$MOUNT_DIR" 2>/dev/null || true
        fi
    fi
    rmdir "$MOUNT_DIR" 2>/dev/null || true
}
trap cleanup_mount EXIT

if command -v fuse2fs &>/dev/null; then
    fuse2fs "$OUTPUT" "$MOUNT_DIR" -o fakeroot 2>/dev/null
    populate_tree "$MOUNT_DIR"
    fusermount -u "$MOUNT_DIR"
    echo "disk: $OUTPUT (${SIZE_MB}MB ext4, label=test-boot)"
elif [[ $EUID -eq 0 ]]; then
    mount -o loop "$OUTPUT" "$MOUNT_DIR"
    populate_tree "$MOUNT_DIR"
    umount "$MOUNT_DIR"
    echo "disk: $OUTPUT (${SIZE_MB}MB ext4, label=test-boot)"
elif command -v debugfs &>/dev/null; then
    populate_with_debugfs "$OUTPUT"
    echo "disk: $OUTPUT (${SIZE_MB}MB ext4, label=test-boot, via debugfs)"
else
    echo "error: need fuse2fs, root, or debugfs to populate disk image" >&2
    exit 1
fi
