use std::fs;
use std::path::{Path, PathBuf};

use crate::types::{Entry, Error, Leaf, Result, Source, SourceState, TreeNode};

// --- Hand-rolled JSON parser for entries.json ---
//
// entries.json is a JSON array of objects with exactly four string fields:
// name, kernel, initrd, cmdline. We parse only this shape.

/// Parse entries.json content into a list of boot entries.
pub fn parse_entries(json: &str) -> Result<Vec<Entry>> {
    let mut parser = JsonParser::new(json);
    parser.skip_ws();
    parser.expect(b'[')?;
    let mut entries = Vec::new();
    parser.skip_ws();
    if parser.peek() == Some(b']') {
        parser.advance();
        return Ok(entries);
    }
    loop {
        entries.push(parse_entry(&mut parser)?);
        parser.skip_ws();
        match parser.peek() {
            Some(b',') => {
                parser.advance();
            }
            Some(b']') => {
                parser.advance();
                break;
            }
            Some(c) => return Err(Error::Parse(format!("expected ',' or ']', got '{}'", c as char))),
            None => return Err(Error::Parse("unexpected end of input".into())),
        }
    }
    Ok(entries)
}

fn parse_entry(p: &mut JsonParser) -> Result<Entry> {
    p.skip_ws();
    p.expect(b'{')?;

    let mut name = None;
    let mut kernel = None;
    let mut initrd = None;
    let mut cmdline = None;

    p.skip_ws();
    if p.peek() == Some(b'}') {
        p.advance();
        return Err(Error::Parse("empty entry object".into()));
    }

    loop {
        p.skip_ws();
        let key = p.parse_string()?;
        p.skip_ws();
        p.expect(b':')?;
        p.skip_ws();
        let val = p.parse_string()?;

        match key.as_str() {
            "name" => name = Some(val),
            "kernel" => kernel = Some(val),
            "initrd" => initrd = Some(val),
            "cmdline" => cmdline = Some(val),
            other => return Err(Error::Parse(format!("unknown field: {other}"))),
        }

        p.skip_ws();
        match p.peek() {
            Some(b',') => {
                p.advance();
            }
            Some(b'}') => {
                p.advance();
                break;
            }
            Some(c) => return Err(Error::Parse(format!("expected ',' or '}}', got '{}'", c as char))),
            None => return Err(Error::Parse("unexpected end of input in object".into())),
        }
    }

    let name = name.ok_or_else(|| Error::Parse("missing field: name".into()))?;
    let kernel = kernel.ok_or_else(|| Error::Parse("missing field: kernel".into()))?;
    let initrd = initrd.ok_or_else(|| Error::Parse("missing field: initrd".into()))?;
    let cmdline = cmdline.ok_or_else(|| Error::Parse("missing field: cmdline".into()))?;

    Ok(Entry { name, kernel, initrd, cmdline })
}

struct JsonParser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> JsonParser<'a> {
    fn new(input: &'a str) -> Self {
        Self { bytes: input.as_bytes(), pos: 0 }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn advance(&mut self) {
        self.pos += 1;
    }

    fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' {
                self.advance();
            } else {
                break;
            }
        }
    }

    fn expect(&mut self, ch: u8) -> Result<()> {
        match self.peek() {
            Some(b) if b == ch => {
                self.advance();
                Ok(())
            }
            Some(b) => Err(Error::Parse(format!("expected '{}', got '{}'", ch as char, b as char))),
            None => Err(Error::Parse(format!("expected '{}', got end of input", ch as char))),
        }
    }

    fn parse_string(&mut self) -> Result<String> {
        self.expect(b'"')?;
        let mut s = String::new();
        loop {
            match self.peek() {
                None => return Err(Error::Parse("unterminated string".into())),
                Some(b'"') => {
                    self.advance();
                    return Ok(s);
                }
                Some(b'\\') => {
                    self.advance();
                    match self.peek() {
                        Some(b'"') => s.push('"'),
                        Some(b'\\') => s.push('\\'),
                        Some(b'/') => s.push('/'),
                        Some(b'n') => s.push('\n'),
                        Some(b't') => s.push('\t'),
                        Some(b'r') => s.push('\r'),
                        Some(c) => return Err(Error::Parse(format!("invalid escape: \\{}", c as char))),
                        None => return Err(Error::Parse("unterminated escape".into())),
                    }
                    self.advance();
                }
                Some(b) => {
                    s.push(b as char);
                    self.advance();
                }
            }
        }
    }
}

