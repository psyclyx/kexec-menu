use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::path::PathBuf;
use std::process;

use kexec_menu_core::evdev;
use kexec_menu_core::kexec;
use kexec_menu_core::mount;
use kexec_menu_core::select;
use kexec_menu_core::tree;
use kexec_menu_core::tui;
use kexec_menu_core::types::{BootSelection, Source, SourceState, TreeNode};

fn main() {
    if std::env::args().any(|a| a == "--help" || a == "-h") {
        print_help();
        return;
    }

    let dry_run = std::env::args().any(|a| a == "--dry-run");
    let auto_default = std::env::args().any(|a| a == "--auto-default");

    if let Err(e) = run(dry_run, auto_default) {
        eprintln!("kexec-menu: {e}");
        process::exit(1);
    }
}

fn print_help() {
    let version = env!("CARGO_PKG_VERSION");
    eprint!("\
kexec-menu {version} — filesystem-agnostic kexec boot menu

USAGE: kexec-menu [OPTIONS]

OPTIONS:
  --dry-run        Print what would be booted, don't kexec
  --auto-default   Boot the default entry without showing the menu
  -h, --help       Show this help

ENVIRONMENT:
  KEXEC_MENU_TIMEOUT          Autoboot timeout in seconds (0=fast, 65535=none)
  KEXEC_MENU_TIMEOUT_DEFAULT  Build-time default timeout (default: 5)

COMPILE-TIME FEATURES:");
    #[cfg(feature = "full-fs-view")]
    eprint!("\n  full-fs-view     Browse mounted filesystems (keybind: f)");
    #[cfg(feature = "rescue-shell")]
    eprint!("\n  rescue-shell     Drop to /bin/sh on failure or keybind (s)");
    #[cfg(feature = "disk-whitelist")]
    eprint!("\n  disk-whitelist   Filter disks via KEXEC_MENU_DISK_WHITELIST");
    eprintln!();
}

fn run(dry_run: bool, auto_default: bool) -> Result<(), Box<dyn std::fmt::Display>> {
    if dry_run {
        eprintln!("kexec-menu: dry-run mode");
    }

    // Discover filesystem sources
    let mut sources = mount::discover_sources().map_err(boxed)?;

    // Build boot trees for each source (1:1 with sources vec)
    let mut trees: Vec<(String, Vec<TreeNode>)> = Vec::new();
    for src in &sources {
        build_source_tree(src, &mut trees);
    }

    // Append static build-time entries
    let static_path = std::path::Path::new(tree::STATIC_ENTRIES_PATH);
    match tree::load_static_entries(static_path) {
        Ok(statics) => {
            for (src, label, tree_nodes) in statics {
                sources.push(src);
                trees.push((label, tree_nodes));
            }
        }
        Err(e) => {
            eprintln!("kexec-menu: warning: static entries: {e}");
        }
    }

    if sources.is_empty() {
        return Err(boxed("no boot sources found"));
    }

    // Resolve default boot selection
    let efi_sel = kexec::read_efi_selection();
    let default = select::resolve_default(&trees, efi_sel.as_ref());

    // --auto-default: skip TUI, boot the default entry directly
    if auto_default {
        return run_auto_default(dry_run, &trees, default.as_ref());
    }

    // Resolve autoboot timeout
    let timeout = resolve_timeout();

    // Enter TUI — multiplex stdin with evdev (gpio-keys, volume buttons)
    let stdin = io::stdin();
    let stdout = io::stdout();
    let input = InputMux::new(stdin.lock().as_raw_fd(), evdev::EvdevReader::open());
    let result = run_tui(input, stdout.lock(), &mut sources, &mut trees, default.as_ref(), timeout);

    match result {
        Ok(TuiResult::Quit) => {
            eprintln!("kexec-menu: user quit");
            Ok(())
        }
        Ok(TuiResult::Boot { leaf_path, entry, source_idx }) => {
            let key_data = source_key_data(&sources, source_idx);
            if dry_run {
                eprintln!("kexec-menu: would boot:");
                eprintln!("  leaf:    {}", leaf_path.display());
                eprintln!("  kernel:  {}", entry.kernel);
                eprintln!("  initrd:  {}", entry.initrd);
                eprintln!("  cmdline: {}", entry.cmdline);
                eprintln!("  entry:   {}", entry.name);
                if key_data.is_some() {
                    eprintln!("  key:     (handoff enabled)");
                }
                Ok(())
            } else {
                let result = kexec::boot_entry(
                    &leaf_path,
                    &entry.kernel,
                    &entry.initrd,
                    &entry.cmdline,
                    &entry.name,
                    key_data.as_ref().map(|(pw, uuid)| (pw.as_bytes(), uuid.as_str())),
                );
                #[cfg(feature = "rescue-shell")]
                if let Err(e) = result {
                    eprintln!("kexec-menu: kexec failed: {e}");
                    return drop_to_shell();
                }
                #[cfg(not(feature = "rescue-shell"))]
                result.map_err(boxed)?;
                Ok(())
            }
        }
        #[cfg(feature = "full-fs-view")]
        Ok(TuiResult::BootFile { path }) => {
            if dry_run {
                eprintln!("kexec-menu: would boot file:");
                eprintln!("  path: {}", path.display());
                Ok(())
            } else {
                let result = kexec::boot_file(&path);
                #[cfg(feature = "rescue-shell")]
                if let Err(e) = result {
                    eprintln!("kexec-menu: kexec failed: {e}");
                    return drop_to_shell();
                }
                #[cfg(not(feature = "rescue-shell"))]
                result.map_err(boxed)?;
                Ok(())
            }
        }
        #[cfg(feature = "rescue-shell")]
        Ok(TuiResult::Shell) => {
            if dry_run {
                eprintln!("kexec-menu: would drop to rescue shell");
                Ok(())
            } else {
                drop_to_shell()
            }
        }
        Err(e) => Err(boxed(e)),
    }
}

fn run_auto_default(
    dry_run: bool,
    trees: &[(String, Vec<TreeNode>)],
    default: Option<&BootSelection>,
) -> Result<(), Box<dyn std::fmt::Display>> {
    let sel = default.ok_or_else(|| boxed("no default entry found"))?;
    let leaf = find_leaf(trees, &sel.leaf_path)
        .ok_or_else(|| boxed("default leaf not found in tree"))?;
    let entry = leaf
        .entries
        .iter()
        .find(|e| e.name == sel.entry_name)
        .ok_or_else(|| boxed("default entry not found in leaf"))?;

    if dry_run {
        eprintln!("kexec-menu: would boot:");
        eprintln!("  leaf:    {}", sel.leaf_path.display());
        eprintln!("  kernel:  {}", entry.kernel);
        eprintln!("  initrd:  {}", entry.initrd);
        eprintln!("  cmdline: {}", entry.cmdline);
        eprintln!("  entry:   {}", entry.name);
        Ok(())
    } else {
        kexec::boot_entry(
            &sel.leaf_path,
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

fn boxed(e: impl std::fmt::Display + 'static) -> Box<dyn std::fmt::Display> {
    Box::new(e)
}

/// Exec into /bin/sh for rescue shell access.
/// On success this does not return. On failure (shell not found), returns an error.
#[cfg(feature = "rescue-shell")]
fn drop_to_shell() -> Result<(), Box<dyn std::fmt::Display>> {
    use std::ffi::CString;

    eprintln!("kexec-menu: dropping to rescue shell");
    let shell = CString::new("/bin/sh").unwrap();
    let argv = [shell.as_ptr(), std::ptr::null()];
    unsafe { libc::execv(shell.as_ptr(), argv.as_ptr()) };
    Err(boxed(io::Error::last_os_error()))
}

/// Extract passphrase and device UUID for key handoff, if the source was unlocked.
fn source_key_data(sources: &[Source], idx: usize) -> Option<(String, String)> {
    let src = sources.get(idx)?;
    let pw = src.passphrase.as_ref()?;
    let dev_name = src.device.file_name()?.to_str()?;
    let uuid = mount::read_device_uuid(dev_name)?;
    Some((pw.clone(), uuid))
}

enum TuiResult {
    Quit,
    Boot {
        leaf_path: PathBuf,
        entry: kexec_menu_core::types::Entry,
        source_idx: usize,
    },
    #[cfg(feature = "full-fs-view")]
    BootFile {
        path: PathBuf,
    },
    #[cfg(feature = "rescue-shell")]
    Shell,
}

enum SideScreen {
    Passphrase {
        source_idx: usize,
        source_label: String,
        input: String,
        error: Option<String>,
    },
    #[cfg(feature = "full-fs-view")]
    FileBrowser {
        source_label: String,
        current_dir: PathBuf,
        root: PathBuf,
        menu: tui::Menu,
        dir_entries: Vec<tui::DirEntry>,
    },
}

/// Resolve autoboot timeout from config layers (EFI var > env > build-time default).
/// Returns None for "no timeout" (infinite wait), Some(secs) otherwise.
fn resolve_timeout() -> Option<u16> {
    // EFI variable takes priority
    if let Some(t) = kexec::read_efi_timeout() {
        return if t == u16::MAX { None } else { Some(t) };
    }
    // Runtime environment variable
    if let Ok(v) = std::env::var("KEXEC_MENU_TIMEOUT") {
        if let Ok(t) = v.parse::<u16>() {
            return if t == u16::MAX { None } else { Some(t) };
        }
    }
    // Build-time default
    let default: u16 = option_env!("KEXEC_MENU_TIMEOUT_DEFAULT")
        .and_then(|v| v.parse().ok())
        .unwrap_or(5);
    if default == u16::MAX { None } else { Some(default) }
}

fn run_tui(
    mut input: InputMux,
    mut output: impl Write,
    sources: &mut Vec<Source>,
    trees: &mut Vec<(String, Vec<TreeNode>)>,
    default: Option<&BootSelection>,
    timeout: Option<u16>,
) -> io::Result<TuiResult> {
    let _raw = RawMode::enter()?;
    tui::hide_cursor(&mut output)?;

    // Apply compile-time theme if set
    if let Some(theme) = tui::Theme::from_env() {
        theme.apply(&mut output)?;
    }

    let mut view = tui::TreeView::build(sources, trees, default);
    let mut side: Option<SideScreen> = None;

    // Autoboot countdown: if timeout is set and a default entry exists,
    // show the menu with a countdown. Any key cancels to normal mode.
    if let Some(secs) = timeout {
        if default.is_some() && !default_is_locked(&view) {
            let poll_ms = if secs == 0 { 100 } else { 1000 };
            let iterations = if secs == 0 { 1u16 } else { secs };

            for remaining in (0..=iterations).rev() {
                tui::render_tree_view(&mut output, &view)?;
                let display_secs = if secs == 0 { 0 } else { remaining };
                tui::render_countdown(&mut output, display_secs)?;

                match input.poll(poll_ms) {
                    Ok(true) => break, // key pressed, cancel countdown
                    Ok(false) if remaining == 0 => {
                        // Timeout expired — boot default
                        cleanup(&mut output)?;
                        if let Some(result) = boot_default(&view) {
                            return Ok(result);
                        }
                        break; // fallback to normal loop if default can't be found
                    }
                    Ok(false) => continue,
                    Err(_) => break,
                }
            }
        }
    }

    loop {
        // Render
        match &side {
            None => {
                tui::render_tree_view(&mut output, &view)?;
            }
            Some(SideScreen::Passphrase { source_label, input: pw_input, error, .. }) => {
                tui::render_passphrase(&mut output, source_label, pw_input, error.as_deref())?;
            }
            #[cfg(feature = "full-fs-view")]
            Some(SideScreen::FileBrowser { source_label, current_dir, root, menu, .. }) => {
                tui::render_file_browser(&mut output, source_label, current_dir, root, menu)?;
            }
        }

        let key = tui::read_key(&mut input)?;

        // Handle input
        match &mut side {
            None => {
                match tui::handle_tree_view_key(&mut view, &key) {
                    tui::Action::Quit => {
                        cleanup(&mut output)?;
                        return Ok(TuiResult::Quit);
                    }
                    tui::Action::Boot { source_idx, entry } => {
                        if let Some(leaf_path) = find_entry_leaf_path(&view) {
                            cleanup(&mut output)?;
                            return Ok(TuiResult::Boot { leaf_path, entry, source_idx });
                        }
                    }
                    tui::Action::UnlockSource(idx) => {
                        let label = sources.get(idx)
                            .map(|s| s.label.clone())
                            .unwrap_or_default();
                        side = Some(SideScreen::Passphrase {
                            source_idx: idx,
                            source_label: label,
                            input: String::new(),
                            error: None,
                        });
                    }
                    #[cfg(feature = "full-fs-view")]
                    tui::Action::OpenFileBrowser => {
                        if let Some(src_idx) = view.cursor_source_idx() {
                            if let Some(mp) = &sources[src_idx].mount_point {
                                let root = mp.clone();
                                if let Ok((file_menu, dir_entries)) = tui::build_file_menu(&root) {
                                    let label = sources[src_idx].label.clone();
                                    side = Some(SideScreen::FileBrowser {
                                        source_label: label,
                                        current_dir: root.clone(),
                                        root,
                                        menu: file_menu,
                                        dir_entries,
                                    });
                                }
                            }
                        }
                    }
                    tui::Action::RefreshSources => {
                        match mount::discover_sources() {
                            Ok(new_sources) => {
                                *sources = new_sources;
                                trees.clear();
                                for src in sources.iter() {
                                    build_source_tree(src, trees);
                                }
                                // Re-append static entries
                                let static_path = std::path::Path::new(tree::STATIC_ENTRIES_PATH);
                                if let Ok(statics) = tree::load_static_entries(static_path) {
                                    for (src, label, tree_nodes) in statics {
                                        sources.push(src);
                                        trees.push((label, tree_nodes));
                                    }
                                }
                                let new_default = select::resolve_default(trees, None);
                                view = tui::TreeView::build(sources, trees, new_default.as_ref());
                            }
                            Err(_) => {} // silently ignore rescan failure
                        }
                    }
                    #[cfg(feature = "rescue-shell")]
                    tui::Action::DropToShell => {
                        cleanup(&mut output)?;
                        return Ok(TuiResult::Shell);
                    }
                    tui::Action::Redraw | tui::Action::None => {}
                    _ => {}
                }
            }
            Some(SideScreen::Passphrase { source_idx, input: pw_input, .. }) => {
                let action = tui::handle_passphrase_key(pw_input, &key);
                match action {
                    tui::Action::SubmitPassphrase => {
                        let si = *source_idx;
                        let passphrase = pw_input.clone();
                        match mount::unlock_and_mount(&sources[si].device, &passphrase) {
                            Ok(mp) => {
                                sources[si].state = SourceState::Mounted;
                                sources[si].mount_point = Some(mp);
                                sources[si].passphrase = Some(passphrase);
                                let mut new_trees = Vec::new();
                                build_source_tree(&sources[si], &mut new_trees);
                                if let Some(t) = new_trees.into_iter().next() {
                                    trees[si] = t;
                                }
                                tui::hide_cursor(&mut output)?;
                                view = tui::TreeView::build(sources, trees, default);
                                side = None;
                            }
                            Err(e) => {
                                if let Some(SideScreen::Passphrase { input, error, .. }) = &mut side {
                                    *error = Some(format!("{e}"));
                                    input.clear();
                                }
                            }
                        }
                    }
                    tui::Action::Back => {
                        tui::hide_cursor(&mut output)?;
                        side = None;
                    }
                    tui::Action::Redraw => {}
                    _ => {}
                }
            }
            #[cfg(feature = "full-fs-view")]
            Some(SideScreen::FileBrowser {
                current_dir, root, menu, dir_entries, ..
            }) => {
                match tui::handle_file_browser_key(menu, dir_entries, current_dir, root, &key) {
                    tui::Action::BootFile { path } => {
                        cleanup(&mut output)?;
                        return Ok(TuiResult::BootFile { path });
                    }
                    tui::Action::OpenDir(idx) => {
                        if let Some(entry) = dir_entries.get(idx) {
                            let new_dir = entry.path.clone();
                            if let Ok((file_menu, new_entries)) = tui::build_file_menu(&new_dir) {
                                *current_dir = new_dir;
                                *menu = file_menu;
                                *dir_entries = new_entries;
                            }
                        }
                    }
                    tui::Action::DirUp => {
                        if let Some(parent) = current_dir.parent() {
                            let parent = parent.to_path_buf();
                            if let Ok((file_menu, new_entries)) = tui::build_file_menu(&parent) {
                                *current_dir = parent;
                                *menu = file_menu;
                                *dir_entries = new_entries;
                            }
                        }
                    }
                    tui::Action::Back => {
                        side = None;
                    }
                    tui::Action::Redraw => {}
                    _ => {}
                }
            }
        }
    }
}

/// Check if the default entry's source is locked (encrypted, not yet unlocked).
fn default_is_locked(view: &tui::TreeView) -> bool {
    // Find the default entry node, then check its source
    for node in &view.nodes {
        if node.is_default {
            if let tui::NodeKind::Entry { source_idx, .. } = &node.kind {
                // Check the source node for this index
                for src_node in &view.nodes {
                    if let tui::NodeKind::Source { idx, state, .. } = &src_node.kind {
                        if idx == source_idx {
                            return matches!(state, tui::NodeSourceState::Encrypted);
                        }
                    }
                }
            }
        }
    }
    false
}

/// Extract the boot result for the default entry from the tree view.
fn boot_default(view: &tui::TreeView) -> Option<TuiResult> {
    for (i, node) in view.nodes.iter().enumerate() {
        if node.is_default {
            if let tui::NodeKind::Entry { entry, source_idx } = &node.kind {
                // Walk backwards to find the leaf path
                for j in (0..=i).rev() {
                    if let tui::NodeKind::Leaf { path, .. } = &view.nodes[j].kind {
                        return Some(TuiResult::Boot {
                            leaf_path: path.clone(),
                            entry: entry.clone(),
                            source_idx: *source_idx,
                        });
                    }
                }
            }
        }
    }
    None
}

/// Find the leaf path for the entry at the current cursor position in the tree view.
fn find_entry_leaf_path(view: &tui::TreeView) -> Option<PathBuf> {
    for i in (0..=view.cursor).rev() {
        if let tui::NodeKind::Leaf { path, .. } = &view.nodes[i].kind {
            return Some(path.clone());
        }
    }
    None
}

fn build_source_tree(src: &Source, trees: &mut Vec<(String, Vec<TreeNode>)>) {
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

// --- Input multiplexer: stdin + evdev ---

/// Multiplexes terminal stdin with evdev input devices.
/// Evdev events are translated to byte sequences that tui::read_key() understands.
struct InputMux {
    stdin_fd: RawFd,
    evdev: Option<evdev::EvdevReader>,
    buf: [u8; 3],
    buf_len: usize,
    buf_pos: usize,
}

impl InputMux {
    fn new(stdin_fd: RawFd, evdev: Option<evdev::EvdevReader>) -> Self {
        Self {
            stdin_fd,
            evdev,
            buf: [0; 3],
            buf_len: 0,
            buf_pos: 0,
        }
    }
}

impl InputMux {
    /// Poll for input readiness with a timeout.
    /// Returns Ok(true) if input is ready, Ok(false) on timeout.
    fn poll(&self, timeout_ms: i32) -> io::Result<bool> {
        let mut pollfds = Vec::with_capacity(1 + self.evdev.as_ref().map_or(0, |e| e.fds().len()));
        pollfds.push(libc::pollfd {
            fd: self.stdin_fd,
            events: libc::POLLIN,
            revents: 0,
        });
        if let Some(evdev) = &self.evdev {
            for &fd in evdev.fds() {
                pollfds.push(libc::pollfd {
                    fd,
                    events: libc::POLLIN,
                    revents: 0,
                });
            }
        }
        loop {
            let ret = unsafe {
                libc::poll(
                    pollfds.as_mut_ptr(),
                    pollfds.len() as libc::nfds_t,
                    timeout_ms,
                )
            };
            if ret < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(e);
            }
            return Ok(ret > 0);
        }
    }
}

impl Read for InputMux {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        // Drain internal buffer first (from multi-byte evdev translations)
        if self.buf_pos < self.buf_len {
            let avail = self.buf_len - self.buf_pos;
            let n = out.len().min(avail);
            out[..n].copy_from_slice(&self.buf[self.buf_pos..self.buf_pos + n]);
            self.buf_pos += n;
            if self.buf_pos >= self.buf_len {
                self.buf_len = 0;
                self.buf_pos = 0;
            }
            return Ok(n);
        }

        // No evdev devices — just read stdin
        let Some(evdev) = &self.evdev else {
            let n = unsafe {
                libc::read(self.stdin_fd, out.as_mut_ptr() as *mut libc::c_void, out.len())
            };
            return if n < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(n as usize)
            };
        };

        // Poll stdin + all evdev fds
        let ev_fds = evdev.fds();
        let nfds = 1 + ev_fds.len();
        let mut pollfds = Vec::with_capacity(nfds);
        pollfds.push(libc::pollfd {
            fd: self.stdin_fd,
            events: libc::POLLIN,
            revents: 0,
        });
        for &fd in ev_fds {
            pollfds.push(libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            });
        }

        loop {
            let ret = unsafe {
                libc::poll(pollfds.as_mut_ptr(), nfds as libc::nfds_t, -1)
            };
            if ret < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(e);
            }

            // stdin ready — pass through directly
            if pollfds[0].revents & libc::POLLIN != 0 {
                let n = unsafe {
                    libc::read(
                        self.stdin_fd,
                        out.as_mut_ptr() as *mut libc::c_void,
                        out.len(),
                    )
                };
                return if n < 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(n as usize)
                };
            }

            // evdev ready — translate to terminal byte sequence
            for pfd in &pollfds[1..] {
                if pfd.revents & libc::POLLIN != 0 {
                    if let Some(key) = evdev.try_read_key(pfd.fd) {
                        let (bytes, len) = evdev_key_to_bytes(&key);
                        if len == 0 {
                            continue;
                        }
                        let n = out.len().min(len);
                        out[..n].copy_from_slice(&bytes[..n]);
                        if n < len {
                            // Buffer remaining bytes for next read() call
                            let rem = len - n;
                            self.buf[..rem].copy_from_slice(&bytes[n..len]);
                            self.buf_len = rem;
                            self.buf_pos = 0;
                        }
                        return Ok(n);
                    }
                }
            }
            // Unmapped evdev event — poll again
        }
    }
}

/// Translate a Key from evdev into the byte sequence that tui::read_key() expects.
fn evdev_key_to_bytes(key: &tui::Key) -> ([u8; 3], usize) {
    match key {
        tui::Key::Up => ([0x1b, b'[', b'A'], 3),
        tui::Key::Down => ([0x1b, b'[', b'B'], 3),
        tui::Key::Right => ([0x1b, b'[', b'C'], 3),
        tui::Key::Left => ([0x1b, b'[', b'D'], 3),
        tui::Key::Enter => ([b'\r', 0, 0], 1),
        tui::Key::Backspace => ([0x7f, 0, 0], 1),
        tui::Key::Char(c) => ([*c as u8, 0, 0], 1),
        // Escape produces a bare 0x1b which would cause read_key to block
        // waiting for a sequence — skip it. Use 'q' or Left to go back.
        _ => ([0, 0, 0], 0),
    }
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
