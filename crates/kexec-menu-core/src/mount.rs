// Block device enumeration, filesystem detection, and mounting.
//
// Reads sysfs and probes superblocks to discover mountable sources.
// Uses libc for mount/umount syscalls. All mounts are read-only.

use std::ffi::CString;
use std::fs;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::types::{Error, Result, Source, SourceState};

// --- Filesystem type detection via superblock magic ---

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FsType {
    Ext4,
    Btrfs,
    Bcachefs,
    Luks,
}

impl FsType {
    pub fn as_str(&self) -> &'static str {
        match self {
            FsType::Ext4 => "ext4",
            FsType::Btrfs => "btrfs",
            FsType::Bcachefs => "bcachefs",
            FsType::Luks => "crypto_LUKS",
        }
    }

    /// Kernel filesystem type string for mount(2).
    pub fn mount_type(&self) -> &'static str {
        match self {
            FsType::Ext4 => "ext4",
            FsType::Btrfs => "btrfs",
            FsType::Bcachefs => "bcachefs",
            FsType::Luks => panic!("LUKS is not directly mountable"),
        }
    }
}

// Superblock magic bytes and their offsets.
// ext4: 0x38 (56) bytes into superblock at offset 1024, magic 0xEF53
// btrfs: "_BHRfS_M" at offset 0x10040
// bcachefs: 0xc68573f6 at offset 0x1008
// LUKS: "LUKS\xba\xbe" at offset 0

const EXT_SUPER_OFFSET: u64 = 1024 + 0x38;
const EXT_MAGIC: [u8; 2] = [0x53, 0xEF]; // little-endian 0xEF53

const BTRFS_MAGIC_OFFSET: u64 = 0x10040;
const BTRFS_MAGIC: &[u8] = b"_BHRfS_M";

const BCACHEFS_SB_START: u64 = 0x1000; // sector 8
const BCACHEFS_SUPER_OFFSET: u64 = BCACHEFS_SB_START + 0x18; // magic field
const BCACHEFS_MAGIC: [u8; 4] = [0xf6, 0x73, 0x85, 0xc6]; // little-endian 0xc68573f6

const LUKS_MAGIC: &[u8] = b"LUKS\xba\xbe";

/// Probe a block device to detect its filesystem type.
pub fn probe_fs_type(dev: &Path) -> Result<Option<FsType>> {
    let mut f = fs::File::open(dev)?;
    let mut buf = [0u8; 8];

    // LUKS: magic at offset 0
    f.seek(SeekFrom::Start(0))?;
    if f.read_exact(&mut buf[..6]).is_ok() && buf[..6] == *LUKS_MAGIC {
        return Ok(Some(FsType::Luks));
    }

    // ext4: magic at offset 1024+0x38
    f.seek(SeekFrom::Start(EXT_SUPER_OFFSET))?;
    if f.read_exact(&mut buf[..2]).is_ok() && buf[..2] == EXT_MAGIC {
        return Ok(Some(FsType::Ext4));
    }

    // bcachefs: magic at offset 0x1008
    f.seek(SeekFrom::Start(BCACHEFS_SUPER_OFFSET))?;
    if f.read_exact(&mut buf[..4]).is_ok() && buf[..4] == BCACHEFS_MAGIC {
        return Ok(Some(FsType::Bcachefs));
    }

    // btrfs: magic at offset 0x10040
    f.seek(SeekFrom::Start(BTRFS_MAGIC_OFFSET))?;
    if f.read_exact(&mut buf).is_ok() && buf == *BTRFS_MAGIC {
        return Ok(Some(FsType::Btrfs));
    }

    Ok(None)
}

// --- Label reading from superblock ---

/// Read a filesystem label from the superblock.
pub fn read_fs_label(dev: &Path, fstype: FsType) -> Result<Option<String>> {
    let mut f = fs::File::open(dev)?;
    match fstype {
        FsType::Ext4 => read_ext4_label(&mut f),
        FsType::Btrfs => read_btrfs_label(&mut f),
        FsType::Bcachefs => read_bcachefs_label(&mut f),
        FsType::Luks => Ok(None), // LUKS has no fs label at this layer
    }
}

fn read_ext4_label(f: &mut fs::File) -> Result<Option<String>> {
    // ext4 volume name: 16 bytes at superblock offset 1024 + 0x78
    f.seek(SeekFrom::Start(1024 + 0x78))?;
    let mut buf = [0u8; 16];
    f.read_exact(&mut buf)?;
    Ok(label_from_bytes(&buf))
}

