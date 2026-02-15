//! File and filesystem-related syscalls
use crate::fs::{
    create_dir, make_pipe, open_file, path_is_dir, remove_path, OpenFlags, Stat, StatMode,
};
use crate::mm::{translated_byte_buffer, translated_str, UserBuffer};
use crate::task::{current_process, current_user_token};
use super::errno::*;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

#[cfg(feature = "ext4")]
use alloc::ffi::CString;
#[cfg(feature = "ext4")]
use lwext4_rust::bindings::ext4_flink;

const AT_FDCWD: isize = -100;
const AT_REMOVEDIR: u32 = 0x200;

fn align_up(value: usize, align: usize) -> usize {
    (value + align - 1) & !(align - 1)
}

fn normalize_path(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for comp in path.split('/') {
        match comp {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(comp),
        }
    }
    if parts.is_empty() {
        String::from("/")
    } else {
        format!("/{}", parts.join("/"))
    }
}

fn resolve_path(base: &str, path: &str) -> String {
    if path.starts_with('/') {
        normalize_path(path)
    } else if base == "/" {
        normalize_path(&format!("/{}", path))
    } else {
        normalize_path(&format!("{}/{}", base.trim_end_matches('/'), path))
    }
}

fn dirfd_base(dirfd: isize) -> Result<String, isize> {
    if dirfd == AT_FDCWD {
        let process = current_process();
        return Ok(process.inner_exclusive_access().cwd.clone());
    }
    if dirfd < 0 {
        return Err(errno(EBADF));
    }
    let process = current_process();
    let inner = process.inner_exclusive_access();
    let fd = dirfd as usize;
    if fd >= inner.fd_table.len() {
        return Err(errno(EBADF));
    }
    let Some(file) = &inner.fd_table[fd] else {
        return Err(errno(EBADF));
    };
    if let Some(inode) = file.inode() {
        if !inode.is_dir() {
            return Err(errno(ENOTDIR));
        }
    } else {
        return Err(errno(ENOTDIR));
    }
    if let Some(path) = file.path() {
        Ok(String::from(path))
    } else {
        Err(errno(ENOTDIR))
    }
}

fn copy_to_user(token: usize, dst: *mut u8, data: &[u8]) -> Result<(), isize> {
    if dst.is_null() {
        return Err(errno(EFAULT));
    }
    let mut offset = 0usize;
    let slices = translated_byte_buffer(token, dst, data.len());
    for slice in slices {
        let len = slice.len().min(data.len() - offset);
        slice[..len].copy_from_slice(&data[offset..offset + len]);
        offset += len;
        if offset >= data.len() {
            break;
        }
    }
    Ok(())
}

pub fn sys_write(fd: usize, buf: *const u8, len: usize) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) && fd != 1 {
        trace!("kernel:pid[{}] sys_write", pid);
    }
    let token = current_user_token();
    let process = current_process();
    let inner = process.inner_exclusive_access();
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
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!("kernel:pid[{}] sys_read", pid);
    }
    let token = current_user_token();
    let process = current_process();
    let inner = process.inner_exclusive_access();
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
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!("kernel:pid[{}] sys_openat", pid);
    }
    if path.is_null() {
        return errno(EFAULT);
    }
    let process = current_process();
    let token = current_user_token();
    let raw_path = translated_str(token, path);
    if raw_path.is_empty() {
        return errno(EINVAL);
    }
    let full_path = if raw_path.starts_with('/') {
        normalize_path(&raw_path)
    } else {
        let base = match dirfd_base(dirfd) {
            Ok(base) => base,
            Err(err) => return err,
        };
        resolve_path(&base, &raw_path)
    };
    let flags = OpenFlags::from_bits_truncate(flags);
    if flags.contains(OpenFlags::DIRECTORY) && !path_is_dir(&full_path) {
        return errno(ENOTDIR);
    }
    if let Some(inode) = open_file(full_path.as_str(), flags) {
        let mut inner = process.inner_exclusive_access();
        let fd = inner.alloc_fd();
        inner.fd_table[fd] = Some(inode);
        fd as isize
    } else {
        errno(ENOENT)
    }
}

