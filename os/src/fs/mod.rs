//! File trait & inode(dir, file, pipe, stdin, stdout)

mod stdio;
mod vfs;
#[cfg(feature = "ext4")]
mod ext4;

use crate::mm::UserBuffer;
use alloc::vec::Vec;

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
}

/// The stat of a inode
#[repr(C)]
#[derive(Debug)]
pub struct Stat {
    /// ID of device containing file
    pub dev: u64,
    /// inode number
    pub ino: u64,
    /// file type and mode
    pub mode: StatMode,
    /// number of hard links
    pub nlink: u32,
    /// unused pad
    pad: [u64; 7],
}

impl Stat {
    /// Construct a minimal Stat with mode and link count.
    pub fn new(mode: StatMode, nlink: u32) -> Self {
        Self {
            dev: 0,
            ino: 0,
            mode,
            nlink,
            pad: [0; 7],
        }
    }
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
        /// read only
        const RDONLY = 0;
        /// write only
        const WRONLY = 1 << 0;
        /// read and write
        const RDWR = 1 << 1;
        /// create new file
        const CREATE = 1 << 9;
        /// truncate file size to 0
        const TRUNC = 1 << 10;
    }
}

impl OpenFlags {
    /// Return (readable, writable) tuple.
    pub fn read_write(&self) -> (bool, bool) {
        if self.is_empty() {
            (true, false)
        } else if self.contains(Self::WRONLY) {
            (false, true)
        } else {
            (true, true)
        }
    }
}

#[cfg(feature = "ext4")]
/// Mount ext4 as root with explicit device size.
pub use vfs::mount_ext4;
#[cfg(feature = "ext4")]
/// Auto-detect ext4 and mount it as root if present.
pub use vfs::mount_ext4_auto;
pub use vfs::{list_apps, mount_easyfs, open_file, path_is_dir};
pub use stdio::{Stdin, Stdout};