fn read_btrfs_label(f: &mut fs::File) -> Result<Option<String>> {
    // btrfs label: 256 bytes at superblock offset 0x1012b
    f.seek(SeekFrom::Start(0x1012b))?;
    let mut buf = [0u8; 256];
    f.read_exact(&mut buf)?;
    Ok(label_from_bytes(&buf))
}

fn read_bcachefs_label(f: &mut fs::File) -> Result<Option<String>> {
    // bcachefs label: 32 bytes at sb_start + 0x48
    f.seek(SeekFrom::Start(BCACHEFS_SB_START + 0x48))?;
    let mut buf = [0u8; 32];
    f.read_exact(&mut buf)?;
    Ok(label_from_bytes(&buf))
}

/// Extract a NUL-terminated UTF-8 label from a fixed-size buffer.
pub fn label_from_bytes(buf: &[u8]) -> Option<String> {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    if end == 0 {
        return None;
    }
    let s = std::str::from_utf8(&buf[..end]).ok()?;
    let trimmed = s.trim();
    if trimmed.is_empty() { None } else { Some(trimmed.to_string()) }
}

// --- Block device enumeration ---

/// A discovered block device.
#[derive(Debug)]
pub struct BlockDevice {
    pub path: PathBuf,
    pub name: String,
    pub size_bytes: u64,
}

/// Enumerate block device partitions from sysfs.
///
/// Reads /sys/block/ for disks, then each disk's child partitions.
/// Returns partition devices (e.g. /dev/sda1, /dev/nvme0n1p1) and
/// whole-disk devices that have no partitions (e.g. /dev/vda with a
/// filesystem directly on it).
pub fn enumerate_block_devices() -> Result<Vec<BlockDevice>> {
    let mut devices = Vec::new();
    let sys_block = Path::new("/sys/block");

    let entries = fs::read_dir(sys_block)?;
    for entry in entries {
        let entry = entry?;
        let disk_name = entry.file_name().to_string_lossy().into_owned();

        // Skip ram disks and loop devices
        if disk_name.starts_with("ram") || disk_name.starts_with("loop") {
            continue;
        }

        let disk_dir = entry.path();
        let mut has_partitions = false;

        // Look for partition subdirectories (they have a "partition" file in sysfs)
        if let Ok(children) = fs::read_dir(&disk_dir) {
            for child in children {
                let child = child?;
                let child_path = child.path();
                if child_path.join("partition").exists() {
                    has_partitions = true;
                    let part_name = child.file_name().to_string_lossy().into_owned();
                    let size = read_sysfs_size(&child_path);
                    if size > 0 {
                        devices.push(BlockDevice {
                            path: PathBuf::from("/dev").join(&part_name),
                            name: part_name,
                            size_bytes: size,
                        });
                    }
                }
            }
        }

        // If no partitions, the disk itself might have a filesystem
        if !has_partitions {
            let size = read_sysfs_size(&disk_dir);
            if size > 0 {
                devices.push(BlockDevice {
                    path: PathBuf::from("/dev").join(&disk_name),
                    name: disk_name,
                    size_bytes: size,
                });
            }
        }
    }

    devices.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(devices)
}

/// Read device size in bytes from sysfs `size` file (reports 512-byte sectors).
fn read_sysfs_size(sysfs_dir: &Path) -> u64 {
    fs::read_to_string(sysfs_dir.join("size"))
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map(|sectors| sectors * 512)
        .unwrap_or(0)
}

// --- Partition label from sysfs ---

/// Try to read a partition label (GPT PARTLABEL) from sysfs.
pub fn read_partition_label(dev_name: &str) -> Option<String> {
    // /sys/class/block/<dev>/device/../<dev>/.. is complex;
    // easier via /dev/disk/by-partlabel/ symlinks
    let by_partlabel = Path::new("/dev/disk/by-partlabel");
    if let Ok(entries) = fs::read_dir(by_partlabel) {
        for entry in entries.flatten() {
            if let Ok(target) = fs::read_link(entry.path()) {
                let target_name = target.file_name()
                    .map(|n| n.to_string_lossy().into_owned());
                if target_name.as_deref() == Some(dev_name) {
                    return Some(entry.file_name().to_string_lossy().into_owned());
                }
            }
        }
    }
    None
}

