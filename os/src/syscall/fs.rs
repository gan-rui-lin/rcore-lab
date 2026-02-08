//! File and filesystem-related syscalls
use crate::fs::{open_file, OpenFlags, Stat, StatMode};
use crate::mm::{translated_byte_buffer, translated_refmut, translated_str, UserBuffer};
use crate::task::{current_task, current_user_token};
use super::errno::*;
use alloc::format;

const AT_FDCWD: isize = -100;

pub fn sys_write(fd: usize, buf: *const u8, len: usize) -> isize {
    let pid = current_task().unwrap().pid.0;
    if crate::syscall::should_trace_syscall(pid) && fd != 1 {
        trace!("kernel:pid[{}] sys_write", pid);
    }
    let token = current_user_token();
    let task = current_task().unwrap();
    let inner = task.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return errno(EBADF);
    }
    if let Some(file) = &inner.fd_table[fd] {
        if !file.writable() {
            return errno(EBADF);
        }
        let file = file.clone();
        // release current task TCB manually to avoid multi-borrow
        drop(inner);
        file.write(UserBuffer::new(translated_byte_buffer(token, buf, len))) as isize
    } else {
        errno(EBADF)
    }
}

pub fn sys_read(fd: usize, buf: *const u8, len: usize) -> isize {
    let pid = current_task().unwrap().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!("kernel:pid[{}] sys_read", pid);
    }
    let token = current_user_token();
    let task = current_task().unwrap();
    let inner = task.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return errno(EBADF);
    }
    if let Some(file) = &inner.fd_table[fd] {
        let file = file.clone();
        if !file.readable() {
            return errno(EBADF);
        }
        // release current task TCB manually to avoid multi-borrow
        drop(inner);
        trace!("kernel: sys_read .. file.read");
        file.read(UserBuffer::new(translated_byte_buffer(token, buf, len))) as isize
    } else {
        errno(EBADF)
    }
}

pub fn sys_openat(dirfd: isize, path: *const u8, flags: u32, _mode: u32) -> isize {
    let pid = current_task().unwrap().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!("kernel:pid[{}] sys_openat", pid);
    }
    if dirfd != AT_FDCWD {
        return errno(ENOTSUP);
    }
    if path.is_null() {
        return errno(EFAULT);
    }
    let task = current_task().unwrap();
    let token = current_user_token();
    let mut path = translated_str(token, path);
    if path.is_empty() {
        return errno(EINVAL);
    }
    if !path.starts_with('/') {
        path = format!("/{}", path);
    }
    let Some(flags) = OpenFlags::from_bits(flags) else {
        return errno(EINVAL);
    };
    if let Some(inode) = open_file(path.as_str(), flags) {
        let mut inner = task.inner_exclusive_access();
        let fd = inner.alloc_fd();
        inner.fd_table[fd] = Some(inode);
        fd as isize
    } else {
        errno(ENOENT)
    }
}

pub fn sys_close(fd: usize) -> isize {
    let pid = current_task().unwrap().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!("kernel:pid[{}] sys_close", pid);
    }
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return errno(EBADF);
    }
    if inner.fd_table[fd].is_none() {
        return errno(EBADF);
    }
    inner.fd_table[fd].take();
    0
}

/// YOUR JOB: Implement fstat.
pub fn sys_fstat(fd: usize, st: *mut Stat) -> isize {
    let pid = current_task().unwrap().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!("kernel:pid[{}] sys_fstat", pid);
    }
    let task = current_task().unwrap();
    let token = current_user_token();
    let inner = task.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return errno(EBADF);
    }
    if inner.fd_table[fd].is_none() {
        return errno(EBADF);
    }
    drop(inner);
    let stat = translated_refmut(token, st);
    *stat = Stat::new(StatMode::FILE, 1);
    0
}

pub fn sys_dup(fd: usize) -> isize {
    let pid = current_task().unwrap().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!("kernel:pid[{}] sys_dup", pid);
    }
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    if fd >= inner.fd_table.len() || inner.fd_table[fd].is_none() {
        return errno(EBADF);
    }
    let new_fd = inner.alloc_fd();
    inner.fd_table[new_fd] = inner.fd_table[fd].clone();
    new_fd as isize
}

pub fn sys_dup3(oldfd: usize, newfd: usize) -> isize {
    let pid = current_task().unwrap().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!("kernel:pid[{}] sys_dup3", pid);
    }
    if oldfd == newfd {
        return errno(EINVAL);
    }
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    if oldfd >= inner.fd_table.len() || inner.fd_table[oldfd].is_none() {
        return errno(EBADF);
    }
    if newfd >= inner.fd_table.len() {
        inner.fd_table.resize_with(newfd + 1, || None);
    }
    inner.fd_table[newfd] = inner.fd_table[oldfd].clone();
    newfd as isize
}

/// YOUR JOB: Implement linkat.
pub fn sys_linkat(
    _old_dirfd: isize,
    _old_name: *const u8,
    _new_dirfd: isize,
    _new_name: *const u8,
    _flags: u32,
) -> isize {
    trace!(
        "kernel:pid[{}] sys_linkat NOT IMPLEMENTED",
        current_task().unwrap().pid.0
    );
    errno(ENOSYS)
}

/// YOUR JOB: Implement unlinkat.
pub fn sys_unlinkat(_dirfd: isize, _name: *const u8, _flags: u32) -> isize {
    trace!(
        "kernel:pid[{}] sys_unlinkat NOT IMPLEMENTED",
        current_task().unwrap().pid.0
    );
    errno(ENOSYS)
}

pub fn sys_getcwd(_buf: *mut u8, _len: usize) -> isize {
    trace!(
        "kernel:pid[{}] sys_getcwd NOT IMPLEMENTED",
        current_task().unwrap().pid.0
    );
    errno(ENOSYS)
}

pub fn sys_chdir(_path: *const u8) -> isize {
    let pid = current_task().unwrap().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!("kernel:pid[{}] sys_chdir", pid);
    }
    if _path.is_null() {
        return errno(EFAULT);
    }
    let token = current_user_token();
    let path = translated_str(token, _path);
    if path.is_empty() {
        return errno(EINVAL);
    }
    if crate::fs::path_is_dir(&path) {
        0
    } else {
        errno(ENOENT)
    }
}

pub fn sys_getdents64(_fd: usize, _buf: *mut u8, _len: usize) -> isize {
    trace!(
        "kernel:pid[{}] sys_getdents64 NOT IMPLEMENTED",
        current_task().unwrap().pid.0
    );
    errno(ENOSYS)
}

pub fn sys_pipe2(_fds: *mut usize, _flags: u32) -> isize {
    trace!(
        "kernel:pid[{}] sys_pipe2 NOT IMPLEMENTED",
        current_task().unwrap().pid.0
    );
    errno(ENOSYS)
}