// --- Boot tree walker ---

/// Walk a boot tree rooted at `root`, returning the tree structure.
///
/// A leaf is a directory containing `entries.json`. Interior nodes are
/// plain directories. Directories that are neither leaves nor contain
/// leaves (directly or transitively) are omitted.
pub fn walk_boot_tree(root: &Path) -> Result<Vec<TreeNode>> {
    let mut nodes = Vec::new();
    let mut dir_entries: Vec<_> = fs::read_dir(root)?.collect::<std::result::Result<Vec<_>, _>>()?;
    dir_entries.sort_by(|a, b| a.file_name().cmp(&b.file_name()));

    for de in dir_entries {
        let path = de.path();
        if !path.is_dir() {
            continue;
        }
        let name = de.file_name().to_string_lossy().into_owned();
        let entries_json = path.join("entries.json");

        if entries_json.is_file() {
            // This is a leaf
            let json = fs::read_to_string(&entries_json)?;
            let entries = parse_entries(&json)?;
            let mtime = fs::metadata(&path)?
                .modified()?
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            nodes.push(TreeNode::Leaf(Leaf { path, entries, mtime }));
        } else {
            // Recurse; only include if it has children
            let children = walk_boot_tree(&path)?;
            if !children.is_empty() {
                nodes.push(TreeNode::Dir { name, children });
            }
        }
    }

    Ok(nodes)
}

// --- Static build-time entries ---
//
// Loaded from /etc/kexec-menu/static.json (placed in initramfs at build time).
// Each entry becomes a Source + single-leaf tree in the UI.
//
// Format: [{"name": "Memtest86+", "dir": "/static/memtest", "kernel": "memtest.bin", "initrd": "", "cmdline": ""}]

pub const STATIC_ENTRIES_PATH: &str = "/etc/kexec-menu/static.json";

/// A raw static entry from the config file.
#[derive(Debug)]
struct StaticConfig {
    name: String,
    dir: String,
    kernel: String,
    initrd: String,
    cmdline: String,
}

fn parse_static_config(json: &str) -> Result<Vec<StaticConfig>> {
    let mut parser = JsonParser::new(json);
    parser.skip_ws();
    parser.expect(b'[')?;
    let mut configs = Vec::new();
    parser.skip_ws();
    if parser.peek() == Some(b']') {
        parser.advance();
        return Ok(configs);
    }
    loop {
        configs.push(parse_one_static_config(&mut parser)?);
        parser.skip_ws();
        match parser.peek() {
            Some(b',') => { parser.advance(); }
            Some(b']') => { parser.advance(); break; }
            Some(c) => return Err(Error::Parse(format!("expected ',' or ']', got '{}'", c as char))),
            None => return Err(Error::Parse("unexpected end of input".into())),
        }
    }
    Ok(configs)
}

