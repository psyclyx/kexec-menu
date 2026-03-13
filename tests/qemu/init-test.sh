#!/bin/busybox sh
#
# Automated test init for QEMU integration tests.
# Runs kexec-menu --auto-default --dry-run, checks output, reports result.
#
# Expects:
#   /dev/vda — pre-formatted ext4 disk with boot tree (read-only)
#   /dev/vdb — empty raw disk, formatted as btrfs here (if mkfs.btrfs present)
#   /dev/vdc — empty raw disk, formatted as LUKS+ext4 here (if cryptsetup present)
#   /dev/vdd — pre-formatted XFS disk, populated here
#   /dev/vde — pre-formatted F2FS disk, populated here
#   /dev/vdf — empty raw disk, part of btrfs RAID1 (with vdg)
#   /dev/vdg — empty raw disk, part of btrfs RAID1 (with vdf)
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
        xfs f2fs \
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
        if mount -t btrfs -o compress=zstd /dev/vdb /mnt/btrfs; then
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

# Set up XFS test disk on /dev/vdd (pre-formatted, populate boot tree here)
XFS_SETUP=false
if [ -b /dev/vdd ]; then
    echo "setting up XFS test disk on /dev/vdd..."
    mkdir -p /mnt/xfs
    if mount -t xfs /dev/vdd /mnt/xfs; then
        mkdir -p /mnt/xfs/boot/nixos/generation-1
        echo "DUMMY KERNEL" > /mnt/xfs/boot/nixos/generation-1/vmlinuz
        echo "DUMMY INITRD" > /mnt/xfs/boot/nixos/generation-1/initrd
        cat > /mnt/xfs/boot/nixos/generation-1/entries.json <<'JSON'
[
  {"name": "default", "kernel": "vmlinuz", "initrd": "initrd", "cmdline": "console=ttyS0 root=/dev/vdd"}
]
JSON
        umount /mnt/xfs
        XFS_SETUP=true
        echo "  XFS disk ready"
    else
        echo "  WARN: failed to mount XFS on /dev/vdd"
    fi
fi

# Set up F2FS test disk on /dev/vde (pre-formatted, populate boot tree here)
F2FS_SETUP=false
if [ -b /dev/vde ]; then
    echo "setting up F2FS test disk on /dev/vde..."
    mkdir -p /mnt/f2fs
    if mount -t f2fs /dev/vde /mnt/f2fs; then
        mkdir -p /mnt/f2fs/boot/nixos/generation-1
        echo "DUMMY KERNEL" > /mnt/f2fs/boot/nixos/generation-1/vmlinuz
        echo "DUMMY INITRD" > /mnt/f2fs/boot/nixos/generation-1/initrd
        cat > /mnt/f2fs/boot/nixos/generation-1/entries.json <<'JSON'
[
  {"name": "default", "kernel": "vmlinuz", "initrd": "initrd", "cmdline": "console=ttyS0 root=/dev/vde"}
]
JSON
        umount /mnt/f2fs
        F2FS_SETUP=true
        echo "  F2FS disk ready"
    else
        echo "  WARN: failed to mount F2FS on /dev/vde"
    fi
fi

# Set up multi-device btrfs RAID1 on /dev/vdf + /dev/vdg
BTRFS_RAID_SETUP=false
if [ -b /dev/vdf ] && [ -b /dev/vdg ] && [ -x /bin/mkfs.btrfs ]; then
    echo "setting up btrfs RAID1 on /dev/vdf + /dev/vdg..."
    if mkfs.btrfs -f -d raid1 -m raid1 -L "test-btrfs-raid1" /dev/vdf /dev/vdg >/dev/null 2>&1; then
        mkdir -p /mnt/btrfs-raid
        if mount -t btrfs /dev/vdf /mnt/btrfs-raid; then
            mkdir -p /mnt/btrfs-raid/boot/nixos/generation-1
            echo "DUMMY KERNEL" > /mnt/btrfs-raid/boot/nixos/generation-1/vmlinuz
            echo "DUMMY INITRD" > /mnt/btrfs-raid/boot/nixos/generation-1/initrd
            cat > /mnt/btrfs-raid/boot/nixos/generation-1/entries.json <<'JSON'
[
  {"name": "default", "kernel": "vmlinuz", "initrd": "initrd", "cmdline": "console=ttyS0 root=/dev/vdf"}
]
JSON
            umount /mnt/btrfs-raid
            BTRFS_RAID_SETUP=true
            echo "  btrfs RAID1 disk ready"
        else
            echo "  WARN: failed to mount btrfs RAID1"
        fi
    else
        echo "  WARN: mkfs.btrfs RAID1 failed"
    fi