pub fn sys_mkdirat(dirfd: isize, path: *const u8, _mode: u32) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!("kernel:pid[{}] sys_mkdirat", pid);
    }
    if path.is_null() {
        return errno(EFAULT);
    }
    let token = current_user_token();
    let raw = translated_str(token, path);
    if raw.is_empty() {
        return errno(EINVAL);
    }
    let base = match dirfd_base(dirfd) {
        Ok(base) => base,
        Err(err) => return err,
    };
    let full_path = if raw.starts_with('/') {
        normalize_path(&raw)
    } else {
        resolve_path(&base, &raw)
    };
    if path_is_dir(&full_path) {
        return errno(EEXIST);
    }
    if create_dir(&full_path) {
        0
    } else {
        errno(EIO)
    }
}

pub fn sys_close(fd: usize) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!("kernel:pid[{}] sys_close", pid);
    }
    let process = current_process();
    let mut inner = process.inner_exclusive_access();
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
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!("kernel:pid[{}] sys_fstat", pid);
    }
    let process = current_process();
    let token = current_user_token();
    let inner = process.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return errno(EBADF);
    }
    let Some(file) = &inner.fd_table[fd] else {
        return errno(EBADF);
    };
    let file = file.clone();
    drop(inner);
    let mut stat = Stat::default();
    let (mode_bits, size) = if let Some(inode) = file.inode() {
        let mode = if inode.is_dir() {
            StatMode::DIR
        } else {
            StatMode::FILE
        };
        (mode.bits() | 0o777, inode.size())
    } else {
        (StatMode::FILE.bits() | 0o666, 0)
    };
    stat.mode = mode_bits;
    stat.nlink = 1;
    stat.size = size as i64;
    stat.blksize = 512;
    stat.blocks = ((size + 511) / 512) as i64;
    let bytes = unsafe {
        core::slice::from_raw_parts(
            (&stat as *const Stat) as *const u8,
            core::mem::size_of::<Stat>(),
        )
    };
    match copy_to_user(token, st as *mut u8, bytes) {
        Ok(_) => 0,
        Err(err) => err,
    }
}

pub fn sys_dup(fd: usize) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!("kernel:pid[{}] sys_dup", pid);
    }
    let process = current_process();
    let mut inner = process.inner_exclusive_access();
    if fd >= inner.fd_table.len() || inner.fd_table[fd].is_none() {
        return errno(EBADF);
    }
    let new_fd = inner.alloc_fd();
    inner.fd_table[new_fd] = inner.fd_table[fd].clone();
    new_fd as isize
}

pub fn sys_dup3(oldfd: usize, newfd: usize) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!("kernel:pid[{}] sys_dup3", pid);
    }
    if oldfd == newfd {
        return errno(EINVAL);
    }
    let process = current_process();
    let mut inner = process.inner_exclusive_access();
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
    old_dirfd: isize,
    old_name: *const u8,
    new_dirfd: isize,
    new_name: *const u8,
    _flags: u32,
) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!("kernel:pid[{}] sys_linkat", pid);
    }
    if old_name.is_null() || new_name.is_null() {
        return errno(EFAULT);
    }
    let token = current_user_token();
    let old_raw = translated_str(token, old_name);
    let new_raw = translated_str(token, new_name);
    if old_raw.is_empty() || new_raw.is_empty() {
        return errno(EINVAL);
    }
    let old_path = if old_raw.starts_with('/') {
        normalize_path(&old_raw)
    } else {
        let base = match dirfd_base(old_dirfd) {
            Ok(base) => base,
            Err(err) => return err,
        };
        resolve_path(&base, &old_raw)
    };
    let new_path = if new_raw.starts_with('/') {
        normalize_path(&new_raw)
    } else {
        let base = match dirfd_base(new_dirfd) {
            Ok(base) => base,
            Err(err) => return err,
        };
        resolve_path(&base, &new_raw)
    };
    if open_file(old_path.as_str(), OpenFlags::from_bits_truncate(0)).is_none() {
        return errno(ENOENT);
    }
    if open_file(new_path.as_str(), OpenFlags::from_bits_truncate(0)).is_some() {
        return errno(EEXIST);
    }
    #[cfg(feature = "ext4")]
    {
        let old_c = match CString::new(old_path) {
            Ok(c) => c,
            Err(_) => return errno(EINVAL),
        };
        let new_c = match CString::new(new_path) {
            Ok(c) => c,
            Err(_) => return errno(EINVAL),
        };
        let rc = unsafe { ext4_flink(old_c.as_ptr(), new_c.as_ptr()) };
        if rc == 0 {
            0
        } else {
            errno(EIO)
        }
    }
    #[cfg(not(feature = "ext4"))]
    {
        errno(ENOTSUP)
    }
}

