# Builds a complete UKI (Unified Kernel Image) — a self-contained EFI binary.
#
# The kernel with CONFIG_EFI_STUB=y acts as the EFI application directly.
# The initrd is embedded via CONFIG_INITRAMFS_SOURCE, and the command line
# via CONFIG_CMDLINE. No systemd-stub dependency.
#
# Output: $out/kexec-menu.efi
#
# Usage (from default.nix):
#   uki-x86_64 = pkgs.callPackage ./uki/uki.nix {
#     arch = "x86_64";
#     initrd = self.initrd-x86_64;
#   };
#   uki-aarch64 = pkgs.pkgsCross.aarch64-multiplatform.callPackage ./uki/uki.nix {
#     arch = "aarch64";
#     initrd = self.initrd-aarch64;
#   };
#
# Args:
#   arch         — "x86_64" or "aarch64"
#   initrd       — pre-built CPIO initrd archive (from initrd.nix)
#   cmdline      — kernel command line to embed
#   extraConfig  — additional kernel structuredExtraConfig attrs
#   extraModules — additional kernel modules to enable
{
  lib,
  runCommand,
  callPackage,
  arch ? "x86_64",
  initrd,
  cmdline ? "console=tty0",
  extraConfig ? {},
  extraModules ? [],
}:

let
  kernel = callPackage ./kernel/kernel.nix {
    inherit arch extraConfig extraModules;
    initramfs = initrd;
    inherit cmdline;
  };

  # x86_64 produces bzImage, aarch64 produces Image
  kernelImage =
    if arch == "x86_64"
    then "${kernel}/bzImage"
    else "${kernel}/Image";

in runCommand "kexec-menu-uki-${arch}" {} ''
  mkdir -p "$out"
  cp ${kernelImage} "$out/kexec-menu.efi"
''
