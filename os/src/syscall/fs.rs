//! File and filesystem-related syscalls
use crate::fs::{
    create_dir, make_pipe, open_file, path_is_dir, remove_path, DevNull, DevUrandom, DevZero,
    OpenFlags, Stat, StatMode, PollEvents,
};
use crate::mm::{translated_byte_buffer, translated_ref, translated_str, translated_refmut, UserBuffer};
#[allow(unused_imports)] // for debug
use core::sync::atomic::{AtomicUsize, Ordering};
#[allow(unused_imports)] // for debug
use crate::task::{current_process, current_task, current_user_token, suspend_current_and_run_next};
use crate::timer::{get_time_ms, get_time_us};
use super::errno::*;
use super::process::TimeSpec;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

/// Check if a path is a character device.
fn is_char_device(path: &str) -> bool {
    matches!(
        path,
        "/dev/null" | "/dev/zero" | "/dev/tty" | "/dev/urandom" | "/dev/random"
            | "/dev/rtc" | "/dev/rtc0" | "/dev/misc/rtc"
    )
}

fn rdev_for_path(path: &str) -> u64 {
    match path {
        "/dev/null" => 0x0103,
        "/dev/zero" => 0x0105,
        "/dev/tty" => 0x0500,
        "/dev/urandom" | "/dev/random" => 0x0109,
        _ => 0,
    }
}

use alloc::collections::BTreeMap;
use crate::sync::UPSafeCell;
use lazy_static::lazy_static;
use core::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

#[cfg(feature = "ext4")]
use alloc::ffi::CString;
#[cfg(feature = "ext4")]
use lwext4_rust::bindings::ext4_flink;

const AT_FDCWD: isize = -100;
const AT_SYMLINK_NOFOLLOW: u32 = 0x100;
const AT_REMOVEDIR: u32 = 0x200;
const AT_NO_AUTOMOUNT: u32 = 0x800;
const AT_EMPTY_PATH: u32 = 0x1000;
const AT_STATX_FORCE_SYNC: u32 = 0x2000;
const AT_STATX_DONT_SYNC: u32 = 0x4000;
const AT_STATX_SYNC_TYPE: u32 = AT_STATX_FORCE_SYNC | AT_STATX_DONT_SYNC;
const UTIME_NOW: isize = 0x3fffffff;
const UTIME_OMIT: isize = 0x3ffffffe;

/// Per-file stored timestamps (atime, mtime).
#[derive(Clone, Copy, Debug)]
struct FileTimestamps {
    atime_sec: i64,
    atime_nsec: i64,
    mtime_sec: i64,
    mtime_nsec: i64,
}

lazy_static! {
    /// Global map: fd-unique-key -> timestamps.
    /// We key by a monotonic counter assigned per open() to track fd-level timestamps
    /// (since multiple fds can share the same path but need independent timestamps via tmpfile).
    static ref TIMESTAMPS: UPSafeCell<BTreeMap<usize, FileTimestamps>> =
        unsafe { UPSafeCell::new(BTreeMap::new()) };
    static ref TS_NEXT_ID: UPSafeCell<usize> = unsafe { UPSafeCell::new(1) };
}

static GETRANDOM_STATE: AtomicU64 = AtomicU64::new(0x9e37_79b9_7f4a_7c15);

#[allow(dead_code)]
fn ts_alloc_id() -> usize {
    let mut id = TS_NEXT_ID.exclusive_access();
    let ret = *id;
    *id += 1;
    ret
}

fn get_current_timespec() -> (i64, i64) {
    let us = get_time_us();
    let sec = (us / 1_000_000) as i64;
    let nsec = ((us % 1_000_000) * 1000) as i64;
    (sec, nsec)
}

fn ts_get(id: usize) -> Option<FileTimestamps> {
    let map = TIMESTAMPS.exclusive_access();
    map.get(&id).copied()
}

fn ts_set(id: usize, ts: FileTimestamps) {
    let mut map = TIMESTAMPS.exclusive_access();
    map.insert(id, ts);
}

#[allow(dead_code)]
fn ts_remove(id: usize) {
    let mut map = TIMESTAMPS.exclusive_access();
    map.remove(&id);
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct StatFs {
    pub f_type: i64,
    pub f_bsize: i64,
    pub f_blocks: u64,
    pub f_bfree: u64,
    pub f_bavail: u64,
    pub f_files: u64,
    pub f_ffree: u64,
    pub f_fsid: [i32; 2],
    pub f_namelen: i64,
    pub f_frsize: i64,
    pub f_flags: i64,
    pub f_spare: [i64; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct StatxTimestamp {
    pub tv_sec: i64,
    pub tv_nsec: u32,
    pub pad: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Statx {
    pub stx_mask: u32,
    pub stx_blksize: u32,
    pub stx_attributes: u64,
    pub stx_nlink: u32,
    pub stx_uid: u32,
    pub stx_gid: u32,
    pub stx_mode: u16,
    pub pad1: u16,
    pub stx_ino: u64,
    pub stx_size: u64,
    pub stx_blocks: u64,
    pub stx_attributes_mask: u64,
    pub stx_atime: StatxTimestamp,
    pub stx_btime: StatxTimestamp,
    pub stx_ctime: StatxTimestamp,
    pub stx_mtime: StatxTimestamp,
    pub stx_rdev_major: u32,
    pub stx_rdev_minor: u32,
    pub stx_dev_major: u32,
    pub stx_dev_minor: u32,
    pub spare: [u64; 14],
}

// static WRITE_LOG_COUNT: AtomicUsize = AtomicUsize::new(0);
// static WRITEV_LOG_COUNT: AtomicUsize = AtomicUsize::new(0);

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

fn build_statfs() -> StatFs {
    StatFs {
        f_type: 0xEF53, // ext4 magic; used as a generic placeholder
        f_bsize: 1024,
        f_blocks: 1024 * 1024,
        f_bfree: 512 * 1024,
        f_bavail: 512 * 1024,
        f_files: 1024 * 1024,
        f_ffree: 512 * 1024,
        f_fsid: [0, 0],
        f_namelen: 255,
        f_frsize: 1024,
        f_flags: 0,
        f_spare: [0; 4],
    }
}

pub fn sys_write(fd: usize, buf: *const u8, len: usize) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) && fd != 1 {
        syscall!("kernel:pid[{}] sys_write", pid);
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
        let written = file.write(UserBuffer::new(translated_byte_buffer(token, buf, len))) as isize;
        // let name = process.inner_exclusive_access().name.clone();
        // if (name == "busybox" || name == "sh") && fd <= 2 && len > 0 {
        //     if written == 0 {
        //         trace!("[sys_write] pid={} name={} fd={} len={} -> 0", pid, name, fd, len);
        //     } else {
        //         let count = WRITE_LOG_COUNT.fetch_add(1, Ordering::Relaxed);
        //         if count < 10 {
        //             trace!(
        //                 "[sys_write] pid={} name={} fd={} len={} -> {}",
        //                 pid,
        //                 name,
        //                 fd,
        //                 len,
        //                 written
        //             );
        //         }
        //     }
        // }
        written
    } else {
        errno(EBADF)
    }
}

pub fn sys_read(fd: usize, buf: *const u8, len: usize) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_read", pid);
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
        syscall!("kernel:pid[{}] sys_openat", pid);
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
    let proc_name = process.inner_exclusive_access().name.clone();
    let trace_so_open = proc_name == "entry-dynamic.exe"
        && (raw_path.contains(".so") || full_path.contains(".so"));
    if trace_so_open {
        info!(
            "[openat-so] pid={} raw={} full={} flags={:#x}",
            pid, raw_path, full_path, flags
        );
    }
    // let proc_name = process.inner_exclusive_access().name.clone();
    // if (proc_name == "busybox" || proc_name == "sh")
    //     && (raw_path.starts_with("./") || raw_path.contains("/basic/"))
    // {
    //     println!(
    //         "[sys_openat] pid={} name={} raw={} full={}",
    //         pid,
    //         proc_name,
    //         raw_path,
    //         full_path
    //     );
    // }
    let flags = OpenFlags::from_bits_truncate(flags);
    if flags.contains(OpenFlags::DIRECTORY) && !path_is_dir(&full_path) {
        return errno(ENOTDIR);
    }
    // Special device files
    let dev_file: Option<Arc<dyn crate::fs::File + Send + Sync>> = match full_path.as_str() {
        "/dev/null" => Some(Arc::new(DevNull)),
        "/dev/zero" => Some(Arc::new(DevZero)),
        "/dev/urandom" | "/dev/random" => Some(Arc::new(DevUrandom)),
        _ => None,
    };
    if let Some(file) = dev_file {
        let mut inner = process.inner_exclusive_access();
        let fd = match inner.alloc_fd() {
            Some(fd) => fd,
            None => return errno(EMFILE),
        };
        inner.fd_table[fd] = Some(file);
        return fd as isize;
    }
    if let Some(inode) = open_file(full_path.as_str(), flags) {
        let mut inner = process.inner_exclusive_access();
        let fd = match inner.alloc_fd() {
            Some(fd) => fd,
            None => return errno(EMFILE),
        };
        inner.fd_table[fd] = Some(inode);
        if trace_so_open {
            info!(
                "[openat-so] pid={} open ok full={} -> fd={}",
                pid, full_path, fd
            );
        }
        fd as isize
    } else {
        if trace_so_open {
            info!("[openat-so] pid={} open failed full={}", pid, full_path);
        }
        errno(ENOENT)
    }
}

/// faccessat - check file existence/permissions
///
/// For now we only validate existence and ignore mode/flags.
pub fn sys_faccessat(dirfd: isize, path: *const u8, _mode: u32, _flags: u32) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_faccessat", pid);
    }
    if path.is_null() {
        return errno(EFAULT);
    }
    let token = current_user_token();
    let raw_path = translated_str(token, path);
    if raw_path.is_empty() {
        return errno(EINVAL);
    }
    let base = match dirfd_base(dirfd) {
        Ok(base) => base,
        Err(err) => return err,
    };
    let full_path = if raw_path.starts_with('/') {
        normalize_path(&raw_path)
    } else {
        resolve_path(&base, &raw_path)
    };
    if open_file(full_path.as_str(), OpenFlags::empty()).is_some() {
        0
    } else {
        errno(ENOENT)
    }
}

