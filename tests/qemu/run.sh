#!/usr/bin/env bash
#
# Boot kexec-menu in QEMU for manual/integration testing.
#
# Usage:
#   nix-shell tests/qemu/shell.nix --run ./tests/qemu/run.sh
#   ./tests/qemu/run.sh --no-build   # skip cargo build, reuse last binary
#   KERNEL=/path/to/vmlinuz ./tests/qemu/run.sh  # custom kernel
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
for cmd in sh mount umount mkdir ls cat sleep poweroff reboot; do
    ln -sf busybox "$INITRD_DIR/bin/$cmd"
done

cp "$BINARY" "$INITRD_DIR/bin/kexec-menu"
cp "$REPO_ROOT/tests/qemu/init.sh" "$INITRD_DIR/init"
chmod +x "$INITRD_DIR/init"

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
