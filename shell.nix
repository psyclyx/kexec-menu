# Development shell.
#
# Usage: nix-shell
let
  sources = import ./npins;
  pkgs = import sources.nixpkgs {};
  musl64 = pkgs.pkgsCross.musl64;
in
pkgs.mkShell {
  nativeBuildInputs = [
    # Rust toolchain (targets musl)
    musl64.buildPackages.rustc
    musl64.buildPackages.cargo
    musl64.buildPackages.clippy
    pkgs.rust-analyzer

    # Testing
    pkgs.qemu

    # Disk image tools (for QEMU tests)
    pkgs.e2fsprogs
    pkgs.btrfs-progs
    pkgs.xfsprogs
    pkgs.f2fs-tools
    pkgs.fuse

    # Initrd assembly
    pkgs.cpio
    pkgs.pkgsStatic.busybox
    pkgs.kmod
  ];

  buildInputs = [
    musl64.stdenv.cc
  ];

  CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER = "x86_64-unknown-linux-musl-cc";
  RUSTFLAGS = "-C target-feature=+crt-static";

  # Kernel for QEMU tests — built from pinned nixpkgs, not the host
  QEMU_KERNEL = "${pkgs.linuxPackages.kernel}/bzImage";
  QEMU_KERNEL_MODULES = "${pkgs.linuxPackages.kernel.modules}/lib/modules/${pkgs.linuxPackages.kernel.modDirVersion}";

  # Static busybox for initrd (the one in PATH may be dynamically linked)
  BUSYBOX_STATIC = "${pkgs.pkgsStatic.busybox}/bin/busybox";

  # Static cryptsetup for LUKS testing in QEMU initrd
  CRYPTSETUP_STATIC = "${pkgs.pkgsStatic.cryptsetup}/bin/cryptsetup";

  # Static mkfs.btrfs for btrfs testing in QEMU initrd
  MKFS_BTRFS_STATIC = "${pkgs.pkgsStatic.btrfs-progs}/bin/mkfs.btrfs";

  shellHook = ''
    echo "kexec-menu dev shell"
    echo "  cargo target: x86_64-unknown-linux-musl (static)"
    echo "  run tests:    ./tests/qemu/run.sh"
  '';
}
