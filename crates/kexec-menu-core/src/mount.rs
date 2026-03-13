// Block device enumeration, filesystem detection, and mounting.
//
// Reads sysfs and probes superblocks to discover mountable sources.
// Uses libc for mount syscalls. All mounts are read-only.

#[allow(unused_imports)]
use std::ffi::CString;
use std::fs;
#[allow(unused_imports)]
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
#[allow(unused_imports)]
use std::process::{Command, Stdio};

use crate::types::{Error, Result, Source, SourceState};

// --- Filesystem type detection via superblock magic ---

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FsType {
    #[cfg(feature = "fs-ext4")]
    Ext4,
    #[cfg(feature = "fs-btrfs")]
    Btrfs,
    #[cfg(feature = "fs-bcachefs")]
    Bcachefs,
    #[cfg(feature = "fs-xfs")]
    Xfs,
    #[cfg(feature = "fs-f2fs")]
    F2fs,
    #[cfg(feature = "fs-luks")]
    Luks,
}

impl FsType {
    #[allow(unreachable_patterns)]
    pub fn as_str(&self) -> &'static str {
        match *self {
            #[cfg(feature = "fs-ext4")]
            FsType::Ext4 => "ext4",
            #[cfg(feature = "fs-btrfs")]
            FsType::Btrfs => "btrfs",
            #[cfg(feature = "fs-bcachefs")]
            FsType::Bcachefs => "bcachefs",
            #[cfg(feature = "fs-xfs")]
            FsType::Xfs => "xfs",
            #[cfg(feature = "fs-f2fs")]
            FsType::F2fs => "f2fs",
            #[cfg(feature = "fs-luks")]
            FsType::Luks => "crypto_LUKS",
            _ => unreachable!(),
        }
    }

    /// Kernel filesystem type string for mount(2).
    /// Returns None for LUKS (not directly mountable).
    #[allow(unreachable_patterns)]
    pub fn mount_type(&self) -> Option<&'static str> {
        match *self {
            #[cfg(feature = "fs-ext4")]
            FsType::Ext4 => Some("ext4"),
            #[cfg(feature = "fs-btrfs")]
            FsType::Btrfs => Some("btrfs"),
            #[cfg(feature = "fs-bcachefs")]
            FsType::Bcachefs => Some("bcachefs"),
            #[cfg(feature = "fs-xfs")]
            FsType::Xfs => Some("xfs"),
            #[cfg(feature = "fs-f2fs")]
            FsType::F2fs => Some("f2fs"),
            #[cfg(feature = "fs-luks")]
            FsType::Luks => None,
            _ => unreachable!(),
        }
    }
}

// Superblock magic bytes and their offsets.

#[cfg(feature = "fs-ext4")]
const EXT_SUPER_OFFSET: u64 = 1024 + 0x38;
#[cfg(feature = "fs-ext4")]
const EXT_MAGIC: [u8; 2] = [0x53, 0xEF]; // little-endian 0xEF53

#[cfg(feature = "fs-btrfs")]
const BTRFS_SB_START: u64 = 0x10000;
#[cfg(feature = "fs-btrfs")]
const BTRFS_MAGIC_OFFSET: u64 = BTRFS_SB_START + 0x40;
#[cfg(feature = "fs-btrfs")]
const BTRFS_MAGIC: &[u8] = b"_BHRfS_M";
#[cfg(feature = "fs-btrfs")]
const BTRFS_FSID_OFFSET: u64 = BTRFS_SB_START + 0x20; // fsid UUID (16 bytes)
#[cfg(feature = "fs-btrfs")]
const BTRFS_NUM_DEVICES_OFFSET: u64 = BTRFS_SB_START + 0x88; // __le64

#[cfg(feature = "fs-bcachefs")]
const BCACHEFS_SB_START: u64 = 0x1000; // sector 8
#[cfg(feature = "fs-bcachefs")]
const BCACHEFS_SUPER_OFFSET: u64 = BCACHEFS_SB_START + 0x18; // magic field
#[cfg(feature = "fs-bcachefs")]
const BCACHEFS_MAGIC: [u8; 4] = [0xf6, 0x73, 0x85, 0xc6]; // little-endian 0xc68573f6
#[cfg(feature = "fs-bcachefs")]
const BCACHEFS_UUID_OFFSET: u64 = BCACHEFS_SB_START + 0x28; // fs UUID (16 bytes)
#[cfg(feature = "fs-bcachefs")]
const BCACHEFS_DEV_IDX_OFFSET: u64 = BCACHEFS_SB_START + 0x7A; // dev_idx(u8) + nr_devices(u8)