pub fn sys_readlinkat(dirfd: isize, path: *const u8, buf: *mut u8, bufsize: usize) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_readlinkat", pid);
    }
    if path.is_null() || buf.is_null() {
        return errno(EFAULT);
    }
    if bufsize == 0 {
        return errno(EINVAL);
    }

    let token = current_user_token();
    let raw_path = translated_str(token, path);
    if raw_path.is_empty() {
        return errno(ENOENT);
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

    // Minimal procfs compatibility for busybox/glibc probes.
    // If /proc/<pid>/exe (or /proc/self/exe) is requested, return a stable executable path.
    let process = current_process();
    let process_name = process.inner_exclusive_access().name.clone();
    let self_exe = full_path == "/proc/self/exe"
        || full_path == format!("/proc/{}/exe", process.pid.0);

    let target = if self_exe {
        if process_name == "busybox" || process_name == "sh" {
            String::from("/bin/sh")
        } else {
            format!("/{}", process_name)
        }
    } else {
        // Generic fs symlink read is not available yet in current VFS abstraction.
        return errno(ENOENT);
    };

    let bytes = target.as_bytes();
    let write_len = bytes.len().min(bufsize);
    let slices = translated_byte_buffer(token, buf, write_len);
    let mut off = 0usize;
    for slice in slices {
        if off >= write_len {
            break;
        }
        let n = slice.len().min(write_len - off);
        slice[..n].copy_from_slice(&bytes[off..off + n]);
        off += n;
    }
    off as isize
}

pub fn sys_getrandom(buf: *mut u8, len: usize, flags: u32) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_getrandom", pid);
    }
    if buf.is_null() {
        return errno(EFAULT);
    }
    // Linux: unsupported flag bits => EINVAL.
    const GRND_NONBLOCK: u32 = 0x0001;
    const GRND_RANDOM: u32 = 0x0002;
    if flags & !(GRND_NONBLOCK | GRND_RANDOM) != 0 {
        return errno(EINVAL);
    }
    if len == 0 {
        return 0;
    }

    let token = current_user_token();
    let slices = translated_byte_buffer(token, buf, len);
    let mut state = GETRANDOM_STATE
        .load(AtomicOrdering::Relaxed)
        .wrapping_add(get_time_us() as u64)
        .wrapping_add((pid as u64) << 32)
        .wrapping_add(len as u64);

    let mut written = 0usize;
    for slice in slices {
        for byte in slice.iter_mut() {
            state ^= state >> 12;
            state ^= state << 25;
            state ^= state >> 27;
            let x = state.wrapping_mul(0x2545_F491_4F6C_DD1D);
            *byte = (x & 0xff) as u8;
            written += 1;
        }
    }
    GETRANDOM_STATE.store(state, AtomicOrdering::Relaxed);
    written as isize
}

pub fn sys_mkdirat(dirfd: isize, path: *const u8, _mode: u32) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_mkdirat", pid);
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
        syscall!("kernel:pid[{}] sys_close", pid);
    }
    let process = current_process();
    let mut inner = process.inner_exclusive_access();
    // if (inner.name == "busybox" || inner.name == "sh") && fd <= 2 {
    //     trace!("[sys_close] pid={} name={} fd={}", pid, inner.name, fd);
    // }
    if fd >= inner.fd_table.len() {
        return errno(EBADF);
    }
    if inner.fd_table[fd].is_none() {
        return errno(EBADF);
    }
    inner.fd_table[fd].take();
    0
}

