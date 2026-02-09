use super::super::core::{VfsInode, VfsNodeKind};
use crate::drivers::BLOCK_DEVICE;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use easy_fs::{EasyFileSystem, Inode};

struct EasyFsInode {
    inode: Arc<Inode>,
}

impl EasyFsInode {
    fn new(inode: Arc<Inode>) -> Self {
        Self { inode }
    }
}

impl VfsInode for EasyFsInode {
    fn kind(&self) -> VfsNodeKind {
        if self.inode.is_dir() {
            VfsNodeKind::Dir
        } else {
            VfsNodeKind::File
        }
    }

    fn read_at(&self, offset: usize, buf: &mut [u8]) -> usize {
        self.inode.read_at(offset, buf)
    }

    fn write_at(&self, offset: usize, buf: &[u8]) -> usize {
        self.inode.write_at(offset, buf)
    }

    fn lookup(&self, name: &str) -> Option<Arc<dyn VfsInode>> {
        self.inode
            .find(name)
            .map(|inode| Arc::new(EasyFsInode::new(inode)) as Arc<dyn VfsInode>)
    }

    fn create(&self, name: &str) -> Option<Arc<dyn VfsInode>> {
        self.inode
            .create(name)
            .map(|inode| Arc::new(EasyFsInode::new(inode)) as Arc<dyn VfsInode>)
    }

    fn create_dir(&self, name: &str) -> Option<Arc<dyn VfsInode>> {
        self.inode
            .create_dir(name)
            .map(|inode| Arc::new(EasyFsInode::new(inode)) as Arc<dyn VfsInode>)
    }

    fn truncate(&self) {
        self.inode.clear();
    }

    fn list(&self) -> Vec<String> {
        self.inode.ls()
    }

    fn size(&self) -> usize {
        self.inode.size()
    }
}

pub(in crate::fs::vfs) fn easyfs_root() -> Arc<dyn VfsInode> {
    let efs = EasyFileSystem::open(BLOCK_DEVICE.clone());
    let root = EasyFileSystem::root_inode(&efs);
    Arc::new(EasyFsInode::new(Arc::new(root)))
}
