// kexec syscall, EFI var read/write, key handoff initrd construction.
//
// Provides the final step: loading a kernel+initrd via kexec_file_load(2),
// triggering the kexec reboot, persisting the boot selection to an EFI
// variable, and constructing a CPIO initrd segment for key handoff.

use std::ffi::CString;
use std::fs;
use std::io;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

use crate::types::{BootSelection, Error, Result};

// kexec_file_load(2) syscall number
#[cfg(target_arch = "x86_64")]
const SYS_KEXEC_FILE_LOAD: libc::c_long = 320;
#[cfg(target_arch = "aarch64")]
const SYS_KEXEC_FILE_LOAD: libc::c_long = 294;

// --- EFI variable persistence ---
//
// The boot selection (leaf_path, entry_name) is stored in an EFI variable
// at /sys/firmware/efi/efivars/<name>-<guid>.
//
// Format: EFI variable attributes (4 bytes LE) + leaf_path + '\n' + entry_name
// This is a simple text format, easy to parse and debug.

const EFI_VAR_NAME: &str = "KexecMenuSelection";
const EFI_TIMEOUT_VAR: &str = "KexecMenuTimeout";
// Project GUID for kexec-menu EFI variables.
const EFI_VAR_GUID: &str = "e518894a-0634-4b2d-b448-e654c0eda6a7";

/// EFI variable attributes: non-volatile + boot service access + runtime access.
const EFI_ATTR_NV_BS_RT: u32 = 0x07;

fn efivar_path() -> PathBuf {
    PathBuf::from(format!(
        "/sys/firmware/efi/efivars/{}-{}",
        EFI_VAR_NAME, EFI_VAR_GUID
    ))
}

/// Serialize a boot selection into the EFI variable payload (without attributes).
pub fn serialize_boot_selection(sel: &BootSelection) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(sel.leaf_path.to_string_lossy().as_bytes());
    buf.push(b'\n');
    buf.extend_from_slice(sel.entry_name.as_bytes());
    buf
}

/// Deserialize a boot selection from EFI variable payload (without attributes).
pub fn deserialize_boot_selection(data: &[u8]) -> Result<BootSelection> {
    let s = std::str::from_utf8(data)
        .map_err(|_| Error::Parse("efi var: invalid UTF-8".into()))?;
    let mut lines = s.splitn(2, '\n');
    let leaf_path = lines
        .next()
        .ok_or_else(|| Error::Parse("efi var: missing leaf_path".into()))?;
    let entry_name = lines
        .next()
        .ok_or_else(|| Error::Parse("efi var: missing entry_name".into()))?;
    if leaf_path.is_empty() || entry_name.is_empty() {
        return Err(Error::Parse("efi var: empty field".into()));
    }
    Ok(BootSelection {
        leaf_path: PathBuf::from(leaf_path),
        entry_name: entry_name.to_string(),
    })
}

/// Read the autoboot timeout from the EFI variable `KexecMenuTimeout`.
/// Returns None if the variable doesn't exist or can't be read.
/// Value is a u16 in seconds (little-endian).
pub fn read_efi_timeout() -> Option<u16> {
    let path = format!(
        "/sys/firmware/efi/efivars/{}-{}",
        EFI_TIMEOUT_VAR, EFI_VAR_GUID
    );
    let data = fs::read(path).ok()?;
    // First 4 bytes are EFI variable attributes, then 2 bytes LE u16
    if data.len() < 6 {
        return None;
    }
    Some(u16::from_le_bytes([data[4], data[5]]))
}

/// Read the last boot selection from the EFI variable.
/// Returns None if the variable doesn't exist or can't be read.
pub fn read_efi_selection() -> Option<BootSelection> {
    let path = efivar_path();
    let data = fs::read(&path).ok()?;
    // First 4 bytes are EFI variable attributes
    if data.len() <= 4 {
        return None;
    }
    deserialize_boot_selection(&data[4..]).ok()
}

