#!/bin/busybox sh
#
# Automated test init for QEMU integration tests.
# Runs kexec-menu --auto-default --dry-run, checks output, reports result.
#
# Expects:
#   /dev/vda — pre-formatted ext4 disk with boot tree (read-only)
#   /dev/vdb — empty raw disk, formatted as btrfs here (if mkfs.btrfs present)
#   /dev/vdc — empty raw disk, formatted as LUKS+ext4 here (if cryptsetup present)
#

export PATH=/bin

mount -t proc proc /proc
mount -t sysfs sysfs /sys
mount -t devtmpfs devtmpfs /dev

echo ""
echo "=== kexec-menu integration test ==="
echo ""

# Load kernel modules via modprobe (handles dependencies automatically)
if [ -d /lib/modules ]; then
    echo "loading kernel modules..."
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
            echo "  loaded: $mod"
        else
            echo "  skip: $mod"
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

# Set up LUKS+ext4 test disk on /dev/vdc if cryptsetup is available
LUKS_SETUP=false
LUKS_PASSPHRASE="test-passphrase"
if [ -b /dev/vdc ] && [ -x /bin/cryptsetup ]; then
    echo "setting up LUKS+ext4 test disk on /dev/vdc..."

    # Create /dev/mapper if needed (devtmpfs may not have it)
    mkdir -p /dev/mapper

    # Format as LUKS with a known passphrase (use pbkdf2 for speed in tests)
    if echo -n "$LUKS_PASSPHRASE" | cryptsetup luksFormat \
            --type luks2 --pbkdf pbkdf2 --pbkdf-force-iterations 1000 \
            --batch-mode /dev/vdc; then
        echo "  LUKS formatted"

        # Open the LUKS container
        if echo -n "$LUKS_PASSPHRASE" | cryptsetup open --type luks \
                --key-file - /dev/vdc test-luks; then
            echo "  LUKS opened as /dev/mapper/test-luks"

            # Format inner device as ext4 and populate with boot entries
            if mke2fs -F -L "test-luks-inner" /dev/mapper/test-luks 2>/dev/null; then
                mkdir -p /mnt/luks
                if mount -t ext4 /dev/mapper/test-luks /mnt/luks; then
                    mkdir -p /mnt/luks/boot/nixos/generation-1
                    echo "DUMMY KERNEL" > /mnt/luks/boot/nixos/generation-1/vmlinuz
                    echo "DUMMY INITRD" > /mnt/luks/boot/nixos/generation-1/initrd
                    cat > /mnt/luks/boot/nixos/generation-1/entries.json <<'JSON'
[
  {"name": "default", "kernel": "vmlinuz", "initrd": "initrd", "cmdline": "console=ttyS0 root=/dev/dm-0"}
]
JSON
                    umount /mnt/luks
                    # Close the LUKS volume so kexec-menu sees it as encrypted
                    cryptsetup close test-luks
                    LUKS_SETUP=true
                    echo "  LUKS+ext4 disk ready (closed)"
                else
                    echo "  WARN: failed to mount ext4 inside LUKS"
                    cryptsetup close test-luks
                fi
            else
                echo "  WARN: mke2fs failed inside LUKS"
                cryptsetup close test-luks
            fi
        else
            echo "  WARN: cryptsetup open failed"
        fi
    else
        echo "  WARN: cryptsetup luksFormat failed"
    fi
elif [ -b /dev/vdc ]; then
    # Fallback: write LUKS magic bytes if cryptsetup not available
    echo "writing LUKS magic header to /dev/vdc (no cryptsetup)..."
    printf 'LUKS\272\276' > /dev/vdc
    LUKS_SETUP=true
    echo "  LUKS magic written"
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

# If LUKS was set up with cryptsetup, verify kexec-menu detected it as encrypted
if [ "$LUKS_SETUP" = true ]; then
    if busybox grep -q "vdc" /proc/mounts 2>/dev/null; then
        echo "FAIL: LUKS device vdc should not be mounted (encrypted)"
        PASS=false
    else
        echo "OK: LUKS source detected as encrypted (not mounted)"
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
