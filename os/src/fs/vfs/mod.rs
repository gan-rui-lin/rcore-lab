mod core;
mod easyfs;
#[cfg(feature = "ext4")]
mod ext4;
mod fat32;
mod file;
mod mount;
mod procfs;

pub use core::{VfsInode, VfsMetadata, VfsNodeKind, VfsStatFs};
pub use file::{create_dir, list_apps, open_file, path_exists, path_is_dir, remove_path};
pub use mount::{mount_easyfs, mount_fat32, mount_fat32_auto, mount_procfs};
#[cfg(feature = "ext4")]
pub use mount::{mount_ext4, mount_ext4_auto};
