//! In-memory anonymous file for memfd_create(2).
use super::vfs::{VfsInode, VfsNodeKind};
use super::{File, PollEvents};
use crate::mm::UserBuffer;
use crate::sync::{UPIntrMutex, UPIntrRwLock};
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

struct MemFdData {
    buf: Vec<u8>,
    size: usize,
    seals: u32,
    allow_sealing: bool,
}

/// VfsInode backed by an in-memory buffer (anonymous file).
pub struct MemFdInode {
    inner: UPIntrRwLock<MemFdData>,
}

impl MemFdInode {
    /// Create an empty memfd inode.
    pub fn new(allow_sealing: bool) -> Self {
        Self {
            inner: unsafe {
                UPIntrRwLock::new(MemFdData {
                    buf: Vec::new(),
                    size: 0,
                    seals: if allow_sealing { 0 } else { 0x0001 },
                    allow_sealing,
                })
            },
        }
    }

    fn write_data_at(data: &mut MemFdData, offset: usize, buf: &[u8]) -> usize {
        if (data.seals & 0x0008) != 0 {
            return 0;
        }
        let end = offset + buf.len();
        if end > data.buf.len() {
            data.buf.resize(end, 0);
        }
        data.buf[offset..end].copy_from_slice(buf);
        if end > data.size {
            data.size = end;
        }
        buf.len()
    }
}

impl VfsInode for MemFdInode {
    fn kind(&self) -> VfsNodeKind {
        VfsNodeKind::File
    }

    fn read_at(&self, offset: usize, buf: &mut [u8]) -> usize {
        let inner = self.inner.read();
        if offset >= inner.size {
            return 0;
        }
        let end = (offset + buf.len()).min(inner.size);
        let n = end - offset;
        buf[..n].copy_from_slice(&inner.buf[offset..end]);
        n
    }

    fn write_at(&self, offset: usize, buf: &[u8]) -> usize {
        let mut inner = self.inner.write();
        Self::write_data_at(&mut inner, offset, buf)
    }

    fn lookup(&self, _name: &str) -> Option<Arc<dyn VfsInode>> {
        None
    }
    fn create(&self, _name: &str) -> Option<Arc<dyn VfsInode>> {
        None
    }
    fn truncate(&self) {
        let mut inner = self.inner.write();
        inner.buf.clear();
        inner.size = 0;
    }
    fn list(&self) -> Vec<String> {
        Vec::new()
    }
    fn size(&self) -> usize {
        self.inner.read().size
    }
}

/// Seekable in-memory file descriptor returned by memfd_create(2).
pub struct MemFdFile {
    inode: Arc<MemFdInode>,
    offset: UPIntrMutex<usize>,
}

impl MemFdFile {
    /// Create a new empty memfd file.
    pub fn new(allow_sealing: bool) -> Self {
        Self {
            inode: Arc::new(MemFdInode::new(allow_sealing)),
            offset: unsafe { UPIntrMutex::new(0) },
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
        let mut off = self.offset.lock();
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
        let mut off = self.offset.lock();
        let mut inner = self.inode.inner.write();
        if (inner.seals & 0x0008) != 0 {
            return 0;
        }
        let mut total = 0;
        for slice in buf.buffers.iter() {
            let n = MemFdInode::write_data_at(&mut inner, *off, slice);
            *off += n;
            total += n;
        }
        total
    }

    fn write_user_buffer(&self, buf: UserBuffer) -> Result<usize, isize> {
        if (self.inode.inner.read().seals & 0x0008) != 0 {
            return Err(-1);
        }
        Ok(self.write(buf))
    }

    fn inode(&self) -> Option<Arc<dyn VfsInode>> {
        Some(self.inode.clone() as Arc<dyn VfsInode>)
    }

    fn get_offset(&self) -> Option<usize> {
        Some(*self.offset.lock())
    }

    fn set_offset(&self, offset: usize) {
        *self.offset.lock() = offset;
    }

    fn get_seals(&self) -> Option<u32> {
        Some(self.inode.inner.read().seals)
    }

    fn add_seals(&self, seals: u32) -> isize {
        let mut inner = self.inode.inner.write();
        if !inner.allow_sealing || (inner.seals & 0x0001) != 0 {
            return -1; // EPERM
        }
        inner.seals |= seals;
        0
    }

    fn poll(&self, _events: PollEvents) -> PollEvents {
        PollEvents::POLLIN | PollEvents::POLLOUT
    }
}