/// Write the boot selection to the EFI variable.
pub fn write_efi_selection(sel: &BootSelection) -> Result<()> {
    let path = efivar_path();

    // Remove immutable flag if the file exists (Linux sets it on efi vars)
    remove_immutable(&path);

    let payload = serialize_boot_selection(sel);
    let mut buf = Vec::with_capacity(4 + payload.len());
    buf.extend_from_slice(&EFI_ATTR_NV_BS_RT.to_le_bytes());
    buf.extend_from_slice(&payload);

    fs::write(&path, &buf)?;
    Ok(())
}

/// Remove the immutable attribute from an efivar file (ioctl FS_IOC_SETFLAGS).
fn remove_immutable(path: &Path) {
    // FS_IOC_SETFLAGS = 0x40086602 on x86_64, 0x40046602 on aarch64
    #[cfg(target_arch = "x86_64")]
    const FS_IOC_SETFLAGS: u32 = 0x40086602;
    #[cfg(target_arch = "aarch64")]
    const FS_IOC_SETFLAGS: u32 = 0x40046602;

    if let Ok(f) = fs::OpenOptions::new().write(true).open(path) {
        let flags: libc::c_long = 0;
        // libc::Ioctl is c_ulong on glibc, c_int on musl; cast to handle both
        unsafe { libc::ioctl(f.as_raw_fd(), FS_IOC_SETFLAGS as libc::Ioctl, &flags) };
    }
}

// --- CPIO key handoff initrd ---
//
// Constructs a newc-format CPIO archive containing a decrypted key file.
// This is passed as an additional initrd segment to kexec, so stage 1
// can find the key at /run/bootmenu-keys/<uuid> without it ever touching disk.

const CPIO_MAGIC: &[u8] = b"070701";

/// Build a newc-format CPIO archive containing a single file.
///
/// The archive contains the directory structure and the file with `content`,
/// placed at `path` (e.g. "run/bootmenu-keys/<uuid>").
/// Terminated with the TRAILER record.
pub fn build_key_cpio(path: &str, content: &[u8]) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut ino: u32 = 1;

    // Create parent directories
    let parts: Vec<&str> = path.split('/').collect();
    for i in 1..parts.len() {
        let dir_path = parts[..i].join("/");
        cpio_append_dir(&mut buf, &dir_path, ino);
        ino += 1;
    }

    // Append the file
    cpio_append_file(&mut buf, path, content, ino);

    // Append trailer
    cpio_append_trailer(&mut buf);

    buf
}

fn cpio_append_dir(buf: &mut Vec<u8>, name: &str, ino: u32) {
    cpio_append_entry(buf, name, ino, 0o040755, 0);
}

fn cpio_append_file(buf: &mut Vec<u8>, name: &str, content: &[u8], ino: u32) {
    cpio_append_entry(buf, name, ino, 0o100600, content.len() as u32);
    buf.extend_from_slice(content);
    // Pad file data to 4-byte boundary
    let pad = (4 - (content.len() % 4)) % 4;
    buf.extend(std::iter::repeat(0u8).take(pad));
}

fn cpio_append_trailer(buf: &mut Vec<u8>) {
    cpio_append_entry(buf, "TRAILER!!!", 0, 0, 0);
}