/// fstatat: stat by path, relative to dirfd.
/// ! 暂时未使用 flags 参数
pub fn sys_fstatat(dirfd: isize, path: *const u8, st: *mut Stat, flags: u32) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_fstatat", pid);
    }
    if path.is_null() || st.is_null() {
        return errno(EFAULT);
    }
    // Linux fstatat/newfstatat accepts these flags; reject unknown bits.
    let supported_flags = AT_EMPTY_PATH | AT_SYMLINK_NOFOLLOW | AT_NO_AUTOMOUNT;
    if flags & !supported_flags != 0 {
        return errno(EINVAL);
    }

    let token = current_user_token();
    let raw_path = translated_str(token, path);
    // AT_EMPTY_PATH: stat the fd itself when path is empty
    if raw_path.is_empty() {
        if flags & AT_EMPTY_PATH == 0 {
            return errno(EINVAL);
        }
        // fstat-like behavior: stat the open fd
        return sys_fstat(dirfd as usize, st);
    }
    let base = if raw_path.starts_with('/') {
        String::new()
    } else {
        match dirfd_base(dirfd) {
            Ok(base) => base,
            Err(err) => return err,
        }
    };
    let full_path = if raw_path.starts_with('/') {
        normalize_path(&raw_path)
    } else {
        resolve_path(&base, &raw_path)
    };
    // trace!(
    //     "[sys_fstatat] pid={} dirfd={} flags={:#x} path={} full={}",
    //     pid,
    //     dirfd,
    //     flags,
    //     raw_path,
    //     full_path
    // );

        // Check for path traversal through non-directory (e.g. /dev/null/invalid)
    let comps: Vec<&str> = full_path.split('/').filter(|s| !s.is_empty()).collect();
    for i in 0..comps.len().saturating_sub(1) {
        let partial = format!("/{}", comps[..=i].join("/"));
        if is_char_device(&partial) {
            return errno(ENOTDIR);
        }
    }

    let mut stat = Stat::default();
    // Handle character devices specially
    if is_char_device(&full_path) {
        stat.mode = StatMode::CHR.bits() | 0o666;
        stat.nlink = 1;
        stat.rdev = rdev_for_path(&full_path);
    } else {
        let open_flags = if path_is_dir(&full_path) {
            OpenFlags::DIRECTORY
        } else {
            OpenFlags::empty()
        };
        let Some(file) = open_file(full_path.as_str(), open_flags) else {
            return errno(ENOENT);
        };
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
    }
    fill_stat_timestamps(&mut stat, None);
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

/// Read two TimeSpec values from user space (the `times[2]` array).
fn read_times_from_user(token: usize, times: *const TimeSpec) -> Option<(TimeSpec, TimeSpec)> {
    if times.is_null() {
        return None;
    }
    let size = core::mem::size_of::<TimeSpec>() * 2;
    let mut data = [0u8; 32]; // 2 * TimeSpec (each 16 bytes on rv64)
    let slices = translated_byte_buffer(token, times as *const u8, size);
    let mut offset = 0usize;
    for slice in slices {
        let len = slice.len().min(size - offset);
        data[offset..offset + len].copy_from_slice(&slice[..len]);
        offset += len;
    }
    let ts0 = unsafe { core::ptr::read_unaligned(data.as_ptr() as *const TimeSpec) };
    let ts1 = unsafe {
        core::ptr::read_unaligned(data.as_ptr().add(core::mem::size_of::<TimeSpec>()) as *const TimeSpec)
    };
    Some((ts0, ts1))
}

/// Apply utimensat semantics to a file's stored timestamps.
fn apply_utimensat_to_fd(fd: usize, times: *const TimeSpec, token: usize) -> isize {
    let process = current_process();
    let inner = process.inner_exclusive_access();
    if fd >= inner.fd_table.len() || inner.fd_table[fd].is_none() {
        return errno(EBADF);
    }
    let file = inner.fd_table[fd].as_ref().unwrap().clone();
    drop(inner);

    let ts_id = match file.ts_id() {
        Some(id) => id,
        None => return 0, // pipes/stdio — just succeed silently
    };

    let (now_sec, now_nsec) = get_current_timespec();

    // Get existing timestamps or default to current time
    let mut ts = ts_get(ts_id).unwrap_or(FileTimestamps {
        atime_sec: now_sec,
        atime_nsec: now_nsec,
        mtime_sec: now_sec,
        mtime_nsec: now_nsec,
    });

    if times.is_null() {
        // NULL times => set both to current time
        ts.atime_sec = now_sec;
        ts.atime_nsec = now_nsec;
        ts.mtime_sec = now_sec;
        ts.mtime_nsec = now_nsec;
    } else if let Some((ts0, ts1)) = read_times_from_user(token, times) {
        // times[0] = atime
        match ts0.tv_nsec as isize {
            UTIME_NOW => {
                ts.atime_sec = now_sec;
                ts.atime_nsec = now_nsec;
            }
            UTIME_OMIT => { /* don't change */ }
            _ => {
                ts.atime_sec = ts0.tv_sec as i64;
                ts.atime_nsec = ts0.tv_nsec as i64;
            }
        }
        // times[1] = mtime
        match ts1.tv_nsec as isize {
            UTIME_NOW => {
                ts.mtime_sec = now_sec;
                ts.mtime_nsec = now_nsec;
            }
            UTIME_OMIT => { /* don't change */ }
            _ => {
                ts.mtime_sec = ts1.tv_sec as i64;
                ts.mtime_nsec = ts1.tv_nsec as i64;
            }
        }
    }

    ts_set(ts_id, ts);
    0
}

/// utimensat - update file timestamps.
///
/// When path is NULL, operates on dirfd (implements futimens).
pub fn sys_utimensat(
    dirfd: isize,
    path: *const u8,
    times: *const TimeSpec,
    _flags: u32,
) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_utimensat dirfd={}", pid, dirfd);
    }
    let token = current_user_token();

    if path.is_null() {
        // futimens(fd, times) => utimensat(fd, NULL, times, 0)
        if dirfd < 0 {
            return errno(EBADF);
        }
        return apply_utimensat_to_fd(dirfd as usize, times, token);
    }
    let raw = translated_str(token, path);
    if raw.is_empty() {
        return errno(ENOENT);
    }
    let full_path = if raw.starts_with('/') {
        normalize_path(&raw)
    } else {
        let base = match dirfd_base(dirfd) {
            Ok(base) => base,
            Err(err) => return err,
        };
        resolve_path(&base, &raw)
    };
    // Check for path traversal through non-directory (e.g. /dev/null/invalid)
    let comps: Vec<&str> = full_path.split('/').filter(|s| !s.is_empty()).collect();
    for i in 0..comps.len().saturating_sub(1) {
        let partial = format!("/{}", comps[..=i].join("/"));
        if is_char_device(&partial) {
            return errno(ENOTDIR);
        }
    }
    if is_char_device(&full_path)
        || open_file(full_path.as_str(), OpenFlags::empty()).is_some()
    {
        0
    } else {
        errno(ENOENT)
    }
}

/// Fill timestamp fields in a Stat from stored timestamps or current time.
fn fill_stat_timestamps(stat: &mut Stat, ts_id: Option<usize>) {
    if let Some(id) = ts_id {
        if let Some(ts) = ts_get(id) {
            stat.atime_sec = ts.atime_sec;
            stat.atime_nsec = ts.atime_nsec;
            stat.mtime_sec = ts.mtime_sec;
            stat.mtime_nsec = ts.mtime_nsec;
            let (now_sec, now_nsec) = get_current_timespec();
            stat.ctime_sec = now_sec;
            stat.ctime_nsec = now_nsec;
            return;
        }
    }
    // No stored timestamps — return current time as default
    let (sec, nsec) = get_current_timespec();
    stat.atime_sec = sec;
    stat.atime_nsec = nsec;
    stat.mtime_sec = sec;
    stat.mtime_nsec = nsec;
    stat.ctime_sec = sec;
    stat.ctime_nsec = nsec;
}

