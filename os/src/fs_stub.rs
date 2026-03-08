//! Minimal filesystem stubs for LoongArch64 builds.
#![allow(missing_docs)]

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use crate::mm::UserBuffer;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VfsNodeKind {
    File,
    Dir,
}

pub trait VfsInode: Send + Sync {
    fn kind(&self) -> VfsNodeKind;
    fn read_at(&self, _offset: usize, _buf: &mut [u8]) -> usize { 0 }
    fn write_at(&self, _offset: usize, _buf: &[u8]) -> usize { 0 }
    fn lookup(&self, _name: &str) -> Option<Arc<dyn VfsInode>> { None }
    fn create(&self, _name: &str) -> Option<Arc<dyn VfsInode>> { None }
    fn create_dir(&self, _name: &str) -> Option<Arc<dyn VfsInode>> { None }
    fn remove(&self, _name: &str, _is_dir: bool) -> bool { false }
    fn truncate(&self) {}
    fn list(&self) -> Vec<String> { Vec::new() }
    fn size(&self) -> usize { 0 }
    fn is_dir(&self) -> bool {
        self.kind() == VfsNodeKind::Dir
    }
}

pub trait File: Send + Sync {
    fn readable(&self) -> bool { false }
    fn writable(&self) -> bool { false }
    fn read(&self, _buf: UserBuffer) -> usize { 0 }
    fn write(&self, _buf: UserBuffer) -> usize { 0 }
    fn read_all(&self) -> Vec<u8> { Vec::new() }
    fn poll(&self, _events: PollEvents) -> PollEvents { PollEvents::POLLIN | PollEvents::POLLOUT }
    fn inode(&self) -> Option<Arc<dyn VfsInode>> { None }
    fn path(&self) -> Option<&str> { None }
    fn get_offset(&self) -> Option<usize> { None }
    fn set_offset(&self, _offset: usize) {}
    fn ts_id(&self) -> Option<usize> { None }
    fn as_socket(&self) -> Option<(smoltcp::iface::SocketHandle, crate::net::SocketType)> {
        None
    }
    fn fd_flags(&self) -> u32 { 0 }
    fn status_flags(&self) -> u32 { 0 }
    fn bound_port(&self) -> u16 { 0 }
    fn set_bound_port(&self, _port: u16) {}
    fn is_listening(&self) -> bool { false }
    fn set_listening(&self, _listening: bool) {}
    fn mark_transferred(&self) {}
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
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
    pub blksize: i32,
    pub _pad2: i32,
    pub blocks: i64,
    pub atime_sec: i64,
    pub atime_nsec: i64,
    pub mtime_sec: i64,
    pub mtime_nsec: i64,
    pub ctime_sec: i64,
    pub ctime_nsec: i64,
    pub _unused: [u32; 2],
}

bitflags::bitflags! {
    pub struct StatMode: u32 {
        const NULL  = 0;
        const CHR   = 0o020000;
        const DIR   = 0o040000;
        const BLK   = 0o060000;
        const FILE  = 0o100000;
        const LNK   = 0o120000;
    }
}

bitflags::bitflags! {
    pub struct OpenFlags: u32 {
        const WRONLY = 1 << 0;
        const RDWR = 1 << 1;
        const CREATE = 1 << 6;
        const TRUNC = 1 << 9;
        const APPEND = 1 << 10;
        const DIRECTORY = 1 << 16;
        const CLOEXEC = 1 << 19;
    }
}

impl OpenFlags {
    pub fn read_write(&self) -> (bool, bool) {
        match self.bits() & 0b11 {
            0 => (true, false),
            1 => (false, true),
            2 => (true, true),
            _ => (true, true),
        }
    }
}

bitflags::bitflags! {
    pub struct PollEvents: i16 {
        const POLLIN = 0x001;
        const POLLPRI = 0x002;
        const POLLOUT = 0x004;
        const POLLERR = 0x008;
        const POLLHUP = 0x010;
        const POLLINVAL = 0x020;
    }
}

pub struct Stdin;
pub struct Stdout;
pub struct DevNull;
pub struct DevZero;

impl File for Stdin {}
impl File for Stdout {}
impl File for DevNull {}
impl File for DevZero {}

pub fn open_file(_path: &str, _flags: OpenFlags) -> Option<Arc<dyn File>> {
    None
}

pub fn create_dir(_path: &str) -> bool { false }
pub fn remove_path(_path: &str, _is_dir: bool) -> bool { false }
pub fn path_is_dir(_path: &str) -> bool { false }
pub fn path_exists(_path: &str) -> bool { false }

pub fn make_pipe(_capacity: usize) -> (Arc<dyn File>, Arc<dyn File>) {
    (Arc::new(DevNull), Arc::new(DevNull))
}