fn cpio_append_entry(buf: &mut Vec<u8>, name: &str, ino: u32, mode: u32, filesize: u32) {
    let namesize = name.len() as u32 + 1; // +1 for NUL
    // Header: 6 magic + 13 fields * 8 hex chars = 110 bytes
    // namesize includes NUL terminator
    let header = format!(
        "{}{:08X}{:08X}{:08X}{:08X}{:08X}{:08X}{:08X}{:08X}{:08X}{:08X}{:08X}{:08X}{:08X}",
        std::str::from_utf8(CPIO_MAGIC).unwrap(),
        ino,       // ino
        mode,      // mode
        0u32,      // uid
        0u32,      // gid
        1u32,      // nlink
        0u32,      // mtime
        filesize,  // filesize
        0u32,      // devmajor
        0u32,      // devminor
        0u32,      // rdevmajor
        0u32,      // rdevminor
        namesize,  // namesize
        0u32,      // check
    );
    buf.extend_from_slice(header.as_bytes());
    buf.extend_from_slice(name.as_bytes());
    buf.push(0); // NUL terminator
    // Pad header+name to 4-byte boundary
    let hdr_total = 110 + namesize as usize;
    let pad = (4 - (hdr_total % 4)) % 4;
    buf.extend(std::iter::repeat(0u8).take(pad));
}

// --- kexec syscall wrappers ---

/// Load a kernel and initrd for kexec using kexec_file_load(2).
///
/// `kernel_path`: path to the kernel image (vmlinuz)
/// `initrd_path`: path to the initrd image
/// `cmdline`: kernel command line
/// `extra_initrd`: optional additional initrd segment (e.g. key CPIO)
///
/// When `extra_initrd` is provided, a combined initrd is built by
/// concatenating the original initrd with the extra segment. The kernel
/// treats concatenated CPIO archives as overlaid layers.
pub fn kexec_load(
    kernel_path: &Path,
    initrd_path: &Path,
    cmdline: &str,
    extra_initrd: Option<&[u8]>,
) -> Result<()> {
    let kernel_fd = fs::File::open(kernel_path)?;

    // Build initrd: original + optional extra segment
    let initrd_data = build_initrd(initrd_path, extra_initrd)?;

    let c_cmdline = CString::new(cmdline)
        .map_err(|_| Error::Parse("cmdline contains NUL byte".into()))?;

    // We need to pass initrd as an fd. Write to a memfd.
    let initrd_fd = memfd_create("kexec-initrd")?;
    write_all_fd(initrd_fd, &initrd_data)?;

    let cmdline_bytes = c_cmdline.as_bytes_with_nul();
    let ret = unsafe {
        libc::syscall(
            SYS_KEXEC_FILE_LOAD,
            kernel_fd.as_raw_fd() as libc::c_long,
            initrd_fd as libc::c_long,
            cmdline_bytes.len() as libc::c_ulong,
            cmdline_bytes.as_ptr(),
            0u64, // flags
        )
    };

    unsafe { libc::close(initrd_fd) };

    if ret != 0 {
        return Err(Error::Io(io::Error::last_os_error()));
    }

    Ok(())
}

/// Build the final initrd by reading the base initrd and optionally appending
/// an extra segment (e.g. key CPIO archive).
fn build_initrd(initrd_path: &Path, extra: Option<&[u8]>) -> Result<Vec<u8>> {
    let mut data = fs::read(initrd_path)?;
    if let Some(extra) = extra {
        data.extend_from_slice(extra);
    }
    Ok(data)
}

/// Create a memfd and return its raw file descriptor.
fn memfd_create(name: &str) -> Result<i32> {
    let c_name = CString::new(name)
        .map_err(|_| Error::Parse("memfd name contains NUL".into()))?;
    let fd = unsafe { libc::memfd_create(c_name.as_ptr(), 0) };
    if fd < 0 {
        return Err(Error::Io(io::Error::last_os_error()));
    }
    Ok(fd)
}

/// Write all data to a raw file descriptor and seek back to start.
fn write_all_fd(fd: i32, data: &[u8]) -> Result<()> {
    let mut offset = 0;
    while offset < data.len() {
        let written = unsafe {
            libc::write(fd, data[offset..].as_ptr() as *const libc::c_void,
                       data.len() - offset)
        };
        if written < 0 {
            return Err(Error::Io(io::Error::last_os_error()));
        }
        offset += written as usize;
    }
    unsafe { libc::lseek(fd, 0, libc::SEEK_SET) };
    Ok(())
}

