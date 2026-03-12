#!/usr/bin/env bash
#
# Boot kexec-menu in QEMU for manual/integration testing.
#
# Usage:
#   nix-shell tests/qemu/shell.nix --run ./tests/qemu/run.sh
#   ./tests/qemu/run.sh --no-build   # skip cargo build, reuse last binary
#
# Dependencies are provided by tests/qemu/shell.nix (recommended) or PATH.
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
    echo "run: nix-shell tests/qemu/shell.nix --run ./tests/qemu/run.sh" >&2
    exit 1
fi

# --- Resolve kernel ---
find_kernel() {
    # Prefer Nix-provided kernel (from shell.nix)
    if [[ -n "${QEMU_KERNEL:-}" && -f "$QEMU_KERNEL" ]]; then
        echo "$QEMU_KERNEL"
        return
    fi
    if [[ -n "${KERNEL:-}" ]]; then
        echo "$KERNEL"
        return
    fi
    # NixOS host kernel
    if [[ -e /run/current-system/kernel ]]; then
        local kdir
        kdir="$(readlink -f /run/current-system/kernel)"
        if [[ -f "$kdir/bzImage" ]]; then
            echo "$kdir/bzImage"
            return
        fi
    fi
    for p in /boot/vmlinuz-linux /boot/vmlinuz; do
        [[ -f "$p" ]] && echo "$p" && return
    done
    echo "error: cannot find a kernel, set KERNEL=/path/to/vmlinuz" >&2
    return 1
}

KERNEL_PATH="$(find_kernel)"
echo "kernel: $KERNEL_PATH"

# --- Resolve kernel modules ---
find_modules_dir() {
    # Prefer Nix-provided modules (from shell.nix)
    if [[ -n "${QEMU_KERNEL_MODULES:-}" && -d "$QEMU_KERNEL_MODULES" ]]; then
        echo "$QEMU_KERNEL_MODULES"
        return
    fi
    # Try host system
    local kver
    kver="$(uname -r)"
    for d in "/run/current-system/kernel-modules/lib/modules/$kver" "/lib/modules/$kver"; do
        [[ -d "$d" ]] && echo "$d" && return
    done
    echo ""
}

MODULES_DIR="$(find_modules_dir)"
if [[ -n "$MODULES_DIR" ]]; then
    echo "modules: $MODULES_DIR"
else
    echo "warning: no kernel modules dir found, QEMU may not have ext4/virtio" >&2
fi

# --- Build kexec-menu ---
if ! $SKIP_BUILD; then
    echo "building kexec-menu for $TARGET..."
    cargo build --manifest-path "$REPO_ROOT/Cargo.toml" \
        --target "$TARGET" --release -p kexec-menu 2>&1
fi

BINARY="$REPO_ROOT/target/$TARGET/release/kexec-menu"
if [[ ! -f "$BINARY" ]]; then
    echo "error: binary not found at $BINARY" >&2
    echo "build with: nix-shell tests/qemu/shell.nix --run './tests/qemu/run.sh'" >&2
    exit 1
fi
echo "binary: $BINARY ($(stat -c%s "$BINARY") bytes)"

# Verify it's statically linked
if ldd "$BINARY" &>/dev/null 2>&1; then
    if ! ldd "$BINARY" 2>&1 | grep -q "not a dynamic executable"; then
        echo "warning: binary is dynamically linked, may not work in QEMU" >&2
    fi
fi

# --- Create test disk ---
mkdir -p "$BUILD_DIR"
DISK="$BUILD_DIR/test-disk.ext4"
"$REPO_ROOT/tests/qemu/create-test-disk.sh" "$DISK"

# --- Create initrd ---
INITRD="$BUILD_DIR/initrd.cpio"
INITRD_DIR="$BUILD_DIR/initrd-root"
rm -rf "$INITRD_DIR"
mkdir -p "$INITRD_DIR"/{bin,dev,proc,sys,mnt,run,etc}

BUSYBOX="$(command -v busybox)"
cp "$BUSYBOX" "$INITRD_DIR/bin/busybox"
for cmd in sh mount umount mkdir ls cat sleep poweroff reboot insmod modprobe; do
    ln -sf busybox "$INITRD_DIR/bin/$cmd"
done

cp "$BINARY" "$INITRD_DIR/bin/kexec-menu"
cp "$REPO_ROOT/tests/qemu/init.sh" "$INITRD_DIR/init"
chmod +x "$INITRD_DIR/init"

# --- Include kernel modules in initrd ---
if [[ -n "$MODULES_DIR" ]]; then
    # Modules needed for virtio block device + ext4
    NEEDED_MODULES=(
        # virtio core
        "drivers/virtio/virtio.ko"
        "drivers/virtio/virtio_ring.ko"
        "drivers/virtio/virtio_pci_modern_dev.ko"
        "drivers/virtio/virtio_pci_legacy_dev.ko"
        "drivers/virtio/virtio_pci.ko"
        # virtio block
        "drivers/block/virtio_blk.ko"
        # ext4 dependencies (crc16 path varies: lib/crc16.ko or lib/crc/crc16.ko)
        "lib/crc16.ko"
        "lib/crc/crc16.ko"
        "crypto/crc32c_generic.ko"
        "lib/libcrc32c.ko"
        "fs/mbcache.ko"
        "fs/jbd2/jbd2.ko"
        # ext4
        "fs/ext4/ext4.ko"
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
fi

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
    -m 256M \
    -nographic \
    -no-reboot