/// YOUR JOB: Implement unlinkat.
pub fn sys_unlinkat(_dirfd: isize, _name: *const u8, _flags: u32) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!("kernel:pid[{}] sys_unlinkat", pid);
    }
    if _name.is_null() {
        return errno(EFAULT);
    }
    let token = current_user_token();
    let raw = translated_str(token, _name);
    if raw.is_empty() {
        return errno(EINVAL);
    }
    let base = match dirfd_base(_dirfd) {
        Ok(base) => base,
        Err(err) => return err,
    };
    let path = if raw.starts_with('/') {
        normalize_path(&raw)
    } else {
        resolve_path(&base, &raw)
    };
    let is_dir = path_is_dir(&path);
    let exists = if is_dir {
        true
    } else {
        open_file(path.as_str(), OpenFlags::from_bits_truncate(0)).is_some()
    };
    if !exists {
        return errno(ENOENT);
    }
    if _flags & AT_REMOVEDIR != 0 {
        if !is_dir {
            return errno(ENOTDIR);
        }
    } else if is_dir {
        return errno(EISDIR);
    }
    if remove_path(&path, is_dir) {
        0
    } else {
        errno(EIO)
    }
}

pub fn sys_getcwd(buf: *mut u8, len: usize) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!("kernel:pid[{}] sys_getcwd", pid);
    }
    if buf.is_null() {
        return errno(EFAULT);
    }
    let process = current_process();
    let cwd = process.inner_exclusive_access().cwd.clone();
    let bytes = cwd.as_bytes();
    if len < bytes.len() + 1 {
        return errno(ERANGE);
    }
    let token = current_user_token();
    let mut out = Vec::with_capacity(bytes.len() + 1);
    out.extend_from_slice(bytes);
    out.push(0);
    match copy_to_user(token, buf, &out) {
        Ok(_) => buf as isize,
        Err(err) => err,
    }
}

pub fn sys_chdir(_path: *const u8) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!("kernel:pid[{}] sys_chdir", pid);
    }
    if _path.is_null() {
        return errno(EFAULT);
    }
    let token = current_user_token();
    let raw = translated_str(token, _path);
    if raw.is_empty() {
        return errno(EINVAL);
    }
    let process = current_process();
    let base = if raw.starts_with('/') {
        String::from("/")
    } else {
        process.inner_exclusive_access().cwd.clone()
    };
    let path = if raw.starts_with('/') {
        normalize_path(&raw)
    } else {
        resolve_path(&base, &raw)
    };
    if path_is_dir(&path) {
        let mut inner = process.inner_exclusive_access();
        inner.cwd = path;
        0
    } else {
        errno(ENOENT)
    }
}

pub fn sys_getdents64(fd: usize, buf: *mut u8, len: usize) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!("kernel:pid[{}] sys_getdents64", pid);
    }
    if buf.is_null() {
        return errno(EFAULT);
    }
    let token = current_user_token();
    let process = current_process();
    let inner = process.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return errno(EBADF);
    }
    let Some(file) = &inner.fd_table[fd] else {
        return errno(EBADF);
    };
    let file = file.clone();
    drop(inner);
    let Some(inode) = file.inode() else {
        return errno(ENOTDIR);
    };
    if !inode.is_dir() {
        return errno(ENOTDIR);
    }
    let entries = inode.list();
    let mut idx = file.get_offset().unwrap_or(0);
    let mut out: Vec<u8> = Vec::new();
    while idx < entries.len() {
        let name = entries[idx].as_bytes();
        let reclen = align_up(19 + name.len() + 1, 8);
        if out.len() + reclen > len {
            break;
        }
        let ino = (idx + 1) as u64;
        let off = (idx + 1) as i64;
        out.extend_from_slice(&ino.to_le_bytes());
        out.extend_from_slice(&off.to_le_bytes());
        out.extend_from_slice(&(reclen as u16).to_le_bytes());
        out.push(0); // d_type = DT_UNKNOWN
        out.extend_from_slice(name);
        out.push(0);
        while out.len() % 8 != 0 {
            out.push(0);
        }
        idx += 1;
    }
    file.set_offset(idx);
    match copy_to_user(token, buf, &out) {
        Ok(_) => out.len() as isize,
        Err(err) => err,
    }
}