/// Trigger the kexec reboot.
///
/// This does not return on success. On failure, returns an error.
pub fn kexec_exec() -> Result<()> {
    // reboot(LINUX_REBOOT_CMD_KEXEC)
    // First: reboot(LINUX_REBOOT_MAGIC1, LINUX_REBOOT_MAGIC2, LINUX_REBOOT_CMD_KEXEC, NULL)
    let ret = unsafe {
        libc::reboot(libc::LINUX_REBOOT_CMD_KEXEC)
    };
    if ret != 0 {
        return Err(Error::Io(io::Error::last_os_error()));
    }
    // Should not reach here
    Ok(())
}

// --- High-level API ---

/// Load and boot a bare kernel file directly via kexec (no initrd, no cmdline).
///
/// Used for directly bootable files found in the filesystem browser
/// (EFI stub kernels, bzImages).
pub fn boot_file(kernel_path: &Path) -> Result<()> {
    let kernel_fd = fs::File::open(kernel_path)?;

    let cmdline = b"\0";
    let ret = unsafe {
        libc::syscall(
            SYS_KEXEC_FILE_LOAD,
            kernel_fd.as_raw_fd() as libc::c_long,
            -1i64 as libc::c_long, // no initrd
            cmdline.len() as libc::c_ulong,
            cmdline.as_ptr(),
            0u64, // flags
        )
    };

    if ret != 0 {
        return Err(Error::Io(io::Error::last_os_error()));
    }

    kexec_exec()
}

