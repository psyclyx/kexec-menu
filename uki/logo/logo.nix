# Generates an 80x80 PPM boot logo by calling scripts/mklogo.sh.
#
# This is a thin Nix wrapper — all build logic lives in mklogo.sh.
#
# Args:
#   colors — attrset with base16 color keys as "R G B" strings (0-255):
#            base00 (background), base05 (foreground), base0D (accent)
#            Only these three are used. Defaults to gruvbox-dark.
{
  runCommand,
  colors ? {},
}:

let
  defaults = {
    base00 = "29 32 33";    # background (gruvbox bg0_h)
    base05 = "213 196 161"; # foreground (gruvbox fg2)
    base0D = "131 165 152"; # accent     (gruvbox aqua)
  };

  c = defaults // colors;
in

runCommand "kexec-menu-logo.ppm" {} ''
  export LOGO_BG="${c.base00}"
  export LOGO_FG="${c.base05}"
  export LOGO_ACCENT="${c.base0D}"

  sh ${../../scripts/mklogo.sh} > "$out"
''