fn split_rdev(rdev: u64) -> (u32, u32) {
    let major = ((rdev >> 8) & 0xff) as u32;
    let minor = (rdev & 0xff) as u32;
    (major, minor)
}

fn stat_from_fd(fd: usize) -> Result<Stat, isize> {
    let process = current_process();
    let inner = process.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return Err(errno(EBADF));
    }
    let Some(file) = &inner.fd_table[fd] else {
        return Err(errno(EBADF));
    };
    let file = file.clone();
    drop(inner);

    let mut stat = Stat::default();
    let path = file.path().unwrap_or("");
    if is_char_device(path) {
        stat.mode = StatMode::CHR.bits() | 0o666;
        stat.rdev = rdev_for_path(path);
    } else {
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
        stat.size = size as i64;
        stat.blksize = 512;
        stat.blocks = ((size + 511) / 512) as i64;
    }
    stat.nlink = 1;
    fill_stat_timestamps(&mut stat, file.ts_id());
    Ok(stat)
}

fn stat_from_path(full_path: &str) -> Result<Stat, isize> {
    let mut stat = Stat::default();
    if is_char_device(full_path) {
        stat.mode = StatMode::CHR.bits() | 0o666;
        stat.nlink = 1;
        stat.rdev = rdev_for_path(full_path);
        fill_stat_timestamps(&mut stat, None);
        return Ok(stat);
    }
    let open_flags = if path_is_dir(full_path) {
        OpenFlags::DIRECTORY
    } else {
        OpenFlags::empty()
    };
    let Some(file) = open_file(full_path, open_flags) else {
        return Err(errno(ENOENT));
    };
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
    fill_stat_timestamps(&mut stat, None);
    Ok(stat)
}

/// YOUR JOB: Implement fstat.
pub fn sys_fstat(fd: usize, st: *mut Stat) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_fstat", pid);
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
    let path = file.path().unwrap_or("");
    if is_char_device(path) {
        stat.mode = StatMode::CHR.bits() | 0o666;
        stat.rdev = rdev_for_path(path);
    } else {
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
        stat.size = size as i64;
        stat.blksize = 512;
        stat.blocks = ((size + 511) / 512) as i64;
    }
    stat.nlink = 1;
    fill_stat_timestamps(&mut stat, file.ts_id());
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

pub fn sys_statx(dirfd: isize, path: *const u8, flags: i32, _mask: u32, buf: *mut Statx) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_statx dirfd={}", pid, dirfd);
    }
    if path.is_null() || buf.is_null() {
        return errno(EFAULT);
    }
    if flags < 0 {
        return errno(EINVAL);
    }
    let flags = flags as u32;
    // Accept the same flag family commonly used by musl/glibc stat wrappers.
    // Unsupported semantic bits are ignored for now, but unknown bits are rejected.
    let supported_flags = AT_EMPTY_PATH | AT_SYMLINK_NOFOLLOW | AT_NO_AUTOMOUNT | AT_STATX_SYNC_TYPE;
    if flags & !supported_flags != 0 {
        return errno(EINVAL);
    }
    if (flags & AT_STATX_SYNC_TYPE) == AT_STATX_SYNC_TYPE {
        // AT_STATX_FORCE_SYNC and AT_STATX_DONT_SYNC are mutually exclusive.
        return errno(EINVAL);
    }

    let token = current_user_token();
    let raw_path = translated_str(token, path);

    let stat = if raw_path.is_empty() {
        if flags & AT_EMPTY_PATH == 0 {
            return errno(ENOENT);
        }
        if dirfd == AT_FDCWD {
            let cwd = current_process().inner_exclusive_access().cwd.clone();
            stat_from_path(&cwd)
        } else if dirfd < 0 {
            return errno(EBADF);
        } else {
            stat_from_fd(dirfd as usize)
        }
    } else {
        let base = if raw_path.starts_with('/') {
            String::new()
        } else {
            match dirfd_base(dirfd) {
                Ok(base) => base,
                Err(err) => return err,
            }
        };
        let full_path = if raw_path.starts_with('/') {
            normalize_path(&raw_path)
        } else {
            resolve_path(&base, &raw_path)
        };
        stat_from_path(&full_path)
    };

    let stat = match stat {
        Ok(stat) => stat,
        Err(err) => return err,
    };

    let mut stx = Statx::default();
    stx.stx_blksize = stat.blksize as u32;
    stx.stx_nlink = stat.nlink;
    stx.stx_uid = stat.uid;
    stx.stx_gid = stat.gid;
    stx.stx_mode = stat.mode as u16;
    stx.stx_ino = stat.ino;
    stx.stx_size = if stat.size < 0 { 0 } else { stat.size as u64 };
    stx.stx_blocks = if stat.blocks < 0 { 0 } else { stat.blocks as u64 };
    stx.stx_atime = StatxTimestamp {
        tv_sec: stat.atime_sec,
        tv_nsec: stat.atime_nsec as u32,
        pad: 0,
    };
    stx.stx_btime = StatxTimestamp::default();
    stx.stx_ctime = StatxTimestamp {
        tv_sec: stat.ctime_sec,
        tv_nsec: stat.ctime_nsec as u32,
        pad: 0,
    };
    stx.stx_mtime = StatxTimestamp {
        tv_sec: stat.mtime_sec,
        tv_nsec: stat.mtime_nsec as u32,
        pad: 0,
    };
    let (rdev_major, rdev_minor) = split_rdev(stat.rdev);
    stx.stx_rdev_major = rdev_major;
    stx.stx_rdev_minor = rdev_minor;

    let bytes = unsafe {
        core::slice::from_raw_parts((&stx as *const Statx) as *const u8, core::mem::size_of::<Statx>())
    };
    match copy_to_user(token, buf as *mut u8, bytes) {
        Ok(_) => 0,
        Err(err) => err,
    }
}

pub fn sys_dup(fd: usize) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_dup", pid);
    }
    let process = current_process();
    let mut inner = process.inner_exclusive_access();
    if fd >= inner.fd_table.len() || inner.fd_table[fd].is_none() {
        return errno(EBADF);
    }
    let new_fd = match inner.alloc_fd() {
        Some(fd) => fd,
        None => return errno(EMFILE),
    };
    inner.fd_table[new_fd] = inner.fd_table[fd].clone();
    new_fd as isize
}

