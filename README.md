# kexec-menu

A filesystem-agnostic kexec boot menu distributed as a UKI.

Mounts available filesystems, discovers boot entries, presents a menu, kexecs
the selection. Runs from a flash drive, netboot, ESP, or anywhere a UEFI
environment can load an EFI binary.

Supported targets: x86_64, aarch64 (UEFI via towboot or similar).

## Building

With Nix:

    nix-build

Without Nix:

    make        # requires Rust toolchain + musl targets

## Testing

    make test                  # unit tests
    make qemu-test             # QEMU integration tests (requires QEMU)
    nix-build -A checks        # both, via Nix

## Spec

See [docs/spec.md](docs/spec.md).
