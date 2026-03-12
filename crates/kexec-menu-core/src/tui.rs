// Terminal UI: source list, boot tree navigation, entry selection.
//
// Design: all rendering writes to `impl Write`, all input reads from `impl Read`.
// This keeps the TUI testable without a real terminal.

use std::io::{self, Read, Write};

use crate::types::{BootSelection, Entry, Source, SourceState, TreeNode};

// --- Keys ---

#[derive(Debug, Clone, PartialEq)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Enter,
    Escape,
    Backspace,
    Char(char),
    Unknown,
}

/// Parse a single key from raw terminal input.
pub fn read_key(input: &mut impl Read) -> io::Result<Key> {
    let mut buf = [0u8; 1];
    input.read_exact(&mut buf)?;
    match buf[0] {
        b'\x1b' => {
            let mut seq = [0u8; 2];
            if input.read_exact(&mut seq).is_err() {
                return Ok(Key::Escape);
            }
            if seq[0] == b'[' {
                match seq[1] {
                    b'A' => Ok(Key::Up),
                    b'B' => Ok(Key::Down),
                    b'C' => Ok(Key::Right),
                    b'D' => Ok(Key::Left),
                    _ => Ok(Key::Unknown),
                }
            } else {
                Ok(Key::Unknown)
            }
        }
        b'\r' | b'\n' => Ok(Key::Enter),
        b'\x7f' | b'\x08' => Ok(Key::Backspace),
        b if b >= 0x20 && b < 0x7f => Ok(Key::Char(b as char)),
        _ => Ok(Key::Unknown),
    }
}

// --- ANSI escape helpers ---

pub fn clear_screen(w: &mut impl Write) -> io::Result<()> {
    w.write_all(b"\x1b[2J\x1b[H")
}

pub fn move_cursor(w: &mut impl Write, row: u16, col: u16) -> io::Result<()> {
    write!(w, "\x1b[{};{}H", row, col)
}

pub fn set_bold(w: &mut impl Write) -> io::Result<()> {
    w.write_all(b"\x1b[1m")
}

pub fn set_reverse(w: &mut impl Write) -> io::Result<()> {
    w.write_all(b"\x1b[7m")
}

pub fn reset_style(w: &mut impl Write) -> io::Result<()> {
    w.write_all(b"\x1b[0m")
}

pub fn set_dim(w: &mut impl Write) -> io::Result<()> {
    w.write_all(b"\x1b[2m")
}

pub fn hide_cursor(w: &mut impl Write) -> io::Result<()> {
    w.write_all(b"\x1b[?25l")
}

pub fn show_cursor(w: &mut impl Write) -> io::Result<()> {
    w.write_all(b"\x1b[?25h")
}

// --- Menu model ---

/// A navigable list of items with a cursor and optional pre-selected index.
pub struct Menu {
    pub items: Vec<MenuItem>,
    pub cursor: usize,
    pub preselected: Option<usize>,
}

