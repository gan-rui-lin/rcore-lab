use super::disk::Ext4Disk;
use super::inode::Ext4Inode;
use crate::sync::UPIntrFreeCell;
use alloc::string::String;
use alloc::sync::Arc;
use core::ffi::c_char;
use easy_fs::BlockDevice;
use lwext4_rust::bindings::{ext4_cache_flush, EOK};
use lwext4_rust::Ext4BlockWrapper;

use super::super::core::VfsInode;

pub(crate) struct Ext4Fs {
    inner: UPIntrFreeCell<Option<Ext4BlockWrapper<Ext4Disk>>>,
}

impl Ext4Fs {
    pub fn new(device: Arc<dyn BlockDevice>, total_bytes: i64) -> Result<Self, i32> {
        let wrapper = Ext4BlockWrapper::<Ext4Disk>::new_with_mount(
            Ext4Disk::new(device, total_bytes),
            "/",
            "ext4_fs0",
        )?;
        Ok(Self {
            inner: unsafe { UPIntrFreeCell::new(Some(wrapper)) },
        })
    }

    pub fn root_inode(&self) -> Arc<dyn VfsInode> {
        Arc::new(Ext4Inode::new_dir(String::from("/")))
    }

    pub fn shutdown(&self) {
        let mut inner = self.inner.exclusive_access();
        if let Some(wrapper) = inner.take() {
            drop(wrapper);
        }
    }

    pub fn flush(&self) {
        let mount_point = b"/\0";
        let r = unsafe { ext4_cache_flush(mount_point.as_ptr() as *const c_char) };
        if r != EOK as i32 {
            warn!("ext4_cache_flush failed: rc={}", r);
        }
    }
}