pub fn sys_pipe2(fds: *mut i32, _flags: u32) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!("kernel:pid[{}] sys_pipe2", pid);
    }
    if fds.is_null() {
        return errno(EFAULT);
    }
    let (read_end, write_end) = make_pipe(0);
    let process = current_process();
    let mut inner = process.inner_exclusive_access();
    let fd0 = inner.alloc_fd();
    inner.fd_table[fd0] = Some(read_end);
    let fd1 = inner.alloc_fd();
    inner.fd_table[fd1] = Some(write_end);
    drop(inner);
    let token = current_user_token();
    let mut data = [0u8; 8];
    data[..4].copy_from_slice(&(fd0 as i32).to_le_bytes());
    data[4..].copy_from_slice(&(fd1 as i32).to_le_bytes());
    match copy_to_user(token, fds as *mut u8, &data) {
        Ok(_) => 0,
        Err(err) => err,
    }
}

pub fn sys_mount(
    _source: *const u8,
    _target: *const u8,
    _fstype: *const u8,
    _flags: u32,
    _data: usize,
) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!("kernel:pid[{}] sys_mount", pid);
    }
    // Mounting is handled at boot; keep as a no-op for tests.
    0
}

pub fn sys_umount2(_target: *const u8, _flags: u32) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!("kernel:pid[{}] sys_umount2", pid);
    }
    0
}

/// lseek - reposition read/write file offset
///
/// # Arguments
/// * `fd` - file descriptor
/// * `offset` - offset value
/// * `whence` - SEEK_SET (0), SEEK_CUR (1), SEEK_END (2)
///
/// # Returns
/// * On success: the resulting offset location
/// * On error: -errno
pub fn sys_lseek(fd: usize, offset: isize, whence: usize) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!("kernel:pid[{}] sys_lseek fd={} offset={} whence={}", pid, fd, offset, whence);
    }

    const SEEK_SET: usize = 0;
    const SEEK_CUR: usize = 1;
    const SEEK_END: usize = 2;

    let process = current_process();
    let inner = process.inner_exclusive_access();

    if fd >= inner.fd_table.len() {
        return errno(EBADF);
    }

    let Some(file) = &inner.fd_table[fd] else {
        return errno(EBADF);
    };

    // Check if this is a pipe or other non-seekable file
    if file.inode().is_none() {
        return errno(ESPIPE);
    }

    let file = file.clone();
    drop(inner);

    let current_offset = file.get_offset().unwrap_or(0) as isize;
    let file_size = if let Some(inode) = file.inode() {
        inode.size() as isize
    } else {
        return errno(ESPIPE);
    };

    let new_offset = match whence {
        SEEK_SET => offset,
        SEEK_CUR => current_offset + offset,
        SEEK_END => file_size + offset,
        _ => return errno(EINVAL),
    };

    if new_offset < 0 {
        return errno(EINVAL);
    }

    file.set_offset(new_offset as usize);
    new_offset
}

