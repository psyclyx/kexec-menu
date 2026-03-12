#!/bin/busybox sh
#
# Minimal init for QEMU test environment.
# Mounts basics, runs kexec-menu --dry-run, then powers off.
#

export PATH=/bin

mount -t proc proc /proc
mount -t sysfs sysfs /sys
mount -t devtmpfs devtmpfs /dev

echo ""
echo "=== kexec-menu QEMU test ==="
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
