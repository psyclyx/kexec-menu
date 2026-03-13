# QEMU integration test — runnable via nix-build.
#
# Usage:
#   nix-build -A tests.qemu     # produces a script
#   $(nix-build -A tests.qemu)  # builds and runs
#
# Wraps the integration test with all dependencies so no nix-shell is needed.
let
  sources = import ../../npins;
  pkgs = import sources.nixpkgs {};
  kernel = pkgs.linuxPackages.kernel;
  kexec-menu = pkgs.pkgsStatic.callPackage ../../package.nix {
    target = "x86_64-unknown-linux-musl";
  };
  cryptsetupStatic = pkgs.pkgsStatic.cryptsetup;
in
pkgs.writeShellApplication {
  name = "qemu-integration-test";

  runtimeInputs = [
    pkgs.qemu
    pkgs.e2fsprogs       # mke2fs, fuse2fs, debugfs
    pkgs.btrfs-progs     # mkfs.btrfs
    pkgs.xfsprogs        # mkfs.xfs
    pkgs.f2fs-tools      # mkfs.f2fs
    pkgs.fuse            # fusermount
    pkgs.cpio
    pkgs.pkgsStatic.busybox
    pkgs.coreutils
    pkgs.zstd
    pkgs.xz
    pkgs.gzip
    pkgs.kmod
  ];

  text = ''
    set -euo pipefail

    REPO_ROOT="$(cd "$(dirname "''${BASH_SOURCE[0]}")"; pwd)"
    # When run from nix store, use the source tree passed as arg or cwd
    REPO_ROOT="''${1:-$(pwd)}"

    export QEMU_KERNEL="${kernel}/bzImage"
    export QEMU_KERNEL_MODULES="${kernel.modules}/lib/modules/${kernel.modDirVersion}"
    export BUSYBOX_STATIC="${pkgs.pkgsStatic.busybox}/bin/busybox"

    BUILD_DIR="$(mktemp -d)"
    trap 'rm -rf "$BUILD_DIR"' EXIT

    BINARY="${kexec-menu}/bin/kexec-menu"
    TIMEOUT_SECS=90

    echo "binary: $BINARY"
    echo "kernel: $QEMU_KERNEL"
    echo "modules: $QEMU_KERNEL_MODULES"

    # --- Create test disks ---
    DISK="$BUILD_DIR/test-disk.ext4"
    "$REPO_ROOT/tests/qemu/create-test-disk.sh" "$DISK"
    # Empty 64MB disk — formatted as btrfs inside QEMU by init-test.sh
    BTRFS_DISK="$BUILD_DIR/test-disk.raw"
    truncate -s 64M "$BTRFS_DISK"
    # Empty 64MB disk — formatted as LUKS+ext4 inside QEMU by init-test.sh
    LUKS_DISK="$BUILD_DIR/test-disk-luks.raw"
    truncate -s 64M "$LUKS_DISK"
    # 64MB disk — pre-formatted as XFS, populated inside QEMU by init-test.sh
    XFS_DISK="$BUILD_DIR/test-disk-xfs.raw"
    truncate -s 64M "$XFS_DISK"
    mkfs.xfs -f -L "test-xfs" "$XFS_DISK"
    # 64MB disk — pre-formatted as F2FS, populated inside QEMU by init-test.sh
    F2FS_DISK="$BUILD_DIR/test-disk-f2fs.raw"
    truncate -s 64M "$F2FS_DISK"
    mkfs.f2fs -f -l "test-f2fs" "$F2FS_DISK"

    # --- Create initrd ---
    INITRD="$BUILD_DIR/initrd-test.cpio"
    INITRD_DIR="$BUILD_DIR/initrd-test-root"
    mkdir -p "$INITRD_DIR"/{bin,dev,proc,sys,mnt,run,etc,tmp}

    cp "$BUSYBOX_STATIC" "$INITRD_DIR/bin/busybox"
    for cmd in sh mount umount mkdir ls cat sleep poweroff insmod grep echo printf dd mke2fs modprobe depmod; do
        ln -sf busybox "$INITRD_DIR/bin/$cmd"
    done

    cp "$BINARY" "$INITRD_DIR/bin/kexec-menu"
    cp "${pkgs.pkgsStatic.btrfs-progs}/bin/mkfs.btrfs" "$INITRD_DIR/bin/mkfs.btrfs"
    cp "${cryptsetupStatic}/bin/cryptsetup" "$INITRD_DIR/bin/cryptsetup"
    cp "$REPO_ROOT/tests/qemu/init-test.sh" "$INITRD_DIR/init"
    chmod +x "$INITRD_DIR/init"

    # --- Include kernel modules (preserving directory structure for modprobe) ---
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
        # xfs
        "fs/xfs/xfs.ko"
        # f2fs
        "fs/f2fs/f2fs.ko"
        # dm-crypt (for LUKS) and dependency chain
        "drivers/dax/dax.ko"
        "drivers/md/dm-mod.ko"
        "lib/asn1_encoder.ko"
        "drivers/tee/tee.ko"
        "security/keys/trusted-keys/trusted.ko"
        "security/keys/encrypted-keys/encrypted-keys.ko"
        "drivers/md/dm-crypt.ko"
        # crypto (for dm-crypt runtime)
        "crypto/xts.ko"
        "crypto/cryptd.ko"
        "arch/x86/crypto/aesni-intel.ko"
    )

    KMOD_BASE="$INITRD_DIR/lib/modules/${kernel.modDirVersion}"
    mkdir -p "$KMOD_BASE/kernel"

    for mod in "''${NEEDED_MODULES[@]}"; do
        src="$QEMU_KERNEL_MODULES/kernel/$mod"
        for ext in "" ".xz" ".zst" ".gz"; do
            if [[ -f "''${src}''${ext}" ]]; then
                dst_dir="$KMOD_BASE/kernel/$(dirname "$mod")"
                mkdir -p "$dst_dir"
                dst="$dst_dir/$(basename "$mod")"
                case "$ext" in
                    .xz)   xz -d -c "''${src}''${ext}" > "$dst" ;;
                    .zst)  zstd -d -q "''${src}''${ext}" -o "$dst" ;;
                    .gz)   gzip -d -c "''${src}''${ext}" > "$dst" ;;
                    "")    cp "''${src}" "$dst" ;;
                esac
                break
            fi
        done
    done

    # Generate modules.dep for modprobe
    depmod -b "$INITRD_DIR" "${kernel.modDirVersion}"

    (cd "$INITRD_DIR" && find . | cpio -o -H newc --quiet) > "$INITRD"

    # --- Run QEMU with timeout, capture output ---
    echo "running QEMU integration test (timeout: ''${TIMEOUT_SECS}s)..."
    OUTPUT_FILE="$BUILD_DIR/test-output.log"

    timeout "$TIMEOUT_SECS" \
        qemu-system-x86_64 \
            -kernel "$QEMU_KERNEL" \
            -initrd "$INITRD" \
            -append "console=ttyS0 panic=-1" \
            -drive "file=$DISK,format=raw,if=virtio,readonly=on" \
            -drive "file=$BTRFS_DISK,format=raw,if=virtio" \
            -drive "file=$LUKS_DISK,format=raw,if=virtio" \
            -drive "file=$XFS_DISK,format=raw,if=virtio" \
            -drive "file=$F2FS_DISK,format=raw,if=virtio" \
            -cpu max \
            -m 512M \
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
        tail -50 "$OUTPUT_FILE"
        exit 1
    else
        echo "FAIL: test did not complete (timeout or crash)"
        echo "--- output ---"
        tail -50 "$OUTPUT_FILE"
        exit 1
    fi
  '';
}
