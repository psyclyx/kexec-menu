use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::process;

use kexec_menu_core::kexec;
use kexec_menu_core::mount;
use kexec_menu_core::select;
use kexec_menu_core::tree;
use kexec_menu_core::tui;
use kexec_menu_core::types::{BootSelection, Source, SourceState, TreeNode};

fn main() {
    let dry_run = std::env::args().any(|a| a == "--dry-run");

    if let Err(e) = run(dry_run) {
        eprintln!("kexec-menu: {e}");
        process::exit(1);
    }
}

fn run(dry_run: bool) -> Result<(), Box<dyn std::fmt::Display>> {
    if dry_run {
        eprintln!("kexec-menu: dry-run mode");
    }

    // Discover filesystem sources
    let sources = mount::discover_sources().map_err(boxed)?;
    if sources.is_empty() {
        eprintln!("kexec-menu: no boot sources found");
        process::exit(1);
    }

    // Build boot trees for each source (1:1 with sources vec)
    let mut trees: Vec<(String, Vec<TreeNode>)> = Vec::new();
    for src in &sources {
        match &src.state {
            SourceState::Mounted => {
                if let Some(mp) = &src.mount_point {
                    let boot_dir = mp.join("boot");
                    if boot_dir.is_dir() {
                        match tree::walk_boot_tree(&boot_dir) {
                            Ok(nodes) => trees.push((src.label.clone(), nodes)),
                            Err(_) => trees.push((src.label.clone(), Vec::new())),
                        }
                    } else {
                        trees.push((src.label.clone(), Vec::new()));
                    }
                } else {
                    trees.push((src.label.clone(), Vec::new()));
                }
            }
            _ => trees.push((src.label.clone(), Vec::new())),
        }
    }

    // Resolve default boot selection
    let efi_sel = kexec::read_efi_selection();
    let default = select::resolve_default(&trees, efi_sel.as_ref());

    // Enter TUI
    let stdin = io::stdin();
    let stdout = io::stdout();
    let result = run_tui(stdin.lock(), stdout.lock(), &sources, &trees, default.as_ref());

    match result {
        Ok(TuiResult::Quit) => {
            eprintln!("kexec-menu: user quit");
            Ok(())
        }
        Ok(TuiResult::Boot { leaf_path, entry }) => {
            if dry_run {
                eprintln!("kexec-menu: would boot:");
                eprintln!("  leaf:    {}", leaf_path.display());
                eprintln!("  kernel:  {}", entry.kernel);
                eprintln!("  initrd:  {}", entry.initrd);
                eprintln!("  cmdline: {}", entry.cmdline);
                eprintln!("  entry:   {}", entry.name);
                Ok(())
            } else {
                kexec::boot_entry(
                    &leaf_path,
                    &entry.kernel,
                    &entry.initrd,
                    &entry.cmdline,
                    &entry.name,
                    None,
                )
                .map_err(boxed)?;
                Ok(())
            }
        }
        Err(e) => Err(boxed(e)),
    }
}

fn boxed(e: impl std::fmt::Display + 'static) -> Box<dyn std::fmt::Display> {
    Box::new(e)
}

enum TuiResult {
    Quit,
    Boot {
        leaf_path: PathBuf,
        entry: kexec_menu_core::types::Entry,
    },
}

