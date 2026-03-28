#![allow(missing_docs)]

use super::super::{File, OpenFlags};
use super::core::{normalize_path, VfsInode, VfsNodeKind, ROOT_VFS};
use crate::mm::UserBuffer;
use crate::sync::UPIntrFreeCell;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

pub struct VfsFile {
    readable: bool,
    writable: bool,
    status_flags: u32,
    path: String,
    ts_id: usize,
    inner: UPIntrFreeCell<VfsFileInner>,
}

/// Monotonic counter for timestamp tracking IDs.
static NEXT_TS_ID: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(1);

struct VfsFileInner {
    offset: usize,
    inode: Arc<dyn VfsInode>,
}

impl VfsFile {
    #[allow(dead_code)]
    pub fn new(readable: bool, writable: bool, inode: Arc<dyn VfsInode>, path: String) -> Self {
        Self::new_with_flags(readable, writable, 0, inode, path)
    }

    pub fn new_with_flags(
        readable: bool,
        writable: bool,
        status_flags: u32,
        inode: Arc<dyn VfsInode>,
        path: String,
    ) -> Self {
        Self {
            readable,
            writable,
            status_flags,
            path,
            ts_id: NEXT_TS_ID.fetch_add(1, core::sync::atomic::Ordering::Relaxed),
            inner: unsafe { UPIntrFreeCell::new(VfsFileInner { offset: 0, inode }) },
        }
    }

    pub fn read_all(&self) -> Vec<u8> {
        let mut inner = self.inner.exclusive_access();
        let file_size = inner.inode.size();
        let mut offset = 0usize;
        let out = if file_size > 0 {
            // Pre-allocate exact size to avoid geometric Vec growth spikes
            // when loading large ELF files in exec().
            let mut data = Vec::with_capacity(file_size);
            data.resize(file_size, 0);
            const READ_ALL_CHUNK: usize = 16 * 1024;
            while offset < file_size {
                let end = core::cmp::min(offset + READ_ALL_CHUNK, file_size);
                let n = inner.inode.read_at(offset, &mut data[offset..end]);
                if n == 0 {
                    break;
                }
                offset += n;
            }
            data.truncate(offset);
            data
        } else {
            const READ_ALL_CHUNK: usize = 4096;
            let mut data = Vec::new();
            let mut buf = [0u8; READ_ALL_CHUNK];
            loop {
                let n = inner.inode.read_at(offset, &mut buf);
                if n == 0 {
                    break;
                }
                data.extend_from_slice(&buf[..n]);
                offset += n;
            }
            data
        };
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

    fn ts_id(&self) -> Option<usize> {
        Some(self.ts_id)
    }

    fn status_flags(&self) -> u32 {
        self.status_flags
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
    let (mut readable, mut writable) = flags.read_write();
    let status_flags = flags.bits() & OpenFlags::PATH.bits();
    if flags.contains(OpenFlags::PATH) {
        readable = false;
        writable = false;
    }
    let vfs = ROOT_VFS.exclusive_access();
    if flags.contains(OpenFlags::CREATE) {
        if let Some(inode) = vfs.resolve_quiet(&path) {
            if flags.contains(OpenFlags::TRUNC) {
                inode.truncate();
            }
            return Some(Arc::new(VfsFile::new_with_flags(
                readable,
                writable,
                status_flags,
                inode,
                path,
            )));
        }
        let (parent, name) = vfs.resolve_parent(&path)?;
        let inode = parent.create(&name)?;
        Some(Arc::new(VfsFile::new_with_flags(
            readable,
            writable,
            status_flags,
            inode,
            path,
        )))
    } else {
        let inode = vfs.resolve_quiet(&path)?;
        if flags.contains(OpenFlags::TRUNC) {
            inode.truncate();
        }
        Some(Arc::new(VfsFile::new_with_flags(
            readable,
            writable,
            status_flags,
            inode,
            path,
        )))
    }
}

pub fn path_is_dir(path: &str) -> bool {
    let path = normalize_path(path);
    let vfs = ROOT_VFS.exclusive_access();
    match vfs.resolve_quiet(&path) {
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
    if vfs.resolve_quiet(&path).is_some() {
        return false;
    }
    let Some((parent, name)) = vfs.resolve_parent(&path) else {
        return false;
    };
    parent.create_dir(&name).is_some()
}

pub fn path_exists(path: &str) -> bool {
    let path = normalize_path(path);
    let vfs = ROOT_VFS.exclusive_access();
    vfs.resolve_quiet(&path).is_some()
}

pub fn remove_path(path: &str, is_dir: bool) -> bool {
    let path = normalize_path(path);
    let vfs = ROOT_VFS.exclusive_access();
    let Some((parent, name)) = vfs.resolve_parent(&path) else {
        return false;
    };
    parent.remove(&name, is_dir)
}
