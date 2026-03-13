# kexec-menu

A filesystem-agnostic kexec boot menu distributed as a UKI.

Mounts available filesystems, discovers boot entries, presents a menu, kexecs
the selection. Runs from a flash drive, netboot, ESP, or anywhere a UEFI
environment can load an EFI binary.

Supported targets: x86_64, aarch64 (UEFI via towboot or similar).

## Building

With Nix:

    nix-build                          # x86_64 static binary
    nix-build -A kexec-menu-aarch64    # aarch64 cross-compile

Without Nix (requires Rust toolchain + musl targets):

    make            # x86_64
    make aarch64    # aarch64
    make all        # both

### Building the UKI

Complete UKI assembly (kernel + initrd + bootmenu binary):

    nix-build -A uki-x86_64     # x86_64 EFI binary
    nix-build -A uki-aarch64    # aarch64 EFI binary

Individual components:

    nix-build -A kernel-x86_64  # minimal kernel
    nix-build -A initrd-x86_64  # CPIO initrd
    nix-build -A logo           # boot logo PPM

The kernel is built from nixpkgs `linuxPackages_latest` with a minimal
tinyconfig + required fragments. The UKI uses `CONFIG_EFI_STUB=y` (no
systemd-stub dependency).

## Feature Flags

Compile-time Cargo features control security-sensitive functionality:

| Feature | Default | Description |
|---|---|---|
| `full-fs-view` | on | Full filesystem browsing keybind |
| `rescue-shell` | on | Rescue shell support in initrd |
| `disk-whitelist` | on | Disk whitelist filtering |

Build a locked-down binary with `--no-default-features`:

    cargo build --release --no-default-features

## Testing

    cargo test --workspace      # unit tests (138 tests)
    make test                   # unit tests via Makefile

QEMU integration tests (boots a VM, mounts ext4/btrfs/LUKS, runs the menu):

    $(nix-build -A tests.qemu)  # build deps and run

NixOS module VM tests (installer layout, specialisations, pruning, dedup, UKI install):

    nix-build -A tests.installer

## Usage

    kexec-menu                 # normal boot menu
    kexec-menu --dry-run       # standalone mode: print selection instead of kexec
    kexec-menu --auto-default  # skip TUI, boot default entry directly

## NixOS Module

Import `modules/nixos.nix` and enable it:

```nix
{ imports = [ kexec-menu.modules.nixos ]; }

{
  boot.loader.kexec-menu = {
    enable = true;
    package = kexec-menu-uki;  # your built UKI package
  };
}
```

### Options

| Option | Type | Default | Description |
|---|---|---|---|
| `enable` | bool | `false` | Enable the kexec-menu bootloader |
| `package` | package | — | The kexec-menu UKI package |
| `bootMountPoint` | string | `"/boot"` | Where boot entries are written |
| `retention` | positive int | `10` | Number of generations to keep |
| `installStrategy` | `"copy"` or `"reflink"` | `"copy"` | How blobs are placed. `copy` skips identical files (any fs). `reflink` uses `cp --reflink=auto` (saves space on bcachefs/btrfs). |
| `ukiInstallPath` | null or string | `null` | If set, copy the UKI to this path on each rebuild |
| `theme` | null or attrs | `null` | Base16 hex colorscheme. Auto-detected from Stylix if present. |

The module hooks into `boot.loader.external` — each `nixos-rebuild` runs the
installer, which copies kernel/initrd/entries.json into a generation leaf
directory under `<bootMountPoint>/nixos/` and prunes old generations beyond
the retention count. Specialisations are included as additional entries.

## Spec

See [docs/spec.md](docs/spec.md).