pub struct MenuItem {
    pub label: String,
    pub detail: String,
    pub state: ItemState,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ItemState {
    Normal,
    Default,
    Locked,
    Error(String),
}

impl Menu {
    pub fn new(items: Vec<MenuItem>, preselected: Option<usize>) -> Self {
        let cursor = preselected.unwrap_or(0).min(items.len().saturating_sub(1));
        Self { items, cursor, preselected }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn move_up(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    pub fn move_down(&mut self) {
        if self.cursor + 1 < self.items.len() {
            self.cursor += 1;
        }
    }

    pub fn selected(&self) -> Option<&MenuItem> {
        self.items.get(self.cursor)
    }

    pub fn selected_index(&self) -> usize {
        self.cursor
    }
}

// --- Unified tree view ---

/// A node in the unified tree view.
#[derive(Debug, Clone)]
pub struct TreeViewNode {
    pub depth: usize,
    pub kind: NodeKind,
    pub expanded: bool,
    pub visible: bool,
    pub is_default: bool,
}

/// The kind of a tree view node.
#[derive(Debug, Clone)]
pub enum NodeKind {
    Source {
        idx: usize,
        label: String,
        state: NodeSourceState,
    },
    Dir {
        name: String,
    },
    Leaf {
        name: String,
        path: std::path::PathBuf,
        entry_count: usize,
    },
    Entry {
        entry: Entry,
        source_idx: usize,
    },
}

/// Source state as visible to the tree view (avoids borrowing SourceState).
#[derive(Debug, Clone, PartialEq)]
pub enum NodeSourceState {
    Mounted,
    Encrypted,
    Error(String),
    Static,
    Empty,
}

/// The unified tree view: one navigable tree of all sources + boot entries.
pub struct TreeView {
    pub nodes: Vec<TreeViewNode>,
    pub cursor: usize,
}

impl TreeView {
    /// Build a tree view from sources, their parsed trees, and the default selection.
    pub fn build(
        sources: &[Source],
        trees: &[(String, Vec<TreeNode>)],
        default: Option<&BootSelection>,
    ) -> Self {
        let mut nodes = Vec::new();
        let mut default_entry_idx = None;

        // Determine which source/path should be expanded to show the default
        let default_source_idx = default.and_then(|sel| {
            trees.iter().enumerate().find_map(|(i, (_, tree))| {
                if tree_contains_path(tree, &sel.leaf_path) {
                    Some(i)
                } else {
                    None
                }
            })
        });

        for (i, src) in sources.iter().enumerate() {
            let state = match &src.state {
                SourceState::Mounted | SourceState::Static => {
                    if trees.get(i).map(|(_, t)| t.is_empty()).unwrap_or(true) {
                        NodeSourceState::Empty
                    } else {
                        match &src.state {
                            SourceState::Static => NodeSourceState::Static,
                            _ => NodeSourceState::Mounted,
                        }
                    }
                }
                SourceState::Encrypted => NodeSourceState::Encrypted,
                SourceState::Error(e) => NodeSourceState::Error(e.clone()),
            };

            let has_default = default_source_idx == Some(i);
            let expandable = matches!(state, NodeSourceState::Mounted | NodeSourceState::Static);

            nodes.push(TreeViewNode {
                depth: 0,
                kind: NodeKind::Source {
                    idx: i,
                    label: src.label.clone(),
                    state,
                },
                expanded: expandable && has_default,
                visible: true,
                is_default: false,
            });

            // Add tree children if source is mounted/static
            if expandable {
                if let Some((_, tree)) = trees.get(i) {
                    Self::add_tree_nodes(
                        &mut nodes,
                        tree,
                        1, // depth
                        i, // source_idx
                        default,
                        &mut default_entry_idx,
                    );
                }
            }
        }

        // Recompute visibility based on expansion state
        let mut view = TreeView { nodes, cursor: 0 };
        view.recompute_visibility();

        // Set cursor to default entry, or first visible node
        if let Some(idx) = default_entry_idx {
            if view.nodes.get(idx).map(|n| n.visible).unwrap_or(false) {
                view.cursor = idx;
            }
        }

        view
    }

    fn add_tree_nodes(
        nodes: &mut Vec<TreeViewNode>,
        tree: &[TreeNode],
        depth: usize,
        source_idx: usize,
        default: Option<&BootSelection>,
        default_entry_idx: &mut Option<usize>,
    ) {
        for tree_node in tree {
            match tree_node {
                TreeNode::Dir { name, children } => {
                    let path_has_default = default
                        .map(|sel| tree_contains_path(children, &sel.leaf_path))
                        .unwrap_or(false);

                    nodes.push(TreeViewNode {
                        depth,
                        kind: NodeKind::Dir { name: name.clone() },
                        expanded: path_has_default,
                        visible: false, // recomputed later
                        is_default: false,
                    });
                    Self::add_tree_nodes(
                        nodes,
                        children,
                        depth + 1,
                        source_idx,
                        default,
                        default_entry_idx,
                    );
                }
                TreeNode::Leaf(leaf) => {
                    let leaf_name = leaf
                        .path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| leaf.path.to_string_lossy().into_owned());

                    let is_default_leaf = default
                        .map(|sel| sel.leaf_path == leaf.path)
                        .unwrap_or(false);

                    nodes.push(TreeViewNode {
                        depth,
                        kind: NodeKind::Leaf {
                            name: leaf_name,
                            path: leaf.path.clone(),
                            entry_count: leaf.entries.len(),
                        },
                        expanded: is_default_leaf,
                        visible: false,
                        is_default: false,
                    });

                    // Add entries under this leaf
                    for entry in &leaf.entries {
                        let is_default_entry = default
                            .map(|sel| {
                                sel.leaf_path == leaf.path && sel.entry_name == entry.name
                            })
                            .unwrap_or(false);

                        let entry_idx = nodes.len();
                        if is_default_entry {
                            *default_entry_idx = Some(entry_idx);
                        }

                        nodes.push(TreeViewNode {
                            depth: depth + 1,
                            kind: NodeKind::Entry {
                                entry: entry.clone(),
                                source_idx,
                            },
                            expanded: false,
                            visible: false,
                            is_default: is_default_entry,
                        });
                    }
                }
            }
        }
    }

    /// Recompute visibility of all nodes based on ancestor expansion state.
    pub fn recompute_visibility(&mut self) {
        // A node is visible iff all ancestors are expanded.
        // Walk the list tracking a "hidden below depth" threshold.
        let mut hidden_below: Option<usize> = None;

        for node in self.nodes.iter_mut() {
            if let Some(threshold) = hidden_below {
                if node.depth <= threshold {
                    // We've exited the collapsed subtree
                    hidden_below = None;
                }
            }

            if hidden_below.is_some() {
                node.visible = false;
            } else {
                node.visible = true;
                if !node.expanded && !matches!(node.kind, NodeKind::Entry { .. }) {
                    // This node is collapsed — hide everything deeper
                    hidden_below = Some(node.depth);
                }
            }
        }
    }

    /// Move cursor to the next visible node.
    pub fn move_down(&mut self) {
        let start = self.cursor + 1;
        for i in start..self.nodes.len() {
            if self.nodes[i].visible {
                self.cursor = i;
                return;
            }
        }
    }

    /// Move cursor to the previous visible node.
    pub fn move_up(&mut self) {
        if self.cursor == 0 {
            return;
        }
        for i in (0..self.cursor).rev() {
            if self.nodes[i].visible {
                self.cursor = i;
                return;
            }
        }
    }

    /// Toggle expand/collapse of the node at the cursor.
    /// Returns true if state changed.
    pub fn toggle(&mut self) -> bool {
        if let Some(node) = self.nodes.get_mut(self.cursor) {
            match &node.kind {
                NodeKind::Entry { .. } => return false, // entries don't toggle
                NodeKind::Source { state, .. } => {
                    match state {
                        NodeSourceState::Encrypted
                        | NodeSourceState::Error(_)
                        | NodeSourceState::Empty => return false,
                        _ => {}
                    }
                }
                _ => {}
            }
            node.expanded = !node.expanded;
            self.recompute_visibility();
            // If cursor is now on an invisible node (shouldn't happen for toggle),
            // move to parent
            self.ensure_cursor_visible();
            true
        } else {
            false
        }
    }

    /// Expand the node at the cursor. Returns true if state changed.
    pub fn expand(&mut self) -> bool {
        if let Some(node) = self.nodes.get(self.cursor) {
            if matches!(node.kind, NodeKind::Entry { .. }) {
                return false;
            }
            if node.expanded {
                return false;
            }
        }
        self.toggle()
    }

    /// Collapse the node at the cursor, or move to parent if already collapsed/entry.
    pub fn collapse(&mut self) -> bool {
        if let Some(node) = self.nodes.get(self.cursor) {
            if !matches!(node.kind, NodeKind::Entry { .. }) && node.expanded {
                return self.toggle();
            }
            // Move to parent: find nearest visible ancestor with lower depth
            let depth = node.depth;
            if depth == 0 {
                return false;
            }
            for i in (0..self.cursor).rev() {
                if self.nodes[i].visible && self.nodes[i].depth < depth {
                    self.cursor = i;
                    return true;
                }
            }
        }
        false
    }

    /// Ensure cursor is on a visible node. If not, move to the nearest visible ancestor.
    fn ensure_cursor_visible(&mut self) {
        if self.nodes.get(self.cursor).map(|n| n.visible).unwrap_or(false) {
            return;
        }
        // Move cursor up to nearest visible node
        for i in (0..self.cursor).rev() {
            if self.nodes[i].visible {
                self.cursor = i;
                return;
            }
        }
        self.cursor = 0;
    }

    /// Get the node at the cursor.
    pub fn selected(&self) -> Option<&TreeViewNode> {
        self.nodes.get(self.cursor)
    }

    /// Count visible nodes.
    pub fn visible_count(&self) -> usize {
        self.nodes.iter().filter(|n| n.visible).count()
    }

    /// Find the source index for the node at the cursor.
    pub fn cursor_source_idx(&self) -> Option<usize> {
        // Walk backward to find the source ancestor
        for i in (0..=self.cursor).rev() {
            if let NodeKind::Source { idx, .. } = &self.nodes[i].kind {
                return Some(*idx);
            }
        }
        None
    }
}

// --- Screen state machine ---

pub enum Screen {
    /// Top-level source list.
    Sources(Menu),
    /// Boot tree for a specific source. `source_idx` tracks which source.
    BootTree {
        source_idx: usize,
        source_label: String,
        /// Flattened tree items with indentation level.
        menu: Menu,
        /// Map from menu index to tree path info.
        nodes: Vec<FlatNode>,
    },
    /// Entry list for a specific leaf.
    Entries {
        source_idx: usize,
        source_label: String,
        leaf_label: String,
        leaf_path: std::path::PathBuf,
        menu: Menu,
        entries: Vec<Entry>,
    },
    /// Passphrase prompt for an encrypted source.
    Passphrase {
        source_idx: usize,
        source_label: String,
        input: String,
        error: Option<String>,
    },
    /// Full filesystem browser for a source.
    FileBrowser {
        source_idx: usize,
        source_label: String,
        /// Current directory being browsed.
        current_dir: std::path::PathBuf,
        /// Root of the mount (can't go above this).
        root: std::path::PathBuf,
        menu: Menu,
        /// Directory entries corresponding to menu items.
        dir_entries: Vec<DirEntry>,
    },
}

/// An entry in a directory listing for the file browser.
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    pub is_bootable: bool,
    pub path: std::path::PathBuf,
}

/// A flattened tree node for display.
#[derive(Debug, Clone)]
pub struct FlatNode {
    pub depth: usize,
    pub kind: FlatNodeKind,
}

#[derive(Debug, Clone)]
pub enum FlatNodeKind {
    Dir { name: String },
    Leaf { name: String, entry_count: usize, path: std::path::PathBuf },
}

/// Flatten a tree into a list of FlatNodes for display.
pub fn flatten_tree(nodes: &[TreeNode], depth: usize) -> Vec<FlatNode> {
    let mut flat = Vec::new();
    for node in nodes {
        match node {
            TreeNode::Dir { name, children } => {
                flat.push(FlatNode {
                    depth,
                    kind: FlatNodeKind::Dir { name: name.clone() },
                });
                flat.extend(flatten_tree(children, depth + 1));
            }
            TreeNode::Leaf(leaf) => {
                let name = leaf
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| leaf.path.to_string_lossy().into_owned());
                flat.push(FlatNode {
                    depth,
                    kind: FlatNodeKind::Leaf {
                        name,
                        entry_count: leaf.entries.len(),
                        path: leaf.path.clone(),
                    },
                });
            }
        }
    }
    flat
}

