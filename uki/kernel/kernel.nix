# Builds a minimal kernel for kexec-menu from tinyconfig + config fragments.
#
# Usage (from default.nix):
#   kernel-x86_64  = callPackage ./uki/kernel/kernel.nix { arch = "x86_64"; }
#   kernel-aarch64 = aarch64Musl.callPackage ./uki/kernel/kernel.nix { arch = "aarch64"; }
#
# Args:
#   arch         — "x86_64" or "aarch64"
#   initramfs    — path to CPIO archive to embed (or null for no built-in initramfs)
#   cmdline      — kernel command line to embed (or "" for none)
#   extraConfig  — additional structuredExtraConfig attrs
#   extraModules — additional kernel modules to enable
#   logo         — path to 80x80 PPM file to replace the default boot logo (or null)
{
  lib,
  linuxPackages_latest,
  arch ? "x86_64",
  initramfs ? null,
  cmdline ? "",
  extraConfig ? {},
  extraModules ? [],
  logo ? null,
}:

let
  inherit (lib.kernel) yes no module freeform;
  force = x: lib.mkForce x;

  # Parse a kernel config fragment file into structuredExtraConfig attrs.
  # Each "CONFIG_FOO=y" line becomes { FOO = force yes; }, etc.
  # Values are wrapped in mkForce to override nixpkgs common-config.nix defaults.
  parseConfigFile = path:
    let
      contents = builtins.readFile path;
      lines = lib.splitString "\n" contents;
      parseLine = line:
        let
          m = builtins.match "CONFIG_([A-Za-z0-9_]+)=(.*)" line;
        in
        if m == null then null
        else {
          name = builtins.elemAt m 0;
          value =
            let v = builtins.elemAt m 1; in
            if v == "y" then force yes
            else if v == "m" then force module
            else if v == "n" then force no
            else force (freeform v);
        };
      parsed = builtins.filter (x: x != null) (map parseLine lines);
    in
    builtins.listToAttrs parsed;

  commonConfig = parseConfigFile ./common.config;
  archConfig = parseConfigFile (./. + "/${arch}.config");

  # Module names to build (extraModules are added as CONFIG_<name>=m)
  moduleConfig = builtins.listToAttrs (map (m: {
    name = m;
    value = force module;
  }) extraModules);

  # Embedded initramfs (for UKI builds)
  initramfsConfig = lib.optionalAttrs (initramfs != null) {
    BLK_DEV_INITRD = force yes;
    INITRAMFS_SOURCE = force (freeform ''"${initramfs}"'');
  };

  # Embedded command line (for UKI builds)
  cmdlineConfig = lib.optionalAttrs (cmdline != "") {
    CMDLINE_BOOL = force yes;
    CMDLINE = force (freeform ''"${cmdline}"'');
  };

  # Merge order: common → arch → extraModules → initramfs → cmdline → extraConfig (last wins)
  mergedConfig = commonConfig // archConfig // moduleConfig
    // initramfsConfig // cmdlineConfig // extraConfig;

  kernel = (linuxPackages_latest.kernel.override {
    # Start from tinyconfig instead of defconfig
    autoModules = false;
    # Disable module support — everything built-in
    preferBuiltin = true;
    structuredExtraConfig = mergedConfig;
    # Suppress interactive config prompts for new options
    ignoreConfigErrors = true;
  }).overrideAttrs (old: lib.optionalAttrs (logo != null) {
    postPatch = (old.postPatch or "") + ''
      cp ${logo} drivers/video/logo/logo_linux_clut224.ppm
    '';
  });

in kernel
