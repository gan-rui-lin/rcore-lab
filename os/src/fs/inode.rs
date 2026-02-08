//! `Arc<Inode>` -> `OSInodeInner`: In order to open files concurrently
//! we need to wrap `Inode` into `Arc`,but `Mutex` in `Inode` prevents
//! file systems from being accessed simultaneously
//!
//! `UPSafeCell<OSInodeInner>` -> `OSInode`: for static `ROOT_INODE`,we
//! need to wrap `OSInodeInner` into `UPSafeCell`
use super::{File, KernelFile};
use crate::drivers::BLOCK_DEVICE;
use crate::mm::UserBuffer;
use crate::sync::UPSafeCell;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use bitflags::*;
use easy_fs::{EasyFileSystem, Inode};
use lazy_static::*;
#[cfg(feature = "ext4")]
use super::ext4::Ext4Fs;

/// inode in memory
/// A wrapper around a filesystem inode
/// to implement File trait atop
pub struct OSInode {
    readable: bool,
    writable: bool,
    inner: UPSafeCell<OSInodeInner>,
}
/// The OS inode inner in 'UPSafeCell'
pub struct OSInodeInner {
    offset: usize,
    inode: Arc<Inode>,
}

impl OSInode {
    /// create a new inode in memory
    pub fn new(readable: bool, writable: bool, inode: Arc<Inode>) -> Self {
        Self {
            readable,
            writable,
            inner: unsafe { UPSafeCell::new(OSInodeInner { offset: 0, inode }) },
        }
    }
    /// read all data from the inode
    pub fn read_all(&self) -> Vec<u8> {
        let mut inner = self.inner.exclusive_access();
        let mut buffer: Vec<u8> = Vec::with_capacity(512);
        buffer.resize(512, 0);
        let mut v: Vec<u8> = Vec::new();
        loop {
            let len = inner.inode.read_at(inner.offset, &mut buffer);
            if len == 0 {
                break;
            }
            inner.offset += len;
            v.extend_from_slice(&buffer[..len]);
        }
        v
    }
}

lazy_static! {
    pub static ref ROOT_INODE: Arc<Inode> = {
        let efs = EasyFileSystem::open(BLOCK_DEVICE.clone());
        Arc::new(EasyFileSystem::root_inode(&efs))
    };
}

enum RootFs {
    Easy,
    #[cfg(feature = "ext4")]
    Ext4(Ext4Fs),
}

lazy_static! {
    static ref ROOT_FS: UPSafeCell<RootFs> = unsafe { UPSafeCell::new(RootFs::Easy) };
}

#[cfg(feature = "ext4")]
/// Mount ext4 as the root filesystem with a given device size (bytes).
pub fn mount_ext4(total_bytes: i64) -> Result<(), i32> {
    let fs = Ext4Fs::new(BLOCK_DEVICE.clone(), total_bytes)?;
    *ROOT_FS.exclusive_access() = RootFs::Ext4(fs);
    Ok(())
}

#[cfg(feature = "ext4")]
/// Probe ext4 superblock and return total bytes if present.
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
/// Auto-detect ext4 on the block device and mount it as root.
pub fn mount_ext4_auto() -> bool {
    if let Some(total_bytes) = probe_ext4_size() {
        mount_ext4(total_bytes).is_ok()
    } else {
        false
    }
}

/// List all apps in the root directory
pub fn list_apps() {
    debug!("/**** APPS ****");
    match &*ROOT_FS.exclusive_access() {
        RootFs::Easy => {
            for app in ROOT_INODE.ls() {
                debug!("{}", app);
            }
        }
        #[cfg(feature = "ext4")]
        RootFs::Ext4(fs) => {
            for app in fs.list_root() {
                debug!("{}", app);
            }
        }
    }
    debug!("**************/");
}

fn split_path(path: &str) -> Vec<&str> {
    path.split('/').filter(|s| !s.is_empty()).collect()
}