fn parse_one_static_config(p: &mut JsonParser) -> Result<StaticConfig> {
    p.skip_ws();
    p.expect(b'{')?;

    let mut name = None;
    let mut dir = None;
    let mut kernel = None;
    let mut initrd = None;
    let mut cmdline = None;

    p.skip_ws();
    if p.peek() == Some(b'}') {
        p.advance();
        return Err(Error::Parse("empty static entry object".into()));
    }

    loop {
        p.skip_ws();
        let key = p.parse_string()?;
        p.skip_ws();
        p.expect(b':')?;
        p.skip_ws();
        let val = p.parse_string()?;

        match key.as_str() {
            "name" => name = Some(val),
            "dir" => dir = Some(val),
            "kernel" => kernel = Some(val),
            "initrd" => initrd = Some(val),
            "cmdline" => cmdline = Some(val),
            other => return Err(Error::Parse(format!("unknown field in static entry: {other}"))),
        }

        p.skip_ws();
        match p.peek() {
            Some(b',') => { p.advance(); }
            Some(b'}') => { p.advance(); break; }
            Some(c) => return Err(Error::Parse(format!("expected ',' or '}}', got '{}'", c as char))),
            None => return Err(Error::Parse("unexpected end of input in object".into())),
        }
    }

    let name = name.ok_or_else(|| Error::Parse("missing field: name".into()))?;
    let dir = dir.ok_or_else(|| Error::Parse("missing field: dir".into()))?;
    let kernel = kernel.ok_or_else(|| Error::Parse("missing field: kernel".into()))?;
    let initrd = initrd.ok_or_else(|| Error::Parse("missing field: initrd".into()))?;
    let cmdline = cmdline.ok_or_else(|| Error::Parse("missing field: cmdline".into()))?;

    Ok(StaticConfig { name, dir, kernel, initrd, cmdline })
}