#[cfg(feature = "fs-xfs")]
const XFS_MAGIC: &[u8] = b"XFSB";

#[cfg(feature = "fs-f2fs")]
const F2FS_SUPER_OFFSET: u64 = 1024;
#[cfg(feature = "fs-f2fs")]
const F2FS_MAGIC: [u8; 4] = [0x10, 0x20, 0xF5, 0xF2]; // little-endian 0xF2F52010

#[cfg(feature = "fs-luks")]
const LUKS_MAGIC: &[u8] = b"LUKS\xba\xbe";

/// Linux ENOKEY errno — returned when bcachefs mount fails due to missing encryption key.
#[cfg(feature = "fs-bcachefs")]
const ENOKEY: i32 = 126;

/// Probe a block device to detect its filesystem type.
pub fn probe_fs_type(dev: &Path) -> Result<Option<FsType>> {
    let mut f = fs::File::open(dev)?;
    let _ = &mut f; // suppress unused warning when no fs features enabled
    let mut buf = [0u8; 8];
    let _ = &mut buf;

    // LUKS: magic at offset 0
    #[cfg(feature = "fs-luks")]
    {
        f.seek(SeekFrom::Start(0))?;
        if f.read_exact(&mut buf[..6]).is_ok() && buf[..6] == *LUKS_MAGIC {
            return Ok(Some(FsType::Luks));
        }
    }

    // XFS: magic "XFSB" at offset 0
    #[cfg(feature = "fs-xfs")]
    {
        f.seek(SeekFrom::Start(0))?;
        if f.read_exact(&mut buf[..4]).is_ok() && buf[..4] == *XFS_MAGIC {
            return Ok(Some(FsType::Xfs));
        }
    }

    // ext4: magic at offset 1024+0x38
    #[cfg(feature = "fs-ext4")]
    {
        f.seek(SeekFrom::Start(EXT_SUPER_OFFSET))?;
        if f.read_exact(&mut buf[..2]).is_ok() && buf[..2] == EXT_MAGIC {
            return Ok(Some(FsType::Ext4));
        }
    }

    // F2FS: magic at offset 1024
    #[cfg(feature = "fs-f2fs")]
    {
        f.seek(SeekFrom::Start(F2FS_SUPER_OFFSET))?;
        if f.read_exact(&mut buf[..4]).is_ok() && buf[..4] == F2FS_MAGIC {
            return Ok(Some(FsType::F2fs));
        }
    }

    // bcachefs: magic at offset 0x1008
    #[cfg(feature = "fs-bcachefs")]
    {
        f.seek(SeekFrom::Start(BCACHEFS_SUPER_OFFSET))?;
        if f.read_exact(&mut buf[..4]).is_ok() && buf[..4] == BCACHEFS_MAGIC {
            return Ok(Some(FsType::Bcachefs));
        }
    }

    // btrfs: magic at offset 0x10040
    #[cfg(feature = "fs-btrfs")]
    {
        f.seek(SeekFrom::Start(BTRFS_MAGIC_OFFSET))?;
        if f.read_exact(&mut buf).is_ok() && buf == *BTRFS_MAGIC {
            return Ok(Some(FsType::Btrfs));
        }
    }

    Ok(None)
}

// --- Label reading from superblock ---

/// Read a filesystem label from the superblock.
#[allow(unreachable_patterns, unused_mut, unused_variables)]
pub fn read_fs_label(dev: &Path, fstype: FsType) -> Result<Option<String>> {
    let mut f = fs::File::open(dev)?;
    match fstype {
        #[cfg(feature = "fs-ext4")]
        FsType::Ext4 => read_ext4_label(&mut f),
        #[cfg(feature = "fs-btrfs")]
        FsType::Btrfs => read_btrfs_label(&mut f),
        #[cfg(feature = "fs-bcachefs")]
        FsType::Bcachefs => read_bcachefs_label(&mut f),
        #[cfg(feature = "fs-xfs")]
        FsType::Xfs => read_xfs_label(&mut f),
        #[cfg(feature = "fs-f2fs")]
        FsType::F2fs => read_f2fs_label(&mut f),
        #[cfg(feature = "fs-luks")]
        FsType::Luks => Ok(None), // LUKS has no fs label at this layer
        _ => unreachable!(),
    }
}

