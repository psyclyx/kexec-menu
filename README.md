# kexec-menu

Filesystem-agnostic kexec boot menu, distributed as a UKI. Mounts block
devices, discovers boot entries, presents a TUI, kexecs the selection.

Targets: x86_64, aarch64 (UEFI).

## Install

### NixOS

```nix
{ imports = [ kexec-menu.modules.nixos ]; }

{ boot.loader.kexec-menu.enable = true; }
```

### Arch Linux

A `PKGBUILD` is included. It builds the static binary only — the UKI
requires additional components (kernel, initrd, static tools). See
[Building the UKI](#uki) for the full pipeline.

    makepkg -si

### Manual

Copy `kexec-menu.efi` to the ESP and add a UEFI boot entry:

    efibootmgr --create --disk /dev/sda --part 1 \
      --label "kexec-menu" --loader '\EFI\kexec-menu\kexec-menu.efi'

## Usage

    kexec-menu                 # boot menu
    kexec-menu --dry-run       # print selection, no kexec
    kexec-menu --auto-default  # boot default entry without TUI

## Entry Format

Boot entries live under `boot/` on any mounted filesystem. A leaf directory
contains `entries.json` and the referenced kernel/initrd blobs.

```json
[
  { "name": "default", "kernel": "vmlinuz", "initrd": "initrd-default", "cmdline": "..." },
  { "name": "gaming",  "kernel": "vmlinuz", "initrd": "initrd-gaming",  "cmdline": "..." }
]
```

All fields required:

| Field | Description |
|---|---|
| `name` | Stable identifier. Used for default selection persistence. |
| `kernel` | Filename in the leaf directory. |
| `initrd` | Filename in the leaf directory. |
| `cmdline` | Full kernel command line. |

## Building

With Nix:

    nix-build                          # x86_64 static binary
    nix-build -A kexec-menu-aarch64    # aarch64 cross-compile

Without Nix (Rust toolchain + musl targets required):

    make            # x86_64
    make aarch64    # aarch64
    make all        # both

### UKI

With Nix:

    nix-build -A uki-x86_64
    nix-build -A uki-aarch64
    nix-build -A kernel-x86_64    # individual components
    nix-build -A initrd-x86_64
    nix-build -A logo

Without Nix (kernel source, static tool binaries, cross toolchain for
aarch64):

    make uki ARCH=x86_64 \
      KERNEL_SRC=~/linux-6.12 \
      BUSYBOX=/path/to/busybox-static \
      CRYPTSETUP=/path/to/cryptsetup-static \
      BCACHEFS=/path/to/bcachefs-static

Full pipeline: binary → initrd → kernel → UKI. Output: `build/kexec-menu.efi`.

Individual steps:

    make logo                              # build/logo.ppm
    make initrd ARCH=x86_64 BUSYBOX=... CRYPTSETUP=... BCACHEFS=...
    make kernel ARCH=x86_64 KERNEL_SRC=~/linux-6.12

#### Prerequisites

- Rust toolchain with musl target (`rustup target add x86_64-unknown-linux-musl`)
- Kernel source tree (e.g. linux-6.12)
- Static binaries: busybox, cryptsetup, bcachefs-tools
- Kernel build deps: make, gcc, flex, bison, bc, perl
- cpio, awk
- aarch64 cross-builds: `aarch64-linux-gnu-gcc`

#### Build variables

| Variable | Description |
|---|---|
| `CMDLINE` | Kernel command line (default: `console=tty0`) |
| `LOGO_BG`, `LOGO_FG`, `LOGO_ACCENT` | Logo colors as `"R G B"` (0-255) |
| `EXTRA_CONFIG` | Additional kernel config fragment |
| `STATIC_JSON` | Static boot entries file embedded in initrd |
| `EXTRA_DIR` | Directory contents copied into initrd root |
| `RESCUE_SHELL` | `1` to include rescue shell applets in initrd |
| `CARGO_FEATURES` | Cargo feature flags |
| `KEXEC_MENU_DISK_WHITELIST` | Compile-time device filter. Comma-separated names or prefix globs (e.g. `nvme*,sda`). Unset = no filtering. Requires `disk-whitelist` feature. |

Kernel: tinyconfig + project fragments (`uki/kernel/`), `CONFIG_EFI_STUB=y`.

## Feature Flags

Compile-time Cargo features:

| Feature | Default | Description |
|---|---|---|
| `full-fs-view` | on | Full filesystem browsing keybind |
| `rescue-shell` | on | Rescue shell |
| `disk-whitelist` | on | Disk filtering support. No effect unless `KEXEC_MENU_DISK_WHITELIST` is set at build time. |

Disable all with `--no-default-features`:

    cargo build --release --no-default-features

## Testing

    cargo test --workspace      # unit tests
    make test

QEMU integration (ext4/btrfs/XFS/F2FS/LUKS/btrfs RAID1, disk filtering):

    $(nix-build -A tests.qemu)

NixOS module VM tests (installer, specialisations, pruning, dedup, UKI install):

    nix-build -A tests.installer

## NixOS Module

```nix
{ imports = [ kexec-menu.modules.nixos ]; }

{ boot.loader.kexec-menu.enable = true; }
```

### Stylix

```nix
{ imports = [
    kexec-menu.modules.nixos
    kexec-menu.modules.stylix
  ];
}
```

Provides `stylix.targets.kexec-menu.enable` (default: `true` when Stylix is
active). Applies the Base16 palette to the TUI.

### Options

| Option | Type | Default | Description |
|---|---|---|---|
| `enable` | bool | `false` | Enable kexec-menu bootloader |
| `package` | package | `uki-${arch}` | UKI package |
| `finalPackage` | package | (read-only) | UKI with theme/timeout overrides |
| `bootMountPoint` | string | `"/boot"` | Boot entry target directory |
| `retention` | positive int | `10` | Generations to keep |
| `installStrategy` | `"copy"` or `"reflink"` | `"copy"` | `copy`: skip identical files. `reflink`: `cp --reflink=auto` (bcachefs/btrfs). |
| `ukiInstallPath` | null or string | `null` | Copy UKI here on rebuild |
| `timeout` | null or uint | `null` | Autoboot seconds. `null` = default (5s). |
| `theme` | null or attrs | `null` | Base16 colorscheme. Use `modules/stylix.nix` for auto-detection. |

Hooks into `boot.loader.external`. Each `nixos-rebuild` writes
kernel/initrd/entries.json to `<bootMountPoint>/nixos/` and prunes past
retention. Specialisations are additional entries in the same leaf.
