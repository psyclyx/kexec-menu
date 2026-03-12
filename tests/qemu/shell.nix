# Nix shell for QEMU test environment.
# Provides: musl-targeted Rust, QEMU, disk image tools, busybox.
#
# Usage: nix-shell tests/qemu/shell.nix --run ./tests/qemu/run.sh
let
  pkgs = import <nixpkgs> {};
  musl = pkgs.pkgsCross.musl64;
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
    pkgs.busybox
  ];

  buildInputs = [
    musl.stdenv.cc  # x86_64-unknown-linux-musl-cc cross-linker
  ];

  CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER = "x86_64-unknown-linux-musl-cc";
  RUSTFLAGS = "-C target-feature=+crt-static";

  shellHook = ''
    echo "QEMU test shell ready"
    echo "  cargo target: x86_64-unknown-linux-musl (static)"
    echo "  run: ./tests/qemu/run.sh"
  '';
}
