fn main() {
    println!("cargo:rerun-if-env-changed=KEXEC_MENU_DISK_WHITELIST");
}
