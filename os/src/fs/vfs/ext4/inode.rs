use super::super::core::{VfsInode, VfsMetadata, VfsNodeKind, VfsStatFs};
use crate::sync::UPIntrFreeCell;
use alloc::ffi::CString;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use lwext4_rust::bindings::{
    ext4_atime_set, ext4_ctime_set, ext4_flink, ext4_fsymlink, ext4_getxattr, ext4_inode,
    ext4_listxattr, ext4_mknod, ext4_mode_set, ext4_mount_point_stats, ext4_mount_stats,
    ext4_mtime_set, ext4_owner_get, ext4_owner_set, ext4_raw_inode_fill, ext4_readlink,
    ext4_removexattr, ext4_setxattr, EOK, O_CREAT, O_RDONLY, O_RDWR, O_TRUNC,
};
use lwext4_rust::{Ext4File, InodeTypes};

const SEEK_SET: u32 = 0;
const ZERO_PAD_CHUNK: usize = 1024 * 1024;

pub struct Ext4Inode {
    path: String,
    kind: VfsNodeKind,
    file: UPIntrFreeCell<Option<Ext4File>>,
}

impl Ext4Inode {
    pub(super) fn new(path: String, kind: VfsNodeKind) -> Self {
        Self {
            path,
            kind,
            file: unsafe { UPIntrFreeCell::new(None) },
        }
    }

    pub(super) fn new_dir(path: String) -> Self {
        Self::new(path, VfsNodeKind::Dir)
    }

    pub(super) fn new_file(path: String) -> Self {
        Self::new(path, VfsNodeKind::File)
    }

    fn open_data_file(&self, for_write: bool) -> Option<Ext4File> {
        let mut file = ext4_open_file(
            self.path.as_str(),
            InodeTypes::EXT4_DE_REG_FILE,
            if for_write { O_RDWR | O_CREAT } else { O_RDWR },
        );
        if file.is_none() && !for_write {
            file = ext4_open_file(self.path.as_str(), InodeTypes::EXT4_DE_REG_FILE, O_RDONLY);
        }
        file
    }

    fn with_data_file<R, F>(&self, for_write: bool, mut f: F) -> Option<R>
    where
        F: FnMut(&mut Ext4File) -> Option<R>,
    {
        let mut slot = self.file.exclusive_access();
        if slot.is_none() {
            *slot = self.open_data_file(for_write);
        }
        if let Some(file) = slot.as_mut() {
            if let Some(ret) = f(file) {
                return Some(ret);
            }
        }
        if let Some(mut stale) = slot.take() {
            let _ = stale.file_close();
        }
        *slot = self.open_data_file(for_write);
        slot.as_mut().and_then(|file| f(file))
    }

    fn close_cached_file(&self) {
        let mut slot = self.file.exclusive_access();
        if let Some(mut file) = slot.take() {
            let _ = file.file_close();
        }
    }
}

impl Drop for Ext4Inode {
    fn drop(&mut self) {
        if self.kind != VfsNodeKind::File {
            return;
        }
        if let Some(mut slot) = self.file.try_exclusive_access() {
            if let Some(mut file) = slot.take() {
                let _ = file.file_close();
            }
        }
    }
}

fn ext4_path_join(base: &str, name: &str) -> String {
    if base == "/" {
        format!("/{}", name)
    } else {
        format!("{}/{}", base.trim_end_matches('/'), name)
    }
}

fn ext4_inode_exists(path: &str, kind: InodeTypes) -> bool {
    let mut file = Ext4File::new(path, kind.clone());
    file.check_inode_exist(path, kind)
}

fn ext4_dir_exists(path: &str) -> bool {
    ext4_inode_exists(path, InodeTypes::EXT4_DE_DIR)
}

fn ext4_open_file(path: &str, kind: InodeTypes, flags: u32) -> Option<Ext4File> {
    let mut file = Ext4File::new(path, kind.clone());
    if file.file_open(path, flags).is_ok() {
        return Some(file);
    }
    None
}

fn read_packed<T: Copy>(field: *const T) -> T {
    unsafe { core::ptr::read_unaligned(field) }
}

fn path_cstring(path: &str) -> Result<CString, isize> {
    CString::new(path).map_err(|_| -22)
}

fn ext4_errno(rc: i32) -> isize {
    if rc < 0 {
        rc as isize
    } else {
        -(rc as isize)
    }
}

