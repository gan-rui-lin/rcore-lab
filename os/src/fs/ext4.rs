#![cfg(feature = "ext4")]
#![allow(missing_docs)]

use crate::sync::UPSafeCell;
use alloc::ffi::CString;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use easy_fs::BlockDevice;
use lwext4_rust::bindings::{
    ext4_dir, ext4_dir_close, ext4_dir_open, EOK, O_CREAT, O_RDONLY, O_RDWR, O_TRUNC,
};
use lwext4_rust::{Ext4BlockWrapper, Ext4File, InodeTypes, KernelDevOp};

use super::vfs::{VfsInode, VfsNodeKind};

const BLOCK_SIZE: usize = 512;
const TRACE_DISK: bool = option_env!("TRACE_DISK").is_some();
const SEEK_SET: u32 = 0;
const SEEK_CUR: u32 = 1;
const SEEK_END: u32 = 2;

struct Ext4Disk {
    block_id: usize,
    offset: usize,
    device: Arc<dyn BlockDevice>,
    total_bytes: i64,
}

impl Ext4Disk {
    pub fn new(device: Arc<dyn BlockDevice>, total_bytes: i64) -> Self {
        Self {
            block_id: 0,
            offset: 0,
            device,
            total_bytes,
        }
    }

    fn size(&self) -> i64 {
        self.total_bytes
    }

    fn position(&self) -> i64 {
        (self.block_id * BLOCK_SIZE + self.offset) as i64
    }

    fn set_position(&mut self, pos: i64) {
        let pos = core::cmp::max(0, pos) as usize;
        self.block_id = pos / BLOCK_SIZE;
        self.offset = pos % BLOCK_SIZE;
    }

    fn read_one(&mut self, buf: &mut [u8]) -> Result<usize, i32> {
        if TRACE_DISK {
            trace!(
                "ext4: disk read block={} off={} len={}",
                self.block_id,
                self.offset,
                buf.len()
            );
        }
        let read_size = if self.offset == 0 && buf.len() >= BLOCK_SIZE {
            self.device.read_block(self.block_id, &mut buf[..BLOCK_SIZE]);
            self.block_id += 1;
            BLOCK_SIZE
        } else {
            let mut data = [0u8; BLOCK_SIZE];
            let start = self.offset;
            let count = buf.len().min(BLOCK_SIZE - self.offset);
            self.device.read_block(self.block_id, &mut data);
            buf[..count].copy_from_slice(&data[start..start + count]);
            self.offset += count;
            if self.offset >= BLOCK_SIZE {
                self.block_id += 1;
                self.offset -= BLOCK_SIZE;
            }
            count
        };
        Ok(read_size)
    }

    fn write_one(&mut self, buf: &[u8]) -> Result<usize, i32> {
        if TRACE_DISK {
            trace!(
                "ext4: disk write block={} off={} len={}",
                self.block_id,
                self.offset,
                buf.len()
            );
        }
        let write_size = if self.offset == 0 && buf.len() >= BLOCK_SIZE {
            self.device.write_block(self.block_id, &buf[..BLOCK_SIZE]);
            self.block_id += 1;
            BLOCK_SIZE
        } else {
            let mut data = [0u8; BLOCK_SIZE];
            let start = self.offset;
            let count = buf.len().min(BLOCK_SIZE - self.offset);
            self.device.read_block(self.block_id, &mut data);
            data[start..start + count].copy_from_slice(&buf[..count]);
            self.device.write_block(self.block_id, &data);
            self.offset += count;
            if self.offset >= BLOCK_SIZE {
                self.block_id += 1;
                self.offset -= BLOCK_SIZE;
            }
            count
        };
        Ok(write_size)
    }
}

impl KernelDevOp for Ext4Disk {
    type DevType = Ext4Disk;

    fn write(dev: &mut Self::DevType, buf: &[u8]) -> Result<usize, i32> {
        let mut write_len = 0;
        let mut remaining = buf;
        while !remaining.is_empty() {
            match dev.write_one(remaining) {
                Ok(0) => break,
                Ok(n) => {
                    remaining = &remaining[n..];
                    write_len += n;
                }
                Err(_) => return Err(-1),
            }
        }
        Ok(write_len)
    }

    fn read(dev: &mut Self::DevType, buf: &mut [u8]) -> Result<usize, i32> {
        let mut read_len = 0;
        let mut remaining = buf;
        while !remaining.is_empty() {
            match dev.read_one(remaining) {
                Ok(0) => break,
                Ok(n) => {
                    let tmp = remaining;
                    remaining = &mut tmp[n..];
                    read_len += n;
                }
                Err(_) => return Err(-1),
            }
        }
        Ok(read_len)
    }

    fn seek(dev: &mut Self::DevType, off: i64, whence: i32) -> Result<i64, i32> {
        let new_pos = match whence as u32 {
            SEEK_SET => Some(off),
            SEEK_CUR => dev.position().checked_add(off),
            SEEK_END => dev.size().checked_add(off),
            _ => None,
        }
        .ok_or(-1)?;
        dev.set_position(new_pos);
        Ok(dev.position())
    }

    fn flush(_dev: &mut Self::DevType) -> Result<usize, i32> {
        Ok(0)
    }
}

pub(crate) struct Ext4Fs {
    _inner: UPSafeCell<Ext4BlockWrapper<Ext4Disk>>,
}

impl Ext4Fs {
    pub fn new(device: Arc<dyn BlockDevice>, total_bytes: i64) -> Result<Self, i32> {
        let wrapper = Ext4BlockWrapper::<Ext4Disk>::new_with_mount(
            Ext4Disk::new(device, total_bytes),
            "/",
            "ext4_fs0",
        )?;
        Ok(Self {
            _inner: unsafe { UPSafeCell::new(wrapper) },
        })
    }