#[cfg(feature = "fs-ext4")]
fn read_ext4_label(f: &mut fs::File) -> Result<Option<String>> {
    // ext4 volume name: 16 bytes at superblock offset 1024 + 0x78
    f.seek(SeekFrom::Start(1024 + 0x78))?;
    let mut buf = [0u8; 16];
    f.read_exact(&mut buf)?;
    Ok(label_from_bytes(&buf))
}

#[cfg(feature = "fs-btrfs")]
fn read_btrfs_label(f: &mut fs::File) -> Result<Option<String>> {
    // btrfs label: 256 bytes at superblock offset 0x1012b
    f.seek(SeekFrom::Start(0x1012b))?;
    let mut buf = [0u8; 256];
    f.read_exact(&mut buf)?;
    Ok(label_from_bytes(&buf))
}

#[cfg(feature = "fs-bcachefs")]
fn read_bcachefs_label(f: &mut fs::File) -> Result<Option<String>> {
    // bcachefs label: 32 bytes at sb_start + 0x48
    f.seek(SeekFrom::Start(BCACHEFS_SB_START + 0x48))?;
    let mut buf = [0u8; 32];
    f.read_exact(&mut buf)?;
    Ok(label_from_bytes(&buf))
}

#[cfg(feature = "fs-xfs")]
fn read_xfs_label(f: &mut fs::File) -> Result<Option<String>> {
    // XFS volume label: 12 bytes at offset 108 in the superblock
    f.seek(SeekFrom::Start(108))?;
    let mut buf = [0u8; 12];
    f.read_exact(&mut buf)?;
    Ok(label_from_bytes(&buf))
}

#[cfg(feature = "fs-f2fs")]
fn read_f2fs_label(f: &mut fs::File) -> Result<Option<String>> {
    // F2FS volume label: UTF-16LE, 512 bytes at superblock offset 1024 + 0x1A0 (416)
    f.seek(SeekFrom::Start(1024 + 0x1A0))?;
    let mut buf = [0u8; 512];
    f.read_exact(&mut buf)?;
    // Decode UTF-16LE, stop at first NUL u16
    let u16s: Vec<u16> = buf.chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&c| c != 0)
        .collect();
    if u16s.is_empty() {
        return Ok(None);
    }
    let s = String::from_utf16(&u16s)
        .map_err(|_| Error::Parse("invalid UTF-16 in F2FS label".into()))?;
    let trimmed = s.trim();
    if trimmed.is_empty() { Ok(None) } else { Ok(Some(trimmed.to_string())) }
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

// --- Multi-device info from superblock ---

/// Information about a device's membership in a multi-device filesystem.
#[derive(Debug, Clone, PartialEq)]
pub struct MultiDeviceInfo {
    /// Filesystem UUID (ties devices together).
    pub fs_uuid: [u8; 16],
    /// Total number of devices in the filesystem.
    pub nr_devices: u32,
    /// This device's index in the set (0-based for bcachefs, 0 for btrfs).
    pub dev_idx: u32,
}

/// Read multi-device membership info from a superblock.
///
/// Returns `None` for single-device filesystems or filesystem types
/// that don't support multi-device (ext4, XFS, F2FS, LUKS).
#[allow(unreachable_patterns, unused_mut, unused_variables)]
pub fn read_multi_device_info(dev: &Path, fstype: FsType) -> Result<Option<MultiDeviceInfo>> {
    match fstype {
        #[cfg(feature = "fs-bcachefs")]
        FsType::Bcachefs => read_bcachefs_multi_device(dev),
        #[cfg(feature = "fs-btrfs")]
        FsType::Btrfs => read_btrfs_multi_device(dev),
        _ => Ok(None),
    }
}

