use std::path::PathBuf;

/// A mountable filesystem source.
pub struct Source {
    pub label: String,
    pub device: PathBuf,
    pub state: SourceState,
    pub mount_point: Option<PathBuf>,
}

pub enum SourceState {
    Mounted,
    Encrypted,
    Error(String),
}

/// A boot entry from entries.json.
pub struct Entry {
    pub name: String,
    pub kernel: String,
    pub initrd: String,
    pub cmdline: String,
}

/// A leaf directory containing entries.json.
pub struct Leaf {
    pub path: PathBuf,
    pub entries: Vec<Entry>,
}

/// A node in the boot tree.
pub enum TreeNode {
    Dir {
        name: String,
        children: Vec<TreeNode>,
    },
    Leaf(Leaf),
}

/// The resolved default boot selection.
pub struct BootSelection {
    pub leaf_path: PathBuf,
    pub entry_name: String,
}