    pub fn root_inode(&self) -> Arc<dyn VfsInode> {
        Arc::new(Ext4Inode::new_dir(String::from("/")))
    }
}

pub struct Ext4Inode {
    path: String,
    kind: VfsNodeKind,
}

impl Ext4Inode {
    fn new(path: String, kind: VfsNodeKind) -> Self {
        Self { path, kind }
    }

    fn new_dir(path: String) -> Self {
        Self::new(path, VfsNodeKind::Dir)
    }

    fn new_file(path: String) -> Self {
        Self::new(path, VfsNodeKind::File)
    }
}

fn ext4_path_join(base: &str, name: &str) -> String {
    if base == "/" {
        format!("/{}", name)
    } else {
        format!("{}/{}", base.trim_end_matches('/'), name)
    }
}

fn ext4_dir_exists(path: &str) -> bool {
    let mut try_paths = [path, ""];
    if let Some(stripped) = path.strip_prefix('/') {
        try_paths[1] = stripped;
    }
    for candidate in try_paths.iter().filter(|p| !p.is_empty()) {
        let c_path = match CString::new(*candidate) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let mut dir: ext4_dir = unsafe { core::mem::zeroed() };
        let rc = unsafe { ext4_dir_open(&mut dir, c_path.as_ptr()) };
        if rc == EOK as i32 {
            unsafe {
                ext4_dir_close(&mut dir);
            }
            trace!("ext4: dir exists path={}", candidate);
            return true;
        }
        trace!("ext4: dir_open failed path={} rc={}", candidate, rc);
    }
    false
}

fn ext4_file_exists(path: &str) -> bool {
    let mut try_paths = [path, ""];
    if let Some(stripped) = path.strip_prefix('/') {
        try_paths[1] = stripped;
    }
    for candidate in try_paths.iter().filter(|p| !p.is_empty()) {
        let mut file = Ext4File::new(candidate, InodeTypes::EXT4_DE_REG_FILE);
        if file.file_open(candidate, O_RDONLY).is_ok() {
            let _ = file.file_close();
            trace!("ext4: file exists path={}", candidate);
            return true;
        }
        trace!("ext4: file_open failed path={}", candidate);
    }
    false
}

impl VfsInode for Ext4Inode {
    fn kind(&self) -> VfsNodeKind {
        self.kind
    }

    fn read_at(&self, offset: usize, buf: &mut [u8]) -> usize {
        if self.kind != VfsNodeKind::File {
            return 0;
        }
        let mut file = Ext4File::new(&self.path, InodeTypes::EXT4_DE_REG_FILE);
        if file.file_open(&self.path, O_RDONLY).is_err() {
            return 0;
        }
        let _ = file.file_seek(offset as i64, SEEK_SET);
        let n = file.file_read(buf).unwrap_or(0);
        let _ = file.file_close();
        n
    }

    fn write_at(&self, offset: usize, buf: &[u8]) -> usize {
        if self.kind != VfsNodeKind::File {
            return 0;
        }
        let mut file = Ext4File::new(&self.path, InodeTypes::EXT4_DE_REG_FILE);
        if file.file_open(&self.path, O_RDWR).is_err() {
            return 0;
        }
        let _ = file.file_seek(offset as i64, SEEK_SET);
        let n = file.file_write(buf).unwrap_or(0);
        let _ = file.file_close();
        n
    }

    fn lookup(&self, name: &str) -> Option<Arc<dyn VfsInode>> {
        if self.kind != VfsNodeKind::Dir {
            return None;
        }
        trace!("ext4: lookup base={} child={}", self.path, name);
        let child = ext4_path_join(&self.path, name);
        if ext4_dir_exists(&child) {
            Some(Arc::new(Ext4Inode::new_dir(child)) as Arc<dyn VfsInode>)
        } else if ext4_file_exists(&child) {
            Some(Arc::new(Ext4Inode::new_file(child)) as Arc<dyn VfsInode>)
        } else {
            None
        }
    }

    fn create(&self, name: &str) -> Option<Arc<dyn VfsInode>> {
        if self.kind != VfsNodeKind::Dir {
            return None;
        }
        let child = ext4_path_join(&self.path, name);
        let mut file = Ext4File::new(&child, InodeTypes::EXT4_DE_REG_FILE);
        if file.file_open(&child, O_CREAT | O_RDWR).is_ok() {
            let _ = file.file_close();
            Some(Arc::new(Ext4Inode::new_file(child)) as Arc<dyn VfsInode>)
        } else {
            None
        }
    }

    fn truncate(&self) {
        if self.kind != VfsNodeKind::File {
            return;
        }
        let mut file = Ext4File::new(&self.path, InodeTypes::EXT4_DE_REG_FILE);
        if file.file_open(&self.path, O_TRUNC | O_RDWR).is_ok() {
            let _ = file.file_close();
        }
    }

    fn list(&self) -> Vec<String> {
        if self.kind != VfsNodeKind::Dir {
            return Vec::new();
        }
        let dir = Ext4File::new(&self.path, InodeTypes::EXT4_DE_DIR);
        let entries = dir.lwext4_dir_entries();
        match entries {
            Ok((names, _types)) => names
                .iter()
                .filter_map(|name| {
                    let end = name.iter().position(|b| *b == 0).unwrap_or(name.len());
                    core::str::from_utf8(&name[..end]).ok().map(String::from)
                })
                .collect(),
            Err(_) => Vec::new(),
        }
    }
}
