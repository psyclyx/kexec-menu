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

## Testing

    cargo test                 # unit tests (135 tests)
    make test                  # unit tests via Makefile

QEMU integration test (requires nix-shell):

    cd tests/qemu && nix-shell --run ./integration-test.sh

## Usage

    kexec-menu                 # normal boot menu
    kexec-menu --dry-run       # standalone mode: print selection instead of kexec
    kexec-menu --auto-default  # skip TUI, boot default entry directly

## Spec

See [docs/spec.md](docs/spec.md).
