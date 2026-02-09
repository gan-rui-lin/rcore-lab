use super::super::core::{VfsInode, VfsNodeKind};
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
}

impl Ext4Inode {
    pub(super) fn new(path: String, kind: VfsNodeKind) -> Self {
        Self { path, kind }
    }

    pub(super) fn new_dir(path: String) -> Self {
        Self::new(path, VfsNodeKind::Dir)
    }

    pub(super) fn new_file(path: String) -> Self {
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

impl VfsInode for Ext4Inode {
    fn kind(&self) -> VfsNodeKind {
        self.kind
    }

    fn read_at(&self, offset: usize, buf: &mut [u8]) -> usize {
        let mut file = Ext4File::new(self.path.as_str(), InodeTypes::EXT4_DE_REG_FILE);
        if file.file_open(self.path.as_str(), O_RDONLY).is_err() {
            return 0;
        }
        let ret = file.file_seek(offset as i64, SEEK_SET);
        if ret.is_err() {
            let _ = file.file_close();
            return 0;
        }
        let read_size = file.file_read(buf).unwrap_or(0);
        let _ = file.file_close();
        read_size as usize
    }

    fn write_at(&self, offset: usize, buf: &[u8]) -> usize {
        let mut file = Ext4File::new(self.path.as_str(), InodeTypes::EXT4_DE_REG_FILE);
        if file.file_open(self.path.as_str(), O_RDWR | O_CREAT).is_err() {
            return 0;
        }
        let ret = file.file_seek(offset as i64, SEEK_SET);
        if ret.is_err() {
            let _ = file.file_close();
            return 0;
        }
        let write_size = file.file_write(buf).unwrap_or(0);
        let _ = file.file_close();
        write_size as usize
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
        let mut file = Ext4File::new(self.path.as_str(), InodeTypes::EXT4_DE_REG_FILE);
        if file.file_open(self.path.as_str(), O_RDWR | O_TRUNC).is_err() {
            return;
        }
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
        let mut file = Ext4File::new(self.path.as_str(), InodeTypes::EXT4_DE_REG_FILE);
        if file.file_open(self.path.as_str(), O_RDONLY).is_err() {
            return 0;
        }
        let size = file.file_size() as usize;
        let _ = file.file_close();
        size
    }
}
