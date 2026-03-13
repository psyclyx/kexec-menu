{ config, lib, pkgs, ... }:

let
  cfg = config.boot.loader.kexec-menu;

  top = import ./..;

  arch =
    if pkgs.stdenv.hostPlatform.isx86_64 then "x86_64"
    else if pkgs.stdenv.hostPlatform.isAarch64 then "aarch64"
    else throw "kexec-menu: unsupported architecture";

  copyBlob = {
    copy = ''
      # Hash dedup: skip copy if destination already has identical content
      copy_blob() {
        local src="$1" dst="$2"
        if [ -f "$dst" ] && cmp -s "$src" "$dst"; then
          return
        fi
        cp "$src" "$dst"
      }
    '';
    reflink = ''
      copy_blob() {
        cp --reflink=auto "$1" "$2"
      }
    '';
  }.${cfg.installStrategy};

  installer = pkgs.writeShellApplication {
    name = "kexec-menu-install";
    runtimeInputs = with pkgs; [ coreutils jq ];
    text = ''
      ${copyBlob}

      toplevel="$1"
      boot_dir="${cfg.bootMountPoint}/nixos"

      # Derive leaf name from store path hash
      store_basename="$(basename "$toplevel")"
      leaf_name="''${store_basename%%-*}"
      leaf_dir="$boot_dir/$leaf_name"

      mkdir -p "$leaf_dir"

      # Copy kernel and initrd
      copy_blob "$(readlink -f "$toplevel/kernel")" "$leaf_dir/vmlinuz"
      copy_blob "$(readlink -f "$toplevel/initrd")" "$leaf_dir/initrd"

      # Read kernel cmdline
      cmdline="$(cat "$toplevel/kernel-params")"

      # Build entries array: default entry first
      entries="$(jq -n --arg cmdline "$cmdline" \
        '[{"name": "default", "kernel": "vmlinuz", "initrd": "initrd", "cmdline": $cmdline}]')"

      # Add specialisation entries
      spec_dir="$toplevel/specialisation"
      if [ -d "$spec_dir" ]; then
        for spec in "$spec_dir"/*/; do
          [ -d "$spec" ] || continue
          spec_name="$(basename "$spec")"

          # Each specialisation has its own kernel, initrd, kernel-params
          spec_kernel="vmlinuz-$spec_name"
          spec_initrd="initrd-$spec_name"

          copy_blob "$(readlink -f "$spec/kernel")" "$leaf_dir/$spec_kernel"
          copy_blob "$(readlink -f "$spec/initrd")" "$leaf_dir/$spec_initrd"
          spec_cmdline="$(cat "$spec/kernel-params")"

          entries="$(echo "$entries" | jq \
            --arg name "$spec_name" \
            --arg kernel "$spec_kernel" \
            --arg initrd "$spec_initrd" \
            --arg cmdline "$spec_cmdline" \
            '. + [{"name": $name, "kernel": $kernel, "initrd": $initrd, "cmdline": $cmdline}]')"
        done
      fi

      echo "$entries" > "$leaf_dir/entries.json"

      # Touch mtime so the bootmenu picks this as most recent
      touch "$leaf_dir"

      # Prune old generations beyond retention count
      mapfile -t leaves < <(
        find "$boot_dir" -mindepth 1 -maxdepth 1 -type d -printf '%T@\t%p\n' \
          | sort -rn | cut -f2
      )
      if [ "''${#leaves[@]}" -gt ${toString cfg.retention} ]; then
        for leaf in "''${leaves[@]:${toString cfg.retention}}"; do
          rm -rf "$leaf"
        done
      fi

      # Optionally install UKI
      ${lib.optionalString (cfg.ukiInstallPath != null) ''
        mkdir -p "$(dirname "${cfg.ukiInstallPath}")"
        cp "${cfg.finalPackage}" "${cfg.ukiInstallPath}"
      ''}
    '';
  };
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
      type = lib.types.enum [ "copy" "reflink" ];
      default = "copy";
      description = ''
        How kernel/initrd blobs are placed into leaf directories.
        - copy: plain cp, skips identical files (works on any filesystem)
        - reflink: cp --reflink=auto (saves space on bcachefs/btrfs)
      '';
    };

    ukiInstallPath = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = "If set, copy the UKI to this path on each rebuild.";
    };

    package = lib.mkOption {
      type = lib.types.package;
      default = top."uki-${arch}";
      defaultText = lib.literalExpression ''
        (import ''${kexec-menu-src})."uki-''${arch}"
      '';
      description = "The kexec-menu UKI package.";
    };

    finalPackage = lib.mkOption {
      type = lib.types.package;
      readOnly = true;
      default =
        let
          binaryOverrides =
            lib.optionalAttrs (cfg.theme != null) { theme = cfg.theme; }
            // lib.optionalAttrs (cfg.timeout != null) { inherit (cfg) timeout; };
        in
          if binaryOverrides == {} then cfg.package
          else
            let
              binaryName = if arch == "aarch64" then "kexec-menu-aarch64" else "kexec-menu";
              binary = top.${binaryName}.override binaryOverrides;
              initrd = top."initrd-${arch}".override { kexec-menu = binary; };
            in top."uki-${arch}".override { inherit initrd; };
      defaultText = lib.literalExpression "cfg.package (with theme/timeout overrides applied)";
      description = "The final kexec-menu UKI package after applying overrides. Read-only.";
    };

    timeout = lib.mkOption {
      type = lib.types.nullOr lib.types.ints.unsigned;
      default = null;
      example = 5;
      description = ''
        Autoboot timeout in seconds. The default entry will be booted
        automatically after this many seconds unless a key is pressed.
        Set to 0 for near-instant boot (100ms interrupt window).
        Set to 65535 to disable autoboot entirely.
        When null, uses the compiled-in default (5 seconds).
      '';
    };

    theme = lib.mkOption {
      type = lib.types.nullOr (lib.types.attrsOf lib.types.str);
      default = null;
      example = lib.literalExpression ''
        {
          base00 = "1d1f21"; base01 = "282a2e"; base02 = "373b41"; base03 = "969896";
          base04 = "b4b7b4"; base05 = "c5c8c6"; base06 = "e0e0e0"; base07 = "ffffff";
          base08 = "cc6666"; base09 = "de935f"; base0A = "f0c674"; base0B = "b5bd68";
          base0C = "8abeb7"; base0D = "81a2be"; base0E = "b294bb"; base0F = "a3685a";
        }
      '';
      description = ''
        Base16 colorscheme for the boot menu TUI. Attrset of hex color values
        (without #). When null, the default terminal palette is used.
        Import modules/stylix.nix for automatic Stylix integration.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    boot.loader.external = {
      enable = true;
      installHook = "${installer}/bin/kexec-menu-install";
    };
  };
}