pub fn sys_dup3(oldfd: usize, newfd: usize) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_dup3", pid);
    }
    if oldfd == newfd {
        return errno(EINVAL);
    }
    let process = current_process();
    let mut inner = process.inner_exclusive_access();
    if oldfd >= inner.fd_table.len() || inner.fd_table[oldfd].is_none() {
        return errno(EBADF);
    }
    let limit = inner.rlimits[crate::task::RLIMIT_NOFILE].rlim_cur as usize;
    if newfd >= limit {
        return errno(EMFILE);
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
        syscall!("kernel:pid[{}] sys_linkat", pid);
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
        syscall!("kernel:pid[{}] sys_unlinkat", pid);
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
    let exists = open_file(path.as_str(), OpenFlags::from_bits_truncate(0)).is_some();
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

pub fn sys_renameat2(
    old_dirfd: isize,
    old_name: *const u8,
    new_dirfd: isize,
    new_name: *const u8,
    flags: u32,
) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_renameat2", pid);
    }
    if flags != 0 {
        return errno(EINVAL);
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
    let old_base = match dirfd_base(old_dirfd) {
        Ok(base) => base,
        Err(err) => return err,
    };
    let new_base = match dirfd_base(new_dirfd) {
        Ok(base) => base,
        Err(err) => return err,
    };
    let old_path = if old_raw.starts_with('/') {
        normalize_path(&old_raw)
    } else {
        resolve_path(&old_base, &old_raw)
    };
    let new_path = if new_raw.starts_with('/') {
        normalize_path(&new_raw)
    } else {
        resolve_path(&new_base, &new_raw)
    };
    if old_path == new_path {
        return 0;
    }

    let old_is_dir = path_is_dir(&old_path);
    if old_is_dir {
        if open_file(new_path.as_str(), OpenFlags::empty()).is_some() {
            return errno(EEXIST);
        }
        if !create_dir(&new_path) {
            return errno(EIO);
        }
        if !remove_path(&old_path, true) {
            return errno(EIO);
        }
        return 0;
    }

    let old_file = match open_file(old_path.as_str(), OpenFlags::empty()) {
        Some(file) => file,
        None => return errno(ENOENT),
    };
    let old_inode = match old_file.inode() {
        Some(inode) => inode,
        None => return errno(EIO),
    };

    if open_file(new_path.as_str(), OpenFlags::empty()).is_some() {
        if path_is_dir(&new_path) {
            return errno(EISDIR);
        }
        if !remove_path(&new_path, false) {
            return errno(EIO);
        }
    }

    let new_file = match open_file(new_path.as_str(), OpenFlags::CREATE | OpenFlags::TRUNC | OpenFlags::WRONLY) {
        Some(file) => file,
        None => return errno(ENOENT),
    };
    let new_inode = match new_file.inode() {
        Some(inode) => inode,
        None => return errno(EIO),
    };

    let mut offset = 0usize;
    let mut buf = [0u8; 512];
    loop {
        let n = old_inode.read_at(offset, &mut buf);
        if n == 0 {
            break;
        }
        let mut written = 0usize;
        while written < n {
            let w = new_inode.write_at(offset + written, &buf[written..n]);
            if w == 0 {
                return errno(EIO);
            }
            written += w;
        }
        offset += n;
    }

    if !remove_path(&old_path, false) {
        return errno(EIO);
    }
    0
}

pub fn sys_getcwd(buf: *mut u8, len: usize) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_getcwd", pid);
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
        syscall!("kernel:pid[{}] sys_chdir", pid);
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
    // let proc_name = process.inner_exclusive_access().name.clone();
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
    let is_dir = path_is_dir(&path);
    // if proc_name == "busybox" && (raw == "basic" || raw.contains("run-all.sh") || raw.contains("basic")) {
    //     println!(
    //         "[sys_chdir] pid={} name={} raw={} resolved={} is_dir={}",
    //         pid,
    //         proc_name,
    //         raw,
    //         path,
    //         is_dir
    //     );
    // }
    if is_dir {
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
        syscall!("kernel:pid[{}] sys_getdents64", pid);
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
        syscall!("kernel:pid[{}] sys_pipe2", pid);
    }
    if fds.is_null() {
        return errno(EFAULT);
    }
    let (read_end, write_end) = make_pipe(0);
    let process = current_process();
    let mut inner = process.inner_exclusive_access();
    let fd0 = match inner.alloc_fd() {
        Some(fd) => fd,
        None => return errno(EMFILE),
    };
    inner.fd_table[fd0] = Some(read_end);
    let fd1 = match inner.alloc_fd() {
        Some(fd) => fd,
        None => {
            inner.fd_table[fd0] = None;
            return errno(EMFILE);
        }
    };
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
        syscall!("kernel:pid[{}] sys_mount", pid);
    }
    // Mounting is handled at boot; keep as a no-op for tests.
    0
}

pub fn sys_umount2(_target: *const u8, _flags: u32) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_umount2", pid);
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
        syscall!("kernel:pid[{}] sys_lseek fd={} offset={} whence={}", pid, fd, offset, whence);
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

/// readv - read data into multiple buffers
///
/// # Arguments
/// * `fd` - file descriptor
/// * `iov` - pointer to iovec array
/// * `iovcnt` - number of iovec elements
///
/// # Returns
/// * On success: number of bytes read
/// * On error: -errno
pub fn sys_readv(fd: usize, iov: *const usize, iovcnt: usize) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_readv fd={} iovcnt={}", pid, fd, iovcnt);
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

    if !file.readable() {
        return errno(EBADF);
    }

    let file = file.clone();
    drop(inner);

    let mut total_read = 0isize;

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
        let read = file.read(UserBuffer::new(buffers));
        total_read += read as isize;
        if read < len {
            break;
        }
    }

    total_read
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
        syscall!("kernel:pid[{}] sys_writev fd={} iovcnt={}", pid, fd, iovcnt);
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

    // let name = current_process().inner_exclusive_access().name.clone();
    // if (name == "busybox" || name == "sh") && fd <= 2 && iovcnt > 0 {
    //     if total_written == 0 {
    //         trace!("[sys_writev] pid={} name={} fd={} iovcnt={} -> 0", pid, name, fd, iovcnt);
    //     } else {
    //         let count = WRITEV_LOG_COUNT.fetch_add(1, Ordering::Relaxed);
    //         if count < 10 {
    //             trace!(
    //                 "[sys_writev] pid={} name={} fd={} iovcnt={} -> {}",
    //                 pid,
    //                 name,
    //                 fd,
    //                 iovcnt,
    //                 total_written
    //             );
    //         }
    //     }
    // }
    total_written
}

