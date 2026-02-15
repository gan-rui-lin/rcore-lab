#![allow(missing_docs)]

use super::core::{normalize_path, VfsInode, VfsNodeKind, ROOT_VFS};
use super::super::{File, OpenFlags};
use crate::mm::UserBuffer;
use crate::sync::UPIntrFreeCell;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

pub struct VfsFile {
    readable: bool,
    writable: bool,
    path: String,
    inner: UPIntrFreeCell<VfsFileInner>,
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
                UPIntrFreeCell::new(VfsFileInner {
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

pub fn create_dir(path: &str) -> bool {
    let path = normalize_path(path);
    let vfs = ROOT_VFS.exclusive_access();
    if vfs.resolve(&path).is_some() {
        return false;
    }
    let Some((parent, name)) = vfs.resolve_parent(&path) else {
        return false;
    };
    parent.create_dir(&name).is_some()
}

pub fn remove_path(path: &str, is_dir: bool) -> bool {
    let path = normalize_path(path);
    let vfs = ROOT_VFS.exclusive_access();
    let Some((parent, name)) = vfs.resolve_parent(&path) else {
        return false;
    };
    parent.remove(&name, is_dir)
}
