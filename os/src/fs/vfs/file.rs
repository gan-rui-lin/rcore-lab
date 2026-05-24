#![allow(missing_docs)]

use super::super::{File, OpenFlags};
use super::core::{normalize_path, with_root_vfs_read, VfsInode, VfsNodeKind};
use crate::mm::UserBuffer;
use crate::sync::UPIntrMutex;
use crate::syscall::user_mem::{self, UserWritePolicy};
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

pub struct VfsFile {
    readable: bool,
    writable: bool,
    status_flags: u32,
    path: String,
    ts_id: usize,
    inode: Arc<dyn VfsInode>,
    offset: UPIntrMutex<usize>,
}

/// Monotonic counter for timestamp tracking IDs.
static NEXT_TS_ID: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(1);

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
            inode,
            offset: unsafe { UPIntrMutex::new(0) },
        }
    }

    pub fn read_at(&self, offset: usize, buf: &mut [u8]) -> usize {
        self.inode.read_at(offset, buf)
    }

    pub fn write_at(&self, offset: usize, buf: &[u8]) -> usize {
        self.inode.write_at(offset, buf)
    }

    pub fn read_all(&self) -> Vec<u8> {
        let file_size = self.inode.size();
        let mut offset = 0usize;
        let out = if file_size > 0 {
            // Pre-allocate exact size to avoid geometric Vec growth spikes
            // when loading large ELF files in exec().
            let mut data = Vec::with_capacity(file_size);
            data.resize(file_size, 0);
            const READ_ALL_CHUNK: usize = 16 * 1024;
            while offset < file_size {
                let end = core::cmp::min(offset + READ_ALL_CHUNK, file_size);
                let n = self.read_at(offset, &mut data[offset..end]);
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
                let n = self.read_at(offset, &mut buf);
                if n == 0 {
                    break;
                }
                data.extend_from_slice(&buf[..n]);
                offset += n;
            }
            data
        };
        *self.offset.lock() = offset;
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
        let mut offset = self.offset.lock();
        let mut total = 0usize;
        for slice in buf.buffers.iter_mut() {
            let n = self.inode.read_at(*offset, *slice);
            if n == 0 {
                break;
            }
            *offset += n;
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
        let mut offset = self.offset.lock();
        let mut total = 0usize;
        let result = user_mem::for_each_user_write_slice(
            token,
            ptr,
            len,
            UserWritePolicy::DemandCowWithForkFallback,
            |slice| {
                let n = self.inode.read_at(*offset, slice);
                if n == 0 {
                    return Ok(0);
                }
                *offset += n;
                total += n;
                Ok(n)
            },
        );
        Some(result.map(|_| total))
    }

    fn write(&self, buf: UserBuffer) -> usize {
        let mut offset = self.offset.lock();
        if (self.status_flags & OpenFlags::APPEND.bits()) != 0 {
            // O_APPEND: each write starts at current EOF.
            *offset = self.inode.size();
        }
        let mut total = 0usize;
        for slice in buf.buffers.iter() {
            let n = self.inode.write_at(*offset, *slice);
            if n == 0 {
                break;
            }
            *offset += n;
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
        Some(self.inode.clone())
    }

    fn path(&self) -> Option<&str> {
        Some(self.path.as_str())
    }

    fn get_offset(&self) -> Option<usize> {
        Some(*self.offset.lock())
    }

    fn set_offset(&self, offset: usize) {
        *self.offset.lock() = offset;
    }

    fn ts_id(&self) -> Option<usize> {
        Some(self.ts_id)
    }

    fn status_flags(&self) -> u32 {
        self.status_flags
    }

    fn read_at_kernel(&self, offset: usize, buf: &mut [u8]) -> usize {
        self.inode.read_at(offset, buf)
    }
}

pub fn list_apps() {
    debug!("/**** APPS ****");
    if let Some(root) = with_root_vfs_read(|vfs| vfs.root_inode()) {
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
    let inode = with_root_vfs_read(|vfs| {
        if flags.contains(OpenFlags::CREATE) {
            if let Some(inode) = vfs.resolve_quiet(&path) {
                return Some(inode);
            }
            let (parent, name) = vfs.resolve_parent(&path)?;
            parent.create(&name)
        } else {
            vfs.resolve_quiet(&path)
        }
    })?;
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

pub fn path_is_dir(path: &str) -> bool {
    let path = normalize_path(path);
    match with_root_vfs_read(|vfs| vfs.resolve_quiet(&path)) {
        Some(inode) => inode.kind() == VfsNodeKind::Dir,
        None => {
            trace!("vfs: path_is_dir not found {}", path);
            false
        }
    }
}

pub fn create_dir(path: &str) -> bool {
    let path = normalize_path(path);
    if with_root_vfs_read(|vfs| vfs.resolve_quiet(&path)).is_some() {
        return false;
    }
    let Some((parent, name)) = with_root_vfs_read(|vfs| vfs.resolve_parent(&path)) else {
        return false;
    };
    parent.create_dir(&name).is_some()
}

pub fn path_exists(path: &str) -> bool {
    let path = normalize_path(path);
    with_root_vfs_read(|vfs| vfs.resolve_quiet(&path)).is_some()
}

pub fn remove_path(path: &str, is_dir: bool) -> bool {
    let path = normalize_path(path);
    let Some((parent, name)) = with_root_vfs_read(|vfs| vfs.resolve_parent(&path)) else {
        return false;
    };
    parent.remove(&name, is_dir)
}