/// Build a source menu from a list of sources.
pub fn build_source_menu(
    sources: &[Source],
    default: Option<&BootSelection>,
    trees: &[(String, Vec<TreeNode>)],
) -> Menu {
    let mut preselected = None;
    let items: Vec<MenuItem> = sources
        .iter()
        .enumerate()
        .map(|(i, src)| {
            let state_label = match &src.state {
                SourceState::Mounted => "",
                SourceState::Encrypted => " [locked]",
                SourceState::Error(_) => " [error]",
                SourceState::Static => " [static]",
            };
            let item_state = match &src.state {
                SourceState::Mounted | SourceState::Static => ItemState::Normal,
                SourceState::Encrypted => ItemState::Locked,
                SourceState::Error(e) => ItemState::Error(e.clone()),
            };

            // Pre-select the source containing the default leaf
            if preselected.is_none() {
                if let Some(sel) = default {
                    if let Some((_, tree)) = trees.get(i) {
                        if tree_contains_path(tree, &sel.leaf_path) {
                            preselected = Some(i);
                        }
                    }
                }
            }

            MenuItem {
                label: src.label.clone(),
                detail: format!("{}{}", src.device.display(), state_label),
                state: item_state,
            }
        })
        .collect();

    Menu::new(items, preselected)
}

/// Build a boot tree menu for a source.
pub fn build_tree_menu(
    tree: &[TreeNode],
    default: Option<&BootSelection>,
) -> (Menu, Vec<FlatNode>) {
    let flat = flatten_tree(tree, 0);
    let preselected = default.and_then(|sel| {
        flat.iter().position(|n| match &n.kind {
            FlatNodeKind::Leaf { path, .. } => *path == sel.leaf_path,
            _ => false,
        })
    });
    let items: Vec<MenuItem> = flat
        .iter()
        .map(|n| {
            let indent = "  ".repeat(n.depth);
            match &n.kind {
                FlatNodeKind::Dir { name } => MenuItem {
                    label: format!("{indent}{name}/"),
                    detail: String::new(),
                    state: ItemState::Normal,
                },
                FlatNodeKind::Leaf { name, entry_count, path } => {
                    let is_default = default
                        .map(|s| s.leaf_path == *path)
                        .unwrap_or(false);
                    MenuItem {
                        label: format!("{indent}{name}"),
                        detail: format!("{entry_count} entries"),
                        state: if is_default { ItemState::Default } else { ItemState::Normal },
                    }
                }
            }
        })
        .collect();

    (Menu::new(items, preselected), flat)
}

/// Build an entry menu for a leaf.
pub fn build_entry_menu(
    entries: &[Entry],
    default: Option<&BootSelection>,
    leaf_path: &std::path::Path,
) -> Menu {
    let preselected = default.and_then(|sel| {
        if sel.leaf_path == leaf_path {
            entries.iter().position(|e| e.name == sel.entry_name)
        } else {
            None
        }
    });
    let items: Vec<MenuItem> = entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let is_default = preselected == Some(i);
            MenuItem {
                label: e.name.clone(),
                detail: e.cmdline.clone(),
                state: if is_default { ItemState::Default } else { ItemState::Normal },
            }
        })
        .collect();
    Menu::new(items, preselected)
}

fn tree_contains_path(nodes: &[TreeNode], path: &std::path::Path) -> bool {
    for node in nodes {
        match node {
            TreeNode::Leaf(leaf) => {
                if leaf.path == path {
                    return true;
                }
            }
            TreeNode::Dir { children, .. } => {
                if tree_contains_path(children, path) {
                    return true;
                }
            }
        }
    }
    false
}

// --- Rendering ---

const TITLE: &str = "kexec-menu";

/// Render a menu screen.
pub fn render_menu(
    w: &mut impl Write,
    title: &str,
    breadcrumb: &str,
    menu: &Menu,
    hint: &str,
) -> io::Result<()> {
    clear_screen(w)?;
    move_cursor(w, 1, 1)?;

    // Title bar
    set_bold(w)?;
    write!(w, " {TITLE}")?;
    reset_style(w)?;
    if !breadcrumb.is_empty() {
        set_dim(w)?;
        write!(w, " > {breadcrumb}")?;
        reset_style(w)?;
    }
    write!(w, "\r\n\r\n")?;

    // Heading
    set_bold(w)?;
    write!(w, " {title}\r\n")?;
    reset_style(w)?;
    write!(w, "\r\n")?;

    // Items
    for (i, item) in menu.items.iter().enumerate() {
        let is_cursor = i == menu.cursor;

        write!(w, " ")?;

        if is_cursor {
            set_reverse(w)?;
        }

        // Default marker
        let marker = match item.state {
            ItemState::Default => "*",
            ItemState::Locked => "L",
            ItemState::Error(_) => "!",
            ItemState::Normal => " ",
        };

        write!(w, " {marker} {}", item.label)?;

        if !item.detail.is_empty() {
            if is_cursor {
                reset_style(w)?;
                set_dim(w)?;
            } else {
                set_dim(w)?;
            }
            write!(w, "  {}", item.detail)?;
            reset_style(w)?;
        } else if is_cursor {
            reset_style(w)?;
        }

        write!(w, "\r\n")?;
    }

    // Hint bar
    write!(w, "\r\n")?;
    set_dim(w)?;
    write!(w, " {hint}")?;
    reset_style(w)?;

    w.flush()
}

/// Render the source list screen.
pub fn render_sources(w: &mut impl Write, menu: &Menu) -> io::Result<()> {
    render_menu(
        w,
        "Boot Sources",
        "",
        menu,
        "↑↓ navigate  Enter select  q quit",
    )
}

/// Render the boot tree screen.
pub fn render_boot_tree(
    w: &mut impl Write,
    source_label: &str,
    menu: &Menu,
) -> io::Result<()> {
    render_menu(
        w,
        "Boot Tree",
        source_label,
        menu,
        "↑↓ navigate  Enter select  Esc back  f full filesystem",
    )
}

/// Render the entry list screen.
pub fn render_entries(
    w: &mut impl Write,
    source_label: &str,
    leaf_label: &str,
    menu: &Menu,
) -> io::Result<()> {
    let breadcrumb = format!("{source_label} > {leaf_label}");
    render_menu(
        w,
        "Boot Entries",
        &breadcrumb,
        menu,
        "↑↓ navigate  Enter boot  Esc back",
    )
}

/// Render the passphrase prompt screen.
pub fn render_passphrase(
    w: &mut impl Write,
    source_label: &str,
    input: &str,
    error: Option<&str>,
) -> io::Result<()> {
    clear_screen(w)?;
    move_cursor(w, 1, 1)?;

    // Title bar
    set_bold(w)?;
    write!(w, " {TITLE}")?;
    reset_style(w)?;
    set_dim(w)?;
    write!(w, " > {source_label}")?;
    reset_style(w)?;
    write!(w, "\r\n\r\n")?;

    // Heading
    set_bold(w)?;
    write!(w, " Unlock Encrypted Source\r\n")?;
    reset_style(w)?;
    write!(w, "\r\n")?;

    // Passphrase input with asterisk masking
    write!(w, " Passphrase: ")?;
    for _ in 0..input.len() {
        write!(w, "*")?;
    }
    show_cursor(w)?;
    write!(w, "\r\n")?;

    if let Some(err) = error {
        write!(w, "\r\n")?;
        set_bold(w)?;
        write!(w, " Error: ")?;
        reset_style(w)?;
        write!(w, "{err}\r\n")?;
    }

    // Hint bar
    write!(w, "\r\n")?;
    set_dim(w)?;
    write!(w, " Enter submit  Esc cancel")?;
    reset_style(w)?;

    w.flush()
}

