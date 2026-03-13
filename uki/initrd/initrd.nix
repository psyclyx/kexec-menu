# Builds a minimal CPIO initrd for the kexec-menu UKI by calling
# scripts/mkinitrd.sh.
#
# This is a thin Nix wrapper — all build logic lives in mkinitrd.sh.
#
# Usage (from default.nix):
#   initrd-x86_64 = callPackage ./uki/initrd/initrd.nix {
#     kexec-menu = self.kexec-menu;
#     busybox = self.busybox-x86_64;
#     cryptsetup = self.cryptsetup-x86_64;
#     bcachefs-tools = self.bcachefs-tools-x86_64;
#   };
#
# Args:
#   kexec-menu       — the static kexec-menu binary package (target arch)
#   busybox          — static busybox package (target arch)
#   cryptsetup       — static cryptsetup package (target arch)
#   bcachefs-tools   — static bcachefs-tools package (target arch)
#   rescueShell      — include rescue shell applets (default: false)
#   staticEntries    — path to static.json, or null
#   extraContents    — attrset of { "/path" = source; } for additional files
{
  lib,
  runCommand,
  cpio,
  kexec-menu,
  busybox,
  cryptsetup,
  bcachefs-tools,
  rescueShell ? false,
  staticEntries ? null,
  extraContents ? {},
}:

runCommand "kexec-menu-initrd" {
  nativeBuildInputs = [ cpio ];
} ''
  # Prepare EXTRA_DIR if extraContents is non-empty
  ${lib.optionalString (extraContents != {}) ''
    extra="$TMPDIR/extra"
    mkdir -p "$extra"
    ${lib.concatStringsSep "\n" (lib.mapAttrsToList (dest: src: ''
      mkdir -p "$extra/$(dirname "${dest}")"
      cp -a ${src} "$extra/${dest}"
    '') extraContents)}
  ''}

  export INIT_SCRIPT=${./init}
  export KEXEC_MENU=${kexec-menu}/bin/kexec-menu
  export BUSYBOX=${busybox}/bin/busybox
  export CRYPTSETUP=${cryptsetup}/bin/cryptsetup
  export BCACHEFS=${bcachefs-tools}/bin/bcachefs
  export OUTPUT="$out"
  ${lib.optionalString rescueShell "export RESCUE_SHELL=1"}
  ${lib.optionalString (staticEntries != null) "export STATIC_JSON=${staticEntries}"}
  ${lib.optionalString (extraContents != {}) ''export EXTRA_DIR="$extra"''}

  bash ${../../scripts/mkinitrd.sh}
''
