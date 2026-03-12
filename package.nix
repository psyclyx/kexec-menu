# Builds the kexec-menu static binary for a given target.
#
# Usage:
#   nix-build -A kexec-menu        # x86_64
#   nix-build -A kexec-menu-aarch64 # cross-compile to aarch64
{
  lib,
  rustPlatform,
  # Overridable target triple; defaults to the host musl target.
  target ? "x86_64-unknown-linux-musl",
}:

rustPlatform.buildRustPackage {
  pname = "kexec-menu";
  version = "0.1.0";

  src = lib.cleanSourceWith {
    src = ./.;
    filter = path: type:
      let
        baseName = builtins.baseNameOf path;
        relPath = lib.removePrefix (toString ./. + "/") (toString path);
      in
      # Include Cargo files, Rust source, and nothing else
      baseName == "Cargo.toml"
      || baseName == "Cargo.lock"
      || lib.hasPrefix "crates" relPath;
  };

  cargoLock.lockFile = ./Cargo.lock;

  CARGO_BUILD_TARGET = target;
  CARGO_BUILD_RUSTFLAGS = "-C target-feature=+crt-static";

  meta = {
    description = "Filesystem-agnostic kexec boot menu (UKI)";
    license = lib.licenses.mit;
    mainProgram = "kexec-menu";
  };
}
