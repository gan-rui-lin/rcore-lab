#![allow(missing_docs)]

use super::core::ROOT_VFS;
use super::easyfs::easyfs_root;
use super::fat32::fat32_root;
use crate::drivers::BLOCK_DEVICE;

#[cfg(feature = "ext4")]
use super::ext4::Ext4Fs;
#[cfg(feature = "ext4")]
use alloc::sync::Arc;

pub fn mount_easyfs() {
    let root = easyfs_root();
    let mut vfs = ROOT_VFS.exclusive_access();
    vfs.mount_root(root);
    trace!("vfs: mounted easy-fs as root");
}

pub fn mount_fat32() -> Result<(), i32> {
    let root = fat32_root().map_err(|_| -1)?;
    let mut vfs = ROOT_VFS.exclusive_access();
    vfs.mount_root(root);
    trace!("vfs: mounted fat32 as root");
    Ok(())
}

pub fn mount_fat32_auto() -> bool {
    mount_fat32().is_ok()
}

#[cfg(feature = "ext4")]
/// Mount ext4 as root with explicit device size.
pub fn mount_ext4(total_bytes: i64) -> Result<(), i32> {
    let fs = Arc::new(Ext4Fs::new(BLOCK_DEVICE.clone(), total_bytes)?);
    let root = fs.root_inode();
    let mut vfs = ROOT_VFS.exclusive_access();
    vfs.mount_root_ext4(root, fs);
    trace!("vfs: mounted ext4 as root");
    Ok(())
}

#[cfg(feature = "ext4")]
fn probe_ext4_size() -> Option<i64> {
    let mut buf = [0u8; 1024];
    let mut block = [0u8; 512];
    BLOCK_DEVICE.read_block(2, &mut block);
    buf[..512].copy_from_slice(&block);
    BLOCK_DEVICE.read_block(3, &mut block);
    buf[512..].copy_from_slice(&block);
    let magic = u16::from_le_bytes([buf[0x38], buf[0x39]]);
    if magic != 0xEF53 {
        return None;
    }
    let blocks_lo = u32::from_le_bytes([buf[0x04], buf[0x05], buf[0x06], buf[0x07]]) as u64;
    let log_block_size =
        u32::from_le_bytes([buf[0x18], buf[0x19], buf[0x1A], buf[0x1B]]) as u32;
    let blocks_hi =
        u32::from_le_bytes([buf[0x150], buf[0x151], buf[0x152], buf[0x153]]) as u64;
    let blocks = blocks_lo | (blocks_hi << 32);
    if blocks == 0 {
        return None;
    }
    let block_size = 1024u64 << log_block_size;
    Some((blocks * block_size) as i64)
}

#[cfg(feature = "ext4")]
/// Auto-detect ext4 and mount it as root if present.
pub fn mount_ext4_auto() -> bool {
    if let Some(total_bytes) = probe_ext4_size() {
        mount_ext4(total_bytes).is_ok()
    } else {
        false
    }
}
