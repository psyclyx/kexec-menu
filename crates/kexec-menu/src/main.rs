fn main() {
    let dry_run = std::env::args().any(|a| a == "--dry-run");
    if dry_run {
        eprintln!("kexec-menu: dry-run mode");
    }
}