/// fcntl - manipulate file descriptor
///
/// # Arguments
/// * `fd` - file descriptor
/// * `cmd` - command (F_GETFL, F_SETFL, F_GETFD, F_SETFD, F_DUPFD, F_DUPFD_CLOEXEC)
/// * `arg` - command-specific argument
///
/// # Returns
/// * On success: depends on command
/// * On error: -errno
pub fn sys_fcntl(fd: usize, cmd: i32, arg: usize) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_fcntl fd={} cmd={} arg={}", pid, fd, cmd, arg);
    }

    const F_DUPFD: i32 = 0;
    const F_DUPFD_CLOEXEC: i32 = 1030;
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

    // if inner.name == "busybox" && (cmd == F_GETFD || cmd == F_SETFD) {
    //     trace!(
    //         "[sys_fcntl] pid={} name={} fd={} cmd={} arg={}",
    //         pid,
    //         inner.name,
    //         fd,
    //         cmd,
    //         arg
    //     );
    // }
    match cmd {
        F_DUPFD | F_DUPFD_CLOEXEC => {
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
            // Get file descriptor flags (FD_CLOEXEC)
            let file = inner.fd_table[fd].as_ref().unwrap();
            file.fd_flags() as isize
        }
        F_SETFD => {
            // Set file descriptor flags
            // For simplicity, accept but ignore
            0
        }
        F_GETFL => {
            // Get file status flags
            let file = &inner.fd_table[fd].as_ref().unwrap();
            let custom_flags = file.status_flags();
            if custom_flags != 0 {
                return custom_flags as isize;
            }
            // Default: derive from readable/writable
            let mut flags = 0u32;
            if file.readable() && file.writable() {
                flags |= 0b10; // O_RDWR
            } else if file.writable() {
                flags |= 0b01; // O_WRONLY
            }
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

/// ioctl system call - Device I/O control
///
/// # Arguments
/// - fd: file descriptor
/// - request: I/O control request code
/// - arg: request-specific argument
///
/// # Returns
/// - Success: 0 or request-specific value
/// - Failure: -errno
pub fn sys_ioctl(fd: usize, request: usize, arg: usize) -> isize {
    use super::errno::*;
    use crate::mm::translated_refmut;

    // Common ioctl request codes
    const TCGETS: usize = 0x5401;      // Get terminal attributes
    const TCSETS: usize = 0x5402;      // Set terminal attributes
    const TIOCGPGRP: usize = 0x540F;   // Get process group
    const TIOCSPGRP: usize = 0x5410;   // Set process group
    const TIOCGWINSZ: usize = 0x5413;  // Get window size
    const TIOCSWINSZ: usize = 0x5414;  // Set window size
    const FIONREAD: usize = 0x541B;    // Get number of bytes available
    const FIONBIO: usize = 0x5421;     // Set/clear non-blocking I/O

    let process = current_process();
    let file = {
        let inner = process.inner_exclusive_access();
        if fd >= inner.fd_table.len() {
            return errno(EBADF);
        }
        let Some(file) = &inner.fd_table[fd] else {
            return errno(EBADF);
        };
        file.clone()
    };

    // Handle common ioctl requests
    match request {
        TCGETS => {
            // Get terminal attributes
            // For simplicity, return success without actual implementation
            // Real implementation would fill termios structure
            if arg == 0 {
                return errno(EFAULT);
            }
            0
        }
        TCSETS => {
            // Set terminal attributes
            // For simplicity, accept but don't actually change anything
            if arg == 0 {
                return errno(EFAULT);
            }
            0
        }
        TIOCGWINSZ => {
            // Get window size
            // Return default terminal size: 24 rows x 80 columns
            if arg == 0 {
                return errno(EFAULT);
            }
            let token = current_user_token();
            let winsize = translated_refmut(token, arg as *mut [u16; 4]);
            winsize[0] = 24;  // ws_row
            winsize[1] = 80;  // ws_col
            winsize[2] = 0;   // ws_xpixel
            winsize[3] = 0;   // ws_ypixel
            0
        }
        TIOCSWINSZ => {
            // Set window size
            // Accept but ignore
            if arg == 0 {
                return errno(EFAULT);
            }
            0
        }
        TIOCGPGRP | TIOCSPGRP => {
            // Process group operations
            // Not fully implemented, return success
            if arg == 0 {
                return errno(EFAULT);
            }
            0
        }
        FIONREAD => {
            // Get number of bytes available to read
            // For regular files, return remaining bytes
            // For pipes/sockets, would need actual buffer check
            if arg == 0 {
                return errno(EFAULT);
            }
            let available = if let Some(inode) = file.inode() {
                let size = inode.size();
                let offset = file.get_offset().unwrap_or(0);
                if size > offset {
                    size - offset
                } else {
                    0
                }
            } else {
                0
            };
            let token = current_user_token();
            let out_ptr = translated_refmut(token, arg as *mut usize);
            *out_ptr = available;
            0
        }
        FIONBIO => {
            // Set/clear non-blocking mode
            // Accept but don't implement (would need File trait changes)
            if arg == 0 {
                return errno(EFAULT);
            }
            0
        }
        _ => {
            // Unknown ioctl request
            // Return ENOTTY (inappropriate ioctl for device)
            errno(ENOTTY)
        }
    }
}

/// ftruncate system call - Truncate file to specified length
///
/// # Arguments
/// - fd: file descriptor
/// - length: new file size in bytes
///
/// # Returns
/// - Success: 0
/// - Failure: -errno
pub fn sys_ftruncate(fd: usize, length: isize) -> isize {
    use super::errno::*;

    if length < 0 {
        return errno(EINVAL);
    }

    let process = current_process();
    let inner = process.inner_exclusive_access();

    if fd >= inner.fd_table.len() {
        return errno(EBADF);
    }

    let Some(file) = &inner.fd_table[fd] else {
        return errno(EBADF);
    };

    // Check if file is writable
    if !file.writable() {
        return errno(EINVAL);
    }

    // For now, we'll accept the call but won't actually truncate
    // Full implementation would require:
    // 1. OSInode to support truncate operation
    // 2. File system layer to handle block allocation/deallocation
    // 3. Handling of growing files (filling with zeros)
    // 4. Handling of shrinking files (freeing blocks)

    // Get current file size
    if let Some(inode) = file.inode() {
        let current_size = inode.size();
        let new_size = length as usize;

        if new_size > current_size {
            // Growing file - would need to allocate blocks and zero-fill
            // For simplicity, we'll just accept it
            debug!(
                "[sys_ftruncate] fd={} grow from {} to {} (not fully implemented)",
                fd, current_size, new_size
            );
        } else if new_size < current_size {
            // Shrinking file - would need to free blocks
            debug!(
                "[sys_ftruncate] fd={} shrink from {} to {} (not fully implemented)",
                fd, current_size, new_size
            );
        }
        // If new_size == current_size, nothing to do

        0
    } else {
        // File has no size (pipe, socket, etc.)
        errno(EINVAL)
    }
}

pub fn sys_statfs(path: *const u8, buf: *mut StatFs) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_statfs", pid);
    }
    if path.is_null() || buf.is_null() {
        return errno(EFAULT);
    }
    let token = current_user_token();
    let raw = translated_str(token, path);
    if raw.is_empty() {
        return errno(EINVAL);
    }
    let cwd = current_process().inner_exclusive_access().cwd.clone();
    let full_path = if raw.starts_with('/') {
        normalize_path(&raw)
    } else {
        resolve_path(&cwd, &raw)
    };
    if open_file(full_path.as_str(), OpenFlags::empty()).is_none() {
        return errno(ENOENT);
    }
    let statfs = build_statfs();
    let bytes = unsafe {
        core::slice::from_raw_parts(
            (&statfs as *const StatFs) as *const u8,
            core::mem::size_of::<StatFs>(),
        )
    };
    match copy_to_user(token, buf as *mut u8, bytes) {
        Ok(_) => 0,
        Err(err) => err,
    }
}

pub fn sys_fstatfs(fd: usize, buf: *mut StatFs) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_fstatfs", pid);
    }
    if buf.is_null() {
        return errno(EFAULT);
    }
    let process = current_process();
    let inner = process.inner_exclusive_access();
    if fd >= inner.fd_table.len() || inner.fd_table[fd].is_none() {
        return errno(EBADF);
    }
    drop(inner);
    let statfs = build_statfs();
    let bytes = unsafe {
        core::slice::from_raw_parts(
            (&statfs as *const StatFs) as *const u8,
            core::mem::size_of::<StatFs>(),
        )
    };
    let token = current_user_token();
    match copy_to_user(token, buf as *mut u8, bytes) {
        Ok(_) => 0,
        Err(err) => err,
    }
}