fn resolve_path(path: &str) -> Option<Arc<Inode>> {
    let comps = split_path(path);
    let mut inode = ROOT_INODE.clone();
    if comps.is_empty() {
        return Some(inode);
    }
    for (idx, comp) in comps.iter().enumerate() {
        let next = inode.find(comp)?;
        if idx + 1 != comps.len() && !next.is_dir() {
            return None;
        }
        inode = next;
    }
    Some(inode)
}

fn resolve_parent(path: &str) -> Option<(Arc<Inode>, String)> {
    let comps = split_path(path);
    if comps.is_empty() {
        return None;
    }
    let name = String::from(*comps.last()?);
    let mut inode = ROOT_INODE.clone();
    for comp in &comps[..comps.len().saturating_sub(1)] {
        let next = inode.find(comp)?;
        if !next.is_dir() {
            return None;
        }
        inode = next;
    }
    Some((inode, name))
}

bitflags! {
    ///  The flags argument to the open() system call is constructed by ORing together zero or more of the following values:
    pub struct OpenFlags: u32 {
        /// readyonly
        const RDONLY = 0;
        /// writeonly
        const WRONLY = 1 << 0;
        /// read and write
        const RDWR = 1 << 1;
        /// create new file
        const CREATE = 1 << 9;
        /// truncate file size to 0
        const TRUNC = 1 << 10;
    }
}

impl OpenFlags {
    /// Do not check validity for simplicity
    /// Return (readable, writable)
    pub fn read_write(&self) -> (bool, bool) {
        if self.is_empty() {
            (true, false)
        } else if self.contains(Self::WRONLY) {
            (false, true)
        } else {
            (true, true)
        }
    }
}

/// Open a file
pub fn open_file(name: &str, flags: OpenFlags) -> Option<Arc<KernelFile>> {
    match &*ROOT_FS.exclusive_access() {
        RootFs::Easy => open_easy_file(name, flags).map(|inode| Arc::new(KernelFile::Easy(inode))),
        #[cfg(feature = "ext4")]
        RootFs::Ext4(fs) => fs
            .open(name, flags)
            .map(|inode| Arc::new(KernelFile::Ext4(inode))),
    }
}

fn open_easy_file(name: &str, flags: OpenFlags) -> Option<Arc<OSInode>> {
    let (readable, writable) = flags.read_write();
    if flags.contains(OpenFlags::CREATE) {
        let (parent, fname) = resolve_parent(name)?;
        if let Some(inode) = parent.find(fname.as_str()) {
            // clear size
            inode.clear();
            Some(Arc::new(OSInode::new(readable, writable, inode)))
        } else {
            // create file
            parent
                .create(fname.as_str())
                .map(|inode| Arc::new(OSInode::new(readable, writable, inode)))
        }
    } else {
        resolve_path(name).map(|inode| {
            if flags.contains(OpenFlags::TRUNC) {
                inode.clear();
            }
            Arc::new(OSInode::new(readable, writable, inode))
        })
    }
}

impl File for OSInode {
    fn readable(&self) -> bool {
        self.readable
    }
    fn writable(&self) -> bool {
        self.writable
    }
    fn read(&self, mut buf: UserBuffer) -> usize {
        let mut inner = self.inner.exclusive_access();
        let mut total_read_size = 0usize;
        for slice in buf.buffers.iter_mut() {
            let read_size = inner.inode.read_at(inner.offset, *slice);
            if read_size == 0 {
                break;
            }
            inner.offset += read_size;
            total_read_size += read_size;
        }
        total_read_size
    }
    fn write(&self, buf: UserBuffer) -> usize {
        let mut inner = self.inner.exclusive_access();
        let mut total_write_size = 0usize;
        for slice in buf.buffers.iter() {
            let write_size = inner.inode.write_at(inner.offset, *slice);
            assert_eq!(write_size, slice.len());
            inner.offset += write_size;
            total_write_size += write_size;
        }
        total_write_size
    }
}
