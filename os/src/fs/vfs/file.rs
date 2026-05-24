#![allow(missing_docs)]

use super::super::{File, OpenFlags};
use super::core::{normalize_path, VfsInode, VfsNodeKind, ROOT_VFS};
use crate::mm::UserBuffer;
use crate::sync::UPIntrFreeCell;
use crate::syscall::user_mem::{self, UserWritePolicy};
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

const SMALL_WRITE_BUFFER_SIZE: usize = 32 * 1024;

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
    write_buf_start: usize,
    write_buf: Vec<u8>,
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
            inner: unsafe {
                UPIntrFreeCell::new(VfsFileInner {
                    offset: 0,
                    inode,
                    write_buf_start: 0,
                    write_buf: Vec::new(),
                })
            },
        }
    }

    fn flush_inner(inner: &mut VfsFileInner) -> usize {
        if inner.write_buf.is_empty() {
            return 0;
        }
        let start = inner.write_buf_start;
        let data = core::mem::take(&mut inner.write_buf);
        let written = inner.inode.write_at(start, data.as_slice());
        if written < data.len() {
            inner.write_buf_start = start + written;
            inner.write_buf.extend_from_slice(&data[written..]);
        }
        written
    }

    fn write_slice(inner: &mut VfsFileInner, slice: &[u8], buffered: bool) -> usize {
        if !buffered {
            Self::flush_inner(inner);
            let n = inner.inode.write_at(inner.offset, slice);
            inner.offset += n;
            return n;
        }

        let mut total = 0usize;
        while total < slice.len() {
            if inner.write_buf.is_empty() {
                inner.write_buf_start = inner.offset;
            }
            let expected = inner.write_buf_start + inner.write_buf.len();
            if inner.offset != expected {
                Self::flush_inner(inner);
                inner.write_buf_start = inner.offset;
            }
            if inner.write_buf.is_empty() && slice.len() - total >= SMALL_WRITE_BUFFER_SIZE {
                let n = inner.inode.write_at(inner.offset, &slice[total..]);
                inner.offset += n;
                total += n;
                if n == 0 {
                    break;
                }
                continue;
            }
            let space = SMALL_WRITE_BUFFER_SIZE - inner.write_buf.len();
            let n = core::cmp::min(space, slice.len() - total);
            inner.write_buf.extend_from_slice(&slice[total..total + n]);
            inner.offset += n;
            total += n;
            if inner.write_buf.len() == SMALL_WRITE_BUFFER_SIZE {
                Self::flush_inner(inner);
                if !inner.write_buf.is_empty() {
                    break;
                }
            }
        }
        total
    }

    pub fn read_all(&self) -> Vec<u8> {
        let mut inner = self.inner.exclusive_access();
        Self::flush_inner(&mut inner);
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
        Self::flush_inner(&mut inner);
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

    fn read_user_buffer(
        &self,
        token: usize,
        ptr: *const u8,
        len: usize,
    ) -> Option<Result<usize, isize>> {
        let mut inner = self.inner.exclusive_access();
        Self::flush_inner(&mut inner);
        let mut total = 0usize;
        let result = user_mem::for_each_user_write_slice(
            token,
            ptr,
            len,
            UserWritePolicy::DemandCowWithForkFallback,
            |slice| {
                let n = inner.inode.read_at(inner.offset, slice);
                if n == 0 {
                    return Ok(0);
                }
                inner.offset += n;
                total += n;
                Ok(n)
            },
        );
        Some(result.map(|_| total))
    }

    fn write(&self, buf: UserBuffer) -> usize {
        let mut inner = self.inner.exclusive_access();
        if (self.status_flags & OpenFlags::APPEND.bits()) != 0 {
            // O_APPEND: each write starts at current EOF.
            inner.offset = if inner.write_buf.is_empty() {
                inner.inode.size()
            } else {
                inner.write_buf_start + inner.write_buf.len()
            };
        }
        let buffered = (self.status_flags & OpenFlags::DIRECT.bits()) == 0;
        let mut total = 0usize;
        for slice in buf.buffers.iter() {
            let n = Self::write_slice(&mut inner, *slice, buffered);
            if n == 0 {
                break;
            }
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
        let mut inner = self.inner.exclusive_access();
        Self::flush_inner(&mut inner);
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
        Self::flush_inner(&mut inner);
        inner.offset = offset;
    }

    fn ts_id(&self) -> Option<usize> {
        Some(self.ts_id)
    }

    fn status_flags(&self) -> u32 {
        self.status_flags
    }

    fn read_at_kernel(&self, offset: usize, buf: &mut [u8]) -> usize {
        let mut inner = self.inner.exclusive_access();
        Self::flush_inner(&mut inner);
        inner.inode.read_at(offset, buf)
    }

    fn flush(&self) -> usize {
        let mut inner = self.inner.exclusive_access();
        Self::flush_inner(&mut inner)
    }
}

impl Drop for VfsFile {
    fn drop(&mut self) {
        if let Some(mut inner) = self.inner.try_exclusive_access() {
            Self::flush_inner(&mut inner);
        }
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
    let status_flags =
        flags.bits() & (OpenFlags::PATH | OpenFlags::APPEND | OpenFlags::DIRECT).bits();
    if flags.contains(OpenFlags::PATH) {
        readable = false;
        writable = false;
    }
    let vfs = ROOT_VFS.exclusive_access();
    if flags.contains(OpenFlags::CREATE) {
        if let Some(inode) = vfs.resolve_quiet(&path) {
            if flags.contains(OpenFlags::TRUNC) {
                inode.truncate();
                crate::mm::invalidate_shared_file_pages_by_path(path.as_str());
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
        // Keep O_CREAT|O_TRUNC semantics even if create() returns an
        // existing inode (some backends may take that path).
        if flags.contains(OpenFlags::TRUNC) {
            inode.truncate();
            crate::mm::invalidate_shared_file_pages_by_path(path.as_str());
        }
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
            crate::mm::invalidate_shared_file_pages_by_path(path.as_str());
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
