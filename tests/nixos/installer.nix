# NixOS VM test for the kexec-menu installer (the NixOS module's boot loader script).
#
# Tests:
#   - Installer creates correct directory layout under /boot/nixos
#   - entries.json has correct structure
#   - Specialisations produce additional entries
#   - Content dedup skips identical files (copy strategy)
#   - Pruning removes old generations beyond retention count
#   - UKI install path copies the UKI
#
# Usage:
#   nix-build tests/nixos/installer.nix
#   # or via default.nix:
#   nix-build -A tests.installer

let
  sources = import ../../npins;
  pkgs = import sources.nixpkgs {};

  mockUki = pkgs.runCommand "mock-uki" {} ''
    echo -n "MOCK_UKI_CONTENT" > $out
  '';

in
pkgs.testers.nixosTest {
  name = "kexec-menu-installer";

  nodes.machine = { pkgs, lib, config, ... }: {
    imports = [ ../../modules/nixos.nix ];

    boot.loader.kexec-menu = {
      enable = true;
      package = mockUki;
      retention = 3;
      ukiInstallPath = "/boot/EFI/kexec-menu.efi";
    };

    boot.loader.grub.enable = false;

    # Expose the installer path so the test script can find it
    environment.etc."kexec-menu-installer-path".text =
      config.boot.loader.external.installHook;
  };

  testScript = ''
    import json

    machine.wait_for_unit("multi-user.target")
    machine.succeed("mkdir -p /boot")

    # Read the installer path from the exposed config
    installer = machine.succeed("cat /etc/kexec-menu-installer-path").strip()
    machine.succeed(f"test -x {installer}")

    # --- Helper: create a mock NixOS toplevel ---
    def make_toplevel(name, cmdline="console=ttyS0 root=/dev/vda1", specs=None):
        """Create a mock NixOS system toplevel directory."""
        path = f"/tmp/{name}"
        machine.succeed(f"mkdir -p {path}")
        machine.succeed(f"echo 'KERNEL_{name}' > {path}/kernel")
        machine.succeed(f"echo 'INITRD_{name}' > {path}/initrd")
        machine.succeed(f"echo '{cmdline}' > {path}/kernel-params")

        if specs:
            for spec_name, spec_cmdline in specs.items():
                spec_dir = f"{path}/specialisation/{spec_name}"
                machine.succeed(f"mkdir -p {spec_dir}")
                machine.succeed(f"echo 'KERNEL_{name}_{spec_name}' > {spec_dir}/kernel")
                machine.succeed(f"echo 'INITRD_{name}_{spec_name}' > {spec_dir}/initrd")
                machine.succeed(f"echo '{spec_cmdline}' > {spec_dir}/kernel-params")

        return path

    # --- Test 1: Basic install ---
    with subtest("basic install creates correct layout"):
        toplevel = make_toplevel("gen1", cmdline="console=ttyS0 root=/dev/vda1")
        machine.succeed(f"{installer} {toplevel}")

        # Should have created a leaf dir under /boot/nixos/
        leaves = machine.succeed("ls /boot/nixos/").strip().split()
        assert len(leaves) == 1, f"expected 1 leaf, got {len(leaves)}: {leaves}"

        leaf = leaves[0]
        leaf_dir = f"/boot/nixos/{leaf}"

        # Check files exist
        machine.succeed(f"test -f {leaf_dir}/vmlinuz")
        machine.succeed(f"test -f {leaf_dir}/initrd")
        machine.succeed(f"test -f {leaf_dir}/entries.json")

        # Validate entries.json
        raw = machine.succeed(f"cat {leaf_dir}/entries.json")
        entries = json.loads(raw)
        assert len(entries) == 1, f"expected 1 entry, got {len(entries)}"
        assert entries[0]["name"] == "default"
        assert entries[0]["kernel"] == "vmlinuz"
        assert entries[0]["initrd"] == "initrd"
        assert "console=ttyS0" in entries[0]["cmdline"]

    # --- Test 2: Install with specialisations ---
    with subtest("specialisations produce additional entries"):
        toplevel = make_toplevel(
            "gen2",
            cmdline="console=ttyS0 root=/dev/vda1 quiet",
            specs={"gaming": "console=ttyS0 root=/dev/vda1 preempt=full"},
        )
        machine.succeed(f"{installer} {toplevel}")

        leaves = machine.succeed("ls /boot/nixos/").strip().split()
        assert len(leaves) == 2, f"expected 2 leaves, got {len(leaves)}: {leaves}"

        # Find the gen2 leaf (the one with 2 entries)
        gen2_leaf = None
        for l in leaves:
            raw = machine.succeed(f"cat /boot/nixos/{l}/entries.json")
            entries = json.loads(raw)
            if len(entries) == 2:
                gen2_leaf = l
                break

        assert gen2_leaf is not None, "no leaf with 2 entries found"
        leaf_dir = f"/boot/nixos/{gen2_leaf}"

        raw = machine.succeed(f"cat {leaf_dir}/entries.json")
        entries = json.loads(raw)
        names = [e["name"] for e in entries]
        assert "default" in names, f"missing 'default' in {names}"
        assert "gaming" in names, f"missing 'gaming' in {names}"

        # Specialisation should have separate kernel/initrd files
        gaming = [e for e in entries if e["name"] == "gaming"][0]
        assert gaming["kernel"] == "vmlinuz-gaming"
        assert gaming["initrd"] == "initrd-gaming"
        machine.succeed(f"test -f {leaf_dir}/vmlinuz-gaming")
        machine.succeed(f"test -f {leaf_dir}/initrd-gaming")

    # --- Test 3: Pruning ---
    with subtest("pruning removes old generations beyond retention"):
        # retention=3, we have 2, add 2 more to trigger pruning
        toplevel3 = make_toplevel("gen3")
        machine.succeed(f"{installer} {toplevel3}")
        toplevel4 = make_toplevel("gen4")
        machine.succeed(f"{installer} {toplevel4}")

        leaves = machine.succeed("ls /boot/nixos/").strip().split()
        assert len(leaves) == 3, f"expected 3 leaves after pruning (retention=3), got {len(leaves)}: {leaves}"

    # --- Test 4: Content dedup (copy strategy) ---
    with subtest("content dedup skips identical files"):
        # Install gen4 again — identical content, file should not be rewritten
        leaf4 = None
        for l in machine.succeed("ls /boot/nixos/").strip().split():
            raw = machine.succeed(f"cat /boot/nixos/{l}/vmlinuz")
            if "KERNEL_gen4" in raw:
                leaf4 = l
                break
        assert leaf4 is not None

        # Record mtime before re-install
        mtime_before = machine.succeed(f"stat -c %Y /boot/nixos/{leaf4}/vmlinuz").strip()
        machine.succeed("sleep 1")
        machine.succeed(f"{installer} /tmp/gen4")
        mtime_after = machine.succeed(f"stat -c %Y /boot/nixos/{leaf4}/vmlinuz").strip()

        assert mtime_before == mtime_after, \
            f"vmlinuz was rewritten despite identical content: {mtime_before} != {mtime_after}"

    # --- Test 5: UKI install path ---
    with subtest("UKI is copied to ukiInstallPath"):
        machine.succeed("test -f /boot/EFI/kexec-menu.efi")
        content = machine.succeed("cat /boot/EFI/kexec-menu.efi").strip()
        assert content == "MOCK_UKI_CONTENT", f"unexpected UKI content: {content}"
  '';
}
