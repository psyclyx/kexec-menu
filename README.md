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

With Nix:

    nix-build -A uki-x86_64     # x86_64 EFI binary
    nix-build -A uki-aarch64    # aarch64 EFI binary

Individual Nix components:

    nix-build -A kernel-x86_64  # minimal kernel
    nix-build -A initrd-x86_64  # CPIO initrd
    nix-build -A logo           # boot logo PPM

Without Nix (requires kernel source, static tool binaries, and a cross
toolchain for aarch64):

    make uki ARCH=x86_64 \
      KERNEL_SRC=~/linux-6.12 \
      BUSYBOX=/path/to/busybox-static \
      CRYPTSETUP=/path/to/cryptsetup-static \
      BCACHEFS=/path/to/bcachefs-static

This orchestrates the full pipeline: binary → initrd → kernel → UKI.
The output lands in `build/kexec-menu.efi`.

Individual steps can be run separately:

    make logo                              # boot logo (build/logo.ppm)
    make initrd ARCH=x86_64 BUSYBOX=... CRYPTSETUP=... BCACHEFS=...
    make kernel ARCH=x86_64 KERNEL_SRC=~/linux-6.12

#### Prerequisites

- Rust toolchain with musl target (`rustup target add x86_64-unknown-linux-musl`)
- Kernel source tree (extracted tarball, e.g. linux-6.12)
- Static binaries for the target arch: busybox, cryptsetup, bcachefs-tools
- Kernel build deps: make, gcc, flex, bison, bc, perl
- cpio (for initrd assembly)
- awk (for logo generation)
- For aarch64 cross-builds: `aarch64-linux-gnu-gcc`

#### Optional variables

| Variable | Description |
|---|---|
| `CMDLINE` | Embedded kernel command line (default: `console=tty0`) |
| `LOGO_BG`, `LOGO_FG`, `LOGO_ACCENT` | Logo colors as `"R G B"` strings (0-255) |
| `EXTRA_CONFIG` | Path to additional kernel config fragment |
| `STATIC_JSON` | Path to static boot entries file to embed in initrd |
| `EXTRA_DIR` | Directory whose contents are copied into the initrd root |
| `RESCUE_SHELL` | Set to `1` to include rescue shell applets in initrd |
| `CARGO_FEATURES` | Cargo feature flags for the binary build |

The kernel is built from tinyconfig + the project's config fragments
(`uki/kernel/`). The UKI uses `CONFIG_EFI_STUB=y` (no systemd-stub
dependency).

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

    cargo test --workspace      # unit tests (174 tests)
    make test                   # unit tests via Makefile

QEMU integration tests (boots a VM, mounts ext4/btrfs/XFS/F2FS/LUKS/multi-device
btrfs RAID1, runs the menu, validates disk-whitelist filtering):

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
  boot.loader.kexec-menu.enable = true;
}
```

The `package` option defaults to the UKI built from the kexec-menu source tree.
Override it only if you need a custom build.

### Stylix Integration

For automatic theme integration with [Stylix](https://github.com/danth/stylix),
import the stylix module:

```nix
{ imports = [
    kexec-menu.modules.nixos
    kexec-menu.modules.stylix
  ];
}
```

This adds `stylix.targets.kexec-menu.enable` (default: `true` when Stylix is
active). The boot menu TUI will use your Stylix Base16 palette automatically.

### Options

| Option | Type | Default | Description |
|---|---|---|---|
| `enable` | bool | `false` | Enable the kexec-menu bootloader |
| `package` | package | `uki-${arch}` | The kexec-menu UKI package |
| `finalPackage` | package | (read-only) | UKI with theme/timeout overrides applied |
| `bootMountPoint` | string | `"/boot"` | Where boot entries are written |
| `retention` | positive int | `10` | Number of generations to keep |
| `installStrategy` | `"copy"` or `"reflink"` | `"copy"` | How blobs are placed. `copy` skips identical files (any fs). `reflink` uses `cp --reflink=auto` (saves space on bcachefs/btrfs). |
| `ukiInstallPath` | null or string | `null` | If set, copy the UKI to this path on each rebuild |
| `timeout` | null or uint | `null` | Autoboot timeout in seconds. `null` = compiled-in default (5s). |
| `theme` | null or attrs | `null` | Base16 hex colorscheme. Import `modules/stylix.nix` for auto-detection. |

The module hooks into `boot.loader.external` — each `nixos-rebuild` runs the
installer, which copies kernel/initrd/entries.json into a generation leaf
directory under `<bootMountPoint>/nixos/` and prunes old generations beyond
the retention count. Specialisations are included as additional entries.

## Spec

See [docs/spec.md](docs/spec.md).
