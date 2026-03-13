# Builds a minimal CPIO initrd for the kexec-menu UKI.
#
# Contents:
#   /init                         — mounts pseudo-fs, runs kexec-menu
#   /bin/kexec-menu               — the static binary
#   /bin/cryptsetup               — for LUKS unlock
#   /bin/bcachefs                 — for bcachefs key unlock
#   /bin/busybox + symlinks       — always present (init needs it); sh symlink
#                                   only with rescueShell=true
#   /etc/kexec-menu/static.json   — static boot entries (optional)
#   /dev /proc /sys /mnt /run /tmp — empty mountpoints
#
# Usage (from default.nix):
#   initrd-x86_64 = callPackage ./uki/initrd/initrd.nix {
#     kexec-menu = self.kexec-menu;
#     targetPkgsStatic = pkgs.pkgsStatic;
#   };
#
# Args:
#   kexec-menu       — the static kexec-menu binary package (target arch)
#   targetPkgsStatic — pkgsStatic for the target architecture
#   rescueShell      — include sh symlink for rescue shell (default: false)
#   staticEntries    — path to static.json, or null
#   extraContents    — attrset of { "/path" = source; } for additional files
{
  lib,
  runCommand,
  cpio,
  kexec-menu,
  targetPkgsStatic,
  rescueShell ? false,
  staticEntries ? null,
  extraContents ? {},
}:

let
  busybox = targetPkgsStatic.busybox;
  cryptsetup = targetPkgsStatic.cryptsetup;
  bcachefs-tools = targetPkgsStatic.bcachefs-tools;

  # Applets needed by /init
  initApplets = [ "mount" "umount" "mkdir" "sleep" "reboot" "echo" ];

  # Additional applets for rescue shell
  rescueApplets = [ "sh" "ls" "cat" "cp" "mv" "rm" "ln" "grep" "vi"
                     "ps" "kill" "dmesg" "df" "du" "find" "head" "tail"
                     "hexdump" "dd" "poweroff" ];

  applets = initApplets ++ lib.optionals rescueShell rescueApplets;

in runCommand "kexec-menu-initrd" {
  nativeBuildInputs = [ cpio ];
} ''
  root="$TMPDIR/initrd-root"
  mkdir -p "$root"/{bin,dev,proc,sys,mnt,run,tmp,etc/kexec-menu}

  # init script
  cp ${./init} "$root/init"
  chmod +x "$root/init"

  # kexec-menu binary
  cp ${kexec-menu}/bin/kexec-menu "$root/bin/kexec-menu"

  # Encryption tools
  cp ${cryptsetup}/bin/cryptsetup "$root/bin/cryptsetup"
  cp ${bcachefs-tools}/bin/bcachefs "$root/bin/bcachefs"

  # Busybox + applet symlinks
  cp ${busybox}/bin/busybox "$root/bin/busybox"
  for cmd in ${lib.concatStringsSep " " applets}; do
    ln -sf busybox "$root/bin/$cmd"
  done

  ${lib.optionalString (staticEntries != null) ''
    cp ${staticEntries} "$root/etc/kexec-menu/static.json"
  ''}

  # Extra contents
  ${lib.concatStringsSep "\n" (lib.mapAttrsToList (dest: src: ''
    mkdir -p "$root/$(dirname "${dest}")"
    cp -a ${src} "$root/${dest}"
  '') extraContents)}

  # Build CPIO archive (deterministic: sorted, null-delimited)
  (cd "$root" && find . -print0 | sort -z | cpio -o -H newc --quiet --null) > "$out"
''