/// Handle a key press on the passphrase prompt screen.
pub fn handle_passphrase_key(input: &mut String, key: &Key) -> Action {
    match key {
        Key::Char(c) => {
            input.push(*c);
            Action::Redraw
        }
        Key::Backspace => {
            input.pop();
            Action::Redraw
        }
        Key::Enter => Action::SubmitPassphrase,
        Key::Escape => Action::Back,
        _ => Action::None,
    }
}

// --- Action ---

/// Result of processing a key in the current screen.
pub enum Action {
    /// Stay on current screen (key was handled, re-render).
    Redraw,
    /// No change needed.
    None,
    /// Navigate to source's boot tree.
    OpenSource(usize),
    /// Navigate to leaf's entries.
    OpenLeaf(usize),
    /// Boot the selected entry.
    Boot { source_idx: usize, entry: Entry },
    /// Go back one screen.
    Back,
    /// Quit the menu.
    Quit,
    /// Navigate to passphrase prompt for an encrypted source.
    UnlockSource(usize),
    /// Submit entered passphrase.
    SubmitPassphrase,
    /// Open full filesystem browser for the current source.
    OpenFileBrowser,
    /// Navigate into a directory in the file browser.
    OpenDir(usize),
    /// Go up one directory in the file browser.
    DirUp,
    /// Boot a file directly from the file browser (kexec a bare kernel).
    BootFile { path: std::path::PathBuf },
}

/// Handle a key press on a source list screen.
pub fn handle_source_key(menu: &mut Menu, key: &Key) -> Action {
    match key {
        Key::Up => { menu.move_up(); Action::Redraw }
        Key::Down => { menu.move_down(); Action::Redraw }
        Key::Enter => {
            let idx = menu.selected_index();
            match menu.items.get(idx).map(|i| &i.state) {
                Some(ItemState::Error(_)) => Action::None,
                Some(ItemState::Locked) => Action::UnlockSource(idx),
                _ => Action::OpenSource(idx),
            }
        }
        Key::Char('q') | Key::Char('Q') => Action::Quit,
        _ => Action::None,
    }
}

/// Handle a key press on a boot tree screen.
pub fn handle_tree_key(menu: &mut Menu, nodes: &[FlatNode], key: &Key) -> Action {
    match key {
        Key::Up => { menu.move_up(); Action::Redraw }
        Key::Down => { menu.move_down(); Action::Redraw }
        Key::Enter => {
            let idx = menu.selected_index();
            if let Some(node) = nodes.get(idx) {
                match &node.kind {
                    FlatNodeKind::Leaf { .. } => Action::OpenLeaf(idx),
                    FlatNodeKind::Dir { .. } => Action::None,
                }
            } else {
                Action::None
            }
        }
        Key::Escape => Action::Back,
        Key::Char('f') | Key::Char('F') => Action::OpenFileBrowser,
        _ => Action::None,
    }
}

/// Handle a key press on an entry list screen.
pub fn handle_entry_key(
    menu: &mut Menu,
    entries: &[Entry],
    source_idx: usize,
    key: &Key,
) -> Action {
    match key {
        Key::Up => { menu.move_up(); Action::Redraw }
        Key::Down => { menu.move_down(); Action::Redraw }
        Key::Enter => {
            if let Some(entry) = entries.get(menu.selected_index()) {
                Action::Boot {
                    source_idx,
                    entry: entry.clone(),
                }
            } else {
                Action::None
            }
        }
        Key::Escape => Action::Back,
        _ => Action::None,
    }
}

// --- Bootable file detection ---

/// Check if a file is a bootable kernel image (PE/EFI stub or bzImage).
///
/// Reads the first 518 bytes and checks:
/// - PE/EFI: starts with "MZ" (0x4D 0x5A)
/// - bzImage: has "HdrS" (0x48 0x64 0x72 0x53) at offset 0x202
pub fn is_bootable_file(path: &std::path::Path) -> bool {
    use std::io::Read as _;
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut buf = [0u8; 518];
    let n = match f.read(&mut buf) {
        Ok(n) => n,
        Err(_) => return false,
    };
    if n >= 2 && buf[0] == 0x4D && buf[1] == 0x5A {
        return true; // PE/EFI binary
    }
    if n >= 0x206 && buf[0x202] == b'H' && buf[0x203] == b'd'
        && buf[0x204] == b'r' && buf[0x205] == b'S'
    {
        return true; // bzImage
    }
    false
}

// --- File browser ---

/// List directory contents and build a menu for the file browser.
pub fn build_file_menu(dir: &std::path::Path) -> io::Result<(Menu, Vec<DirEntry>)> {
    let mut entries = Vec::new();
    let mut read_dir: Vec<_> = std::fs::read_dir(dir)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    read_dir.sort_by(|a, b| a.file_name().cmp(&b.file_name()));

    for de in read_dir {
        let name = de.file_name().to_string_lossy().into_owned();
        let ft = de.file_type()?;
        let is_dir = ft.is_dir();
        let path = de.path();
        let is_bootable = !is_dir && is_bootable_file(&path);
        entries.push(DirEntry {
            name,
            is_dir,
            is_bootable,
            path,
        });
    }

    let items: Vec<MenuItem> = entries
        .iter()
        .map(|e| MenuItem {
            label: if e.is_dir {
                format!("{}/", e.name)
            } else {
                e.name.clone()
            },
            detail: if e.is_bootable { "[bootable]".into() } else { String::new() },
            state: ItemState::Normal,
        })
        .collect();

    Ok((Menu::new(items, None), entries))
}

/// Render the file browser screen.
pub fn render_file_browser(
    w: &mut impl Write,
    source_label: &str,
    current_dir: &std::path::Path,
    root: &std::path::Path,
    menu: &Menu,
) -> io::Result<()> {
    let rel = current_dir
        .strip_prefix(root)
        .unwrap_or(current_dir);
    let path_display = if rel.as_os_str().is_empty() {
        "/".to_string()
    } else {
        format!("/{}", rel.display())
    };
    let breadcrumb = format!("{source_label} [fs]");
    render_menu(
        w,
        &path_display,
        &breadcrumb,
        menu,
        "↑↓ navigate  Enter open  Esc back  b boot tree",
    )
}