/// writev - write data from multiple buffers
///
/// # Arguments
/// * `fd` - file descriptor
/// * `iov` - pointer to iovec array
/// * `iovcnt` - number of iovec elements
///
/// # Returns
/// * On success: number of bytes written
/// * On error: -errno
pub fn sys_writev(fd: usize, iov: *const usize, iovcnt: usize) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) && fd != 1 {
        trace!("kernel:pid[{}] sys_writev fd={} iovcnt={}", pid, fd, iovcnt);
    }

    if iov.is_null() {
        return errno(EFAULT);
    }

    if iovcnt == 0 {
        return 0;
    }

    let token = current_user_token();
    let process = current_process();
    let inner = process.inner_exclusive_access();

    if fd >= inner.fd_table.len() {
        return errno(EBADF);
    }

    let Some(file) = &inner.fd_table[fd] else {
        return errno(EBADF);
    };

    if !file.writable() {
        return errno(EBADF);
    }

    let file = file.clone();
    drop(inner);

    let mut total_written = 0isize;

    // Read iovec structures from user space
    for i in 0..iovcnt {
        let iov_ptr = unsafe { iov.add(i * 2) };
        let iov_buffers = translated_byte_buffer(token, iov_ptr as *const u8, 16);

        let mut iov_data = [0u8; 16];
        let mut offset = 0;
        for slice in iov_buffers {
            let len = slice.len().min(16 - offset);
            iov_data[offset..offset + len].copy_from_slice(&slice[..len]);
            offset += len;
            if offset >= 16 {
                break;
            }
        }

        let base = usize::from_le_bytes([
            iov_data[0], iov_data[1], iov_data[2], iov_data[3],
            iov_data[4], iov_data[5], iov_data[6], iov_data[7],
        ]);
        let len = usize::from_le_bytes([
            iov_data[8], iov_data[9], iov_data[10], iov_data[11],
            iov_data[12], iov_data[13], iov_data[14], iov_data[15],
        ]);

        if base == 0 || len == 0 {
            continue;
        }

        let buffers = translated_byte_buffer(token, base as *const u8, len);
        let written = file.write(UserBuffer::new(buffers));
        total_written += written as isize;
    }

    total_written
}

/// fcntl - manipulate file descriptor
///
/// # Arguments
/// * `fd` - file descriptor
/// * `cmd` - command (F_GETFL, F_SETFL, F_GETFD, F_SETFD, F_DUPFD)
/// * `arg` - command-specific argument
///
/// # Returns
/// * On success: depends on command
/// * On error: -errno
pub fn sys_fcntl(fd: usize, cmd: i32, arg: usize) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!("kernel:pid[{}] sys_fcntl fd={} cmd={} arg={}", pid, fd, cmd, arg);
    }

    const F_DUPFD: i32 = 0;
    const F_GETFD: i32 = 1;
    const F_SETFD: i32 = 2;
    const F_GETFL: i32 = 3;
    const F_SETFL: i32 = 4;

    let process = current_process();
    let mut inner = process.inner_exclusive_access();

    if fd >= inner.fd_table.len() {
        return errno(EBADF);
    }

    let Some(_file) = &inner.fd_table[fd] else {
        return errno(EBADF);
    };

    match cmd {
        F_DUPFD => {
            // Duplicate fd to the lowest numbered available fd >= arg
            let new_fd = if arg < inner.fd_table.len() {
                let mut found = None;
                for i in arg..inner.fd_table.len() {
                    if inner.fd_table[i].is_none() {
                        found = Some(i);
                        break;
                    }
                }
                if let Some(i) = found {
                    i
                } else {
                    inner.fd_table.len()
                }
            } else {
                inner.fd_table.len()
            };

            if new_fd >= inner.fd_table.len() {
                inner.fd_table.resize_with(new_fd + 1, || None);
            }
            inner.fd_table[new_fd] = inner.fd_table[fd].clone();
            new_fd as isize
        }
        F_GETFD => {
            // Get file descriptor flags (currently only FD_CLOEXEC is supported)
            // For simplicity, return 0 (no flags set)
            0
        }
        F_SETFD => {
            // Set file descriptor flags
            // For simplicity, accept but ignore
            0
        }
        F_GETFL => {
            // Get file status flags
            // Return basic flags based on file properties
            let file = &inner.fd_table[fd].as_ref().unwrap();
            let mut flags = 0u32;
            if file.readable() && file.writable() {
                flags |= 0b10; // O_RDWR
            } else if file.writable() {
                flags |= 0b01; // O_WRONLY
            }
            // O_RDONLY is 0
            flags as isize
        }
        F_SETFL => {
            // Set file status flags
            // For simplicity, accept but ignore (would need to modify File trait)
            0
        }
        _ => errno(EINVAL),
    }
}