fn ext4_mode_to_kind(mode: u32) -> VfsNodeKind {
    match mode & 0o170000 {
        0o010000 => VfsNodeKind::Fifo,
        0o020000 => VfsNodeKind::Char,
        0o040000 => VfsNodeKind::Dir,
        0o060000 => VfsNodeKind::Block,
        0o100000 => VfsNodeKind::File,
        0o120000 => VfsNodeKind::Symlink,
        0o140000 => VfsNodeKind::Socket,
        _ => VfsNodeKind::Unknown,
    }
}

fn ext4_kind_to_mknod_type(mode: u32) -> Option<i32> {
    match mode & 0o170000 {
        0o010000 => Some(InodeTypes::EXT4_DE_FIFO as i32),
        0o020000 => Some(InodeTypes::EXT4_DE_CHRDEV as i32),
        0o060000 => Some(InodeTypes::EXT4_DE_BLKDEV as i32),
        0o140000 => Some(InodeTypes::EXT4_DE_SOCK as i32),
        _ => None,
    }
}

fn ext4_raw_metadata(path: &str) -> Result<VfsMetadata, isize> {
    let c_path = path_cstring(path)?;
    let mut ino = 0u32;
    let mut raw: ext4_inode = unsafe { core::mem::zeroed() };
    let rc = unsafe { ext4_raw_inode_fill(c_path.as_ptr(), &mut ino, &mut raw) };
    if rc != EOK as i32 {
        return Err(ext4_errno(rc));
    }
    let mut uid = 0u32;
    let mut gid = 0u32;
    let rc = unsafe { ext4_owner_get(c_path.as_ptr(), &mut uid, &mut gid) };
    if rc != EOK as i32 {
        return Err(ext4_errno(rc));
    }
    let size_lo = read_packed(core::ptr::addr_of!(raw.size_lo)) as u64;
    let size_hi = read_packed(core::ptr::addr_of!(raw.size_hi)) as u64;
    let mode = read_packed(core::ptr::addr_of!(raw.mode)) as u32;
    let blocks_lo = read_packed(core::ptr::addr_of!(raw.blocks_count_lo)) as u64;
    let blocks_hi = unsafe { read_packed(core::ptr::addr_of!(raw.osd2.linux2.blocks_high)) as u64 };
    let block0 = read_packed(core::ptr::addr_of!(raw.blocks[0])) as u64;
    let block1 = read_packed(core::ptr::addr_of!(raw.blocks[1])) as u64;
    let rdev = if block0 != 0 { block0 } else { block1 };
    Ok(VfsMetadata {
        kind: ext4_mode_to_kind(mode),
        dev: 1,
        ino: ino as u64,
        mode,
        nlink: read_packed(core::ptr::addr_of!(raw.links_count)) as u32,
        uid,
        gid,
        rdev,
        size: size_lo | (size_hi << 32),
        blksize: 512,
        blocks: blocks_lo | (blocks_hi << 32),
        atime_sec: read_packed(core::ptr::addr_of!(raw.access_time)) as i64,
        mtime_sec: read_packed(core::ptr::addr_of!(raw.modification_time)) as i64,
        ctime_sec: read_packed(core::ptr::addr_of!(raw.change_inode_time)) as i64,
    })
}

fn ext4_set_time(path: &str, atime_sec: Option<i64>, mtime_sec: Option<i64>) -> Result<(), isize> {
    let c_path = path_cstring(path)?;
    if let Some(atime_sec) = atime_sec {
        let rc = unsafe { ext4_atime_set(c_path.as_ptr(), atime_sec as u32) };
        if rc != EOK as i32 {
            return Err(ext4_errno(rc));
        }
    }
    if let Some(mtime_sec) = mtime_sec {
        let rc = unsafe { ext4_mtime_set(c_path.as_ptr(), mtime_sec as u32) };
        if rc != EOK as i32 {
            return Err(ext4_errno(rc));
        }
    }
    let now = crate::timer::get_time_us() / 1_000_000;
    let rc = unsafe { ext4_ctime_set(c_path.as_ptr(), now as u32) };
    if rc != EOK as i32 {
        return Err(ext4_errno(rc));
    }
    Ok(())
}

impl VfsInode for Ext4Inode {
    fn kind(&self) -> VfsNodeKind {
        self.kind
    }

    fn read_at(&self, offset: usize, buf: &mut [u8]) -> usize {
        if self.kind != VfsNodeKind::File {
            return 0;
        }
        self.with_data_file(false, |file| {
            let size = file.file_size() as usize;
            if offset >= size {
                return Some(0);
            }
            if file.file_seek(offset as i64, SEEK_SET).is_err() {
                return None;
            }
            Some(file.file_read(buf).unwrap_or(0) as usize)
        })
        .unwrap_or_else(|| {
            error!("ext4: open/seek failed path={} (read)", self.path);
            0
        })
    }

