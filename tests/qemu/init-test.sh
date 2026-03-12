#!/bin/busybox sh
#
# Automated test init for QEMU integration tests.
# Runs kexec-menu --auto-default --dry-run, checks output, reports result.
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
        ext4; do
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

echo ""
if [ "$PASS" = true ]; then
    echo "TEST_RESULT=PASS"
else
    echo "TEST_RESULT=FAIL"
fi
echo ""

poweroff -f