/// Execute a boot entry: load kernel via kexec, save selection, trigger reboot.
///
/// `leaf_path`: path to the leaf directory containing kernel/initrd
/// `entry`: the entry from entries.json
/// `key_data`: optional decrypted key for handoff to stage 1
/// `key_uuid`: UUID of the encrypted source (for key path)
pub fn boot_entry(
    leaf_path: &Path,
    kernel: &str,
    initrd: &str,
    cmdline: &str,
    entry_name: &str,
    key_data: Option<(&[u8], &str)>, // (key_bytes, source_uuid)
) -> Result<()> {
    let kernel_path = leaf_path.join(kernel);
    let initrd_path = leaf_path.join(initrd);

    let extra_initrd = key_data.map(|(key, uuid)| {
        let key_path = format!("run/bootmenu-keys/{}", uuid);
        build_key_cpio(&key_path, key)
    });

    kexec_load(
        &kernel_path,
        &initrd_path,
        cmdline,
        extra_initrd.as_deref(),
    )?;

    // Persist selection to EFI var (best-effort, don't fail the boot)
    let sel = BootSelection {
        leaf_path: leaf_path.to_path_buf(),
        entry_name: entry_name.to_string(),
    };
    let _ = write_efi_selection(&sel);

    kexec_exec()
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- EFI var serialization ---

    #[test]
    fn serialize_roundtrip() {
        let sel = BootSelection {
            leaf_path: PathBuf::from("/mnt/boot/nixos/gen42"),
            entry_name: "default".into(),
        };
        let data = serialize_boot_selection(&sel);
        let back = deserialize_boot_selection(&data).unwrap();
        assert_eq!(back, sel);
    }

    #[test]
    fn serialize_format() {
        let sel = BootSelection {
            leaf_path: PathBuf::from("/mnt/boot/gen1"),
            entry_name: "gaming".into(),
        };
        let data = serialize_boot_selection(&sel);
        assert_eq!(data, b"/mnt/boot/gen1\ngaming");
    }

    #[test]
    fn deserialize_missing_newline() {
        let data = b"no-newline-here";
        assert!(deserialize_boot_selection(data).is_err());
    }

    #[test]
    fn deserialize_empty_path() {
        let data = b"\nentry";
        assert!(deserialize_boot_selection(data).is_err());
    }

    #[test]
    fn deserialize_empty_name() {
        let data = b"/mnt/boot/gen1\n";
        assert!(deserialize_boot_selection(data).is_err());
    }

    #[test]
    fn deserialize_with_newlines_in_name() {
        // entry_name should capture everything after first newline
        let data = b"/mnt/boot/gen1\nname\nwith\nnewlines";
        let sel = deserialize_boot_selection(data).unwrap();
        assert_eq!(sel.leaf_path, PathBuf::from("/mnt/boot/gen1"));
        assert_eq!(sel.entry_name, "name\nwith\nnewlines");
    }

    // --- CPIO construction ---

    #[test]
    fn cpio_has_magic() {
        let cpio = build_key_cpio("run/key", b"secret");
        // Every entry starts with "070701"
        let magic = b"070701";
        assert!(cpio.windows(6).any(|w| w == magic));
    }

    #[test]
    fn cpio_has_trailer() {
        let cpio = build_key_cpio("run/key", b"secret");
        assert!(cpio.windows(10).any(|w| w == b"TRAILER!!!"));
    }

    #[test]
    fn cpio_contains_file_data() {
        let content = b"my-secret-key-data";
        let cpio = build_key_cpio("run/bootmenu-keys/test-uuid", content);
        assert!(cpio.windows(content.len()).any(|w| w == content));
    }

    #[test]
    fn cpio_creates_parent_dirs() {
        let cpio = build_key_cpio("run/bootmenu-keys/uuid", b"key");
        let cpio_str = String::from_utf8_lossy(&cpio);
        // Should contain "run" and "run/bootmenu-keys" directory entries
        assert!(cpio_str.contains("run\0"), "missing 'run' dir entry");
        assert!(cpio_str.contains("run/bootmenu-keys\0"), "missing 'run/bootmenu-keys' dir entry");
    }

    #[test]
    fn cpio_header_length() {
        // Each newc header is exactly 110 bytes
        let cpio = build_key_cpio("a", b"x");
        // First entry should start with magic at offset 0
        assert_eq!(&cpio[..6], b"070701");
        // Header is 110 bytes, then "a\0" (2 bytes) = 112, pad to 4 = 112
        // Then file data "x" (1 byte) pad to 4 = 4
        // Then trailer header...
    }

    #[test]
    fn cpio_alignment() {
        // Verify the CPIO is properly 4-byte aligned throughout
        let cpio = build_key_cpio("run/bootmenu-keys/test", b"hello");
        // Total length should be 4-byte aligned for each record
        // We can verify by checking that each "070701" magic appears at
        // a 4-byte aligned offset (relative to the start of the previous record)
        let mut offsets: Vec<usize> = Vec::new();
        for i in 0..cpio.len().saturating_sub(5) {
            if &cpio[i..i + 6] == b"070701" {
                offsets.push(i);
            }
        }
        // Should have at least 3 entries (dir "run", dir "run/bootmenu-keys",
        // file, trailer)
        assert!(offsets.len() >= 4, "expected >=4 entries, got {}", offsets.len());
        // Each offset should be 4-byte aligned
        for off in &offsets {
            assert_eq!(off % 4, 0, "entry at offset {} not 4-byte aligned", off);
        }
    }

    #[test]
    fn cpio_empty_content() {
        let cpio = build_key_cpio("run/key", b"");
        assert!(cpio.windows(10).any(|w| w == b"TRAILER!!!"));
    }

    #[test]
    fn cpio_file_mode_restricted() {
        // The file entry should have mode 0o100600 (0x81C0 in hex = 000081C0 in 8-char hex)
        // It appears at offset 14 in the header (after magic 6 + ino 8 = 14)
        let cpio = build_key_cpio("a", b"x");
        // Find the second 070701 (first is dir or file, depends)
        // For path "a" there are no parent dirs, so first entry is the file
        let mode_hex = &cpio[14..22];
        assert_eq!(mode_hex, b"00008180", "file mode should be 0o100600 (hex 8180)");
    }
}
