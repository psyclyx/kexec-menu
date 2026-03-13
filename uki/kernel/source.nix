# Pinned kernel source for kexec-menu UKI builds.
#
# Both Nix and Makefile should use this version. The Makefile has its own
# KERNEL_VERSION variable that must be kept in sync (kernel updates are rare
# and deliberate).
{
  version = "6.12.6";
  hash = "sha256-1FCrIV3k4fi7heD0IWdg+jP9AktFJrFE9M4NkBKynJ4=";
}
