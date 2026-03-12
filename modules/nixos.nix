{ config, lib, pkgs, ... }:

let
  cfg = config.boot.loader.kexec-menu;
in
{
  options.boot.loader.kexec-menu = {
    enable = lib.mkEnableOption "kexec-menu bootloader";

    bootMountPoint = lib.mkOption {
      type = lib.types.str;
      default = "/boot";
      description = "Mount point where boot entries are written.";
    };

    retention = lib.mkOption {
      type = lib.types.ints.positive;
      default = 10;
      description = "Number of boot entry generations to keep.";
    };

    installStrategy = lib.mkOption {
      type = lib.types.enum [ "copy" "reflink" "snapshot" ];
      default = "copy";
      description = ''
        How kernel/initrd blobs are placed into leaf directories.
        - copy: plain cp (works on any filesystem)
        - reflink: cp --reflink=auto (saves space on bcachefs/btrfs)
        - snapshot: subvolume snapshot of previous leaf, overwrite changed files
      '';
    };

    ukiInstallPath = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = "If set, copy the UKI to this path on each rebuild.";
    };

    package = lib.mkOption {
      type = lib.types.package;
      description = "The kexec-menu UKI package.";
    };
  };

  config = lib.mkIf cfg.enable {
    # Installer script will be wired up in a subsequent iteration.
    # boot.loader.external = {
    #   enable = true;
    #   installBootLoader = "${installer}/bin/kexec-menu-install";
    # };
  };
}
