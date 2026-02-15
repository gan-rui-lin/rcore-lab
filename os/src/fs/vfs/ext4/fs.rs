use super::disk::Ext4Disk;
use super::inode::Ext4Inode;
use crate::sync::UPIntrFreeCell;
use alloc::string::String;
use alloc::sync::Arc;
use easy_fs::BlockDevice;
use lwext4_rust::Ext4BlockWrapper;

use super::super::core::VfsInode;

pub(crate) struct Ext4Fs {
    _inner: UPIntrFreeCell<Ext4BlockWrapper<Ext4Disk>>,
}

impl Ext4Fs {
    pub fn new(device: Arc<dyn BlockDevice>, total_bytes: i64) -> Result<Self, i32> {
        let wrapper = Ext4BlockWrapper::<Ext4Disk>::new_with_mount(
            Ext4Disk::new(device, total_bytes),
            "/",
            "ext4_fs0",
        )?;
        Ok(Self {
            _inner: unsafe { UPIntrFreeCell::new(wrapper) },
        })
    }

    pub fn root_inode(&self) -> Arc<dyn VfsInode> {
        Arc::new(Ext4Inode::new_dir(String::from("/")))
    }
}
