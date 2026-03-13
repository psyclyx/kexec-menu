# Top-level entry point for nix-build / nix-env.
#
# Attributes:
#   kexec-menu          — static x86_64 binary
#   kexec-menu-aarch64  — static aarch64 binary (cross-compiled)
#   kernel-x86_64       — minimal kernel for x86_64 UKI
#   kernel-aarch64      — minimal kernel for aarch64 UKI
#   tests.installer     — NixOS VM test for the installer/module
#
# Usage:
#   nix-build             # builds kexec-menu (x86_64)
#   nix-build -A kexec-menu-aarch64
#   nix-build -A kernel-x86_64
#   nix-build -A kernel-aarch64
#   nix-build -A tests.installer
#   $(nix-build -A tests.qemu)   # QEMU integration test (requires KVM)
let
  sources = import ./npins;
  pkgs = import sources.nixpkgs {};
  musl64 = pkgs.pkgsCross.musl64;
  aarch64Musl = pkgs.pkgsCross.aarch64-multiplatform-musl;
in
{
  kexec-menu = musl64.callPackage ./package.nix {
    target = "x86_64-unknown-linux-musl";
  };

  kexec-menu-aarch64 = aarch64Musl.callPackage ./package.nix {
    target = "aarch64-unknown-linux-musl";
  };

  kernel-x86_64 = pkgs.callPackage ./uki/kernel/kernel.nix {
    arch = "x86_64";
  };

  kernel-aarch64 = pkgs.pkgsCross.aarch64-multiplatform.callPackage ./uki/kernel/kernel.nix {
    arch = "aarch64";
  };

  tests = {
    installer = import ./tests/nixos/installer.nix;
    qemu = import ./tests/qemu/test.nix;
  };
}
