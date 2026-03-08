//! Minimal network stubs for LoongArch64 builds.
#![allow(missing_docs)]

const ENOSYS: isize = 38;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SocketType {
    Tcp,
    Udp,
}

pub mod syscall {
    use super::ENOSYS;

    pub fn sys_socket(_domain: usize, _ty: usize, _protocol: usize) -> isize { -ENOSYS }
    pub fn sys_socketpair() -> isize { -ENOSYS }
    pub fn sys_bind(_fd: usize, _addr: *const u8, _len: usize) -> isize { -ENOSYS }
    pub fn sys_listen(_fd: usize, _backlog: usize) -> isize { -ENOSYS }
    pub fn sys_accept(_fd: usize, _addr: *mut u8, _len: *mut u32) -> isize { -ENOSYS }
    pub fn sys_connect(_fd: usize, _addr: *const u8, _len: usize) -> isize { -ENOSYS }
    pub fn sys_getsockname(_fd: usize, _addr: *mut u8, _len: *mut u32) -> isize { -ENOSYS }
    pub fn sys_getpeername(_fd: usize, _addr: *mut u8, _len: *mut u32) -> isize { -ENOSYS }
    pub fn sys_sendto(
        _fd: usize,
        _buf: *const u8,
        _len: usize,
        _flags: usize,
        _addr: *const u8,
        _addrlen: usize,
    ) -> isize {
        -ENOSYS
    }
    pub fn sys_recvfrom(
        _fd: usize,
        _buf: *mut u8,
        _len: usize,
        _flags: usize,
        _addr: *mut u8,
        _addrlen: *mut u32,
    ) -> isize {
        -ENOSYS
    }
    pub fn sys_setsockopt(
        _fd: usize,
        _level: usize,
        _opt: usize,
        _val: *const u8,
        _len: usize,
    ) -> isize {
        -ENOSYS
    }
    pub fn sys_getsockopt(
        _fd: usize,
        _level: usize,
        _opt: usize,
        _val: *mut u8,
        _len: *mut u32,
    ) -> isize {
        -ENOSYS
    }
    pub fn sys_shutdown_socket(_fd: usize, _how: i32) -> isize { -ENOSYS }
    pub fn sys_sendmsg() -> isize { -ENOSYS }
    pub fn sys_recvmsg() -> isize { -ENOSYS }
}
