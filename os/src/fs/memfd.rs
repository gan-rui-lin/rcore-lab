//! In-memory anonymous file for memfd_create(2).
use super::vfs::{VfsInode, VfsNodeKind};
use super::{File, PollEvents};
use crate::mm::UserBuffer;
use crate::sync::UPIntrFreeCell;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

struct MemFdInner {
    buf: Vec<u8>,
    size: usize,
}

/// VfsInode backed by an in-memory buffer (anonymous file).
pub struct MemFdInode {
    inner: UPIntrFreeCell<MemFdInner>,
}

impl MemFdInode {
    /// Create an empty memfd inode.
    pub fn new() -> Self {
        Self {
            inner: unsafe { UPIntrFreeCell::new(MemFdInner { buf: Vec::new(), size: 0 }) },
        }
    }
}

impl VfsInode for MemFdInode {
    fn kind(&self) -> VfsNodeKind {
        VfsNodeKind::File
    }

    fn read_at(&self, offset: usize, buf: &mut [u8]) -> usize {
        let inner = self.inner.exclusive_access();
        if offset >= inner.size {
            return 0;
        }
        let end = (offset + buf.len()).min(inner.size);
        let n = end - offset;
        buf[..n].copy_from_slice(&inner.buf[offset..end]);
        n
    }

    fn write_at(&self, offset: usize, buf: &[u8]) -> usize {
        let mut inner = self.inner.exclusive_access();
        let end = offset + buf.len();
        if end > inner.buf.len() {
            inner.buf.resize(end, 0);
        }
        inner.buf[offset..end].copy_from_slice(buf);
        if end > inner.size {
            inner.size = end;
        }
        buf.len()
    }

    fn lookup(&self, _name: &str) -> Option<Arc<dyn VfsInode>> {
        None
    }
    fn create(&self, _name: &str) -> Option<Arc<dyn VfsInode>> {
        None
    }
    fn truncate(&self) {
        let mut inner = self.inner.exclusive_access();
        inner.buf.clear();
        inner.size = 0;
    }
    fn list(&self) -> Vec<String> {
        Vec::new()
    }
    fn size(&self) -> usize {
        self.inner.exclusive_access().size
    }
}

/// Seekable in-memory file descriptor returned by memfd_create(2).
pub struct MemFdFile {
    inode: Arc<MemFdInode>,
    offset: UPIntrFreeCell<usize>,
}

impl MemFdFile {
    /// Create a new empty memfd file.
    pub fn new() -> Self {
        Self {
            inode: Arc::new(MemFdInode::new()),
            offset: unsafe { UPIntrFreeCell::new(0) },
        }
    }
}

impl File for MemFdFile {
    fn readable(&self) -> bool {
        true
    }
    fn writable(&self) -> bool {
        true
    }

    fn read(&self, buf: UserBuffer) -> usize {
        let mut off = self.offset.exclusive_access();
        let mut total = 0;
        for slice in buf.buffers.iter() {
            let n = self.inode.read_at(*off, unsafe {
                core::slice::from_raw_parts_mut(slice.as_ptr() as *mut u8, slice.len())
            });
            *off += n;
            total += n;
            if n < slice.len() {
                break;
            }
        }
        total
    }

    fn write(&self, buf: UserBuffer) -> usize {
        let mut off = self.offset.exclusive_access();
        let mut total = 0;
        for slice in buf.buffers.iter() {
            let n = self.inode.write_at(*off, slice);
            *off += n;
            total += n;
        }
        total
    }

    fn inode(&self) -> Option<Arc<dyn VfsInode>> {
        Some(self.inode.clone() as Arc<dyn VfsInode>)
    }

    fn get_offset(&self) -> Option<usize> {
        Some(*self.offset.exclusive_access())
    }

    fn set_offset(&self, offset: usize) {
        *self.offset.exclusive_access() = offset;
    }

    fn poll(&self, _events: PollEvents) -> PollEvents {
        PollEvents::POLLIN | PollEvents::POLLOUT
    }
}
