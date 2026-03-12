#!/bin/busybox sh
#
# Automated test init for QEMU integration tests.
# Runs kexec-menu --auto-default --dry-run, checks output, reports result.
#
# Expects:
#   /dev/vda — pre-formatted ext4 disk with boot tree (read-only)
#   /dev/vdb — empty raw disk, formatted as btrfs here (if mkfs.btrfs present)
#

export PATH=/bin

mount -t proc proc /proc
mount -t sysfs sysfs /sys
mount -t devtmpfs devtmpfs /dev

echo ""
echo "=== kexec-menu integration test ==="
echo ""

# Load kernel modules
if [ -d /lib/modules ]; then
    echo "loading kernel modules..."
    for mod in \
        virtio_ring virtio \
        virtio_pci_modern_dev virtio_pci_legacy_dev virtio_pci \
        virtio_blk \
        crc16 crc32c-cryptoapi mbcache jbd2 \
        ext4 \
        xor raid6_pq btrfs; do
        ko="/lib/modules/${mod}.ko"
        if [ -f "$ko" ]; then
            if insmod "$ko"; then
                echo "  loaded: $mod"
            else
                echo "  FAILED: $mod (exit $?)"
            fi
        else
            echo "  skip: $mod (not found)"
        fi
    done
    echo "modules done, waiting for devices..."
    sleep 1
fi

# Set up btrfs test disk on /dev/vdb if mkfs.btrfs is available
BTRFS_SETUP=false
if [ -b /dev/vdb ] && [ -x /bin/mkfs.btrfs ]; then
    echo "setting up btrfs test disk on /dev/vdb..."
    if mkfs.btrfs -f -L "test-btrfs" /dev/vdb >/dev/null 2>&1; then
        mkdir -p /mnt/btrfs
        if mount -t btrfs /dev/vdb /mnt/btrfs; then
            # Populate with boot tree (same structure as ext4 disk)
            mkdir -p /mnt/btrfs/boot/nixos/generation-1
            echo "DUMMY KERNEL" > /mnt/btrfs/boot/nixos/generation-1/vmlinuz
            echo "DUMMY INITRD" > /mnt/btrfs/boot/nixos/generation-1/initrd
            cat > /mnt/btrfs/boot/nixos/generation-1/entries.json <<'JSON'
[
  {"name": "default", "kernel": "vmlinuz", "initrd": "initrd", "cmdline": "console=ttyS0 root=/dev/vdb"}
]
JSON
            umount /mnt/btrfs
            BTRFS_SETUP=true
            echo "  btrfs disk ready"
        else
            echo "  WARN: failed to mount btrfs"
        fi
    else
        echo "  WARN: mkfs.btrfs failed"
    fi
elif [ -b /dev/vdb ]; then
    echo "  skip: btrfs setup (mkfs.btrfs not found)"
fi

# Run kexec-menu in auto-default dry-run mode, capture stderr
/bin/kexec-menu --dry-run --auto-default 2>/tmp/kexec-output
STATUS=$?

OUTPUT="$(cat /tmp/kexec-output)"
echo "$OUTPUT"
echo ""

# Validate output
PASS=true

if [ "$STATUS" -ne 0 ]; then
    echo "FAIL: exit status $STATUS"
    PASS=false
fi

if ! echo "$OUTPUT" | busybox grep -q "would boot:"; then
    echo "FAIL: missing 'would boot:' in output"
    PASS=false
fi

if ! echo "$OUTPUT" | busybox grep -q "kernel:"; then
    echo "FAIL: missing 'kernel:' in output"
    PASS=false
fi

if ! echo "$OUTPUT" | busybox grep -q "initrd:"; then
    echo "FAIL: missing 'initrd:' in output"
    PASS=false
fi

# If btrfs was set up, verify kexec-menu mounted it
if [ "$BTRFS_SETUP" = true ]; then
    if busybox grep -q "btrfs" /proc/mounts 2>/dev/null; then
        echo "OK: btrfs source was mounted by kexec-menu"
    else
        echo "FAIL: btrfs source not mounted (expected /mnt/kexec-menu/vdb)"
        PASS=false
    fi
fi

echo ""
if [ "$PASS" = true ]; then
    echo "TEST_RESULT=PASS"
else
    echo "TEST_RESULT=FAIL"
fi
echo ""

poweroff -f
