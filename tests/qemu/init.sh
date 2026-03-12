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

# Load kernel modules via modprobe (handles dependencies automatically)
if [ -d /lib/modules ]; then
    echo "Loading kernel modules..."
    for mod in \
        virtio_ring virtio \
        virtio_pci_modern_dev virtio_pci_legacy_dev virtio_pci \
        virtio_blk \
        crc16 crc32c-cryptoapi mbcache jbd2 \
        ext4 \
        xor raid6_pq btrfs \
        cryptd aesni-intel xts \
        dm-crypt; do
        if modprobe "$mod" 2>/dev/null; then
            echo "  loaded $mod"
        else
            echo "  skip $mod (not found or error)"
        fi
    done
    echo ""
    # Give the kernel a moment to settle devices
    sleep 1
fi

# Set up LUKS+ext4 on /dev/vdc if cryptsetup is available
if [ -b /dev/vdc ] && [ -x /bin/cryptsetup ]; then
    echo "Setting up LUKS+ext4 on /dev/vdc..."
    mkdir -p /dev/mapper
    LUKS_PASS="test-passphrase"
    if echo -n "$LUKS_PASS" | cryptsetup luksFormat \
            --type luks2 --pbkdf pbkdf2 --pbkdf-force-iterations 1000 \
            --batch-mode /dev/vdc; then
        if echo -n "$LUKS_PASS" | cryptsetup open --type luks \
                --key-file - /dev/vdc test-luks; then
            mke2fs -F -L "test-luks-inner" /dev/mapper/test-luks 2>/dev/null
            mkdir -p /mnt/luks
            mount -t ext4 /dev/mapper/test-luks /mnt/luks 2>/dev/null
            mkdir -p /mnt/luks/boot/nixos/generation-1
            echo "DUMMY KERNEL" > /mnt/luks/boot/nixos/generation-1/vmlinuz
            echo "DUMMY INITRD" > /mnt/luks/boot/nixos/generation-1/initrd
            cat > /mnt/luks/boot/nixos/generation-1/entries.json <<'JSON'
[
  {"name": "default", "kernel": "vmlinuz", "initrd": "initrd", "cmdline": "console=ttyS0 root=/dev/dm-0"}
]
JSON
            umount /mnt/luks
            cryptsetup close test-luks
            echo "LUKS+ext4 disk ready (closed)"
        fi
    fi
elif [ -b /dev/vdc ]; then
    printf 'LUKS\272\276' > /dev/vdc
    echo "LUKS magic written to /dev/vdc (no cryptsetup)"
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
