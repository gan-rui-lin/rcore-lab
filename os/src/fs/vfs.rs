//! Minimal VFS core with mount points and path resolution.
#![allow(missing_docs)]

use super::{File, OpenFlags};
use crate::drivers::BLOCK_DEVICE;
use crate::mm::UserBuffer;
use crate::sync::UPSafeCell;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use easy_fs::{EasyFileSystem, Inode};
use lazy_static::*;

#[cfg(feature = "ext4")]
use super::ext4::Ext4Fs;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VfsNodeKind {
    File,
    Dir,
}

#[allow(dead_code)]
pub trait VfsInode: Send + Sync {
    fn kind(&self) -> VfsNodeKind;
    fn read_at(&self, offset: usize, buf: &mut [u8]) -> usize;
    fn write_at(&self, offset: usize, buf: &[u8]) -> usize;
    fn lookup(&self, name: &str) -> Option<Arc<dyn VfsInode>>;
    fn create(&self, name: &str) -> Option<Arc<dyn VfsInode>>;
    fn truncate(&self);
    fn list(&self) -> Vec<String>;
    fn size(&self) -> usize {
        0
    }

    fn is_dir(&self) -> bool {
        self.kind() == VfsNodeKind::Dir
    }
}

#[allow(dead_code)]
pub struct Dentry {
    name: String,
    parent: Option<alloc::sync::Weak<Dentry>>,
    inode: Arc<dyn VfsInode>,
}

#[allow(dead_code)]
impl Dentry {
    pub fn new(name: &str, parent: Option<Arc<Dentry>>, inode: Arc<dyn VfsInode>) -> Arc<Self> {
        Arc::new(Self {
            name: name.to_string(),
            parent: parent.map(|p| Arc::downgrade(&p)),
            inode,
        })
    }

    pub fn inode(&self) -> Arc<dyn VfsInode> {
        self.inode.clone()
    }

    pub fn path(&self) -> String {
        if let Some(parent) = self.parent.as_ref().and_then(|p| p.upgrade()) {
            let parent_path = parent.path();
            if parent_path == "/" {
                parent_path + &self.name
            } else {
                parent_path + "/" + &self.name
            }
        } else {
            String::from("/")
        }
    }
}

struct MountPoint {
    path: String,
    root: Arc<dyn VfsInode>,
    #[cfg(feature = "ext4")]
    _ext4_guard: Option<Arc<Ext4Fs>>,
}

struct Vfs {
    mounts: Vec<MountPoint>,
}

impl Vfs {
    fn new() -> Self {
        Self {
            mounts: vec![MountPoint {
                path: String::from("/"),
                root: Arc::new(NullInode),
                #[cfg(feature = "ext4")]
                _ext4_guard: None,
            }],
        }
    }

    #[allow(dead_code)]
    fn mount_root(&mut self, root: Arc<dyn VfsInode>) {
        if let Some(mount) = self.mounts.iter_mut().find(|m| m.path == "/") {
            mount.root = root;
        } else {
            self.mounts.push(MountPoint {
                path: String::from("/"),
                root,
                #[cfg(feature = "ext4")]
                _ext4_guard: None,
            });
        }
    }

    #[cfg(feature = "ext4")]
    fn mount_root_ext4(&mut self, root: Arc<dyn VfsInode>, guard: Arc<Ext4Fs>) {
        if let Some(mount) = self.mounts.iter_mut().find(|m| m.path == "/") {
            mount.root = root;
            mount._ext4_guard = Some(guard);
        } else {
            self.mounts.push(MountPoint {
                path: String::from("/"),
                root,
                _ext4_guard: Some(guard),
            });
        }
    }

