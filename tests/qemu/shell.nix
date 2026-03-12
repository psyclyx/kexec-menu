# Nix shell for QEMU test environment.
# Provides: musl-targeted Rust, QEMU, disk image tools, busybox, kernel.
#
# Usage: nix-shell tests/qemu/shell.nix --run ./tests/qemu/run.sh
let
  pkgs = import <nixpkgs> {};
  musl = pkgs.pkgsCross.musl64;
  kernel = pkgs.linuxPackages.kernel;
in
pkgs.mkShell {
  nativeBuildInputs = [
    # Rust + musl cross-compiler
    musl.buildPackages.rustc
    musl.buildPackages.cargo

    # QEMU
    pkgs.qemu

    # Disk image tools
    pkgs.e2fsprogs  # mke2fs, debugfs, fuse2fs
    pkgs.fuse       # fusermount (for fuse2fs)

    # Initrd
    pkgs.cpio
    pkgs.pkgsStatic.busybox

    # Module tools (for depmod/modprobe in initrd prep)
    pkgs.kmod
  ];

  buildInputs = [
    musl.stdenv.cc  # x86_64-unknown-linux-musl-cc cross-linker
  ];

  CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER = "x86_64-unknown-linux-musl-cc";
  RUSTFLAGS = "-C target-feature=+crt-static";

  # Expose kernel paths for run.sh
  QEMU_KERNEL = "${kernel}/bzImage";
  QEMU_KERNEL_MODULES = "${kernel.modules}/lib/modules/${kernel.modDirVersion}";

  shellHook = ''
    echo "QEMU test shell ready"
    echo "  cargo target: x86_64-unknown-linux-musl (static)"
    echo "  kernel: ${kernel}/bzImage"
    echo "  modules: ${kernel.modules}/lib/modules/${kernel.modDirVersion}"
    echo "  run: ./tests/qemu/run.sh"
  '';
}
