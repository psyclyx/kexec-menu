#!/bin/sh
# mkinitrd.sh — assemble a CPIO initrd for kexec-menu
#
# Creates a minimal initramfs containing the kexec-menu binary, filesystem
# and encryption tools, busybox applets, and the init script.
#
# Required environment variables (paths to static binaries for target arch):
#   KEXEC_MENU    — path to kexec-menu static binary
#   BUSYBOX       — path to busybox static binary
#   CRYPTSETUP    — path to cryptsetup static binary
#   BCACHEFS      — path to bcachefs static binary
#
# Optional:
#   STATIC_JSON   — path to static.json boot entries file
#   EXTRA_DIR     — path to directory whose contents are copied into the initrd root
#   RESCUE_SHELL  — set to "1" to include rescue shell applets (sh, ls, cat, etc.)
#   OUTPUT        — output file path (default: stdout)
#
# Usage:
#   KEXEC_MENU=target/x86_64-unknown-linux-musl/release/kexec-menu \
#   BUSYBOX=/usr/bin/busybox CRYPTSETUP=/usr/bin/cryptsetup \
#   BCACHEFS=/usr/bin/bcachefs ./scripts/mkinitrd.sh > initrd.cpio
#
# Dependencies: cpio, standard coreutils

set -eu

die() { echo "mkinitrd: error: $1" >&2; exit 1; }

# --- Validate required inputs ---

[ -n "${KEXEC_MENU:-}" ] || die "KEXEC_MENU not set"
[ -n "${BUSYBOX:-}" ]    || die "BUSYBOX not set"
[ -n "${CRYPTSETUP:-}" ] || die "CRYPTSETUP not set"
[ -n "${BCACHEFS:-}" ]   || die "BCACHEFS not set"

[ -f "$KEXEC_MENU" ]  || die "KEXEC_MENU not found: $KEXEC_MENU"
[ -f "$BUSYBOX" ]      || die "BUSYBOX not found: $BUSYBOX"
[ -f "$CRYPTSETUP" ]   || die "CRYPTSETUP not found: $CRYPTSETUP"
[ -f "$BCACHEFS" ]     || die "BCACHEFS not found: $BCACHEFS"

command -v cpio >/dev/null 2>&1 || die "cpio not found"

# --- Set up working directory ---

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

ROOT="$WORK/initrd-root"
mkdir -p "$ROOT"/{bin,dev,proc,sys,mnt,run,tmp,etc/kexec-menu}

# --- Init script ---

if [ -n "${INIT_SCRIPT:-}" ]; then
  INIT_SRC="$INIT_SCRIPT"
else
  SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
  INIT_SRC="$SCRIPT_DIR/../uki/initrd/init"
fi
[ -f "$INIT_SRC" ] || die "init script not found: $INIT_SRC"
cp "$INIT_SRC" "$ROOT/init"
chmod +x "$ROOT/init"

# --- Binaries ---

cp "$KEXEC_MENU"  "$ROOT/bin/kexec-menu"
cp "$BUSYBOX"     "$ROOT/bin/busybox"
cp "$CRYPTSETUP"  "$ROOT/bin/cryptsetup"
cp "$BCACHEFS"    "$ROOT/bin/bcachefs"

chmod +x "$ROOT/bin/kexec-menu" "$ROOT/bin/busybox" \
         "$ROOT/bin/cryptsetup" "$ROOT/bin/bcachefs"

# --- Busybox applet symlinks ---

# Applets needed by /init
for cmd in mount umount mkdir sleep reboot echo; do
    ln -sf busybox "$ROOT/bin/$cmd"
done

# Rescue shell applets (optional)
if [ "${RESCUE_SHELL:-0}" = "1" ]; then
    for cmd in sh ls cat cp mv rm ln grep vi \
               ps kill dmesg df du find head tail \
               hexdump dd poweroff; do
        ln -sf busybox "$ROOT/bin/$cmd"
    done
fi

# --- Optional static entries ---

if [ -n "${STATIC_JSON:-}" ]; then
    [ -f "$STATIC_JSON" ] || die "STATIC_JSON not found: $STATIC_JSON"
    cp "$STATIC_JSON" "$ROOT/etc/kexec-menu/static.json"
fi

# --- Optional extra contents ---

if [ -n "${EXTRA_DIR:-}" ]; then
    [ -d "$EXTRA_DIR" ] || die "EXTRA_DIR not a directory: $EXTRA_DIR"
    cp -a "$EXTRA_DIR"/. "$ROOT/"
fi

# --- Build CPIO archive ---

build_cpio() {
    (cd "$ROOT" && find . -print0 | sort -z | cpio -o -H newc --quiet --null)
}

if [ -n "${OUTPUT:-}" ]; then
    build_cpio > "$OUTPUT"
else
    build_cpio
fi