/// sendfile system call - Transfer data between file descriptors
///
/// # Arguments
/// - out_fd: destination file descriptor
/// - in_fd: source file descriptor
/// - offset: pointer to offset (if non-null, read starts from this offset)
/// - count: number of bytes to transfer
///
/// # Returns
/// - Success: number of bytes transferred
/// - Failure: -errno
///
/// Note: This is a simplified implementation that accepts the parameters
/// but returns ENOSYS (not implemented) as it requires kernel buffer management.
/// A full implementation would need:
/// 1. Kernel-space buffer pool for zero-copy transfers
/// 2. Proper handling of file offset updates
/// 3. Support for splice/pipe operations
pub fn sys_sendfile(out_fd: usize, in_fd: usize, offset: *mut isize, count: usize) -> isize {
    if count == 0 {
        return 0;
    }

    let process = current_process();
    let (in_file, out_file) = {
        let inner = process.inner_exclusive_access();
        if in_fd >= inner.fd_table.len() || out_fd >= inner.fd_table.len() {
            return errno(EBADF);
        }
        let Some(in_file) = &inner.fd_table[in_fd] else {
            return errno(EBADF);
        };
        let Some(out_file) = &inner.fd_table[out_fd] else {
            return errno(EBADF);
        };
        if !in_file.readable() || !out_file.writable() {
            return errno(EBADF);
        }
        (in_file.clone(), out_file.clone())
    };

    let in_inode = in_file.inode();
    let out_inode = out_file.inode();

    // Linux semantics: offset!=NULL requires a seekable input fd.
    if !offset.is_null() && in_inode.is_none() {
        return errno(ESPIPE);
    }

    let mut src_off = if offset.is_null() {
        in_file.get_offset().unwrap_or(0)
    } else {
        let token = current_user_token();
        let offset_ref = translated_refmut(token, offset);
        if *offset_ref < 0 {
            return errno(EINVAL);
        }
        *offset_ref as usize
    };

    let mut out_off = out_file.get_offset().unwrap_or(0);
    let mut transferred = 0usize;
    const SENDFILE_CHUNK: usize = 16 * 1024;

    while transferred < count {
        let want = core::cmp::min(SENDFILE_CHUNK, count - transferred);
        let mut kbuf = alloc::vec![0u8; want];

        let nread = if let Some(inode) = &in_inode {
            inode.read_at(src_off, &mut kbuf)
        } else {
            let read_buf = unsafe {
                UserBuffer::new(alloc::vec![core::slice::from_raw_parts_mut(
                    kbuf.as_mut_ptr(),
                    want,
                )])
            };
            in_file.read(read_buf)
        };

        if nread == 0 {
            break;
        }

        let nwritten = if let Some(inode) = &out_inode {
            inode.write_at(out_off, &kbuf[..nread])
        } else {
            let write_buf = unsafe {
                UserBuffer::new(alloc::vec![core::slice::from_raw_parts_mut(
                    kbuf.as_mut_ptr(),
                    nread,
                )])
            };
            out_file.write(write_buf)
        };

        if nwritten == 0 {
            break;
        }

        src_off = src_off.saturating_add(nwritten);
        out_off = out_off.saturating_add(nwritten);
        transferred += nwritten;

        // partial write: return current progress to match typical sendfile behavior
        if nwritten < nread {
            break;
        }
    }

    if !offset.is_null() {
        let token = current_user_token();
        let offset_ref = translated_refmut(token, offset);
        *offset_ref = src_off as isize;
    } else if in_inode.is_some() {
        in_file.set_offset(src_off);
    }

    if out_inode.is_some() {
        out_file.set_offset(out_off);
    }

    transferred as isize
}


#[derive(Debug, Copy, Clone)]
#[repr(C)]
pub struct PollFd {
    /// file descriptor
    fd: i32,
    /// requested events    
    events: PollEvents,
    /// returned events
    revents: PollEvents,
}

fn read_user_bytes(token: usize, src: *const u8, len: usize) -> Result<Vec<u8>, isize> {
    if src.is_null() {
        return Err(errno(EFAULT));
    }
    if len == 0 {
        return Ok(Vec::new());
    }
    let mut out = vec![0u8; len];
    let mut offset = 0usize;
    let slices = translated_byte_buffer(token, src, len);
    for slice in slices {
        let n = slice.len().min(len - offset);
        out[offset..offset + n].copy_from_slice(&slice[..n]);
        offset += n;
        if offset >= len {
            break;
        }
    }
    if offset < len {
        return Err(errno(EFAULT));
    }
    Ok(out)
}

#[inline]
fn fdset_len_bytes(nfds: usize) -> usize {
    let bits_per_word = core::mem::size_of::<usize>() * 8;
    nfds
        .saturating_add(bits_per_word - 1)
        / bits_per_word
        * core::mem::size_of::<usize>()
}

#[inline]
fn fdset_test(set: &[u8], fd: usize) -> bool {
    let bits_per_word = core::mem::size_of::<usize>() * 8;
    let word_size = core::mem::size_of::<usize>();
    let word_idx = fd / bits_per_word;
    let bit = fd % bits_per_word;
    let off = word_idx * word_size;
    if off + word_size > set.len() {
        return false;
    }
    let mut raw = [0u8; core::mem::size_of::<usize>()];
    raw.copy_from_slice(&set[off..off + word_size]);
    let word = usize::from_ne_bytes(raw);
    (word & (1usize << bit)) != 0
}

#[inline]
fn fdset_set(set: &mut [u8], fd: usize) {
    let bits_per_word = core::mem::size_of::<usize>() * 8;
    let word_size = core::mem::size_of::<usize>();
    let word_idx = fd / bits_per_word;
    let bit = fd % bits_per_word;
    let off = word_idx * word_size;
    if off + word_size > set.len() {
        return;
    }
    let mut raw = [0u8; core::mem::size_of::<usize>()];
    raw.copy_from_slice(&set[off..off + word_size]);
    let mut word = usize::from_ne_bytes(raw);
    word |= 1usize << bit;
    set[off..off + word_size].copy_from_slice(&word.to_ne_bytes());
}

