# Top-level entry point for nix-build / nix-env.
#
# Attributes:
#   kexec-menu              — static x86_64 binary
#   kexec-menu-aarch64      — static aarch64 binary (cross-compiled)
#   busybox-x86_64          — static busybox for x86_64 initrd
#   busybox-aarch64         — static busybox for aarch64 initrd
#   cryptsetup-x86_64       — static cryptsetup for x86_64 initrd
#   cryptsetup-aarch64      — static cryptsetup for aarch64 initrd
#   bcachefs-tools-x86_64   — static bcachefs-tools for x86_64 initrd
#   bcachefs-tools-aarch64  — static bcachefs-tools for aarch64 initrd
#   kernel-x86_64           — minimal kernel for x86_64 UKI
#   kernel-aarch64          — minimal kernel for aarch64 UKI
#   initrd-x86_64           — CPIO initrd for x86_64 UKI
#   initrd-aarch64          — CPIO initrd for aarch64 UKI
#   logo                    — boot logo PPM (80x80, base16-colorizable)
#   uki-x86_64              — complete UKI EFI binary for x86_64
#   uki-aarch64             — complete UKI EFI binary for aarch64
#   tests.installer         — NixOS VM test for the installer/module
#
# Usage:
#   nix-build                          # builds kexec-menu (x86_64)
#   nix-build -A kexec-menu-aarch64
#   nix-build -A busybox-x86_64
#   nix-build -A cryptsetup-x86_64
#   nix-build -A bcachefs-tools-x86_64
#   nix-build -A kernel-x86_64
#   nix-build -A initrd-x86_64
#   nix-build -A logo
#   nix-build -A uki-x86_64
#   nix-build -A uki-aarch64
#   nix-build -A tests.installer
#   $(nix-build -A tests.qemu)   # QEMU integration test (requires KVM)
let
  sources = import ./npins;
  pkgs = import sources.nixpkgs {};
  musl64 = pkgs.pkgsCross.musl64;
  aarch64Musl = pkgs.pkgsCross.aarch64-multiplatform-musl;

  logo = pkgs.callPackage ./uki/logo/logo.nix {};

  # Pinned kernel source — shared between x86_64 and aarch64 builds
  kernelSource = import ./uki/kernel/source.nix;
  kernelSrc = pkgs.fetchurl {
    url = "https://cdn.kernel.org/pub/linux/kernel/v${builtins.head (builtins.split "\\." kernelSource.version)}.x/linux-${kernelSource.version}.tar.xz";
    hash = kernelSource.hash;
  };

  self = {
    # ── Binary ──────────────────────────────────────────────────────────

    kexec-menu = musl64.callPackage ./package.nix {
      target = "x86_64-unknown-linux-musl";
    };

    kexec-menu-aarch64 = aarch64Musl.callPackage ./package.nix {
      target = "aarch64-unknown-linux-musl";
    };

    # ── Initrd components ───────────────────────────────────────────────

    busybox-x86_64 = musl64.pkgsStatic.busybox;
    busybox-aarch64 = aarch64Musl.pkgsStatic.busybox;

    cryptsetup-x86_64 = musl64.pkgsStatic.cryptsetup;
    cryptsetup-aarch64 = aarch64Musl.pkgsStatic.cryptsetup;

    bcachefs-tools-x86_64 = musl64.pkgsStatic.bcachefs-tools;
    bcachefs-tools-aarch64 = aarch64Musl.pkgsStatic.bcachefs-tools;

    # ── Logo ────────────────────────────────────────────────────────────

    inherit logo;

    # ── Kernel ──────────────────────────────────────────────────────────

    kernel-x86_64 = pkgs.callPackage ./uki/kernel/kernel.nix {
      inherit kernelSrc;
      arch = "x86_64";
    };

    kernel-aarch64 = pkgs.pkgsCross.aarch64-multiplatform.callPackage ./uki/kernel/kernel.nix {
      inherit kernelSrc;
      arch = "aarch64";
    };

    # ── Initrd ──────────────────────────────────────────────────────────

    initrd-x86_64 = pkgs.callPackage ./uki/initrd/initrd.nix {
      kexec-menu = self.kexec-menu;
      busybox = self.busybox-x86_64;
      cryptsetup = self.cryptsetup-x86_64;
      bcachefs-tools = self.bcachefs-tools-x86_64;
    };

    initrd-aarch64 = pkgs.callPackage ./uki/initrd/initrd.nix {
      kexec-menu = self.kexec-menu-aarch64;
      busybox = self.busybox-aarch64;
      cryptsetup = self.cryptsetup-aarch64;
      bcachefs-tools = self.bcachefs-tools-aarch64;
    };

    # ── UKI ─────────────────────────────────────────────────────────────

    uki-x86_64 = pkgs.callPackage ./uki/uki.nix {
      arch = "x86_64";
      inherit kernelSrc;
      initrd = self.initrd-x86_64;
      logo = self.logo;
    };

    uki-aarch64 = pkgs.pkgsCross.aarch64-multiplatform.callPackage ./uki/uki.nix {
      arch = "aarch64";
      inherit kernelSrc;
      initrd = self.initrd-aarch64;
      logo = self.logo;
    };

    # ── Tests ───────────────────────────────────────────────────────────

    tests = {
      installer = import ./tests/nixos/installer.nix;
      qemu = import ./tests/qemu/test.nix;
    };
  };
in self
