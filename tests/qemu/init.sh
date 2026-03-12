#!/bin/busybox sh
#
# Minimal init for QEMU test environment.
# Mounts basics, loads modules, runs kexec-menu --dry-run, then powers off.
#

export PATH=/bin

mount -t proc proc /proc
mount -t sysfs sysfs /sys
mount -t devtmpfs devtmpfs /dev

echo ""
echo "=== kexec-menu QEMU test ==="
echo ""

# Load kernel modules (decompressed .ko files in /lib/modules/)
if [ -d /lib/modules ]; then
    echo "Loading kernel modules..."
    # Load order matters: dependencies first
    for mod in \
        virtio virtio_ring \
        virtio_pci_modern_dev virtio_pci_legacy_dev virtio_pci \
        virtio_blk \
        crc16 crc32c_generic libcrc32c mbcache jbd2 \
        ext4; do
        ko="/lib/modules/${mod}.ko"
        if [ -f "$ko" ]; then
            insmod "$ko" 2>/dev/null && echo "  loaded $mod" || echo "  skip $mod (already loaded or error)"
        fi
    done
    echo ""
    # Give the kernel a moment to settle devices
    sleep 1
fi

# Show discovered block devices
echo "Block devices:"
ls -la /dev/vd* 2>/dev/null || echo "  (no virtio devices)"
echo ""

# Run the boot menu in dry-run mode
/bin/kexec-menu --dry-run
STATUS=$?

echo ""
echo "=== kexec-menu exited with status $STATUS ==="
echo ""

# Drop to shell on failure for debugging, otherwise power off
if [ "$STATUS" -ne 0 ]; then
    echo "Dropping to rescue shell (type 'poweroff' to exit)"
    exec /bin/sh
fi

poweroff -f
