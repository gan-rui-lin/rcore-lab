//! File trait & inode(dir, file, pipe, stdin, stdout)

mod pipe;
mod stdio;
mod vfs;

use crate::mm::UserBuffer;
use alloc::sync::Arc;
use alloc::vec::Vec;
use vfs::VfsInode;

/// trait File for all file types
pub trait File: Send + Sync {
    /// the file readable?
    fn readable(&self) -> bool;
    /// the file writable?
    fn writable(&self) -> bool;
    /// read from the file to buf, return the number of bytes read
    fn read(&self, buf: UserBuffer) -> usize;
    /// write to the file from buf, return the number of bytes written
    fn write(&self, buf: UserBuffer) -> usize;
    /// read entire file content into a buffer
    fn read_all(&self) -> Vec<u8> {
        Vec::new()
    }
    /// Optional: underlying inode, for metadata queries.
    fn inode(&self) -> Option<Arc<dyn VfsInode>> {
        None
    }
    /// Optional: absolute path of this file.
    fn path(&self) -> Option<&str> {
        None
    }
    /// Optional: get current file offset.
    fn get_offset(&self) -> Option<usize> {
        None
    }
    /// Optional: set current file offset.
    fn set_offset(&self, _offset: usize) {}
}

/// Linux-compatible stat layout (riscv64).
#[allow(missing_docs)]
#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
pub struct Stat {
    pub dev: u64,
    pub ino: u64,
    pub mode: u32,
    pub nlink: u32,
    pub uid: u32,
    pub gid: u32,
    pub rdev: u64,
    pub _pad: u64,
    pub size: i64,
    pub blksize: i64,
    pub _pad2: i32,
    pub blocks: i64,
    pub atime_sec: i64,
    pub atime_nsec: i64,
    pub mtime_sec: i64,
    pub mtime_nsec: i64,
    pub ctime_sec: i64,
    pub ctime_nsec: i64,
    pub _unused: [i64; 2],
}

bitflags! {
    /// The mode of a inode
    /// whether a directory or a file
    pub struct StatMode: u32 {
        /// null
        const NULL  = 0;
        /// directory
        const DIR   = 0o040000;
        /// ordinary regular file
        const FILE  = 0o100000;
    }
}

bitflags! {
    /// The flags argument to the open() system call.
    pub struct OpenFlags: u32 {
        /// write only
        const WRONLY = 1 << 0;
        /// read and write
        const RDWR = 1 << 1;
        /// create new file
        const CREATE = 1 << 6;
        /// truncate file size to 0
        const TRUNC = 1 << 9;
        /// append
        const APPEND = 1 << 10;
        /// must be a directory
        const DIRECTORY = 1 << 16;
        /// close-on-exec
        const CLOEXEC = 1 << 19;
    }
}

impl OpenFlags {
    /// Return (readable, writable) tuple.
    pub fn read_write(&self) -> (bool, bool) {
        match self.bits() & 0b11 {
            0 => (true, false),
            1 => (false, true),
            2 => (true, true),
            _ => (true, true),
        }
    }
}

#[cfg(feature = "ext4")]
/// Mount ext4 as root with explicit device size.
pub use vfs::mount_ext4;
#[cfg(feature = "ext4")]
/// Auto-detect ext4 and mount it as root if present.
pub use vfs::mount_ext4_auto;
pub use vfs::{
    create_dir, list_apps, mount_easyfs, mount_fat32, mount_fat32_auto, open_file, path_is_dir,
    remove_path,
};
pub use pipe::make_pipe;
pub use stdio::{Stdin, Stdout};
