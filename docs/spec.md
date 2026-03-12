# kexec-menu spec

## Overview

kexec-menu is a kexec boot menu distributed as a UKI (Unified Kernel Image).
It mounts available filesystems, discovers boot entries from a directory tree,
presents a menu, and kexecs the selected entry.

The UKI has no knowledge of NixOS, bcachefs layout, nix store paths, or any
distro-specific concepts. It reads entries.json and kexecs.

Targets: x86_64-unknown-linux-musl, aarch64-unknown-linux-musl.
aarch64 assumes a UEFI environment (towboot or similar).

---

## Entry Format

A **leaf** is a directory containing `entries.json`. Interior nodes are plain
directories. The tree lives under `boot/` on a mounted filesystem.

Leaf directories contain kernel and initrd blobs alongside `entries.json`.
Everything needed to kexec is self-contained in the leaf.

### entries.json

```json
[
  { "name": "default", "kernel": "vmlinuz", "initrd": "initrd-default", "cmdline": "..." },
  { "name": "gaming",  "kernel": "vmlinuz", "initrd": "initrd-gaming",  "cmdline": "..." }
]
```

Fields (all required):
- `name`: stable identifier, used for default selection, should not change across updates
- `kernel`: filename within the leaf directory
- `initrd`: filename within the leaf directory
- `cmdline`: full kernel command line

### Bootable files

EFI binaries and bzImages found while browsing a filesystem are directly
selectable from the full filesystem view. They are not synthesized into the
boot tree and do not appear in the default boot tree view.

---

## UX Structure

### Top level — sources

Each mounted (or mountable) filesystem appears as a top-level source, labeled
with the best available identifier (priority: partition label > filesystem
label > UUID). Static build-time entries (e.g. memtest, netboot.xyz) also
appear at the top level.

On startup, automount all visible block devices read-only where possible:
- Clean filesystems: mount immediately
- Encrypted (bcachefs native, LUKS): shown as locked, passphrase prompted on selection
- Errors (needs fsck, unrecognized, etc.): shown with error indicator, do not
  automount; user can attempt to open and handle errors in the rescue shell

### Default view — boot tree

Opening a source shows its `boot/` tree. This is the expected path for normal
use. No filesystem clutter.

### Full filesystem view

Accessible via keybind from the boot tree view. Browse the entire filesystem.
Selectable: any EFI binary or bzImage (kexec'd directly), any leaf directory
(opens entries.json menu). For power users and recovery.

### Design principle

Simple, powerful, gets out of the way in the expected case, discoverable when
something else is needed.

---

## Default Selection

EFI var (`e518894a-0634-4b2d-b448-e654c0eda6a7`) stores `(leaf_path, entry_name)`, written at kexec time.

1. Load `(leaf_path, entry_name)` from EFI var. If absent, go to 4.
2. Find most recently modified sibling of `leaf_path` (mtime).
3. Find entry with `name == entry_name`. If found, use it. Else use first entry in leaf.
4. Fallback: globally most recently modified leaf across all sources, first entry.

At any level of the tree, the cursor pre-selects the child leading toward the
default entry. Full tree remains navigable.

Installer scripts are responsible for setting mtime on new leaves. The
bootmenu does not interpret leaf directory names.

---

## Filesystem Mounting

Automount all visible block devices read-only on startup where possible.
Encrypted or errored sources shown but not automounted.

Supported:
- ext4, btrfs, bcachefs (unencrypted): mount directly
- bcachefs native encryption: prompt on selection
- LUKS: prompt on selection

Multiple filesystems appear as separate top-level sources. Boot trees are not merged.

**Key handoff to stage 1:** pass decrypted key as an additional initrd segment
to kexec. Stage 1 finds key at `/run/bootmenu-keys/<uuid>`. Never written to disk.

The UKI may be loaded from anywhere: flash drive, netboot, ESP, bcachefs
volume. No assumptions about its own provenance.

---

## Appearance

Build-time colorscheme and font configuration.
NixOS module adds a stylix target if stylix is present.

---

## UKI

Self-contained EFI binary. Small and fast is a design goal — bloat is a bug.
Separate kernel configs for x86_64 and aarch64. Rescue shell is a build-time
option (default: on).

Contents: minimal Linux kernel, initrd with bcachefs-tools, cryptsetup,
kexec-tools, bootmenu binary, minimal userspace (busybox or equivalent).

Buildable without Nix via Makefile. Nix is the primary build path.

---

## NixOS Module (modules/nixos.nix)

Importing alone is a no-op. Users must explicitly enable.
Fits NixOS bootloader option conventions.

Responsibilities:
- On rebuild: new leaf written to `boot/nixos/` via
  `boot.loader.external.installBootLoader`
- Installer script receives closure `$1`:
  - `$1/kernel`, `$1/initrd` copied into leaf as blobs
  - `$1/kernel-params` is cmdline for default entry
  - `$1/specialisation/<n>/` become additional entries in same leaf
- Blob deduplication (hash-based, or reflinks on bcachefs) is installer's concern
- Prune old leaves past configurable retention
- Optionally install UKI to a configured location — module option, not required
- Adds stylix target if stylix is present

bcachefs installer strategy is configurable (see below) and is an
enhancement over the naive copy, not required for correctness.

---

## Separation of Concerns

| Concern | Owner |
|---------|-------|
| Mounting filesystems | UKI |
| Enumerating boot trees | UKI |
| Default selection | UKI |
| TUI, kexec | UKI |
| EFI var (last booted) | UKI |
| Key handoff via initrd segment | UKI |
| Writing leaves, blobs, entries.json | Installer script |
| Blob deduplication / reflinks | Installer script |
| Pruning old leaves | Installer script |
| Installing UKI to ESP or elsewhere | NixOS module (optional) |

### bcachefs installer strategies (NixOS module option)

Three modes, selected via module option:

- **copy** (default, works on any filesystem): copy blobs into new leaf,
  deduplicate by hash (skip copy if destination already has identical content).
- **reflink**: create new leaf dir, reflink blobs from previous leaf, then
  overwrite only what has changed. Requires bcachefs. Saves space without
  requiring subvolume layout assumptions.
- **snapshot**: snapshot the previous leaf subvolume to create the new leaf,
  then overwrite changed files. Requires bcachefs with leaves as subvolumes.
  Most efficient. Assumes the user has structured their boot tree leaves as
  bcachefs subvolumes — document this clearly; it is not required.

The UKI is unaffected by which strategy is used. The on-disk result is
identical from the UKI's perspective.
