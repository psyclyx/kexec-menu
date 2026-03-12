// Default entry selection algorithm.

use std::path::Path;

use crate::types::{BootSelection, Leaf, TreeNode};

/// Resolve which boot entry to pre-select.
///
/// Algorithm (from spec):
/// 1. If `efi_var` is Some, find the most recently modified sibling leaf
///    (same parent directory as `efi_var.leaf_path`).
/// 2. In that leaf, match `efi_var.entry_name`; if missing, use first entry.
/// 3. Fallback: globally most recent leaf, first entry.
pub fn resolve_default(
    trees: &[(String, Vec<TreeNode>)],
    efi_var: Option<&BootSelection>,
) -> Option<BootSelection> {
    if let Some(var) = efi_var {
        if let Some(sel) = resolve_from_efi_var(trees, var) {
            return Some(sel);
        }
    }
    resolve_global_fallback(trees)
}

fn resolve_from_efi_var(
    trees: &[(String, Vec<TreeNode>)],
    var: &BootSelection,
) -> Option<BootSelection> {
    let var_parent = Path::new(&var.leaf_path).parent()?;

    // Collect sibling leaves (same parent directory)
    let mut siblings: Vec<&Leaf> = Vec::new();
    for (_, tree) in trees {
        collect_leaves(tree, &mut |leaf| {
            if leaf.path.parent() == Some(var_parent) {
                siblings.push(leaf);
            }
        });
    }

    if siblings.is_empty() {
        return None;
    }

    // Pick the most recently modified sibling
    let newest = siblings.iter().max_by_key(|l| l.mtime)?;

    // Match entry by name, else first
    let entry_name = newest
        .entries
        .iter()
        .find(|e| e.name == var.entry_name)
        .or_else(|| newest.entries.first())
        .map(|e| e.name.clone())?;

    Some(BootSelection {
        leaf_path: newest.path.clone(),
        entry_name,
    })
}

fn resolve_global_fallback(trees: &[(String, Vec<TreeNode>)]) -> Option<BootSelection> {
    let mut newest: Option<&Leaf> = None;
    for (_, tree) in trees {
        collect_leaves(tree, &mut |leaf| {
            if newest.map_or(true, |n| leaf.mtime > n.mtime) {
                newest = Some(leaf);
            }
        });
    }
    let leaf = newest?;
    let entry_name = leaf.entries.first()?.name.clone();
    Some(BootSelection {
        leaf_path: leaf.path.clone(),
        entry_name,
    })
}