/// Load static entries from a config file, returning (sources, trees) to append.
/// Returns empty vecs if the config file doesn't exist.
pub fn load_static_entries(path: &Path) -> Result<Vec<(Source, String, Vec<TreeNode>)>> {
    let json = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(Error::Io(e)),
    };
    let configs = parse_static_config(&json)?;
    let mut result = Vec::new();

    for cfg in configs {
        let dir_path = PathBuf::from(&cfg.dir);
        let entry = Entry {
            name: cfg.name.clone(),
            kernel: cfg.kernel,
            initrd: cfg.initrd,
            cmdline: cfg.cmdline,
        };
        let leaf = Leaf {
            path: dir_path.clone(),
            entries: vec![entry],
            mtime: 0,
        };
        let tree = vec![TreeNode::Leaf(leaf)];
        let source = Source {
            label: cfg.name.clone(),
            device: dir_path,
            state: SourceState::Static,
            mount_point: None,
        };
        result.push((source, cfg.name, tree));
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // --- JSON parsing tests ---

    #[test]
    fn parse_single_entry() {
        let json = r#"[{"name":"default","kernel":"vmlinuz","initrd":"initrd","cmdline":"root=/dev/sda1"}]"#;
        let entries = parse_entries(json).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "default");
        assert_eq!(entries[0].kernel, "vmlinuz");
        assert_eq!(entries[0].initrd, "initrd");
        assert_eq!(entries[0].cmdline, "root=/dev/sda1");
    }

    #[test]
    fn parse_multiple_entries() {
        let json = r#"[
            {"name": "default", "kernel": "vmlinuz", "initrd": "initrd-default", "cmdline": "root=/dev/sda1"},
            {"name": "gaming",  "kernel": "vmlinuz", "initrd": "initrd-gaming",  "cmdline": "root=/dev/sda1 mitigations=off"}
        ]"#;
        let entries = parse_entries(json).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "default");
        assert_eq!(entries[1].name, "gaming");
        assert_eq!(entries[1].cmdline, "root=/dev/sda1 mitigations=off");
    }

    #[test]
    fn parse_empty_array() {
        let entries = parse_entries("[]").unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn parse_escaped_strings() {
        let json = r#"[{"name": "with\"quote", "kernel": "vmlinuz", "initrd": "initrd", "cmdline": "a\\b"}]"#;
        let entries = parse_entries(json).unwrap();
        assert_eq!(entries[0].name, "with\"quote");
        assert_eq!(entries[0].cmdline, "a\\b");
    }

    #[test]
    fn parse_missing_field() {
        let json = r#"[{"name": "x", "kernel": "k", "initrd": "i"}]"#;
        let err = parse_entries(json).unwrap_err();
        assert!(matches!(err, Error::Parse(ref s) if s.contains("cmdline")));
    }

    #[test]
    fn parse_unknown_field() {
        let json = r#"[{"name":"x","kernel":"k","initrd":"i","cmdline":"c","extra":"bad"}]"#;
        let err = parse_entries(json).unwrap_err();
        assert!(matches!(err, Error::Parse(ref s) if s.contains("unknown")));
    }

    #[test]
    fn parse_empty_object() {
        let json = r#"[{}]"#;
        let err = parse_entries(json).unwrap_err();
        assert!(matches!(err, Error::Parse(_)));
    }

    #[test]
    fn parse_not_array() {
        let json = r#"{"name":"x"}"#;
        let err = parse_entries(json).unwrap_err();
        assert!(matches!(err, Error::Parse(_)));
    }

    #[test]
    fn parse_unterminated_string() {
        let json = r#"[{"name": "unterminated}]"#;
        let err = parse_entries(json).unwrap_err();
        assert!(matches!(err, Error::Parse(_)));
    }

    // --- Tree walking tests ---

    fn make_leaf(dir: &Path, entries_json: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join("entries.json"), entries_json).unwrap();
    }

    fn entry_json(name: &str) -> String {
        format!(
            r#"[{{"name":"{name}","kernel":"vmlinuz","initrd":"initrd","cmdline":"root=/dev/sda1"}}]"#
        )
    }

    #[test]
    fn walk_single_leaf() {
        let tmp = tempdir();
        make_leaf(&tmp.join("gen1"), &entry_json("default"));

        let tree = walk_boot_tree(&tmp).unwrap();
        assert_eq!(tree.len(), 1);
        match &tree[0] {
            TreeNode::Leaf(leaf) => {
                assert_eq!(leaf.entries[0].name, "default");
            }
            _ => panic!("expected leaf"),
        }
    }

    #[test]
    fn walk_nested_tree() {
        let tmp = tempdir();
        // boot/nixos/gen1 (leaf)
        // boot/nixos/gen2 (leaf)
        // boot/other/gen1 (leaf)
        make_leaf(&tmp.join("nixos").join("gen1"), &entry_json("default"));
        make_leaf(&tmp.join("nixos").join("gen2"), &entry_json("default"));
        make_leaf(&tmp.join("other").join("gen1"), &entry_json("other"));

        let tree = walk_boot_tree(&tmp).unwrap();
        assert_eq!(tree.len(), 2); // nixos, other
        match &tree[0] {
            TreeNode::Dir { name, children } => {
                assert_eq!(name, "nixos");
                assert_eq!(children.len(), 2);
            }
            _ => panic!("expected dir"),
        }
    }

    #[test]
    fn walk_empty_dirs_omitted() {
        let tmp = tempdir();
        fs::create_dir_all(tmp.join("empty")).unwrap();
        make_leaf(&tmp.join("real").join("gen1"), &entry_json("x"));

        let tree = walk_boot_tree(&tmp).unwrap();
        assert_eq!(tree.len(), 1);
        match &tree[0] {
            TreeNode::Dir { name, .. } => assert_eq!(name, "real"),
            _ => panic!("expected dir"),
        }
    }

    #[test]
    fn walk_files_ignored() {
        let tmp = tempdir();
        fs::write(tmp.join("random.txt"), "hello").unwrap();
        make_leaf(&tmp.join("gen1"), &entry_json("x"));

        let tree = walk_boot_tree(&tmp).unwrap();
        assert_eq!(tree.len(), 1);
    }

    #[test]
    fn walk_sorted_by_name() {
        let tmp = tempdir();
        make_leaf(&tmp.join("charlie"), &entry_json("c"));
        make_leaf(&tmp.join("alpha"), &entry_json("a"));
        make_leaf(&tmp.join("bravo"), &entry_json("b"));

        let tree = walk_boot_tree(&tmp).unwrap();
        let names: Vec<&str> = tree.iter().map(|n| match n {
            TreeNode::Leaf(l) => l.entries[0].name.as_str(),
            _ => unreachable!(),
        }).collect();
        assert_eq!(names, vec!["a", "b", "c"]);
    }

    fn tempdir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("kexec-test-{}", std::process::id()));
        let dir = dir.join(format!("{}", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    // --- Static entry config parsing tests ---

    #[test]
    fn parse_static_single_entry() {
        let json = r#"[{"name":"Memtest86+","dir":"/static/memtest","kernel":"memtest.bin","initrd":"","cmdline":""}]"#;
        let configs = parse_static_config(json).unwrap();
        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].name, "Memtest86+");
        assert_eq!(configs[0].dir, "/static/memtest");
        assert_eq!(configs[0].kernel, "memtest.bin");
    }

    #[test]
    fn parse_static_multiple_entries() {
        let json = r#"[
            {"name": "Memtest86+", "dir": "/static/memtest", "kernel": "memtest.bin", "initrd": "", "cmdline": ""},
            {"name": "netboot.xyz", "dir": "/static/netboot", "kernel": "netboot.efi", "initrd": "", "cmdline": ""}
        ]"#;
        let configs = parse_static_config(json).unwrap();
        assert_eq!(configs.len(), 2);
        assert_eq!(configs[0].name, "Memtest86+");
        assert_eq!(configs[1].name, "netboot.xyz");
    }

    #[test]
    fn parse_static_empty_array() {
        let configs = parse_static_config("[]").unwrap();
        assert!(configs.is_empty());
    }

    #[test]
    fn parse_static_missing_field() {
        let json = r#"[{"name": "x", "dir": "/d", "kernel": "k"}]"#;
        let err = parse_static_config(json).unwrap_err();
        assert!(matches!(err, Error::Parse(_)));
    }

    #[test]
    fn parse_static_unknown_field() {
        let json = r#"[{"name":"x","dir":"/d","kernel":"k","initrd":"i","cmdline":"c","extra":"bad"}]"#;
        let err = parse_static_config(json).unwrap_err();
        assert!(matches!(err, Error::Parse(ref s) if s.contains("unknown")));
    }

    // --- Static entry loading tests ---

    #[test]
    fn load_static_entries_missing_file() {
        let result = load_static_entries(Path::new("/nonexistent/static.json")).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn load_static_entries_from_file() {
        let tmp = tempdir();
        let config_path = tmp.join("static.json");
        fs::write(&config_path, r#"[{"name":"Memtest","dir":"/static/mt","kernel":"mt.bin","initrd":"","cmdline":""}]"#).unwrap();

        let result = load_static_entries(&config_path).unwrap();
        assert_eq!(result.len(), 1);
        let (src, label, tree) = &result[0];
        assert_eq!(src.label, "Memtest");
        assert_eq!(label, "Memtest");
        assert!(matches!(src.state, SourceState::Static));
        assert!(src.mount_point.is_none());
        assert_eq!(tree.len(), 1);
        match &tree[0] {
            TreeNode::Leaf(leaf) => {
                assert_eq!(leaf.path, PathBuf::from("/static/mt"));
                assert_eq!(leaf.entries.len(), 1);
                assert_eq!(leaf.entries[0].name, "Memtest");
                assert_eq!(leaf.entries[0].kernel, "mt.bin");
            }
            _ => panic!("expected leaf"),
        }
    }

    #[test]
    fn load_static_entries_empty_config() {
        let tmp = tempdir();
        let config_path = tmp.join("static.json");
        fs::write(&config_path, "[]").unwrap();
        let result = load_static_entries(&config_path).unwrap();
        assert!(result.is_empty());
    }
}