    fn write_at(&self, offset: usize, buf: &[u8]) -> usize {
        if self.kind != VfsNodeKind::File {
            return 0;
        }
        self.with_data_file(true, |file| {
            if file.file_seek(offset as i64, SEEK_SET).is_err() {
                // lwext4 rejects SEEK_SET beyond EOF with EINVAL.
                // Extend from EOF to `offset` by writing zero chunks, then write `buf`.
                // Use reasonably large chunks so large sparse extensions (e.g. fallocate)
                // do not degenerate into tiny-byte loops.
                let size = file.file_size() as usize;
                if offset < size {
                    // offset is within file but seek failed — reopen
                    return None;
                }
                if file.file_seek(size as i64, SEEK_SET).is_err() {
                    return None;
                }
                let mut remaining = offset - size;
                let mut zeros = Vec::with_capacity(ZERO_PAD_CHUNK);
                zeros.resize(ZERO_PAD_CHUNK, 0);
                while remaining > 0 {
                    let chunk = remaining.min(zeros.len());
                    let wrote = file.file_write(&zeros[..chunk]).unwrap_or(0) as usize;
                    if wrote == 0 {
                        return None;
                    }
                    remaining -= wrote;
                }
            }
            Some(file.file_write(buf).unwrap_or(0) as usize)
        })
        .unwrap_or_else(|| {
            error!("ext4: open/seek failed path={} (write)", self.path);
            0
        })
    }

    fn lookup(&self, name: &str) -> Option<Arc<dyn VfsInode>> {
        if self.kind != VfsNodeKind::Dir {
            return None;
        }
        let path = ext4_path_join(self.path.as_str(), name);
        let metadata = ext4_raw_metadata(&path).ok()?;
        Some(Arc::new(Ext4Inode::new(path, metadata.kind)))
    }

    fn create(&self, name: &str) -> Option<Arc<dyn VfsInode>> {
        if self.kind != VfsNodeKind::Dir {
            return None;
        }
        let path = ext4_path_join(self.path.as_str(), name);
        if ext4_inode_exists(&path, InodeTypes::EXT4_DE_REG_FILE) {
            return Some(Arc::new(Ext4Inode::new_file(path)));
        }
        let mut file = Ext4File::new(path.as_str(), InodeTypes::EXT4_DE_REG_FILE);
        if file.file_open(path.as_str(), O_CREAT | O_RDWR).is_err() {
            return None;
        }
        let _ = file.file_close();
        Some(Arc::new(Ext4Inode::new_file(path)))
    }

    fn create_dir(&self, name: &str) -> Option<Arc<dyn VfsInode>> {
        if self.kind != VfsNodeKind::Dir {
            return None;
        }
        let path = ext4_path_join(self.path.as_str(), name);
        if ext4_dir_exists(&path) {
            return Some(Arc::new(Ext4Inode::new_dir(path)));
        }
        let mut dir = Ext4File::new(path.as_str(), InodeTypes::EXT4_DE_DIR);
        if dir.dir_mk(path.as_str()).is_err() {
            error!("ext4: dir_mk failed path={}", path);
            return None;
        }
        Some(Arc::new(Ext4Inode::new_dir(path)))
    }

    fn remove(&self, name: &str, is_dir: bool) -> bool {
        if self.kind != VfsNodeKind::Dir {
            return false;
        }
        let path = ext4_path_join(self.path.as_str(), name);
        if is_dir {
            let mut dir = Ext4File::new(path.as_str(), InodeTypes::EXT4_DE_DIR);
            dir.dir_rm(path.as_str()).is_ok()
        } else {
            let mut file = Ext4File::new(path.as_str(), InodeTypes::EXT4_DE_REG_FILE);
            file.file_remove(path.as_str()).is_ok()
        }
    }

    fn truncate(&self) {
        if self.kind != VfsNodeKind::File {
            return;
        }
        self.close_cached_file();
        let mut file = match ext4_open_file(
            self.path.as_str(),
            InodeTypes::EXT4_DE_REG_FILE,
            O_RDWR | O_TRUNC,
        ) {
            Some(file) => file,
            None => {
                error!("ext4: open failed path={} (truncate)", self.path);
                return;
            }
        };
        let _ = file.file_close();
    }