fn collect_leaves<'a>(nodes: &'a [TreeNode], f: &mut impl FnMut(&'a Leaf)) {
    for node in nodes {
        match node {
            TreeNode::Leaf(leaf) => f(leaf),
            TreeNode::Dir { children, .. } => collect_leaves(children, f),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use crate::types::Entry;

    fn entry(name: &str) -> Entry {
        Entry {
            name: name.into(),
            kernel: "vmlinuz".into(),
            initrd: "initrd".into(),
            cmdline: "root=/dev/sda1".into(),
        }
    }

    fn leaf(path: &str, names: &[&str], mtime: u64) -> TreeNode {
        TreeNode::Leaf(Leaf {
            path: PathBuf::from(path),
            entries: names.iter().map(|n| entry(n)).collect(),
            mtime,
        })
    }

    fn source(label: &str, nodes: Vec<TreeNode>) -> (String, Vec<TreeNode>) {
        (label.into(), nodes)
    }

    // --- No EFI var: global fallback ---

    #[test]
    fn fallback_picks_most_recent_leaf() {
        let trees = vec![source("disk", vec![
            leaf("/mnt/boot/nixos/gen1", &["default"], 100),
            leaf("/mnt/boot/nixos/gen2", &["default"], 200),
        ])];
        let sel = resolve_default(&trees, None).unwrap();
        assert_eq!(sel.leaf_path, PathBuf::from("/mnt/boot/nixos/gen2"));
        assert_eq!(sel.entry_name, "default");
    }

    #[test]
    fn fallback_across_sources() {
        let trees = vec![
            source("disk1", vec![leaf("/mnt1/boot/gen1", &["a"], 100)]),
            source("disk2", vec![leaf("/mnt2/boot/gen1", &["b"], 300)]),
        ];
        let sel = resolve_default(&trees, None).unwrap();
        assert_eq!(sel.leaf_path, PathBuf::from("/mnt2/boot/gen1"));
        assert_eq!(sel.entry_name, "b");
    }

    #[test]
    fn fallback_uses_first_entry() {
        let trees = vec![source("disk", vec![
            leaf("/mnt/boot/gen1", &["default", "gaming"], 100),
        ])];
        let sel = resolve_default(&trees, None).unwrap();
        assert_eq!(sel.entry_name, "default");
    }

    #[test]
    fn empty_trees_returns_none() {
        let trees: Vec<(String, Vec<TreeNode>)> = vec![];
        assert!(resolve_default(&trees, None).is_none());
    }

    #[test]
    fn empty_source_returns_none() {
        let trees = vec![source("disk", vec![])];
        assert!(resolve_default(&trees, None).is_none());
    }

    // --- With EFI var ---

    #[test]
    fn efi_var_selects_newest_sibling() {
        let trees = vec![source("disk", vec![
            leaf("/mnt/boot/nixos/gen1", &["default"], 100),
            leaf("/mnt/boot/nixos/gen2", &["default", "gaming"], 200),
            leaf("/mnt/boot/other/gen1", &["default"], 300),
        ])];
        let var = BootSelection {
            leaf_path: PathBuf::from("/mnt/boot/nixos/gen1"),
            entry_name: "default".into(),
        };
        let sel = resolve_default(&trees, Some(&var)).unwrap();
        // Should pick gen2 (newest sibling of gen1 under /mnt/boot/nixos/)
        assert_eq!(sel.leaf_path, PathBuf::from("/mnt/boot/nixos/gen2"));
        assert_eq!(sel.entry_name, "default");
    }

    #[test]
    fn efi_var_matches_entry_name() {
        let trees = vec![source("disk", vec![
            leaf("/mnt/boot/nixos/gen1", &["default", "gaming"], 100),
            leaf("/mnt/boot/nixos/gen2", &["default", "gaming"], 200),
        ])];
        let var = BootSelection {
            leaf_path: PathBuf::from("/mnt/boot/nixos/gen1"),
            entry_name: "gaming".into(),
        };
        let sel = resolve_default(&trees, Some(&var)).unwrap();
        assert_eq!(sel.leaf_path, PathBuf::from("/mnt/boot/nixos/gen2"));
        assert_eq!(sel.entry_name, "gaming");
    }

    #[test]
    fn efi_var_entry_missing_falls_to_first() {
        let trees = vec![source("disk", vec![
            leaf("/mnt/boot/nixos/gen1", &["default"], 100),
            leaf("/mnt/boot/nixos/gen2", &["default"], 200),
        ])];
        let var = BootSelection {
            leaf_path: PathBuf::from("/mnt/boot/nixos/gen1"),
            entry_name: "gaming".into(), // doesn't exist in gen2
        };
        let sel = resolve_default(&trees, Some(&var)).unwrap();
        assert_eq!(sel.leaf_path, PathBuf::from("/mnt/boot/nixos/gen2"));
        assert_eq!(sel.entry_name, "default");
    }

    #[test]
    fn efi_var_parent_gone_falls_to_global() {
        let trees = vec![source("disk", vec![
            leaf("/mnt/boot/other/gen1", &["default"], 100),
        ])];
        let var = BootSelection {
            leaf_path: PathBuf::from("/mnt/boot/nixos/gen1"), // parent gone
            entry_name: "default".into(),
        };
        let sel = resolve_default(&trees, Some(&var)).unwrap();
        // Falls back to global: /mnt/boot/other/gen1
        assert_eq!(sel.leaf_path, PathBuf::from("/mnt/boot/other/gen1"));
        assert_eq!(sel.entry_name, "default");
    }

    #[test]
    fn efi_var_with_nested_tree() {
        let trees = vec![source("disk", vec![
            TreeNode::Dir {
                name: "nixos".into(),
                children: vec![
                    leaf("/mnt/boot/nixos/gen1", &["default"], 100),
                    leaf("/mnt/boot/nixos/gen2", &["default"], 200),
                ],
            },
            TreeNode::Dir {
                name: "arch".into(),
                children: vec![
                    leaf("/mnt/boot/arch/gen1", &["default"], 300),
                ],
            },
        ])];
        let var = BootSelection {
            leaf_path: PathBuf::from("/mnt/boot/nixos/gen1"),
            entry_name: "default".into(),
        };
        let sel = resolve_default(&trees, Some(&var)).unwrap();
        assert_eq!(sel.leaf_path, PathBuf::from("/mnt/boot/nixos/gen2"));
    }

    #[test]
    fn efi_var_single_sibling_selects_itself() {
        let trees = vec![source("disk", vec![
            leaf("/mnt/boot/nixos/gen1", &["default", "gaming"], 100),
        ])];
        let var = BootSelection {
            leaf_path: PathBuf::from("/mnt/boot/nixos/gen1"),
            entry_name: "gaming".into(),
        };
        let sel = resolve_default(&trees, Some(&var)).unwrap();
        assert_eq!(sel.leaf_path, PathBuf::from("/mnt/boot/nixos/gen1"));
        assert_eq!(sel.entry_name, "gaming");
    }
}
