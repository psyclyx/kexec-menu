use std::path::PathBuf;

/// Errors returned by core operations.
#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Parse(String),
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "io: {e}"),
            Error::Parse(msg) => write!(f, "parse: {msg}"),
        }
    }
}

pub type Result<T> = core::result::Result<T, Error>;

/// A mountable filesystem source.
pub struct Source {
    pub label: String,
    pub device: PathBuf,
    pub state: SourceState,
    pub mount_point: Option<PathBuf>,
    /// Passphrase used to unlock an encrypted source, retained for key handoff.
    pub passphrase: Option<String>,
}

pub enum SourceState {
    Mounted,
    Encrypted,
    Error(String),
    Static,
}

/// A boot entry from entries.json.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub name: String,
    pub kernel: String,
    pub initrd: String,
    pub cmdline: String,
}

/// A leaf directory containing entries.json.
#[derive(Debug, Clone, PartialEq)]
pub struct Leaf {
    pub path: PathBuf,
    pub entries: Vec<Entry>,
    /// Modification time as seconds since UNIX epoch.
    pub mtime: u64,
}

/// A node in the boot tree.
#[derive(Debug, Clone, PartialEq)]
pub enum TreeNode {
    Dir {
        name: String,
        children: Vec<TreeNode>,
    },
    Leaf(Leaf),
}

/// The resolved default boot selection.
#[derive(Debug, Clone, PartialEq)]
pub struct BootSelection {
    pub leaf_path: PathBuf,
    pub entry_name: String,
}
