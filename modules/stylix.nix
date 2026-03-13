# Stylix target integration for kexec-menu.
#
# Import this module alongside nixos.nix when Stylix is in use.
# It defines stylix.targets.kexec-menu.enable and, when active,
# sets boot.loader.kexec-menu.theme from the Stylix palette.
{ config, lib, ... }:

{
  options.stylix.targets.kexec-menu.enable =
    config.lib.stylix.mkEnableTarget "the kexec-menu boot menu" true;

  config = lib.mkIf
    (config.stylix.enable && config.stylix.targets.kexec-menu.enable) {
    boot.loader.kexec-menu.theme =
      let c = config.lib.stylix.colors; in
      builtins.mapAttrs (_: toString) {
        inherit (c) base00 base01 base02 base03 base04 base05
                     base06 base07 base08 base09 base0A base0B
                     base0C base0D base0E base0F;
      };
  };
}