/// Read UUID from sysfs or /dev/disk/by-uuid/.
pub fn read_device_uuid(dev_name: &str) -> Option<String> {
    let by_uuid = Path::new("/dev/disk/by-uuid");
    if let Ok(entries) = fs::read_dir(by_uuid) {
        for entry in entries.flatten() {
            if let Ok(target) = fs::read_link(entry.path()) {
                let target_name = target.file_name()
                    .map(|n| n.to_string_lossy().into_owned());
                if target_name.as_deref() == Some(dev_name) {
                    return Some(entry.file_name().to_string_lossy().into_owned());
                }
            }
        }
    }
    None
}

// --- Mounting ---

const MOUNT_BASE: &str = "/mnt/kexec-menu";

/// Mount a filesystem read-only and return the mount point.
pub fn mount_ro(dev: &Path, fstype: FsType) -> Result<PathBuf> {
    let dev_name = dev.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown".to_string());

    let mount_point = PathBuf::from(MOUNT_BASE).join(&dev_name);
    fs::create_dir_all(&mount_point)?;

    let c_dev = path_to_cstring(dev)?;
    let c_target = path_to_cstring(&mount_point)?;
    let c_fstype = CString::new(fstype.mount_type())
        .map_err(|_| Error::Parse("invalid fstype string".into()))?;

    // MS_RDONLY = 1
    let flags: libc::c_ulong = libc::MS_RDONLY;

    let ret = unsafe {
        libc::mount(
            c_dev.as_ptr(),
            c_target.as_ptr(),
            c_fstype.as_ptr(),
            flags,
            std::ptr::null(),
        )
    };

    if ret != 0 {
        return Err(Error::Io(io::Error::last_os_error()));
    }

    Ok(mount_point)
}

/// Unmount a filesystem.
pub fn umount(mount_point: &Path) -> Result<()> {
    let c_target = path_to_cstring(mount_point)?;

    let ret = unsafe { libc::umount(c_target.as_ptr()) };

    if ret != 0 {
        return Err(Error::Io(io::Error::last_os_error()));
    }

    Ok(())
}

fn path_to_cstring(p: &Path) -> Result<CString> {
    CString::new(p.as_os_str().as_encoded_bytes())
        .map_err(|_| Error::Parse("path contains NUL byte".into()))
}

// --- Source discovery (high-level API) ---

/// Best-effort label for a device: partition label > fs label > UUID > device name.
pub fn best_label(dev: &BlockDevice, fstype: Option<FsType>) -> String {
    if let Some(label) = read_partition_label(&dev.name) {
        return label;
    }
    if let Some(ft) = fstype {
        if let Ok(Some(label)) = read_fs_label(&dev.path, ft) {
            return label;
        }
    }
    if let Some(uuid) = read_device_uuid(&dev.name) {
        return uuid;
    }
    dev.name.clone()
}

/// Discover all mountable sources by enumerating block devices and probing.
///
/// For each device:
/// - Probe filesystem type
/// - Determine label
/// - Attempt read-only mount (clean filesystems)
/// - Mark encrypted/errored sources appropriately
pub fn discover_sources() -> Result<Vec<Source>> {
    let devices = enumerate_block_devices()?;
    let mut sources = Vec::new();

    for dev in &devices {
        let fstype = match probe_fs_type(&dev.path) {
            Ok(Some(ft)) => ft,
            Ok(None) => continue, // no recognized filesystem
            Err(_) => continue,   // can't read device, skip
        };

        let label = best_label(dev, Some(fstype));

        match fstype {
            FsType::Luks => {
                sources.push(Source {
                    label,
                    device: dev.path.clone(),
                    state: SourceState::Encrypted,
                    mount_point: None,
                });
            }
            FsType::Bcachefs if bcachefs_is_encrypted(&dev.path).unwrap_or(false) => {
                sources.push(Source {
                    label,
                    device: dev.path.clone(),
                    state: SourceState::Encrypted,
                    mount_point: None,
                });
            }
            FsType::Ext4 | FsType::Btrfs | FsType::Bcachefs => {
                match mount_ro(&dev.path, fstype) {
                    Ok(mp) => {
                        sources.push(Source {
                            label,
                            device: dev.path.clone(),
                            state: SourceState::Mounted,
                            mount_point: Some(mp),
                        });
                    }
                    Err(e) => {
                        sources.push(Source {
                            label,
                            device: dev.path.clone(),
                            state: SourceState::Error(format!("{e}")),
                            mount_point: None,
                        });
                    }
                }
            }
        }
    }

    Ok(sources)
}

// --- Bcachefs encryption detection ---

