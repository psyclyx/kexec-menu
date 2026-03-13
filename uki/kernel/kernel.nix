# Builds a minimal kernel for kexec-menu by calling scripts/mkkernel.sh.
#
# This is a thin Nix wrapper — all build logic lives in mkkernel.sh.
# The kernel source is pinned in source.nix.
#
# Usage (from default.nix):
#   kernel-x86_64  = callPackage ./uki/kernel/kernel.nix { arch = "x86_64"; }
#   kernel-aarch64 = callPackage ./uki/kernel/kernel.nix { arch = "aarch64"; }
#
# Args:
#   arch         — "x86_64" or "aarch64"
#   kernelSrc    — pre-fetched kernel source tarball
#   initramfs    — path to CPIO archive to embed (or null)
#   cmdline      — kernel command line to embed (or "")
#   extraConfig  — path to additional config fragment file (or null)
#   logo         — path to 80x80 PPM boot logo (or null)
{
  lib,
  runCommand,
  kernelSrc,
  arch ? "x86_64",
  initramfs ? null,
  cmdline ? "",
  extraConfig ? null,
  logo ? null,

  # Build dependencies
  flex,
  bison,
  bc,
  perl,
  elfutils,
  openssl,
  gnumake,
  stdenv,
  gzip,
  cpio,
  zstd,
  python3,
  pkg-config,
}:

let
  imageName = if arch == "x86_64" then "bzImage" else "Image";
  configDir = ./.;
in

runCommand "kexec-menu-kernel-${arch}" {
  nativeBuildInputs = [
    flex bison bc perl elfutils openssl gnumake stdenv.cc
    gzip cpio zstd python3 pkg-config
  ];
} ''
  src="$TMPDIR/linux-src"
  mkdir -p "$src"
  tar -xf ${kernelSrc} -C "$src" --strip-components=1

  mkdir -p "$out"

  export KERNEL_SRC="$src"
  export ARCH=${arch}
  export CONFIG_DIR=${configDir}
  export OUTPUT="$out/${imageName}"
  ${lib.optionalString (initramfs != null) "export INITRAMFS=${initramfs}"}
  ${lib.optionalString (cmdline != "") ''export CMDLINE="${cmdline}"''}
  ${lib.optionalString (extraConfig != null) "export EXTRA_CONFIG=${extraConfig}"}
  ${lib.optionalString (logo != null) "export LOGO=${logo}"}

  bash ${../../scripts/mkkernel.sh}
''
