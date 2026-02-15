use super::disk::Fat32IoError;
use super::fs::Fat32Fs;
use crate::sync::UPIntrFreeCell;
use super::super::core::{VfsInode, VfsNodeKind};
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use fatfs::{Dir, Error, File, LossyOemCpConverter, SeekFrom};
use fatfs::{DefaultTimeProvider, Read, Seek, Write};

pub struct Fat32Inode {
    path: String,
    kind: VfsNodeKind,
    fs: Arc<UPIntrFreeCell<Fat32Fs>>,
}

impl Fat32Inode {
    pub(super) fn new(path: String, kind: VfsNodeKind, fs: Arc<UPIntrFreeCell<Fat32Fs>>) -> Self {
        Self { path, kind, fs }
    }

    pub(super) fn new_dir(path: String, fs: Arc<UPIntrFreeCell<Fat32Fs>>) -> Self {
        Self::new(path, VfsNodeKind::Dir, fs)
    }

    pub(super) fn new_file(path: String, fs: Arc<UPIntrFreeCell<Fat32Fs>>) -> Self {
        Self::new(path, VfsNodeKind::File, fs)
    }
}

type Fat32Dir<'a> = Dir<'a, super::disk::Fat32Disk, DefaultTimeProvider, LossyOemCpConverter>;
type Fat32File<'a> = File<'a, super::disk::Fat32Disk, DefaultTimeProvider, LossyOemCpConverter>;

type Fat32Result<T> = Result<T, Error<Fat32IoError>>;

fn fat32_rel_path(path: &str) -> &str {
    path.trim_start_matches('/')
}

fn fat32_path_join(base: &str, name: &str) -> String {
    if base == "/" {
        format!("/{}", name)
    } else {
        format!("{}/{}", base.trim_end_matches('/'), name)
    }
}

fn fat32_open_dir<'a>(fs: &'a Fat32Fs, path: &str) -> Fat32Result<Fat32Dir<'a>> {
    let root = fs.root_dir();
    let rel = fat32_rel_path(path);
    if rel.is_empty() {
        Ok(root)
    } else {
        root.open_dir(rel)
    }
}

fn fat32_open_file<'a>(fs: &'a Fat32Fs, path: &str) -> Fat32Result<Fat32File<'a>> {
    let root = fs.root_dir();
    let rel = fat32_rel_path(path);
    if rel.is_empty() {
        Err(Error::InvalidInput)
    } else {
        root.open_file(rel)
    }
}

fn fat32_create_file<'a>(fs: &'a Fat32Fs, path: &str) -> Fat32Result<Fat32File<'a>> {
    let root = fs.root_dir();
    let rel = fat32_rel_path(path);
    if rel.is_empty() {
        Err(Error::InvalidInput)
    } else {
        root.create_file(rel)
    }
}

impl VfsInode for Fat32Inode {
    fn kind(&self) -> VfsNodeKind {
        self.kind
    }

    fn read_at(&self, offset: usize, buf: &mut [u8]) -> usize {
        if self.kind != VfsNodeKind::File {
            return 0;
        }
        let fs = self.fs.exclusive_access();
        let mut file = match fat32_open_file(&fs, &self.path) {
            Ok(file) => file,
            Err(_) => return 0,
        };
        if file.seek(SeekFrom::Start(offset as u64)).is_err() {
            return 0;
        }
        Read::read(&mut file, buf).unwrap_or(0)
    }

    fn write_at(&self, offset: usize, buf: &[u8]) -> usize {
        if self.kind != VfsNodeKind::File {
            return 0;
        }
        let fs = self.fs.exclusive_access();
        let mut file = match fat32_open_file(&fs, &self.path) {
            Ok(file) => file,
            Err(Error::NotFound) => match fat32_create_file(&fs, &self.path) {
                Ok(file) => file,
                Err(_) => return 0,
            },
            Err(_) => return 0,
        };
        if file.seek(SeekFrom::Start(offset as u64)).is_err() {
            return 0;
        }
        Write::write(&mut file, buf).unwrap_or(0)
    }

    fn lookup(&self, name: &str) -> Option<Arc<dyn VfsInode>> {
        if self.kind != VfsNodeKind::Dir {
            return None;
        }
        let path = fat32_path_join(self.path.as_str(), name);
        let fs = self.fs.exclusive_access();
        if fat32_open_dir(&fs, &path).is_ok() {
            Some(Arc::new(Fat32Inode::new_dir(path, self.fs.clone())) as Arc<dyn VfsInode>)
        } else if fat32_open_file(&fs, &path).is_ok() {
            Some(Arc::new(Fat32Inode::new_file(path, self.fs.clone())) as Arc<dyn VfsInode>)
        } else {
            None
        }
    }

    fn create(&self, name: &str) -> Option<Arc<dyn VfsInode>> {
        if self.kind != VfsNodeKind::Dir {
            return None;
        }
        let path = fat32_path_join(self.path.as_str(), name);
        let fs = self.fs.exclusive_access();
        fat32_create_file(&fs, &path).ok()?;
        Some(Arc::new(Fat32Inode::new_file(path, self.fs.clone())) as Arc<dyn VfsInode>)
    }

    fn create_dir(&self, name: &str) -> Option<Arc<dyn VfsInode>> {
        if self.kind != VfsNodeKind::Dir {
            return None;
        }
        let path = fat32_path_join(self.path.as_str(), name);
        let fs = self.fs.exclusive_access();
        let root = fs.root_dir();
        let rel = fat32_rel_path(&path);
        if root.create_dir(rel).is_err() {
            return None;
        }
        Some(Arc::new(Fat32Inode::new_dir(path, self.fs.clone())) as Arc<dyn VfsInode>)
    }

    fn remove(&self, name: &str, _is_dir: bool) -> bool {
        if self.kind != VfsNodeKind::Dir {
            return false;
        }
        let fs = self.fs.exclusive_access();
        let dir = match fat32_open_dir(&fs, &self.path) {
            Ok(dir) => dir,
            Err(_) => return false,
        };
        dir.remove(name).is_ok()
    }

    fn truncate(&self) {
        if self.kind != VfsNodeKind::File {
            return;
        }
        let fs = self.fs.exclusive_access();
        let mut file = match fat32_open_file(&fs, &self.path) {
            Ok(file) => file,
            Err(_) => return,
        };
        if file.seek(SeekFrom::Start(0)).is_err() {
            return;
        }
        let _ = file.truncate();
    }

    fn list(&self) -> Vec<String> {
        if self.kind != VfsNodeKind::Dir {
            return Vec::new();
        }
        let fs = self.fs.exclusive_access();
        let dir = match fat32_open_dir(&fs, &self.path) {
            Ok(dir) => dir,
            Err(_) => return Vec::new(),
        };
        let mut names = Vec::new();
        for entry in dir.iter() {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            let name = entry.file_name();
            if name == "." || name == ".." {
                continue;
            }
            names.push(name);
        }
        names
    }

    fn size(&self) -> usize {
        if self.kind != VfsNodeKind::File {
            return 0;
        }
        let fs = self.fs.exclusive_access();
        let mut file = match fat32_open_file(&fs, &self.path) {
            Ok(file) => file,
            Err(_) => return 0,
        };
        file.seek(SeekFrom::End(0)).ok().unwrap_or(0) as usize
    }
}