/// Check if a bcachefs filesystem is encrypted by probing the superblock.
/// BCH_SB_ENCRYPTION_TYPE is bits 10-14 of flags[1]; nonzero means encrypted.
pub fn bcachefs_is_encrypted(dev: &Path) -> Result<bool> {
    let mut f = fs::File::open(dev)?;
    // flags[1] is at sb_start + 0x98
    f.seek(SeekFrom::Start(BCACHEFS_SB_START + 0x98))?;
    let mut buf = [0u8; 8];
    f.read_exact(&mut buf)?;
    let flags1 = u64::from_le_bytes(buf);
    // BCH_SB_ENCRYPTION_TYPE: bits 10..14
    let enc_type = (flags1 >> 10) & 0xF;
    Ok(enc_type != 0)
}

// --- Encrypted source unlocking ---

/// Unlock an encrypted source and mount it read-only.
///
/// For LUKS: opens the container via cryptsetup, probes inner fs, mounts.
/// For bcachefs: unlocks via bcachefs tool, then mounts directly.
pub fn unlock_and_mount(dev: &Path, passphrase: &str) -> Result<PathBuf> {
    let fstype = probe_fs_type(dev)?
        .ok_or_else(|| Error::Parse("unknown filesystem".into()))?;
    match fstype {
        FsType::Luks => {
            let mapped = unlock_luks(dev, passphrase)?;
            let inner_fs = probe_fs_type(&mapped)?
                .ok_or_else(|| Error::Parse("no filesystem inside LUKS container".into()))?;
            mount_ro(&mapped, inner_fs)
        }
        FsType::Bcachefs => {
            unlock_bcachefs(dev, passphrase)?;
            mount_ro(dev, FsType::Bcachefs)
        }
        other => Err(Error::Parse(format!("{} is not encrypted", other.as_str()))),
    }
}

/// Open a LUKS container via cryptsetup. Returns the mapped device path.
fn unlock_luks(dev: &Path, passphrase: &str) -> Result<PathBuf> {
    let dev_name = dev.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown".to_string());
    let mapper_name = format!("kexec-{}", dev_name);

    let mut child = Command::new("cryptsetup")
        .args(["open", "--type", "luks", "--key-file", "-"])
        .arg(dev)
        .arg(&mapper_name)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(passphrase.as_bytes());
    }

    let output = child.wait_with_output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::Parse(format!("cryptsetup: {}", stderr.trim())));
    }

    Ok(PathBuf::from("/dev/mapper").join(&mapper_name))
}