fn run_tui(
    mut input: impl Read,
    mut output: impl Write,
    sources: &[Source],
    trees: &[(String, Vec<TreeNode>)],
    default: Option<&BootSelection>,
) -> io::Result<TuiResult> {
    let _raw = RawMode::enter()?;
    tui::hide_cursor(&mut output)?;

    let mut screen = tui::Screen::Sources(tui::build_source_menu(sources, default, trees));

    loop {
        match &screen {
            tui::Screen::Sources(menu) => {
                tui::render_sources(&mut output, menu)?;
            }
            tui::Screen::BootTree { source_label, menu, .. } => {
                tui::render_boot_tree(&mut output, source_label, menu)?;
            }
            tui::Screen::Entries { source_label, leaf_label, menu, .. } => {
                tui::render_entries(&mut output, source_label, leaf_label, menu)?;
            }
        }

        let key = tui::read_key(&mut input)?;

        match &mut screen {
            tui::Screen::Sources(menu) => {
                match tui::handle_source_key(menu, &key) {
                    tui::Action::Quit => {
                        cleanup(&mut output)?;
                        return Ok(TuiResult::Quit);
                    }
                    tui::Action::OpenSource(idx) => {
                        let tree = trees.get(idx).map(|(_, t)| t.as_slice()).unwrap_or(&[]);
                        let label = trees.get(idx).map(|(l, _)| l.as_str()).unwrap_or("");
                        let (menu, nodes) = tui::build_tree_menu(tree, default);
                        screen = tui::Screen::BootTree {
                            source_idx: idx,
                            source_label: label.to_string(),
                            menu,
                            nodes,
                        };
                    }
                    tui::Action::Redraw => {}
                    _ => {}
                }
            }
            tui::Screen::BootTree { source_idx, source_label, menu, nodes } => {
                match tui::handle_tree_key(menu, nodes, &key) {
                    tui::Action::Back => {
                        screen = tui::Screen::Sources(
                            tui::build_source_menu(sources, default, trees),
                        );
                    }
                    tui::Action::OpenLeaf(flat_idx) => {
                        if let Some(node) = nodes.get(flat_idx) {
                            if let tui::FlatNodeKind::Leaf { name, path, .. } = &node.kind {
                                if let Some(leaf) = find_leaf(trees, path) {
                                    let entry_menu = tui::build_entry_menu(
                                        &leaf.entries, default, &leaf.path,
                                    );
                                    screen = tui::Screen::Entries {
                                        source_idx: *source_idx,
                                        source_label: source_label.clone(),
                                        leaf_label: name.clone(),
                                        leaf_path: leaf.path.clone(),
                                        menu: entry_menu,
                                        entries: leaf.entries.clone(),
                                    };
                                }
                            }
                        }
                    }
                    tui::Action::Redraw => {}
                    _ => {}
                }
            }
            tui::Screen::Entries { source_idx, source_label, leaf_path, menu, entries, .. } => {
                match tui::handle_entry_key(menu, entries, *source_idx, &key) {
                    tui::Action::Back => {
                        let si = *source_idx;
                        let tree = trees.get(si).map(|(_, t)| t.as_slice()).unwrap_or(&[]);
                        let label = source_label.clone();
                        let (menu, nodes) = tui::build_tree_menu(tree, default);
                        screen = tui::Screen::BootTree {
                            source_idx: si,
                            source_label: label,
                            menu,
                            nodes,
                        };
                    }
                    tui::Action::Boot { entry, .. } => {
                        cleanup(&mut output)?;
                        return Ok(TuiResult::Boot {
                            leaf_path: leaf_path.clone(),
                            entry,
                        });
                    }
                    tui::Action::Redraw => {}
                    _ => {}
                }
            }
        }
    }
}

fn cleanup(output: &mut impl Write) -> io::Result<()> {
    tui::show_cursor(output)?;
    tui::clear_screen(output)?;
    output.flush()
}

fn find_leaf<'a>(
    trees: &'a [(String, Vec<TreeNode>)],
    path: &std::path::Path,
) -> Option<&'a kexec_menu_core::types::Leaf> {
    for (_, nodes) in trees {
        if let Some(leaf) = find_leaf_in_nodes(nodes, path) {
            return Some(leaf);
        }
    }
    None
}

fn find_leaf_in_nodes<'a>(
    nodes: &'a [TreeNode],
    path: &std::path::Path,
) -> Option<&'a kexec_menu_core::types::Leaf> {
    for node in nodes {
        match node {
            TreeNode::Leaf(leaf) if leaf.path == path => return Some(leaf),
            TreeNode::Dir { children, .. } => {
                if let Some(leaf) = find_leaf_in_nodes(children, path) {
                    return Some(leaf);
                }
            }
            _ => {}
        }
    }
    None
}

// --- Terminal raw mode ---

struct RawMode {
    original: libc::termios,
}

impl RawMode {
    fn enter() -> io::Result<Self> {
        let fd = io::stdin().as_raw_fd();
        let mut original: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(fd, &mut original) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut raw = original;
        raw.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG);
        raw.c_iflag &= !(libc::IXON | libc::ICRNL);
        raw.c_cc[libc::VMIN] = 1;
        raw.c_cc[libc::VTIME] = 0;
        if unsafe { libc::tcsetattr(fd, libc::TCSAFLUSH, &raw) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { original })
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let fd = io::stdin().as_raw_fd();
        unsafe { libc::tcsetattr(fd, libc::TCSAFLUSH, &self.original) };
    }
}