    fn truncate_to(&self, size: usize) {
        if self.kind != VfsNodeKind::File {
            return;
        }
        let current = self.size();
        if size == current {
            return;
        }
        if size == 0 {
            self.truncate();
            return;
        }
        if size > current {
            // Extend: write a single zero byte at offset size-1.
            // Our write_at fills the gap with zeros via SEEK_END approach.
            self.write_at(size - 1, &[0u8; 1]);
        } else {
            // Shrink: use ext4's ftruncate (works correctly for shrinking).
            self.with_data_file(true, |file| {
                file.file_truncate(size as u64).ok().map(|_| ())
            });
            // ext4_ftruncate does NOT update file->fsize in the cached ext4_file
            // struct, so close the cached file to force a fresh open on next access.
            self.close_cached_file();
        }
    }

    fn list(&self) -> Vec<String> {
        if self.kind != VfsNodeKind::Dir {
            return Vec::new();
        }
        let dir = Ext4File::new(self.path.as_str(), InodeTypes::EXT4_DE_DIR);
        let entries = dir.lwext4_dir_entries().ok().map(|(names, _)| names);
        entries
            .unwrap_or_default()
            .into_iter()
            .filter_map(|mut name| {
                while matches!(name.last(), Some(0)) {
                    name.pop();
                }
                String::from_utf8(name).ok()
            })
            .collect()
    }

    fn size(&self) -> usize {
        ext4_raw_metadata(self.path.as_str())
            .map(|metadata| metadata.size as usize)
            .unwrap_or(0)
    }

    fn metadata(&self) -> Option<VfsMetadata> {
        ext4_raw_metadata(self.path.as_str()).ok()
    }

    fn chmod(&self, mode: u32) -> Result<(), isize> {
        let c_path = path_cstring(self.path.as_str())?;
        let file_type = self
            .metadata()
            .map(|metadata| metadata.mode & 0o170000)
            .unwrap_or_else(|| match self.kind {
                VfsNodeKind::Dir => 0o040000,
                VfsNodeKind::Symlink => 0o120000,
                VfsNodeKind::Char => 0o020000,
                VfsNodeKind::Block => 0o060000,
                VfsNodeKind::Fifo => 0o010000,
                VfsNodeKind::Socket => 0o140000,
                _ => 0o100000,
            });
        let rc = unsafe { ext4_mode_set(c_path.as_ptr(), file_type | (mode & 0o7777)) };
        if rc != EOK as i32 {
            return Err(ext4_errno(rc));
        }
        ext4_set_time(self.path.as_str(), None, None)
    }

    fn chown(&self, uid: Option<u32>, gid: Option<u32>) -> Result<(), isize> {
        let metadata = ext4_raw_metadata(self.path.as_str())?;
        let c_path = path_cstring(self.path.as_str())?;
        let rc = unsafe {
            ext4_owner_set(
                c_path.as_ptr(),
                uid.unwrap_or(metadata.uid),
                gid.unwrap_or(metadata.gid),
            )
        };
        if rc != EOK as i32 {
            return Err(ext4_errno(rc));
        }
        ext4_set_time(self.path.as_str(), None, None)
    }

    fn utimens(&self, atime_sec: Option<i64>, mtime_sec: Option<i64>) -> Result<(), isize> {
        ext4_set_time(self.path.as_str(), atime_sec, mtime_sec)
    }

    fn link_to(&self, new_path: &str) -> Result<(), isize> {
        let old_c = path_cstring(self.path.as_str())?;
        let new_c = path_cstring(new_path)?;
        let rc = unsafe { ext4_flink(old_c.as_ptr(), new_c.as_ptr()) };
        if rc != EOK as i32 {
            return Err(ext4_errno(rc));
        }
        Ok(())
    }

    fn symlink(&self, name: &str, target: &str) -> Result<Arc<dyn VfsInode>, isize> {
        if self.kind != VfsNodeKind::Dir {
            return Err(-20);
        }
        let path = ext4_path_join(self.path.as_str(), name);
        let target_c = path_cstring(target)?;
        let path_c = path_cstring(path.as_str())?;
        let rc = unsafe { ext4_fsymlink(target_c.as_ptr(), path_c.as_ptr()) };
        if rc != EOK as i32 {
            return Err(ext4_errno(rc));
        }
        Ok(Arc::new(Ext4Inode::new(path, VfsNodeKind::Symlink)))
    }

    fn readlink(&self) -> Result<Vec<u8>, isize> {
        let c_path = path_cstring(self.path.as_str())?;
        let mut buf = Vec::new();
        buf.resize(4096, 0);
        let mut read = 0usize;
        let rc = unsafe {
            ext4_readlink(
                c_path.as_ptr(),
                buf.as_mut_ptr() as *mut core::ffi::c_char,
                buf.len(),
                &mut read,
            )
        };
        if rc != EOK as i32 {
            return Err(ext4_errno(rc));
        }
        buf.truncate(read);
        Ok(buf)
    }