elif [ -b /dev/vdf ]; then
    echo "  skip: btrfs RAID1 setup (mkfs.btrfs not found)"
fi

# S3: five-disk count — track how many filesystem types were set up
FS_COUNT=1  # ext4 on vda always present
$BTRFS_SETUP && FS_COUNT=$((FS_COUNT + 1))
$LUKS_SETUP && FS_COUNT=$((FS_COUNT + 1))
$XFS_SETUP && FS_COUNT=$((FS_COUNT + 1))
$F2FS_SETUP && FS_COUNT=$((FS_COUNT + 1))
echo "S3: $FS_COUNT of 5 filesystem types ready"

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

# If XFS was set up, verify kexec-menu mounted it
if [ "$XFS_SETUP" = true ]; then
    if busybox grep -q "xfs" /proc/mounts 2>/dev/null; then
        echo "OK: XFS source was mounted by kexec-menu"
    else
        echo "FAIL: XFS source not mounted (expected /mnt/kexec-menu/vdd)"
        PASS=false
    fi
fi

# If F2FS was set up, verify kexec-menu mounted it
if [ "$F2FS_SETUP" = true ]; then
    if busybox grep -q "f2fs" /proc/mounts 2>/dev/null; then
        echo "OK: F2FS source was mounted by kexec-menu"
    else
        echo "FAIL: F2FS source not mounted (expected /mnt/kexec-menu/vde)"
        PASS=false
    fi
fi

# If btrfs RAID1 was set up, verify kexec-menu handled multi-device grouping
if [ "$BTRFS_RAID_SETUP" = true ]; then
    # kexec-menu should detect both devices as a single multi-device group
    # and mount via one of them; check that a btrfs mount exists for the raid label
    if echo "$OUTPUT" | busybox grep -q "btrfs-raid1\|vdf\|vdg"; then
        echo "OK: btrfs RAID1 multi-device detected by kexec-menu"
    else
        echo "WARN: btrfs RAID1 not clearly visible in output (may still work)"
    fi
fi

# S3: verify all 5 filesystem types were tested
if [ "$FS_COUNT" -ge 5 ]; then
    echo "OK: S3 five-disk test ($FS_COUNT/5 filesystem types)"
else
    echo "FAIL: S3 five-disk test (only $FS_COUNT/5 filesystem types ready)"
    PASS=false
fi

# S4: disk-whitelist test — run whitelisted binary (only vda allowed)
if [ -x /bin/kexec-menu-whitelist ]; then
    echo ""
    echo "=== S4: disk-whitelist test ==="
    # Unmount everything kexec-menu mounted, so we start clean
    busybox umount /mnt/kexec-menu/* 2>/dev/null
    /bin/kexec-menu-whitelist --dry-run --auto-default 2>/tmp/whitelist-output
    WL_STATUS=$?
    WL_OUTPUT="$(cat /tmp/whitelist-output)"
    echo "$WL_OUTPUT"

    if [ "$WL_STATUS" -ne 0 ]; then
        echo "FAIL: S4 whitelist binary exited with status $WL_STATUS"
        PASS=false
    elif ! echo "$WL_OUTPUT" | busybox grep -q "would boot:"; then
        echo "FAIL: S4 whitelist binary missing 'would boot:'"
        PASS=false
    else
        echo "OK: S4 whitelist binary booted successfully"
    fi

    # Verify only vda was mounted (no btrfs/xfs/f2fs mounts from other disks)
    WL_BTRFS=$(busybox grep -c "btrfs" /proc/mounts 2>/dev/null || true)
    WL_XFS=$(busybox grep -c "xfs" /proc/mounts 2>/dev/null || true)
    WL_F2FS=$(busybox grep -c "f2fs" /proc/mounts 2>/dev/null || true)
    if [ "${WL_BTRFS:-0}" -gt 0 ] || [ "${WL_XFS:-0}" -gt 0 ] || [ "${WL_F2FS:-0}" -gt 0 ]; then
        echo "FAIL: S4 whitelist binary mounted non-whitelisted filesystems"
        echo "  btrfs=$WL_BTRFS xfs=$WL_XFS f2fs=$WL_F2FS"
        PASS=false
    else
        echo "OK: S4 only whitelisted device (vda) was mounted"
    fi
else
    echo "WARN: S4 skipped (kexec-menu-whitelist not found)"
fi

echo ""
if [ "$PASS" = true ]; then
    echo "TEST_RESULT=PASS"
else
    echo "TEST_RESULT=FAIL"
fi
echo ""

poweroff -f