/// Unlock a bcachefs filesystem by adding the key to the kernel keyring.
fn unlock_bcachefs(dev: &Path, passphrase: &str) -> Result<()> {
    let mut child = Command::new("bcachefs")
        .arg("unlock")
        .arg(dev)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(passphrase.as_bytes());
    }

    let output = child.wait_with_output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::Parse(format!("bcachefs unlock: {}", stderr.trim())));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- label_from_bytes tests ---

    #[test]
    fn label_nul_terminated() {
        let mut buf = [0u8; 16];
        buf[..5].copy_from_slice(b"hello");
        assert_eq!(label_from_bytes(&buf), Some("hello".into()));
    }

    #[test]
    fn label_full_buffer_no_nul() {
        let buf = *b"abcdefghijklmnop";
        assert_eq!(label_from_bytes(&buf), Some("abcdefghijklmnop".into()));
    }

    #[test]
    fn label_empty() {
        let buf = [0u8; 16];
        assert_eq!(label_from_bytes(&buf), None);
    }

    #[test]
    fn label_whitespace_only() {
        let mut buf = [0u8; 16];
        buf[..3].copy_from_slice(b"   ");
        assert_eq!(label_from_bytes(&buf), None);
    }

    #[test]
    fn label_with_trailing_spaces() {
        let mut buf = [0u8; 16];
        buf[..8].copy_from_slice(b"myfs   \0");
        assert_eq!(label_from_bytes(&buf), Some("myfs".into()));
    }

    // --- FsType tests ---

    #[test]
    fn fstype_as_str() {
        assert_eq!(FsType::Ext4.as_str(), "ext4");
        assert_eq!(FsType::Btrfs.as_str(), "btrfs");
        assert_eq!(FsType::Bcachefs.as_str(), "bcachefs");
        assert_eq!(FsType::Luks.as_str(), "crypto_LUKS");
    }

    #[test]
    fn fstype_mount_type() {
        assert_eq!(FsType::Ext4.mount_type(), "ext4");
        assert_eq!(FsType::Btrfs.mount_type(), "btrfs");
        assert_eq!(FsType::Bcachefs.mount_type(), "bcachefs");
    }

    #[test]
    #[should_panic(expected = "LUKS is not directly mountable")]
    fn fstype_luks_mount_panics() {
        let _ = FsType::Luks.mount_type();
    }

    // --- probe_fs_type tests with synthetic block devices ---

    #[test]
    fn probe_ext4_magic() {
        let tmp = test_device(2048);
        write_at(&tmp, EXT_SUPER_OFFSET, &EXT_MAGIC);
        assert_eq!(probe_fs_type(&tmp).unwrap(), Some(FsType::Ext4));
    }

    #[test]
    fn probe_luks_magic() {
        let tmp = test_device(2048);
        write_at(&tmp, 0, LUKS_MAGIC);
        assert_eq!(probe_fs_type(&tmp).unwrap(), Some(FsType::Luks));
    }

    #[test]
    fn probe_btrfs_magic() {
        let tmp = test_device(0x10048);
        write_at(&tmp, BTRFS_MAGIC_OFFSET, BTRFS_MAGIC);
        assert_eq!(probe_fs_type(&tmp).unwrap(), Some(FsType::Btrfs));
    }

    #[test]
    fn probe_bcachefs_magic() {
        let tmp = test_device(0x101c); // must cover magic at 0x1018
        write_at(&tmp, BCACHEFS_SUPER_OFFSET, &BCACHEFS_MAGIC);
        assert_eq!(probe_fs_type(&tmp).unwrap(), Some(FsType::Bcachefs));
    }

    #[test]
    fn probe_unknown() {
        let tmp = test_device(0x10048);
        assert_eq!(probe_fs_type(&tmp).unwrap(), None);
    }

    #[test]
    fn probe_luks_priority_over_ext4() {
        // If both magics are present, LUKS (checked first) wins
        let tmp = test_device(2048);
        write_at(&tmp, 0, LUKS_MAGIC);
        write_at(&tmp, EXT_SUPER_OFFSET, &EXT_MAGIC);
        assert_eq!(probe_fs_type(&tmp).unwrap(), Some(FsType::Luks));
    }

    // --- bcachefs encryption detection tests ---

    #[test]
    fn bcachefs_not_encrypted() {
        let tmp = test_device(0x10a0); // must cover flags[1] at 0x1098
        write_at(&tmp, BCACHEFS_SUPER_OFFSET, &BCACHEFS_MAGIC);
        // flags[1] is all zeros — no encryption
        assert!(!bcachefs_is_encrypted(&tmp).unwrap());
    }

    #[test]
    fn bcachefs_encrypted() {
        let tmp = test_device(0x10a0);
        write_at(&tmp, BCACHEFS_SUPER_OFFSET, &BCACHEFS_MAGIC);
        // Set BCH_SB_ENCRYPTION_TYPE (bits 10-14 of flags[1]) to 1
        // 1 << 10 = 0x400
        let flags1: u64 = 1 << 10;
        write_at(&tmp, BCACHEFS_SB_START + 0x98, &flags1.to_le_bytes());
        assert!(bcachefs_is_encrypted(&tmp).unwrap());
    }

    #[test]
    fn bcachefs_encrypted_type_2() {
        let tmp = test_device(0x10a0);
        write_at(&tmp, BCACHEFS_SUPER_OFFSET, &BCACHEFS_MAGIC);
        // encryption type 3 (bits 10-14 = 0b0011)
        let flags1: u64 = 3 << 10;
        write_at(&tmp, BCACHEFS_SB_START + 0x98, &flags1.to_le_bytes());
        assert!(bcachefs_is_encrypted(&tmp).unwrap());
    }

    #[test]
    fn bcachefs_other_flags_not_encryption() {
        let tmp = test_device(0x10a0);
        write_at(&tmp, BCACHEFS_SUPER_OFFSET, &BCACHEFS_MAGIC);
        // Set bits around but not in the encryption field (bits 8-9, 14-15)
        let flags1: u64 = (1 << 8) | (1 << 9) | (1 << 14) | (1 << 15);
        write_at(&tmp, BCACHEFS_SB_START + 0x98, &flags1.to_le_bytes());
        assert!(!bcachefs_is_encrypted(&tmp).unwrap());
    }

    // --- Test helpers ---

    fn test_device(size: usize) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "kexec-mount-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("device");
        let data = vec![0u8; size];
        fs::write(&path, &data).unwrap();
        path
    }

    fn write_at(path: &Path, offset: u64, data: &[u8]) {
        use std::io::Write;
        let mut f = fs::OpenOptions::new().write(true).open(path).unwrap();
        f.seek(SeekFrom::Start(offset)).unwrap();
        f.write_all(data).unwrap();
    }
}
