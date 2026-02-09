#![allow(missing_docs)]

mod disk;
mod fs;
mod inode;

pub(in crate::fs::vfs) use fs::fat32_root;