pub fn sys_pselect6(
    nfds: usize,
    readfds: *mut usize,
    writefds: *mut usize,
    exceptfds: *mut usize,
    timeout: *const TimeSpec,
    _sigmask: usize,
) -> isize {
    let token = current_user_token();
    let fdset_len = fdset_len_bytes(nfds);

    let in_read = if readfds.is_null() {
        None
    } else {
        match read_user_bytes(token, readfds as *const u8, fdset_len) {
            Ok(v) => Some(v),
            Err(e) => return e,
        }
    };
    let in_write = if writefds.is_null() {
        None
    } else {
        match read_user_bytes(token, writefds as *const u8, fdset_len) {
            Ok(v) => Some(v),
            Err(e) => return e,
        }
    };
    let in_except = if exceptfds.is_null() {
        None
    } else {
        match read_user_bytes(token, exceptfds as *const u8, fdset_len) {
            Ok(v) => Some(v),
            Err(e) => return e,
        }
    };

    let deadline = if timeout.is_null() {
        None
    } else {
        let raw = match read_user_bytes(
            token,
            timeout as *const u8,
            core::mem::size_of::<TimeSpec>(),
        ) {
            Ok(v) => v,
            Err(e) => return e,
        };
        let spec = unsafe { core::ptr::read_unaligned(raw.as_ptr() as *const TimeSpec) };
        if spec.tv_nsec >= 1_000_000_000 {
            return errno(EINVAL);
        }
        let timeout_ms = spec
            .tv_sec
            .saturating_mul(1000)
            .saturating_add(spec.tv_nsec / 1_000_000);
        Some(get_time_ms().saturating_add(timeout_ms))
    };

    loop {
        let mut out_read = vec![0u8; fdset_len];
        let mut out_write = vec![0u8; fdset_len];
        let mut out_except = vec![0u8; fdset_len];
        let mut ready_count = 0isize;

        for fd in 0..nfds {
            let watch_read = in_read
                .as_ref()
                .map(|set| fdset_test(set.as_slice(), fd))
                .unwrap_or(false);
            let watch_write = in_write
                .as_ref()
                .map(|set| fdset_test(set.as_slice(), fd))
                .unwrap_or(false);
            let watch_except = in_except
                .as_ref()
                .map(|set| fdset_test(set.as_slice(), fd))
                .unwrap_or(false);
            if !watch_read && !watch_write && !watch_except {
                continue;
            }

            let file = {
                let process = current_process();
                let inner = process.inner_exclusive_access();
                if fd >= inner.fd_table.len() {
                    return errno(EBADF);
                }
                let Some(file) = &inner.fd_table[fd] else {
                    return errno(EBADF);
                };
                file.clone()
            };

            let ready = file.poll(
                PollEvents::POLLIN
                    | PollEvents::POLLOUT
                    | PollEvents::POLLPRI
                    | PollEvents::POLLERR
                    | PollEvents::POLLHUP,
            );

            let mut fd_ready = false;
            if watch_read && ready.intersects(PollEvents::POLLIN | PollEvents::POLLHUP | PollEvents::POLLERR) {
                fdset_set(out_read.as_mut_slice(), fd);
                fd_ready = true;
            }
            if watch_write && ready.intersects(PollEvents::POLLOUT | PollEvents::POLLERR) {
                fdset_set(out_write.as_mut_slice(), fd);
                fd_ready = true;
            }
            if watch_except && ready.contains(PollEvents::POLLPRI) {
                fdset_set(out_except.as_mut_slice(), fd);
                fd_ready = true;
            }
            if fd_ready {
                ready_count += 1;
            }
        }

        if ready_count > 0 {
            if !readfds.is_null() {
                if let Err(e) = copy_to_user(token, readfds as *mut u8, out_read.as_slice()) {
                    return e;
                }
            }
            if !writefds.is_null() {
                if let Err(e) = copy_to_user(token, writefds as *mut u8, out_write.as_slice()) {
                    return e;
                }
            }
            if !exceptfds.is_null() {
                if let Err(e) = copy_to_user(token, exceptfds as *mut u8, out_except.as_slice()) {
                    return e;
                }
            }
            return ready_count;
        }

        if let Some(deadline) = deadline {
            if get_time_ms() >= deadline {
                if !readfds.is_null() && fdset_len > 0 {
                    let _ = copy_to_user(token, readfds as *mut u8, out_read.as_slice());
                }
                if !writefds.is_null() && fdset_len > 0 {
                    let _ = copy_to_user(token, writefds as *mut u8, out_write.as_slice());
                }
                if !exceptfds.is_null() && fdset_len > 0 {
                    let _ = copy_to_user(token, exceptfds as *mut u8, out_except.as_slice());
                }
                return 0;
            }
        }
        suspend_current_and_run_next();
    }
}

pub fn sys_ppoll(fds: *mut PollFd, nfds: usize, timeout: *const TimeSpec) -> isize {
    if fds.is_null() {
        return errno(EFAULT);
    }

    let token = current_user_token();

    let mut poll_fds: Vec<&mut PollFd> = Vec::new();
    let deadline = if timeout.is_null() {
        None
    } else {
        let spec = *translated_ref(token, timeout);
        if spec.tv_nsec >= 1_000_000_000 {
            return errno(EINVAL);
        }
        let timeout_ms = spec
            .tv_sec
            .saturating_mul(1000)
            .saturating_add(spec.tv_nsec / 1_000_000);
        Some(get_time_ms().saturating_add(timeout_ms))
    };

    for i in 0..nfds {
        let poll_fd = translated_refmut(token, unsafe { fds.add(i) });
        poll_fds.push(poll_fd);
    }

    loop {
        let mut ret = 0;
        for fd in poll_fds.iter_mut() {
            fd.revents = PollEvents::empty(); // reset revents before checking
            if fd.fd < 0 {
                // Ignore negative fds per poll/ppoll semantics.
                continue;
            }

            let file = {
                let process = current_process();
                let inner = process.inner_exclusive_access();
                if (fd.fd as usize) >= inner.fd_table.len() {
                    fd.revents |= PollEvents::POLLINVAL;
                    ret += 1;
                    continue;
                };
                let Some(file) = &inner.fd_table[fd.fd as usize] else {
                    fd.revents |= PollEvents::POLLINVAL;
                    ret += 1;
                    continue;
                };
                file.clone()
            };
            let request = fd.events | PollEvents::POLLERR | PollEvents::POLLHUP;
            let ready = file.poll(request);
            if !ready.is_empty() {
                fd.revents |= ready;
                ret += 1;
            }
        }
        if ret != 0 {
            return ret;
        }
        if let Some(deadline) = deadline {
            if get_time_ms() >= deadline {
                return 0;
            }
        }
        suspend_current_and_run_next();
    }
}

/// sys_pread64 (syscall 67) - read at a given offset without changing file position
pub fn sys_pread64(fd: usize, buf: *const u8, count: usize, offset: usize) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_pread64 fd={} count={} offset={}", pid, fd, count, offset);
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
    if !file.readable() {
        return errno(EBADF);
    }
    let Some(inode) = file.inode() else {
        return errno(ESPIPE);
    };
    let mut total = 0usize;
    let mut off = offset;
    let slices = translated_byte_buffer(token, buf, count);
    for slice in slices {
        let n = inode.read_at(off, slice);
        if n == 0 {
            break;
        }
        off += n;
        total += n;
        if n < slice.len() {
            break;
        }
    }
    total as isize
}

/// sys_pwrite64 (syscall 68) - write at a given offset without changing file position
pub fn sys_pwrite64(fd: usize, buf: *const u8, count: usize, offset: usize) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_pwrite64 fd={} count={} offset={}", pid, fd, count, offset);
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
    if !file.writable() {
        return errno(EBADF);
    }
    let Some(inode) = file.inode() else {
        return errno(ESPIPE);
    };
    let mut total = 0usize;
    let mut off = offset;
    let slices = translated_byte_buffer(token, buf, count);
    for slice in slices {
        let n = inode.write_at(off, slice);
        if n == 0 {
            break;
        }
        off += n;
        total += n;
        if n < slice.len() {
            break;
        }
    }
    total as isize
}

/// sys_set_robust_list (syscall 99) - stub
pub fn sys_set_robust_list(_head: usize, _len: usize) -> isize {
    0
}

/// sys_get_robust_list (syscall 100) - stub
pub fn sys_get_robust_list(_pid: usize, _head: *mut u8, _len: *mut u8) -> isize {
    0
}
