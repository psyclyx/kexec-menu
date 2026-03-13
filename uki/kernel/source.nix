# Pinned kernel source for kexec-menu UKI builds.
#
# Both Nix and Makefile should use this version. The Makefile has its own
# KERNEL_VERSION variable that must be kept in sync (kernel updates are rare
# and deliberate).
{
  version = "6.12.76";
  hash = "sha256-u7Q+g0xG5r1JpcKPIuZ5qTdENATh9lMgTUskkp862JY=";
}
