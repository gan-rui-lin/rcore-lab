use super::super::core::{VfsInode, VfsNodeKind};
use crate::sync::UPIntrFreeCell;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use lwext4_rust::bindings::{O_CREAT, O_RDONLY, O_RDWR, O_TRUNC};
use lwext4_rust::{Ext4File, InodeTypes};

const SEEK_SET: u32 = 0;

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
    let mut try_paths = [path, ""];
    if let Some(stripped) = path.strip_prefix('/') {
        try_paths[1] = stripped;
    }
    for candidate in try_paths.iter().filter(|p| !p.is_empty()) {
        let mut file = Ext4File::new(candidate, kind.clone());
        if file.check_inode_exist(candidate, kind.clone()) {
            trace!("ext4: inode exists kind={:?} path={}", kind, candidate);
            return true;
        }
        trace!("ext4: inode missing kind={:?} path={}", kind, candidate);
    }
    false
}

fn ext4_dir_exists(path: &str) -> bool {
    ext4_inode_exists(path, InodeTypes::EXT4_DE_DIR)
}

fn ext4_file_exists(path: &str) -> bool {
    ext4_inode_exists(path, InodeTypes::EXT4_DE_REG_FILE)
}

fn ext4_open_file(path: &str, kind: InodeTypes, flags: u32) -> Option<Ext4File> {
    let mut file = Ext4File::new(path, kind.clone());
    if file.file_open(path, flags).is_ok() {
        return Some(file);
    }
    if let Some(stripped) = path.strip_prefix('/') {
        let mut file = Ext4File::new(stripped, kind);
        if file.file_open(stripped, flags).is_ok() {
            return Some(file);
        }
    }
    None
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
                // Some lwext4 backends reject SEEK_SET beyond EOF with EINVAL.
                // Extend the file first, then seek again to emulate sparse growth.
                let size = file.file_size() as usize;
                if offset <= size {
                    return None;
                }
                let new_size = offset.checked_add(buf.len())?;
                if file.file_truncate(new_size as u64).is_err() {
                    return None;
                }
                // For sparse-zero extension (e.g. fallocate emulation), truncate
                // already guarantees visible zeros without requiring a large seek+write.
                if buf.iter().all(|&b| b == 0) {
                    return Some(buf.len());
                }
                if file.file_seek(offset as i64, SEEK_SET).is_err() {
                    return None;
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
        if ext4_dir_exists(&path) {
            Some(Arc::new(Ext4Inode::new_dir(path)))
        } else if ext4_file_exists(&path) {
            Some(Arc::new(Ext4Inode::new_file(path)))
        } else {
            None
        }
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
        if self.kind != VfsNodeKind::File {
            return 0;
        }
        self.with_data_file(false, |file| Some(file.file_size() as usize))
            .unwrap_or_else(|| {
                error!("ext4: open failed path={} (size)", self.path);
                0
            })
    }
}