    fn resolve_mount<'a>(&'a self, path: &'a str) -> Option<(&'a MountPoint, &'a str)> {
        let mut best: Option<&MountPoint> = None;
        for mount in &self.mounts {
            let is_root = mount.path == "/";
            if is_root || path == mount.path || path.starts_with(&(mount.path.clone() + "/")) {
                if best
                    .as_ref()
                    .map(|m| m.path.len() < mount.path.len())
                    .unwrap_or(true)
                {
                    best = Some(mount);
                }
            }
        }
        let mount = best?;
        let rel = if mount.path == "/" {
            path.trim_start_matches('/')
        } else {
            path.get(mount.path.len()..)?.trim_start_matches('/')
        };
        Some((mount, rel))
    }

    fn resolve(&self, path: &str) -> Option<Arc<dyn VfsInode>> {
        let path = normalize_path(path);
        let (mount, rel) = self.resolve_mount(&path)?;
        let mut current = mount.root.clone();
        if rel.is_empty() {
            return Some(current);
        }
        trace!("vfs: resolve path={} rel={}", path, rel);
        for comp in rel.split('/').filter(|s| !s.is_empty()) {
            match current.lookup(comp) {
                Some(next) => current = next,
                None => {
                    trace!("vfs: resolve failed at {} for {}", comp, path);
                    return None;
                }
            }
        }
        Some(current)
    }

    fn resolve_parent(&self, path: &str) -> Option<(Arc<dyn VfsInode>, String)> {
        let path = normalize_path(path);
        if path == "/" {
            return None;
        }
        let mut comps: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let name = comps.pop()?.to_string();
        let parent_path = if comps.is_empty() {
            String::from("/")
        } else {
            format!("/{}", comps.join("/"))
        };
        let parent = self.resolve(&parent_path)?;
        Some((parent, name))
    }

    fn root_inode(&self) -> Option<Arc<dyn VfsInode>> {
        self.resolve("/")
    }
}

lazy_static! {
    static ref ROOT_VFS: UPSafeCell<Vfs> = unsafe { UPSafeCell::new(Vfs::new()) };
}

struct NullInode;

impl VfsInode for NullInode {
    fn kind(&self) -> VfsNodeKind {
        VfsNodeKind::Dir
    }

    fn read_at(&self, _offset: usize, _buf: &mut [u8]) -> usize {
        0
    }

    fn write_at(&self, _offset: usize, _buf: &[u8]) -> usize {
        0
    }

    fn lookup(&self, _name: &str) -> Option<Arc<dyn VfsInode>> {
        None
    }

    fn create(&self, _name: &str) -> Option<Arc<dyn VfsInode>> {
        None
    }

    fn truncate(&self) {}

    fn list(&self) -> Vec<String> {
        Vec::new()
    }
}

pub struct VfsFile {
    readable: bool,
    writable: bool,
    path: String,
    inner: UPSafeCell<VfsFileInner>,
}

struct VfsFileInner {
    offset: usize,
    inode: Arc<dyn VfsInode>,
}

impl VfsFile {
    pub fn new(readable: bool, writable: bool, inode: Arc<dyn VfsInode>, path: String) -> Self {
        Self {
            readable,
            writable,
            path,
            inner: unsafe {
                UPSafeCell::new(VfsFileInner {
                    offset: 0,
                    inode,
                })
            },
        }
    }

    pub fn read_all(&self) -> Vec<u8> {
        let mut inner = self.inner.exclusive_access();
        let mut offset = 0usize;
        let mut buf = [0u8; 512];
        let mut out = Vec::new();
        loop {
            let n = inner.inode.read_at(offset, &mut buf);
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
            offset += n;
        }
        inner.offset = offset;
        out
    }
}

impl File for VfsFile {
    fn readable(&self) -> bool {
        self.readable
    }

    fn writable(&self) -> bool {
        self.writable
    }

    fn read(&self, mut buf: UserBuffer) -> usize {
        let mut inner = self.inner.exclusive_access();
        let mut total = 0usize;
        for slice in buf.buffers.iter_mut() {
            let n = inner.inode.read_at(inner.offset, *slice);
            if n == 0 {
                break;
            }
            inner.offset += n;
            total += n;
            if n < slice.len() {
                break;
            }
        }
        total
    }

    fn write(&self, buf: UserBuffer) -> usize {
        let mut inner = self.inner.exclusive_access();
        let mut total = 0usize;
        for slice in buf.buffers.iter() {
            let n = inner.inode.write_at(inner.offset, *slice);
            if n == 0 {
                break;
            }
            inner.offset += n;
            total += n;
            if n < slice.len() {
                break;
            }
        }
        total
    }

    fn read_all(&self) -> Vec<u8> {
        self.read_all()
    }

    fn inode(&self) -> Option<Arc<dyn VfsInode>> {
        let inner = self.inner.exclusive_access();
        Some(inner.inode.clone())
    }

