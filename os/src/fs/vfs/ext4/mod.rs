#![cfg(feature = "ext4")]
#![allow(missing_docs)]

mod disk;
mod fs;
mod inode;

pub(crate) use fs::Ext4Fs;