    fn mknod(&self, name: &str, mode: u32, dev: u32) -> Result<Arc<dyn VfsInode>, isize> {
        if self.kind != VfsNodeKind::Dir {
            return Err(-20);
        }
        let filetype = ext4_kind_to_mknod_type(mode).ok_or(-22isize)?;
        let path = ext4_path_join(self.path.as_str(), name);
        let c_path = path_cstring(path.as_str())?;
        let rc = unsafe { ext4_mknod(c_path.as_ptr(), filetype, dev) };
        if rc != EOK as i32 {
            return Err(ext4_errno(rc));
        }
        let inode = Arc::new(Ext4Inode::new(
            path,
            ext4_mode_to_kind(mode),
        ));
        let _ = inode.chmod(mode & 0o7777);
        Ok(inode)
    }

    fn setxattr(&self, name: &str, value: &[u8]) -> Result<(), isize> {
        let c_path = path_cstring(self.path.as_str())?;
        let c_name = path_cstring(name)?;
        let rc = unsafe {
            ext4_setxattr(
                c_path.as_ptr(),
                c_name.as_ptr(),
                name.len(),
                value.as_ptr() as *const core::ffi::c_void,
                value.len(),
            )
        };
        if rc != EOK as i32 {
            return Err(ext4_errno(rc));
        }
        Ok(())
    }

    fn getxattr(&self, name: &str) -> Result<Vec<u8>, isize> {
        let c_path = path_cstring(self.path.as_str())?;
        let c_name = path_cstring(name)?;
        let mut size = 0usize;
        let rc = unsafe {
            ext4_getxattr(
                c_path.as_ptr(),
                c_name.as_ptr(),
                name.len(),
                core::ptr::null_mut(),
                0,
                &mut size,
            )
        };
        if rc != EOK as i32 {
            return Err(ext4_errno(rc));
        }
        let mut data = Vec::new();
        data.resize(size, 0);
        let rc = unsafe {
            ext4_getxattr(
                c_path.as_ptr(),
                c_name.as_ptr(),
                name.len(),
                data.as_mut_ptr() as *mut core::ffi::c_void,
                data.len(),
                &mut size,
            )
        };
        if rc != EOK as i32 {
            return Err(ext4_errno(rc));
        }
        data.truncate(size);
        Ok(data)
    }

    fn listxattr(&self) -> Result<Vec<u8>, isize> {
        let c_path = path_cstring(self.path.as_str())?;
        let mut size = 0usize;
        let rc = unsafe { ext4_listxattr(c_path.as_ptr(), core::ptr::null_mut(), 0, &mut size) };
        if rc != EOK as i32 {
            return Err(ext4_errno(rc));
        }
        let mut list = Vec::new();
        list.resize(size, 0);
        let rc = unsafe {
            ext4_listxattr(
                c_path.as_ptr(),
                list.as_mut_ptr() as *mut core::ffi::c_char,
                list.len(),
                &mut size,
            )
        };
        if rc != EOK as i32 {
            return Err(ext4_errno(rc));
        }
        list.truncate(size);
        Ok(list)
    }

    fn removexattr(&self, name: &str) -> Result<(), isize> {
        let c_path = path_cstring(self.path.as_str())?;
        let c_name = path_cstring(name)?;
        let rc = unsafe { ext4_removexattr(c_path.as_ptr(), c_name.as_ptr(), name.len()) };
        if rc != EOK as i32 {
            return Err(ext4_errno(rc));
        }
        Ok(())
    }

    fn statfs(&self) -> Option<VfsStatFs> {
        let c_mount = path_cstring("/").ok()?;
        let mut stats: ext4_mount_stats = unsafe { core::mem::zeroed() };
        let rc = unsafe { ext4_mount_point_stats(c_mount.as_ptr(), &mut stats) };
        if rc != EOK as i32 {
            return None;
        }
        Some(VfsStatFs {
            f_type: 0xEF53,
            f_bsize: stats.block_size as i64,
            f_blocks: stats.blocks_count,
            f_bfree: stats.free_blocks_count,
            f_bavail: stats.free_blocks_count,
            f_files: stats.inodes_count as u64,
            f_ffree: stats.free_inodes_count as u64,
            f_namelen: 255,
            f_frsize: stats.block_size as i64,
            f_flags: 0,
        })
    }
}
