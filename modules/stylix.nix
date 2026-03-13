# Stylix integration for kexec-menu.
#
# Imported automatically by nixos.nix. No-op when Stylix is not present.
# When Stylix is active, defines stylix.targets.kexec-menu.enable (default: true)
# and sets the boot menu theme from the Stylix palette.
{ config, lib, options, ... }:

let
  hasStylix = options ? stylix;
in
{
  options = lib.optionalAttrs hasStylix {
    stylix.targets.kexec-menu.enable =
      config.lib.stylix.mkEnableTarget "the kexec-menu boot menu" true;
  };

  config = lib.mkIf
    (hasStylix && config.stylix.enable && config.stylix.targets.kexec-menu.enable) {
    boot.loader.kexec-menu.theme =
      let c = config.lib.stylix.colors; in
      builtins.mapAttrs (_: toString) {
        inherit (c) base00 base01 base02 base03 base04 base05
                     base06 base07 base08 base09 base0A base0B
                     base0C base0D base0E base0F;
      };
  };
}