    fn path(&self) -> Option<&str> {
        Some(self.path.as_str())
    }

    fn get_offset(&self) -> Option<usize> {
        let inner = self.inner.exclusive_access();
        Some(inner.offset)
    }

    fn set_offset(&self, offset: usize) {
        let mut inner = self.inner.exclusive_access();
        inner.offset = offset;
    }
}

pub fn list_apps() {
    debug!("/**** APPS ****");
    let vfs = ROOT_VFS.exclusive_access();
    if let Some(root) = vfs.root_inode() {
        for app in root.list() {
            debug!("{}", app);
        }
    }
    debug!("**************/");
}

pub fn open_file(path: &str, flags: OpenFlags) -> Option<Arc<dyn File>> {
    let path = normalize_path(path);
    let (readable, writable) = flags.read_write();
    let vfs = ROOT_VFS.exclusive_access();
    if flags.contains(OpenFlags::CREATE) {
        if let Some(inode) = vfs.resolve(&path) {
            if flags.contains(OpenFlags::TRUNC) {
                inode.truncate();
            }
            return Some(Arc::new(VfsFile::new(readable, writable, inode, path)));
        }
        let (parent, name) = vfs.resolve_parent(&path)?;
        let inode = parent.create(&name)?;
        Some(Arc::new(VfsFile::new(readable, writable, inode, path)))
    } else {
        let inode = vfs.resolve(&path)?;
        if flags.contains(OpenFlags::TRUNC) {
            inode.truncate();
        }
        Some(Arc::new(VfsFile::new(readable, writable, inode, path)))
    }
}

pub fn path_is_dir(path: &str) -> bool {
    let path = normalize_path(path);
    let vfs = ROOT_VFS.exclusive_access();
    match vfs.resolve(&path) {
        Some(inode) => inode.kind() == VfsNodeKind::Dir,
        None => {
            trace!("vfs: path_is_dir not found {}", path);
            false
        }
    }
}

pub fn mount_easyfs() {
    let root = easyfs_root();
    let mut vfs = ROOT_VFS.exclusive_access();
    vfs.mount_root(root);
    trace!("vfs: mounted easy-fs as root");
}

#[cfg(feature = "ext4")]
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
pub fn mount_ext4_auto() -> bool {
    if let Some(total_bytes) = probe_ext4_size() {
        mount_ext4(total_bytes).is_ok()
    } else {
        false
    }
}

fn normalize_path(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for comp in path.split('/') {
        match comp {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(comp),
        }
    }
    if parts.is_empty() {
        String::from("/")
    } else {
        format!("/{}", parts.join("/"))
    }
}

struct EasyFsInode {
    inode: Arc<Inode>,
}

impl EasyFsInode {
    fn new(inode: Arc<Inode>) -> Self {
        Self { inode }
    }
}

impl VfsInode for EasyFsInode {
    fn kind(&self) -> VfsNodeKind {
        if self.inode.is_dir() {
            VfsNodeKind::Dir
        } else {
            VfsNodeKind::File
        }
    }

    fn read_at(&self, offset: usize, buf: &mut [u8]) -> usize {
        self.inode.read_at(offset, buf)
    }

    fn write_at(&self, offset: usize, buf: &[u8]) -> usize {
        self.inode.write_at(offset, buf)
    }

    fn lookup(&self, name: &str) -> Option<Arc<dyn VfsInode>> {
        self.inode
            .find(name)
            .map(|inode| Arc::new(EasyFsInode::new(inode)) as Arc<dyn VfsInode>)
    }

    fn create(&self, name: &str) -> Option<Arc<dyn VfsInode>> {
        self.inode
            .create(name)
            .map(|inode| Arc::new(EasyFsInode::new(inode)) as Arc<dyn VfsInode>)
    }

    fn truncate(&self) {
        self.inode.clear();
    }

    fn list(&self) -> Vec<String> {
        self.inode.ls()
    }
}

fn easyfs_root() -> Arc<dyn VfsInode> {
    let efs = EasyFileSystem::open(BLOCK_DEVICE.clone());
    let root = EasyFileSystem::root_inode(&efs);
    Arc::new(EasyFsInode::new(Arc::new(root)))
}
