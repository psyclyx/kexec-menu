# Top-level entry point for nix-build / nix-env.
#
# Attributes:
#   kexec-menu          — static x86_64 binary
#   kexec-menu-aarch64  — static aarch64 binary (cross-compiled)
#   kernel-x86_64       — minimal kernel for x86_64 UKI
#   kernel-aarch64      — minimal kernel for aarch64 UKI
#   initrd-x86_64       — CPIO initrd for x86_64 UKI
#   initrd-aarch64      — CPIO initrd for aarch64 UKI
#   uki-x86_64          — complete UKI EFI binary for x86_64
#   uki-aarch64          — complete UKI EFI binary for aarch64
#   tests.installer     — NixOS VM test for the installer/module
#
# Usage:
#   nix-build             # builds kexec-menu (x86_64)
#   nix-build -A kexec-menu-aarch64
#   nix-build -A kernel-x86_64
#   nix-build -A kernel-aarch64
#   nix-build -A initrd-x86_64
#   nix-build -A initrd-aarch64
#   nix-build -A uki-x86_64
#   nix-build -A uki-aarch64
#   nix-build -A tests.installer
#   $(nix-build -A tests.qemu)   # QEMU integration test (requires KVM)
let
  sources = import ./npins;
  pkgs = import sources.nixpkgs {};
  musl64 = pkgs.pkgsCross.musl64;
  aarch64Musl = pkgs.pkgsCross.aarch64-multiplatform-musl;

  self = {
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

    initrd-x86_64 = pkgs.callPackage ./uki/initrd/initrd.nix {
      kexec-menu = self.kexec-menu;
      targetPkgsStatic = musl64.pkgsStatic;
    };

    initrd-aarch64 = pkgs.callPackage ./uki/initrd/initrd.nix {
      kexec-menu = self.kexec-menu-aarch64;
      targetPkgsStatic = aarch64Musl.pkgsStatic;
    };

    uki-x86_64 = pkgs.callPackage ./uki/uki.nix {
      arch = "x86_64";
      initrd = self.initrd-x86_64;
    };

    uki-aarch64 = pkgs.pkgsCross.aarch64-multiplatform.callPackage ./uki/uki.nix {
      arch = "aarch64";
      initrd = self.initrd-aarch64;
    };

    tests = {
      installer = import ./tests/nixos/installer.nix;
      qemu = import ./tests/qemu/test.nix;
    };
  };
in self