/// Handle a key press on the file browser screen.
pub fn handle_file_browser_key(
    menu: &mut Menu,
    dir_entries: &[DirEntry],
    current_dir: &std::path::Path,
    root: &std::path::Path,
    key: &Key,
) -> Action {
    match key {
        Key::Up => { menu.move_up(); Action::Redraw }
        Key::Down => { menu.move_down(); Action::Redraw }
        Key::Enter => {
            let idx = menu.selected_index();
            if let Some(entry) = dir_entries.get(idx) {
                if entry.is_dir {
                    Action::OpenDir(idx)
                } else if entry.is_bootable {
                    Action::BootFile { path: entry.path.clone() }
                } else {
                    Action::None
                }
            } else {
                Action::None
            }
        }
        Key::Escape | Key::Char('b') | Key::Char('B') => {
            // If we're deeper than root, go up one dir; otherwise back to boot tree
            if current_dir != root {
                Action::DirUp
            } else {
                Action::Back
            }
        }
        _ => Action::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use crate::types::Leaf;

    // --- Input parsing tests ---

    #[test]
    fn parse_arrow_up() {
        let mut input: &[u8] = b"\x1b[A";
        assert_eq!(read_key(&mut input).unwrap(), Key::Up);
    }

    #[test]
    fn parse_arrow_down() {
        let mut input: &[u8] = b"\x1b[B";
        assert_eq!(read_key(&mut input).unwrap(), Key::Down);
    }

    #[test]
    fn parse_enter_cr() {
        let mut input: &[u8] = b"\r";
        assert_eq!(read_key(&mut input).unwrap(), Key::Enter);
    }

    #[test]
    fn parse_enter_lf() {
        let mut input: &[u8] = b"\n";
        assert_eq!(read_key(&mut input).unwrap(), Key::Enter);
    }

    #[test]
    fn parse_char() {
        let mut input: &[u8] = b"q";
        assert_eq!(read_key(&mut input).unwrap(), Key::Char('q'));
    }

    #[test]
    fn parse_bare_escape() {
        // Escape followed by EOF -> Escape key
        let mut input: &[u8] = b"\x1b";
        assert_eq!(read_key(&mut input).unwrap(), Key::Escape);
    }

    // --- Menu navigation tests ---

    fn test_menu(n: usize) -> Menu {
        let items: Vec<MenuItem> = (0..n)
            .map(|i| MenuItem {
                label: format!("item{i}"),
                detail: String::new(),
                state: ItemState::Normal,
            })
            .collect();
        Menu::new(items, None)
    }

    #[test]
    fn menu_starts_at_zero() {
        let m = test_menu(3);
        assert_eq!(m.cursor, 0);
    }

    #[test]
    fn menu_starts_at_preselected() {
        let items: Vec<MenuItem> = (0..3)
            .map(|i| MenuItem {
                label: format!("item{i}"),
                detail: String::new(),
                state: ItemState::Normal,
            })
            .collect();
        let m = Menu::new(items, Some(2));
        assert_eq!(m.cursor, 2);
    }

    #[test]
    fn menu_move_down() {
        let mut m = test_menu(3);
        m.move_down();
        assert_eq!(m.cursor, 1);
        m.move_down();
        assert_eq!(m.cursor, 2);
        m.move_down(); // at end, stays
        assert_eq!(m.cursor, 2);
    }

    #[test]
    fn menu_move_up() {
        let mut m = test_menu(3);
        m.move_up(); // at start, stays
        assert_eq!(m.cursor, 0);
        m.cursor = 2;
        m.move_up();
        assert_eq!(m.cursor, 1);
    }

    #[test]
    fn menu_preselected_clamped() {
        let items = vec![MenuItem {
            label: "only".into(),
            detail: String::new(),
            state: ItemState::Normal,
        }];
        let m = Menu::new(items, Some(99));
        assert_eq!(m.cursor, 0);
    }

    // --- Flatten tree tests ---

    fn entry(name: &str) -> crate::types::Entry {
        crate::types::Entry {
            name: name.into(),
            kernel: "vmlinuz".into(),
            initrd: "initrd".into(),
            cmdline: "root=/dev/sda1".into(),
        }
    }

    fn leaf_node(path: &str, names: &[&str]) -> TreeNode {
        TreeNode::Leaf(Leaf {
            path: PathBuf::from(path),
            entries: names.iter().map(|n| entry(n)).collect(),
            mtime: 100,
        })
    }

    #[test]
    fn flatten_simple_tree() {
        let tree = vec![
            TreeNode::Dir {
                name: "nixos".into(),
                children: vec![
                    leaf_node("/boot/nixos/gen1", &["default"]),
                    leaf_node("/boot/nixos/gen2", &["default"]),
                ],
            },
            leaf_node("/boot/other", &["other"]),
        ];
        let flat = flatten_tree(&tree, 0);
        assert_eq!(flat.len(), 4); // dir + 2 leaves + 1 leaf
        assert_eq!(flat[0].depth, 0);
        assert_eq!(flat[1].depth, 1);
        assert_eq!(flat[2].depth, 1);
        assert_eq!(flat[3].depth, 0);
    }

    #[test]
    fn flatten_preserves_names() {
        let tree = vec![leaf_node("/boot/gen1", &["default"])];
        let flat = flatten_tree(&tree, 0);
        match &flat[0].kind {
            FlatNodeKind::Leaf { name, .. } => assert_eq!(name, "gen1"),
            _ => panic!("expected leaf"),
        }
    }

    // --- Render tests (output sanity) ---

    #[test]
    fn render_sources_produces_output() {
        let mut buf = Vec::new();
        let menu = test_menu(2);
        render_sources(&mut buf, &menu).unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("kexec-menu"));
        assert!(output.contains("Boot Sources"));
        assert!(output.contains("item0"));
        assert!(output.contains("item1"));
    }

    // --- Action handling tests ---

    #[test]
    fn source_key_enter_opens() {
        let mut menu = test_menu(2);
        let action = handle_source_key(&mut menu, &Key::Enter);
        assert!(matches!(action, Action::OpenSource(0)));
    }

    #[test]
    fn source_key_enter_locked_unlocks() {
        let items = vec![MenuItem {
            label: "locked".into(),
            detail: String::new(),
            state: ItemState::Locked,
        }];
        let mut menu = Menu::new(items, None);
        let action = handle_source_key(&mut menu, &Key::Enter);
        assert!(matches!(action, Action::UnlockSource(0)));
    }

    #[test]
    fn source_key_q_quits() {
        let mut menu = test_menu(1);
        let action = handle_source_key(&mut menu, &Key::Char('q'));
        assert!(matches!(action, Action::Quit));
    }

    #[test]
    fn tree_key_enter_on_dir_noop() {
        let nodes = vec![FlatNode {
            depth: 0,
            kind: FlatNodeKind::Dir { name: "nixos".into() },
        }];
        let items = vec![MenuItem {
            label: "nixos/".into(),
            detail: String::new(),
            state: ItemState::Normal,
        }];
        let mut menu = Menu::new(items, None);
        let action = handle_tree_key(&mut menu, &nodes, &Key::Enter);
        assert!(matches!(action, Action::None));
    }

    #[test]
    fn tree_key_enter_on_leaf_opens() {
        let nodes = vec![FlatNode {
            depth: 0,
            kind: FlatNodeKind::Leaf {
                name: "gen1".into(),
                entry_count: 1,
                path: PathBuf::from("/boot/gen1"),
            },
        }];
        let items = vec![MenuItem {
            label: "gen1".into(),
            detail: "1 entries".into(),
            state: ItemState::Normal,
        }];
        let mut menu = Menu::new(items, None);
        let action = handle_tree_key(&mut menu, &nodes, &Key::Enter);
        assert!(matches!(action, Action::OpenLeaf(0)));
    }

    #[test]
    fn entry_key_enter_boots() {
        let entries = vec![entry("default")];
        let items = vec![MenuItem {
            label: "default".into(),
            detail: String::new(),
            state: ItemState::Normal,
        }];
        let mut menu = Menu::new(items, None);
        let action = handle_entry_key(&mut menu, &entries, 0, &Key::Enter);
        match action {
            Action::Boot { source_idx, entry: e } => {
                assert_eq!(source_idx, 0);
                assert_eq!(e.name, "default");
            }
            _ => panic!("expected Boot action"),
        }
    }

    #[test]
    fn entry_key_escape_goes_back() {
        let entries = vec![entry("default")];
        let items = vec![MenuItem {
            label: "default".into(),
            detail: String::new(),
            state: ItemState::Normal,
        }];
        let mut menu = Menu::new(items, None);
        let action = handle_entry_key(&mut menu, &entries, 0, &Key::Escape);
        assert!(matches!(action, Action::Back));
    }

    // --- Backspace key parsing ---

    #[test]
    fn parse_backspace_del() {
        let mut input: &[u8] = b"\x7f";
        assert_eq!(read_key(&mut input).unwrap(), Key::Backspace);
    }

    #[test]
    fn parse_backspace_bs() {
        let mut input: &[u8] = b"\x08";
        assert_eq!(read_key(&mut input).unwrap(), Key::Backspace);
    }

    // --- Passphrase input handling tests ---

    #[test]
    fn passphrase_char_appends() {
        let mut input = String::new();
        let action = handle_passphrase_key(&mut input, &Key::Char('a'));
        assert!(matches!(action, Action::Redraw));
        assert_eq!(input, "a");
        handle_passphrase_key(&mut input, &Key::Char('b'));
        assert_eq!(input, "ab");
    }

    #[test]
    fn passphrase_backspace_removes() {
        let mut input = String::from("abc");
        let action = handle_passphrase_key(&mut input, &Key::Backspace);
        assert!(matches!(action, Action::Redraw));
        assert_eq!(input, "ab");
    }

    #[test]
    fn passphrase_backspace_empty_noop() {
        let mut input = String::new();
        handle_passphrase_key(&mut input, &Key::Backspace);
        assert_eq!(input, "");
    }

    #[test]
    fn passphrase_enter_submits() {
        let mut input = String::from("secret");
        let action = handle_passphrase_key(&mut input, &Key::Enter);
        assert!(matches!(action, Action::SubmitPassphrase));
    }

    #[test]
    fn passphrase_escape_cancels() {
        let mut input = String::from("partial");
        let action = handle_passphrase_key(&mut input, &Key::Escape);
        assert!(matches!(action, Action::Back));
    }

    #[test]
    fn render_passphrase_output() {
        let mut buf = Vec::new();
        render_passphrase(&mut buf, "my-disk", "secret", None).unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("kexec-menu"));
        assert!(output.contains("my-disk"));
        assert!(output.contains("Unlock Encrypted Source"));
        assert!(output.contains("******")); // 6 asterisks for "secret"
        assert!(!output.contains("secret")); // passphrase not leaked
    }

    #[test]
    fn render_passphrase_with_error() {
        let mut buf = Vec::new();
        render_passphrase(&mut buf, "disk", "", Some("bad passphrase")).unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("Error:"));
        assert!(output.contains("bad passphrase"));
    }

    // --- File browser tests ---

    fn make_tempdir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "kexec-tui-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn build_file_menu_lists_entries() {
        let tmp = make_tempdir();
        std::fs::create_dir(tmp.join("subdir")).unwrap();
        std::fs::write(tmp.join("file.txt"), "hello").unwrap();

        let (menu, entries) = build_file_menu(&tmp).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(menu.items.len(), 2);

        // Sorted: file.txt before subdir
        assert_eq!(entries[0].name, "file.txt");
        assert!(!entries[0].is_dir);
        assert_eq!(entries[1].name, "subdir");
        assert!(entries[1].is_dir);

        // Dir entry has trailing slash in label
        assert_eq!(menu.items[1].label, "subdir/");
        assert_eq!(menu.items[0].label, "file.txt");
    }

    #[test]
    fn build_file_menu_empty_dir() {
        let tmp = make_tempdir();
        let (menu, entries) = build_file_menu(&tmp).unwrap();
        assert!(entries.is_empty());
        assert!(menu.is_empty());
    }

    #[test]
    fn file_browser_enter_on_dir_opens() {
        let entries = vec![DirEntry {
            name: "subdir".into(),
            is_dir: true,
            is_bootable: false,
            path: PathBuf::from("/mnt/subdir"),
        }];
        let items = vec![MenuItem {
            label: "subdir/".into(),
            detail: String::new(),
            state: ItemState::Normal,
        }];
        let mut menu = Menu::new(items, None);
        let root = PathBuf::from("/mnt");
        let current = PathBuf::from("/mnt");
        let action = handle_file_browser_key(&mut menu, &entries, &current, &root, &Key::Enter);
        assert!(matches!(action, Action::OpenDir(0)));
    }

    #[test]
    fn file_browser_enter_on_file_noop() {
        let entries = vec![DirEntry {
            name: "file.txt".into(),
            is_dir: false,
            is_bootable: false,
            path: PathBuf::from("/mnt/file.txt"),
        }];
        let items = vec![MenuItem {
            label: "file.txt".into(),
            detail: String::new(),
            state: ItemState::Normal,
        }];
        let mut menu = Menu::new(items, None);
        let root = PathBuf::from("/mnt");
        let current = PathBuf::from("/mnt");
        let action = handle_file_browser_key(&mut menu, &entries, &current, &root, &Key::Enter);
        assert!(matches!(action, Action::None));
    }

    #[test]
    fn file_browser_escape_at_root_goes_back() {
        let entries = vec![];
        let mut menu = Menu::new(vec![], None);
        let root = PathBuf::from("/mnt");
        let current = PathBuf::from("/mnt");
        let action = handle_file_browser_key(&mut menu, &entries, &current, &root, &Key::Escape);
        assert!(matches!(action, Action::Back));
    }

    #[test]
    fn file_browser_escape_in_subdir_goes_up() {
        let entries = vec![];
        let mut menu = Menu::new(vec![], None);
        let root = PathBuf::from("/mnt");
        let current = PathBuf::from("/mnt/subdir");
        let action = handle_file_browser_key(&mut menu, &entries, &current, &root, &Key::Escape);
        assert!(matches!(action, Action::DirUp));
    }

    #[test]
    fn file_browser_b_at_root_goes_back() {
        let entries = vec![];
        let mut menu = Menu::new(vec![], None);
        let root = PathBuf::from("/mnt");
        let current = PathBuf::from("/mnt");
        let action = handle_file_browser_key(&mut menu, &entries, &current, &root, &Key::Char('b'));
        assert!(matches!(action, Action::Back));
    }

    #[test]
    fn render_file_browser_output() {
        let mut buf = Vec::new();
        let items = vec![MenuItem {
            label: "subdir/".into(),
            detail: String::new(),
            state: ItemState::Normal,
        }];
        let menu = Menu::new(items, None);
        let root = PathBuf::from("/mnt");
        let current = PathBuf::from("/mnt/boot");
        render_file_browser(&mut buf, "my-disk", &current, &root, &menu).unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("kexec-menu"));
        assert!(output.contains("my-disk [fs]"));
        assert!(output.contains("/boot"));
        assert!(output.contains("subdir/"));
        assert!(output.contains("boot tree"));
    }

    #[test]
    fn render_file_browser_at_root() {
        let mut buf = Vec::new();
        let menu = Menu::new(vec![], None);
        let root = PathBuf::from("/mnt");
        render_file_browser(&mut buf, "disk", &root, &root, &menu).unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("/"));
    }

    // --- Bootable file detection tests ---

    #[test]
    fn detect_pe_binary() {
        let tmp = make_tempdir();
        let path = tmp.join("test.efi");
        let mut data = vec![0u8; 64];
        data[0] = 0x4D; // 'M'
        data[1] = 0x5A; // 'Z'
        std::fs::write(&path, &data).unwrap();
        assert!(is_bootable_file(&path));
    }

    #[test]
    fn detect_bzimage() {
        let tmp = make_tempdir();
        let path = tmp.join("vmlinuz");
        let mut data = vec![0u8; 0x206];
        data[0x202] = b'H';
        data[0x203] = b'd';
        data[0x204] = b'r';
        data[0x205] = b'S';
        std::fs::write(&path, &data).unwrap();
        assert!(is_bootable_file(&path));
    }

    #[test]
    fn detect_non_bootable() {
        let tmp = make_tempdir();
        let path = tmp.join("readme.txt");
        std::fs::write(&path, "hello world").unwrap();
        assert!(!is_bootable_file(&path));
    }

    #[test]
    fn detect_empty_file() {
        let tmp = make_tempdir();
        let path = tmp.join("empty");
        std::fs::write(&path, b"").unwrap();
        assert!(!is_bootable_file(&path));
    }

    #[test]
    fn detect_too_small_for_bzimage() {
        let tmp = make_tempdir();
        let path = tmp.join("small");
        std::fs::write(&path, &[0u8; 100]).unwrap();
        assert!(!is_bootable_file(&path));
    }

    #[test]
    fn detect_nonexistent_file() {
        assert!(!is_bootable_file(std::path::Path::new("/nonexistent/path")));
    }

    #[test]
    fn file_browser_enter_on_bootable_boots() {
        let entries = vec![DirEntry {
            name: "vmlinuz".into(),
            is_dir: false,
            is_bootable: true,
            path: PathBuf::from("/mnt/boot/vmlinuz"),
        }];
        let items = vec![MenuItem {
            label: "vmlinuz".into(),
            detail: "[bootable]".into(),
            state: ItemState::Normal,
        }];
        let mut menu = Menu::new(items, None);
        let root = PathBuf::from("/mnt");
        let current = PathBuf::from("/mnt/boot");
        let action = handle_file_browser_key(&mut menu, &entries, &current, &root, &Key::Enter);
        match action {
            Action::BootFile { path } => assert_eq!(path, PathBuf::from("/mnt/boot/vmlinuz")),
            _ => panic!("expected BootFile action"),
        }
    }

    #[test]
    fn build_file_menu_marks_bootable() {
        let tmp = make_tempdir();
        // Create a PE file
        let mut pe_data = vec![0u8; 64];
        pe_data[0] = 0x4D;
        pe_data[1] = 0x5A;
        std::fs::write(tmp.join("kernel.efi"), &pe_data).unwrap();
        // Create a normal file
        std::fs::write(tmp.join("readme.txt"), "hello").unwrap();

        let (_menu, entries) = build_file_menu(&tmp).unwrap();
        let efi = entries.iter().find(|e| e.name == "kernel.efi").unwrap();
        let txt = entries.iter().find(|e| e.name == "readme.txt").unwrap();
        assert!(efi.is_bootable);
        assert!(!txt.is_bootable);
    }

    #[test]
    fn tree_key_f_opens_file_browser() {
        let nodes = vec![FlatNode {
            depth: 0,
            kind: FlatNodeKind::Leaf {
                name: "gen1".into(),
                entry_count: 1,
                path: PathBuf::from("/boot/gen1"),
            },
        }];
        let items = vec![MenuItem {
            label: "gen1".into(),
            detail: "1 entries".into(),
            state: ItemState::Normal,
        }];
        let mut menu = Menu::new(items, None);
        let action = handle_tree_key(&mut menu, &nodes, &Key::Char('f'));
        assert!(matches!(action, Action::OpenFileBrowser));
    }

    // --- Arrow key parsing tests ---

    #[test]
    fn parse_arrow_left() {
        let mut input: &[u8] = b"\x1b[D";
        assert_eq!(read_key(&mut input).unwrap(), Key::Left);
    }

    #[test]
    fn parse_arrow_right() {
        let mut input: &[u8] = b"\x1b[C";
        assert_eq!(read_key(&mut input).unwrap(), Key::Right);
    }

    // --- TreeView tests ---

    fn test_sources() -> Vec<Source> {
        vec![Source {
            label: "nvme0n1p2 (bcachefs)".into(),
            device: PathBuf::from("/dev/nvme0n1p2"),
            state: SourceState::Mounted,
            mount_point: Some(PathBuf::from("/mnt/boot")),
        }]
    }

    fn test_tree() -> Vec<(String, Vec<TreeNode>)> {
        vec![("nvme0n1p2".into(), vec![
            TreeNode::Dir {
                name: "nixos".into(),
                children: vec![
                    leaf_node("/boot/nixos/gen2", &["NixOS default", "NixOS fallback"]),
                    leaf_node("/boot/nixos/gen1", &["NixOS default"]),
                ],
            },
        ])]
    }

    fn test_default() -> BootSelection {
        BootSelection {
            leaf_path: PathBuf::from("/boot/nixos/gen2"),
            entry_name: "NixOS default".into(),
        }
    }

    #[test]
    fn tree_view_build_basic() {
        let sources = test_sources();
        let trees = test_tree();
        let default = test_default();
        let view = TreeView::build(&sources, &trees, Some(&default));

        // Source + Dir(nixos) + Leaf(gen2) + 2 entries + Leaf(gen1) + 1 entry = 7
        assert_eq!(view.nodes.len(), 7);
        assert!(matches!(view.nodes[0].kind, NodeKind::Source { .. }));
        assert!(matches!(view.nodes[1].kind, NodeKind::Dir { .. }));
        assert!(matches!(view.nodes[2].kind, NodeKind::Leaf { .. }));
        assert!(matches!(view.nodes[3].kind, NodeKind::Entry { .. }));
        assert!(matches!(view.nodes[4].kind, NodeKind::Entry { .. }));
        assert!(matches!(view.nodes[5].kind, NodeKind::Leaf { .. }));
        assert!(matches!(view.nodes[6].kind, NodeKind::Entry { .. }));
    }

    #[test]
    fn tree_view_default_expanded() {
        let sources = test_sources();
        let trees = test_tree();
        let default = test_default();
        let view = TreeView::build(&sources, &trees, Some(&default));

        // Source expanded (contains default)
        assert!(view.nodes[0].expanded);
        // Dir "nixos" expanded (contains default leaf)
        assert!(view.nodes[1].expanded);
        // Leaf gen2 expanded (is default leaf)
        assert!(view.nodes[2].expanded);
        // Leaf gen1 collapsed (not default)
        assert!(!view.nodes[5].expanded);
    }

    #[test]
    fn tree_view_cursor_on_default_entry() {
        let sources = test_sources();
        let trees = test_tree();
        let default = test_default();
        let view = TreeView::build(&sources, &trees, Some(&default));

        // Cursor should be on the default entry (index 3: "NixOS default" under gen2)
        assert_eq!(view.cursor, 3);
        assert!(view.nodes[3].is_default);
    }

    #[test]
    fn tree_view_visibility() {
        let sources = test_sources();
        let trees = test_tree();
        let default = test_default();
        let view = TreeView::build(&sources, &trees, Some(&default));

        // Source, Dir, Leaf(gen2), Entry, Entry all visible (default path expanded)
        assert!(view.nodes[0].visible); // source
        assert!(view.nodes[1].visible); // nixos/
        assert!(view.nodes[2].visible); // gen2
        assert!(view.nodes[3].visible); // entry default
        assert!(view.nodes[4].visible); // entry fallback
        // gen1 visible (sibling of gen2, parent is expanded)
        assert!(view.nodes[5].visible);
        // gen1's entry hidden (gen1 is collapsed)
        assert!(!view.nodes[6].visible);
    }

    #[test]
    fn tree_view_move_down_skips_invisible() {
        let sources = test_sources();
        let trees = test_tree();
        let default = test_default();
        let mut view = TreeView::build(&sources, &trees, Some(&default));

        // Cursor at index 3 (default entry)
        assert_eq!(view.cursor, 3);
        view.move_down(); // -> index 4 (fallback entry)
        assert_eq!(view.cursor, 4);
        view.move_down(); // -> index 5 (gen1, visible), skipping nothing
        assert_eq!(view.cursor, 5);
        view.move_down(); // -> should NOT move (gen1's entry is invisible, nothing after)
        assert_eq!(view.cursor, 5);
    }

    #[test]
    fn tree_view_move_up() {
        let sources = test_sources();
        let trees = test_tree();
        let default = test_default();
        let mut view = TreeView::build(&sources, &trees, Some(&default));

        // Start at cursor 3
        view.move_up(); // -> 2 (gen2 leaf)
        assert_eq!(view.cursor, 2);
        view.move_up(); // -> 1 (nixos dir)
        assert_eq!(view.cursor, 1);
        view.move_up(); // -> 0 (source)
        assert_eq!(view.cursor, 0);
        view.move_up(); // stays at 0
        assert_eq!(view.cursor, 0);
    }

    #[test]
    fn tree_view_toggle_collapse() {
        let sources = test_sources();
        let trees = test_tree();
        let default = test_default();
        let mut view = TreeView::build(&sources, &trees, Some(&default));

        // Move cursor to nixos dir (index 1), collapse it
        view.cursor = 1;
        assert!(view.nodes[1].expanded);
        let changed = view.toggle();
        assert!(changed);
        assert!(!view.nodes[1].expanded);

        // Children should now be invisible
        assert!(!view.nodes[2].visible); // gen2
        assert!(!view.nodes[3].visible); // entry
        assert!(!view.nodes[4].visible); // entry
        assert!(!view.nodes[5].visible); // gen1
        assert!(!view.nodes[6].visible); // entry
    }

    #[test]
    fn tree_view_toggle_expand() {
        let sources = test_sources();
        let trees = test_tree();
        let default = test_default();
        let mut view = TreeView::build(&sources, &trees, Some(&default));

        // gen1 is collapsed, move cursor there and expand
        view.cursor = 5;
        assert!(!view.nodes[5].expanded);
        view.toggle();
        assert!(view.nodes[5].expanded);
        // gen1's entry should now be visible
        assert!(view.nodes[6].visible);
    }

    #[test]
    fn tree_view_collapse_moves_cursor_to_parent() {
        let sources = test_sources();
        let trees = test_tree();
        let default = test_default();
        let mut view = TreeView::build(&sources, &trees, Some(&default));

        // Cursor on default entry (3), collapse should go to parent leaf (2)
        assert_eq!(view.cursor, 3);
        let changed = view.collapse();
        assert!(changed);
        assert_eq!(view.cursor, 2);
    }

    #[test]
    fn tree_view_collapse_already_collapsed_goes_to_parent() {
        let sources = test_sources();
        let trees = test_tree();
        let default = test_default();
        let mut view = TreeView::build(&sources, &trees, Some(&default));

        // gen1 is collapsed, cursor there, collapse should go to parent dir
        view.cursor = 5;
        assert!(!view.nodes[5].expanded);
        let changed = view.collapse();
        assert!(changed);
        assert_eq!(view.cursor, 1); // nixos dir
    }

    #[test]
    fn tree_view_expand_already_expanded_noop() {
        let sources = test_sources();
        let trees = test_tree();
        let default = test_default();
        let mut view = TreeView::build(&sources, &trees, Some(&default));

        view.cursor = 1; // nixos dir, already expanded
        let changed = view.expand();
        assert!(!changed);
    }

    #[test]
    fn tree_view_toggle_entry_noop() {
        let sources = test_sources();
        let trees = test_tree();
        let default = test_default();
        let mut view = TreeView::build(&sources, &trees, Some(&default));

        // Entries can't be toggled
        view.cursor = 3;
        let changed = view.toggle();
        assert!(!changed);
    }

    #[test]
    fn tree_view_no_default() {
        let sources = test_sources();
        let trees = test_tree();
        let view = TreeView::build(&sources, &trees, None);

        // Without default, source should be collapsed
        assert!(!view.nodes[0].expanded);
        // Cursor at 0
        assert_eq!(view.cursor, 0);
        // Only source visible
        assert_eq!(view.visible_count(), 1);
    }

    #[test]
    fn tree_view_encrypted_source() {
        let sources = vec![Source {
            label: "sda1 (luks)".into(),
            device: PathBuf::from("/dev/sda1"),
            state: SourceState::Encrypted,
            mount_point: None,
        }];
        let trees = vec![("sda1".into(), Vec::new())];
        let view = TreeView::build(&sources, &trees, None);

        assert_eq!(view.nodes.len(), 1);
        assert!(matches!(
            &view.nodes[0].kind,
            NodeKind::Source { state: NodeSourceState::Encrypted, .. }
        ));
        // Can't toggle encrypted source
        let mut view = view;
        let changed = view.toggle();
        assert!(!changed);
    }

    #[test]
    fn tree_view_multiple_sources() {
        let sources = vec![
            Source {
                label: "nvme0n1p2".into(),
                device: PathBuf::from("/dev/nvme0n1p2"),
                state: SourceState::Mounted,
                mount_point: Some(PathBuf::from("/mnt/a")),
            },
            Source {
                label: "sda1".into(),
                device: PathBuf::from("/dev/sda1"),
                state: SourceState::Mounted,
                mount_point: Some(PathBuf::from("/mnt/b")),
            },
        ];
        let trees = vec![
            ("nvme0n1p2".into(), vec![leaf_node("/boot/a/gen1", &["entry1"])]),
            ("sda1".into(), vec![leaf_node("/boot/b/gen1", &["entry2"])]),
        ];
        let default = BootSelection {
            leaf_path: PathBuf::from("/boot/b/gen1"),
            entry_name: "entry2".into(),
        };
        let view = TreeView::build(&sources, &trees, Some(&default));

        // First source collapsed, second expanded
        assert!(!view.nodes[0].expanded);
        // Find second source
        let src2_idx = view.nodes.iter().position(|n| {
            matches!(&n.kind, NodeKind::Source { idx: 1, .. })
        }).unwrap();
        assert!(view.nodes[src2_idx].expanded);
    }

    #[test]
    fn tree_view_cursor_source_idx() {
        let sources = test_sources();
        let trees = test_tree();
        let default = test_default();
        let view = TreeView::build(&sources, &trees, Some(&default));

        // Cursor on entry under source 0
        assert_eq!(view.cursor_source_idx(), Some(0));
    }

    #[test]
    fn tree_view_collapse_source_hides_all() {
        let sources = test_sources();
        let trees = test_tree();
        let default = test_default();
        let mut view = TreeView::build(&sources, &trees, Some(&default));

        // Collapse the source
        view.cursor = 0;
        view.toggle();
        assert_eq!(view.visible_count(), 1); // only the source itself
    }

    #[test]
    fn tree_view_empty_source_not_expandable() {
        let sources = vec![Source {
            label: "empty-disk".into(),
            device: PathBuf::from("/dev/sdc1"),
            state: SourceState::Mounted,
            mount_point: Some(PathBuf::from("/mnt/empty")),
        }];
        let trees = vec![("empty-disk".into(), Vec::new())];
        let view = TreeView::build(&sources, &trees, None);

        assert_eq!(view.nodes.len(), 1);
        assert!(matches!(
            &view.nodes[0].kind,
            NodeKind::Source { state: NodeSourceState::Empty, .. }
        ));
        let mut view = view;
        let changed = view.toggle();
        assert!(!changed);
    }

    #[test]
    fn tree_view_left_at_top_level_noop() {
        let sources = test_sources();
        let trees = test_tree();
        let mut view = TreeView::build(&sources, &trees, None);

        // Source at depth 0, collapsed — left should do nothing
        view.cursor = 0;
        let changed = view.collapse();
        assert!(!changed);
    }
}