#[cfg(feature = "fs-bcachefs")]
fn read_bcachefs_multi_device(dev: &Path) -> Result<Option<MultiDeviceInfo>> {
    let mut f = fs::File::open(dev)?;

    // Read fs UUID (16 bytes at BCACHEFS_UUID_OFFSET)
    let mut uuid = [0u8; 16];
    f.seek(SeekFrom::Start(BCACHEFS_UUID_OFFSET))?;
    f.read_exact(&mut uuid)?;

    // Read dev_idx (1 byte) and nr_devices (1 byte)
    f.seek(SeekFrom::Start(BCACHEFS_DEV_IDX_OFFSET))?;
    let mut buf = [0u8; 2];
    f.read_exact(&mut buf)?;
    let dev_idx = buf[0] as u32;
    let nr_devices = buf[1] as u32;

    if nr_devices <= 1 {
        return Ok(None);
    }

    Ok(Some(MultiDeviceInfo {
        fs_uuid: uuid,
        nr_devices,
        dev_idx,
    }))
}

#[cfg(feature = "fs-btrfs")]
fn read_btrfs_multi_device(dev: &Path) -> Result<Option<MultiDeviceInfo>> {
    let mut f = fs::File::open(dev)?;

    // Read fsid (16 bytes at BTRFS_FSID_OFFSET)
    let mut uuid = [0u8; 16];
    f.seek(SeekFrom::Start(BTRFS_FSID_OFFSET))?;
    f.read_exact(&mut uuid)?;

    // Read num_devices (u64 LE at BTRFS_NUM_DEVICES_OFFSET)
    f.seek(SeekFrom::Start(BTRFS_NUM_DEVICES_OFFSET))?;
    let mut buf = [0u8; 8];
    f.read_exact(&mut buf)?;
    let num_devices = u64::from_le_bytes(buf);

    if num_devices <= 1 {
        return Ok(None);
    }

    Ok(Some(MultiDeviceInfo {
        fs_uuid: uuid,
        nr_devices: num_devices as u32,
        dev_idx: 0, // btrfs doesn't have a fixed device index in the superblock
    }))
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
    for entry in entries.flatten() {
        let disk_name = entry.file_name().to_string_lossy().into_owned();

        // Skip ram disks and loop devices
        if disk_name.starts_with("ram") || disk_name.starts_with("loop") {
            continue;
        }

        let disk_dir = entry.path();
        let mut has_partitions = false;

        // Look for partition subdirectories (they have a "partition" file in sysfs)
        if let Ok(children) = fs::read_dir(&disk_dir) {
            for child in children.flatten() {
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
    let mount_type = fstype.mount_type()
        .ok_or_else(|| Error::Parse(format!("{} is not directly mountable", fstype.as_str())))?;
    let c_fstype = CString::new(mount_type)
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

fn path_to_cstring(p: &Path) -> Result<CString> {
    CString::new(p.as_os_str().as_encoded_bytes())
        .map_err(|_| Error::Parse("path contains NUL byte".into()))
}

// --- Disk whitelist ---

/// Check whether a device name matches a whitelist pattern.
/// Patterns: exact match or prefix glob (e.g. "nvme*" matches "nvme0n1p1").
/// A "/dev/" prefix on the pattern is stripped before matching.
#[cfg(feature = "disk-whitelist")]
fn pattern_matches(pattern: &str, dev_name: &str) -> bool {
    let pat = pattern.strip_prefix("/dev/").unwrap_or(pattern);
    if let Some(prefix) = pat.strip_suffix('*') {
        dev_name.starts_with(prefix)
    } else {
        dev_name == pat
    }
}

/// Check whether a device name is allowed by the compile-time whitelist.
/// Returns true if no whitelist is configured (allow all).
#[cfg(feature = "disk-whitelist")]
fn device_allowed(dev_name: &str) -> bool {
    match option_env!("KEXEC_MENU_DISK_WHITELIST") {
        None => true,
        Some(list) if list.is_empty() => true,
        Some(list) => list.split(',')
            .any(|pat| pattern_matches(pat.trim(), dev_name)),
    }
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
        #[cfg(feature = "disk-whitelist")]
        if !device_allowed(&dev.name) {
            continue;
        }
        let fstype = match probe_fs_type(&dev.path) {
            Ok(Some(ft)) => ft,
            Ok(None) => continue, // no recognized filesystem
            Err(_) => continue,   // can't read device, skip
        };

        let label = best_label(dev, Some(fstype));

        #[cfg(feature = "fs-luks")]
        if matches!(fstype, FsType::Luks) {
            sources.push(Source {
                label,
                device: dev.path.clone(),
                state: SourceState::Encrypted,
                mount_point: None,
                passphrase: None,
            });
            continue;
        }

        // All non-LUKS filesystem types are directly mountable
        match mount_ro(&dev.path, fstype) {
            Ok(mp) => {
                sources.push(Source {
                    label,
                    device: dev.path.clone(),
                    state: SourceState::Mounted,
                    mount_point: Some(mp),
                    passphrase: None,
                });
            }
            #[cfg(feature = "fs-bcachefs")]
            Err(Error::Io(ref e))
                if fstype == FsType::Bcachefs
                    && e.raw_os_error() == Some(ENOKEY) =>
            {
                sources.push(Source {
                    label,
                    device: dev.path.clone(),
                    state: SourceState::Encrypted,
                    mount_point: None,
                    passphrase: None,
                });
            }
            Err(e) => {
                sources.push(Source {
                    label,
                    device: dev.path.clone(),
                    state: SourceState::Error(format!("{e}")),
                    mount_point: None,
                    passphrase: None,
                });
            }
        }
    }

    Ok(sources)
}

// --- Encrypted source unlocking ---

/// Unlock an encrypted source and mount it read-only.
///
/// For LUKS: opens the container via cryptsetup, probes inner fs, mounts.
/// For bcachefs: unlocks via bcachefs tool, then mounts directly.
#[allow(unreachable_patterns, unused_variables)]
pub fn unlock_and_mount(dev: &Path, passphrase: &str) -> Result<PathBuf> {
    let fstype = probe_fs_type(dev)?
        .ok_or_else(|| Error::Parse("unknown filesystem".into()))?;
    match fstype {
        #[cfg(feature = "fs-luks")]
        FsType::Luks => {
            let mapped = unlock_luks(dev, passphrase)?;
            let inner_fs = probe_fs_type(&mapped)?
                .ok_or_else(|| Error::Parse("no filesystem inside LUKS container".into()))?;
            mount_ro(&mapped, inner_fs)
        }
        #[cfg(feature = "fs-bcachefs")]
        FsType::Bcachefs => {
            unlock_bcachefs(dev, passphrase)?;
            mount_ro(dev, FsType::Bcachefs)
        }
        other => Err(Error::Parse(format!("{} is not encrypted", other.as_str()))),
    }
}

/// Open a LUKS container via cryptsetup. Returns the mapped device path.
#[cfg(feature = "fs-luks")]
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
#[cfg(feature = "fs-bcachefs")]
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

    #[cfg(feature = "fs-ext4")]
    #[test]
    fn fstype_ext4_as_str() {
        assert_eq!(FsType::Ext4.as_str(), "ext4");
        assert_eq!(FsType::Ext4.mount_type(), Some("ext4"));
    }

    #[cfg(feature = "fs-btrfs")]
    #[test]
    fn fstype_btrfs_as_str() {
        assert_eq!(FsType::Btrfs.as_str(), "btrfs");
        assert_eq!(FsType::Btrfs.mount_type(), Some("btrfs"));
    }

    #[cfg(feature = "fs-bcachefs")]
    #[test]
    fn fstype_bcachefs_as_str() {
        assert_eq!(FsType::Bcachefs.as_str(), "bcachefs");
        assert_eq!(FsType::Bcachefs.mount_type(), Some("bcachefs"));
    }

    #[cfg(feature = "fs-xfs")]
    #[test]
    fn fstype_xfs_as_str() {
        assert_eq!(FsType::Xfs.as_str(), "xfs");
        assert_eq!(FsType::Xfs.mount_type(), Some("xfs"));
    }

    #[cfg(feature = "fs-f2fs")]
    #[test]
    fn fstype_f2fs_as_str() {
        assert_eq!(FsType::F2fs.as_str(), "f2fs");
        assert_eq!(FsType::F2fs.mount_type(), Some("f2fs"));
    }

    #[cfg(feature = "fs-luks")]
    #[test]
    fn fstype_luks_as_str() {
        assert_eq!(FsType::Luks.as_str(), "crypto_LUKS");
        assert_eq!(FsType::Luks.mount_type(), None);
    }

    // --- probe_fs_type tests with synthetic block devices ---

    #[cfg(feature = "fs-ext4")]
    #[test]
    fn probe_ext4_magic() {
        let tmp = test_device(2048);
        write_at(&tmp, EXT_SUPER_OFFSET, &EXT_MAGIC);
        assert_eq!(probe_fs_type(&tmp).unwrap(), Some(FsType::Ext4));
    }

    #[cfg(feature = "fs-luks")]
    #[test]
    fn probe_luks_magic() {
        let tmp = test_device(2048);
        write_at(&tmp, 0, LUKS_MAGIC);
        assert_eq!(probe_fs_type(&tmp).unwrap(), Some(FsType::Luks));
    }

    #[cfg(feature = "fs-btrfs")]
    #[test]
    fn probe_btrfs_magic() {
        let tmp = test_device(0x10048);
        write_at(&tmp, BTRFS_MAGIC_OFFSET, BTRFS_MAGIC);
        assert_eq!(probe_fs_type(&tmp).unwrap(), Some(FsType::Btrfs));
    }

    #[cfg(feature = "fs-bcachefs")]
    #[test]
    fn probe_bcachefs_magic() {
        let tmp = test_device(0x101c); // must cover magic at 0x1018
        write_at(&tmp, BCACHEFS_SUPER_OFFSET, &BCACHEFS_MAGIC);
        assert_eq!(probe_fs_type(&tmp).unwrap(), Some(FsType::Bcachefs));
    }

    #[cfg(feature = "fs-xfs")]
    #[test]
    fn probe_xfs_magic() {
        let tmp = test_device(2048);
        write_at(&tmp, 0, XFS_MAGIC);
        assert_eq!(probe_fs_type(&tmp).unwrap(), Some(FsType::Xfs));
    }

    #[cfg(feature = "fs-f2fs")]
    #[test]
    fn probe_f2fs_magic() {
        let tmp = test_device(2048);
        write_at(&tmp, F2FS_SUPER_OFFSET, &F2FS_MAGIC);
        assert_eq!(probe_fs_type(&tmp).unwrap(), Some(FsType::F2fs));
    }

    #[test]
    fn probe_unknown() {
        let tmp = test_device(0x10048);
        assert_eq!(probe_fs_type(&tmp).unwrap(), None);
    }

    #[cfg(all(feature = "fs-luks", feature = "fs-ext4"))]
    #[test]
    fn probe_luks_priority_over_ext4() {
        // If both magics are present, LUKS (checked first) wins
        let tmp = test_device(2048);
        write_at(&tmp, 0, LUKS_MAGIC);
        write_at(&tmp, EXT_SUPER_OFFSET, &EXT_MAGIC);
        assert_eq!(probe_fs_type(&tmp).unwrap(), Some(FsType::Luks));
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

    #[allow(dead_code)]
    fn write_at(path: &Path, offset: u64, data: &[u8]) {
        use std::io::Write;
        let mut f = fs::OpenOptions::new().write(true).open(path).unwrap();
        f.seek(SeekFrom::Start(offset)).unwrap();
        f.write_all(data).unwrap();
    }

    // --- multi-device info tests ---

    #[cfg(feature = "fs-bcachefs")]
    #[test]
    fn multi_device_bcachefs_two_devices() {
        let tmp = test_device(0x1000 + 0x7C); // must cover nr_devices at 0x107B
        // Write bcachefs magic so it's a valid superblock
        write_at(&tmp, BCACHEFS_SUPER_OFFSET, &BCACHEFS_MAGIC);
        // Write a test UUID
        let uuid = [1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
        write_at(&tmp, BCACHEFS_UUID_OFFSET, &uuid);
        // dev_idx=0, nr_devices=2
        write_at(&tmp, BCACHEFS_DEV_IDX_OFFSET, &[0, 2]);

        let info = read_multi_device_info(&tmp, FsType::Bcachefs).unwrap();
        let info = info.expect("should return Some for multi-device");
        assert_eq!(info.fs_uuid, uuid);
        assert_eq!(info.nr_devices, 2);
        assert_eq!(info.dev_idx, 0);
    }

    #[cfg(feature = "fs-bcachefs")]
    #[test]
    fn multi_device_bcachefs_second_device() {
        let tmp = test_device(0x1000 + 0x7C);
        write_at(&tmp, BCACHEFS_SUPER_OFFSET, &BCACHEFS_MAGIC);
        let uuid = [0xAA; 16];
        write_at(&tmp, BCACHEFS_UUID_OFFSET, &uuid);
        // dev_idx=1, nr_devices=3
        write_at(&tmp, BCACHEFS_DEV_IDX_OFFSET, &[1, 3]);

        let info = read_multi_device_info(&tmp, FsType::Bcachefs).unwrap().unwrap();
        assert_eq!(info.fs_uuid, uuid);
        assert_eq!(info.nr_devices, 3);
        assert_eq!(info.dev_idx, 1);
    }

    #[cfg(feature = "fs-bcachefs")]
    #[test]
    fn multi_device_bcachefs_single_returns_none() {
        let tmp = test_device(0x1000 + 0x7C);
        write_at(&tmp, BCACHEFS_SUPER_OFFSET, &BCACHEFS_MAGIC);
        // dev_idx=0, nr_devices=1
        write_at(&tmp, BCACHEFS_DEV_IDX_OFFSET, &[0, 1]);

        let info = read_multi_device_info(&tmp, FsType::Bcachefs).unwrap();
        assert!(info.is_none(), "single device should return None");
    }

    #[cfg(feature = "fs-btrfs")]
    #[test]
    fn multi_device_btrfs_two_devices() {
        let tmp = test_device(0x10090); // must cover num_devices at 0x10088
        // Write btrfs magic
        write_at(&tmp, BTRFS_MAGIC_OFFSET, BTRFS_MAGIC);
        // Write fsid UUID
        let uuid = [0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80,
                     0x90, 0xA0, 0xB0, 0xC0, 0xD0, 0xE0, 0xF0, 0x01];
        write_at(&tmp, BTRFS_FSID_OFFSET, &uuid);
        // num_devices = 2 (u64 LE)
        write_at(&tmp, BTRFS_NUM_DEVICES_OFFSET, &2u64.to_le_bytes());

        let info = read_multi_device_info(&tmp, FsType::Btrfs).unwrap();
        let info = info.expect("should return Some for multi-device");
        assert_eq!(info.fs_uuid, uuid);
        assert_eq!(info.nr_devices, 2);
        assert_eq!(info.dev_idx, 0);
    }

    #[cfg(feature = "fs-btrfs")]
    #[test]
    fn multi_device_btrfs_single_returns_none() {
        let tmp = test_device(0x10090);
        write_at(&tmp, BTRFS_MAGIC_OFFSET, BTRFS_MAGIC);
        // num_devices = 1
        write_at(&tmp, BTRFS_NUM_DEVICES_OFFSET, &1u64.to_le_bytes());

        let info = read_multi_device_info(&tmp, FsType::Btrfs).unwrap();
        assert!(info.is_none(), "single device should return None");
    }

    #[cfg(feature = "fs-ext4")]
    #[test]
    fn multi_device_ext4_returns_none() {
        let tmp = test_device(2048);
        write_at(&tmp, EXT_SUPER_OFFSET, &EXT_MAGIC);
        let info = read_multi_device_info(&tmp, FsType::Ext4).unwrap();
        assert!(info.is_none(), "ext4 doesn't support multi-device");
    }

    #[cfg(feature = "fs-xfs")]
    #[test]
    fn multi_device_xfs_returns_none() {
        let tmp = test_device(2048);
        write_at(&tmp, 0, XFS_MAGIC);
        let info = read_multi_device_info(&tmp, FsType::Xfs).unwrap();
        assert!(info.is_none(), "XFS doesn't support multi-device");
    }

    // --- disk-whitelist pattern matching tests ---

    #[cfg(feature = "disk-whitelist")]
    mod whitelist {
        use super::super::pattern_matches;

        #[test]
        fn exact_match() {
            assert!(pattern_matches("sda1", "sda1"));
            assert!(!pattern_matches("sda1", "sda2"));
        }

        #[test]
        fn glob_suffix() {
            assert!(pattern_matches("nvme*", "nvme0n1p1"));
            assert!(pattern_matches("sd*", "sda1"));
            assert!(!pattern_matches("nvme*", "sda1"));
        }

        #[test]
        fn dev_prefix_stripped() {
            assert!(pattern_matches("/dev/sda1", "sda1"));
            assert!(pattern_matches("/dev/nvme*", "nvme0n1p1"));
        }

        #[test]
        fn empty_prefix_glob_matches_all() {
            assert!(pattern_matches("*", "anything"));
        }

        #[test]
        fn no_partial_match_without_glob() {
            assert!(!pattern_matches("sda", "sda1"));
        }
    }
}
