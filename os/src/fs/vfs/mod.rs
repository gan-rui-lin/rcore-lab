mod core;
mod easyfs;
mod file;
mod mount;
#[cfg(feature = "ext4")]
mod ext4;
mod fat32;

pub use core::VfsInode;
pub use file::{create_dir, list_apps, open_file, path_is_dir};
pub use mount::{mount_easyfs, mount_fat32, mount_fat32_auto};
#[cfg(feature = "ext4")]
pub use mount::{mount_ext4, mount_ext4_auto};
