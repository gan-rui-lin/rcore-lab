//! Process management syscalls
//!
use alloc::format;
use alloc::sync::Arc;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::{
    fs::{open_file, OpenFlags},
    mm::{translated_byte_buffer, translated_ref, translated_refmut, translated_str, MapPermission, VirtAddr},
    sbi::shutdown,
    task::{
        current_process, current_task, current_user_token, exit_current_and_run_next,
        pid2process, suspend_current_and_run_next, SignalAction, SignalFlags,
        MAX_SIG,
    },
    timer::{get_time, get_time_us},
};

use super::errno::*;
use crate::config::{CLOCK_FREQ, PAGE_SIZE};

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct TimeSpec {
    pub tv_sec: usize,
    pub tv_nsec: usize,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Tms {
    pub tms_utime: i64,
    pub tms_stime: i64,
    pub tms_cutime: i64,
    pub tms_cstime: i64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct UtsName {
    pub sysname: [u8; 65],
    pub nodename: [u8; 65],
    pub release: [u8; 65],
    pub version: [u8; 65],
    pub machine: [u8; 65],
    pub domainname: [u8; 65],
}

impl Default for UtsName {
    fn default() -> Self {
        Self {
            sysname: [0; 65],
            nodename: [0; 65],
            release: [0; 65],
            version: [0; 65],
            machine: [0; 65],
            domainname: [0; 65],
        }
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

fn read_from_user<T: Copy>(token: usize, src: *const T) -> Result<T, isize> {
    if src.is_null() {
        return Err(errno(EFAULT));
    }
    let size = core::mem::size_of::<T>();
    let mut data = vec![0u8; size];
    let slices = translated_byte_buffer(token, src as *const u8, size);
    let mut offset = 0usize;
    for slice in slices {
        let len = slice.len().min(size - offset);
        data[offset..offset + len].copy_from_slice(&slice[..len]);
        offset += len;
        if offset >= size {
            break;
        }
    }
    let value = unsafe { core::ptr::read_unaligned(data.as_ptr() as *const T) };
    Ok(value)
}

pub fn sys_exit(exit_code: i32) -> ! {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!("kernel:pid[{}] sys_exit", pid);
    }
    exit_current_and_run_next(exit_code);
    panic!("Unreachable in sys_exit!");
}

pub fn sys_yield() -> isize {
    //trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}

pub fn sys_getpid() -> isize {
    trace!("kernel: sys_getpid pid:{}", current_process().pid.0);
    current_process().pid.0 as isize
}

pub fn sys_getppid() -> isize {
    let process = current_process();
    let inner = process.inner_exclusive_access();
    if let Some(parent) = inner.parent.as_ref().and_then(|p| p.upgrade()) {
        parent.pid.0 as isize
    } else {
        0
    }
}

pub fn sys_fork() -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!("kernel:pid[{}] sys_fork", pid);
    }
    let current_process = current_process();
    let new_process = current_process.fork();
    let new_pid = new_process.pid.0;
    let new_task = new_process.inner_exclusive_access().get_task(0);
    let trap_cx = new_task.inner_exclusive_access().get_trap_cx();
    trap_cx.x[10] = 0;
    new_pid as isize
}

pub fn sys_exec(path: *const u8, mut argv: *const usize, mut envp: *const usize) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!("kernel:pid[{}] sys_exec", pid);
    }
    if path.is_null() {
        return errno(EFAULT);
    }
    let token = current_user_token();
    let raw_path = translated_str(token, path);
    if crate::syscall::should_trace_syscall(pid) {
        trace!("kernel:pid[{}] sys_exec path={}", pid, raw_path);
    }
    let mut args: Vec<String> = Vec::new();
    if !argv.is_null() {
        loop {
            let arg_ptr = *translated_ref(token, argv);
            if arg_ptr == 0 {
                break;
            }
            args.push(translated_str(token, arg_ptr as *const u8));
            unsafe {
                argv = argv.add(1);
            }
        }
    }
    if args.is_empty() {
        args.push(raw_path.clone());
    }
    let mut envs: Vec<String> = Vec::new();
    if !envp.is_null() {
        loop {
            let env_ptr = *translated_ref(token, envp);
            if env_ptr == 0 {
                break;
            }
            envs.push(translated_str(token, env_ptr as *const u8));
            unsafe {
                envp = envp.add(1);
            }
        }
    }
    let process = current_process();
    let exec_path = if raw_path.starts_with('/') {
        raw_path.clone()
    } else {
        let cwd = process.inner_exclusive_access().cwd.clone();
        if cwd == "/" {
            format!("/{}", raw_path)
        } else {
            format!("{}/{}", cwd.trim_end_matches('/'), raw_path)
        }
    };
    if let Some(app_inode) = open_file(exec_path.as_str(), OpenFlags::empty()) {
        let all_data = app_inode.read_all();
        {
            let mut inner = process.inner_exclusive_access();
            let name = exec_path
                .rsplit('/')
                .find(|part| !part.is_empty())
                .unwrap_or(exec_path.as_str());
            inner.name = String::from(name);
        }
        process.exec(all_data.as_slice(), args, envs);
        0
    } else {
        errno(ENOENT)
    }
}

/// If there is not a child process whose pid is same as given, return -ECHILD.
/// Else if there is a child process but it is still running, return -EAGAIN.
pub fn sys_waitpid(pid: isize, exit_code_ptr: *mut i32) -> isize {
    loop {
        let process = current_process();
        let mut inner = process.inner_exclusive_access();
        if !inner.children.iter().any(|p| pid == -1 || pid as usize == p.getpid()) {
            return errno(ECHILD);
        }
        let pair = inner.children.iter().enumerate().find(|(_, p)| {
            p.inner_exclusive_access().is_zombie && (pid == -1 || pid as usize == p.getpid())
        });
        if let Some((idx, _)) = pair {
            let child = inner.children.remove(idx);
            if Arc::strong_count(&child) > 1 {
                trace!(
                    "kernel:pid[{}] waitpid: child pid {} has {} refs",
                    process.getpid(),
                    child.getpid(),
                    Arc::strong_count(&child)
                );
            }
            let found_pid = child.getpid();
            let exit_code = child.inner_exclusive_access().exit_code;
            if !exit_code_ptr.is_null() {
                let status = (exit_code & 0xff) << 8;
                *translated_refmut(inner.memory_set.token(), exit_code_ptr) = status;
            }
            return found_pid as isize;
        }
        drop(inner);
        suspend_current_and_run_next();
    }
}

/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
pub fn sys_get_time(_ts: *mut TimeVal, _tz: usize) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!("kernel:pid[{}] sys_get_time", pid);
    }
    if _ts.is_null() {
        return errno(EFAULT);
    }
    let us = get_time_us();
    let tv = TimeVal {
        sec: us / 1_000_000,
        usec: us % 1_000_000,
    };
    let bytes = unsafe {
        core::slice::from_raw_parts(
            (&tv as *const TimeVal) as *const u8,
            core::mem::size_of::<TimeVal>(),
        )
    };
    let token = current_user_token();
    match copy_to_user(token, _ts as *mut u8, bytes) {
        Ok(_) => 0,
        Err(err) => err,
    }
}

pub fn sys_nanosleep(req: *const TimeSpec, rem: *mut TimeSpec) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!("kernel:pid[{}] sys_nanosleep", pid);
    }
    let token = current_user_token();
    let req = match read_from_user::<TimeSpec>(token, req) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let sleep_us = req
        .tv_sec
        .saturating_mul(1_000_000)
        .saturating_add(req.tv_nsec / 1_000);
    let target = get_time_us().saturating_add(sleep_us);
    while get_time_us() < target {
        suspend_current_and_run_next();
    }
    if !rem.is_null() {
        let zero = TimeSpec::default();
        let bytes = unsafe {
            core::slice::from_raw_parts(
                (&zero as *const TimeSpec) as *const u8,
                core::mem::size_of::<TimeSpec>(),
            )
        };
        let _ = copy_to_user(token, rem as *mut u8, bytes);
    }
    0
}

pub fn sys_times(tms: *mut Tms) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!("kernel:pid[{}] sys_times", pid);
    }
    let ticks = (get_time() * 100 / CLOCK_FREQ) as i64;
    if !tms.is_null() {
        let tms_val = Tms {
            tms_utime: ticks,
            tms_stime: 0,
            tms_cutime: 0,
            tms_cstime: 0,
        };
        let bytes = unsafe {
            core::slice::from_raw_parts(
                (&tms_val as *const Tms) as *const u8,
                core::mem::size_of::<Tms>(),
            )
        };
        let token = current_user_token();
        if let Err(err) = copy_to_user(token, tms as *mut u8, bytes) {
            return err;
        }
    }
    ticks as isize
}

pub fn sys_uname(uts: *mut UtsName) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!("kernel:pid[{}] sys_uname", pid);
    }
    if uts.is_null() {
        return errno(EFAULT);
    }
    let mut uname = UtsName::default();
    fn fill(dst: &mut [u8], s: &str) {
        let bytes = s.as_bytes();
        let len = bytes.len().min(dst.len() - 1);
        dst[..len].copy_from_slice(&bytes[..len]);
        dst[len] = 0;
    }
    fill(&mut uname.sysname, "Linux");
    fill(&mut uname.nodename, "rcore");
    fill(&mut uname.release, "5.10.0");
    fill(&mut uname.version, "rcore");
    fill(&mut uname.machine, "riscv64");
    fill(&mut uname.domainname, "ruos");
    let bytes = unsafe {
        core::slice::from_raw_parts(
            (&uname as *const UtsName) as *const u8,
            core::mem::size_of::<UtsName>(),
        )
    };
    let token = current_user_token();
    match copy_to_user(token, uts as *mut u8, bytes) {
        Ok(_) => 0,
        Err(err) => err,
    }
}

/// YOUR JOB: Implement mmap.
pub fn sys_mmap(start: usize, len: usize, prot: usize, flags: usize, fd: usize, offset: usize) -> isize {
    const PROT_READ: usize = 0x1;
    const PROT_WRITE: usize = 0x2;
    const PROT_EXEC: usize = 0x4;
    const MAP_FIXED: usize = 0x10;
    const MAP_ANON: usize = 0x20;

    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!("kernel:pid[{}] sys_mmap", pid);
    }
    if len == 0 {
        return errno(EINVAL);
    }
    if start % PAGE_SIZE != 0 && (flags & MAP_FIXED) != 0 {
        return errno(EINVAL);
    }
    let mut map_perm = MapPermission::U;
    if (prot & PROT_READ) != 0 {
        map_perm |= MapPermission::R;
    }
    if (prot & PROT_WRITE) != 0 {
        map_perm |= MapPermission::W;
    }
    if (prot & PROT_EXEC) != 0 {
        map_perm |= MapPermission::X;
    }
    let len = (len + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
    let process = current_process();
    let mut inner = process.inner_exclusive_access();
    let start = if (flags & MAP_FIXED) != 0 && start != 0 {
        start
    } else {
        let base = (inner.mmap_base + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        inner.mmap_base = base + len;
        base
    };
    inner
        .memory_set
        .insert_framed_area(VirtAddr(start), VirtAddr(start + len), map_perm);
    drop(inner);

    // file-backed mapping (best-effort)
    if (flags & MAP_ANON) == 0 && fd != usize::MAX {
        if offset % PAGE_SIZE != 0 {
            return errno(EINVAL);
        }
        let inode = {
            let inner = process.inner_exclusive_access();
            if fd < inner.fd_table.len() {
                inner.fd_table[fd]
                    .as_ref()
                    .and_then(|file| file.inode())
            } else {
                None
            }
        };
        if let Some(inode) = inode {
            let token = current_user_token();
            let slices = translated_byte_buffer(token, start as *const u8, len);
            let mut file_off = offset;
            for slice in slices {
                let n = inode.read_at(file_off, slice);
                file_off += n;
                if n < slice.len() {
                    break;
                }
            }
        }
    }

    start as isize
}

/// YOUR JOB: Implement munmap.
pub fn sys_munmap(_start: usize, _len: usize) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!("kernel:pid[{}] sys_munmap", pid);
    }
    if _start % PAGE_SIZE != 0 || _len == 0 {
        return errno(EINVAL);
    }
    let process = current_process();
    let mut inner = process.inner_exclusive_access();
    inner
        .memory_set
        .remove_area_with_start_vpn(VirtAddr(_start).floor());
    0
}

/// change data segment size
pub fn sys_sbrk(arg: isize) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!("kernel:pid[{}] sys_sbrk", pid);
    }
    let process = current_process();
    let mut inner = process.inner_exclusive_access();
    let current_brk = inner.program_brk;
    let heap_bottom = inner.heap_bottom;
    if arg == 0 {
        return current_brk as isize;
    }
    let is_abs = (arg as usize) >= heap_bottom;
    let delta = if is_abs {
        arg - current_brk as isize
    } else {
        arg
    };
    let new_brk = (current_brk as isize + delta) as usize;
    if new_brk < heap_bottom {
        return errno(ENOMEM);
    }
    let result = if delta < 0 {
        inner
            .memory_set
            .shrink_to(VirtAddr(heap_bottom), VirtAddr(new_brk))
    } else {
        inner
            .memory_set
            .append_to(VirtAddr(heap_bottom), VirtAddr(new_brk))
    };
    if result {
        inner.program_brk = new_brk;
        if is_abs {
            current_brk as isize + delta
        } else {
            current_brk as isize
        }
    } else {
        errno(ENOMEM)
    }
}

/// YOUR JOB: Implement spawn.
/// HINT: fork + exec =/= spawn
pub fn sys_spawn(_path: *const u8) -> isize {
    trace!(
        "kernel:pid[{}] sys_spawn NOT IMPLEMENTED",
        current_process().pid.0
    );
    errno(ENOSYS)
}

// YOUR JOB: Set task priority.
pub fn sys_set_priority(_prio: isize) -> isize {
    trace!(
        "kernel:pid[{}] sys_set_priority NOT IMPLEMENTED",
        current_process().pid.0
    );
    errno(ENOSYS)
}

pub fn sys_kill(pid: usize, signum: i32) -> isize {
    let pid_now = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid_now) {
        trace!("kernel:pid[{}] sys_kill pid={} signum={}", pid_now, pid, signum);
    }
    if signum <= 0 || signum > MAX_SIG as i32 {
        return errno(EINVAL);
    }
    let flag = match SignalFlags::from_bits(1u32 << signum) {
        Some(flag) => flag,
        None => return errno(EINVAL),
    };
    let process = match pid2process(pid) {
        Some(process) => process,
        None => return errno(ESRCH),
    };
    let mut inner = process.inner_exclusive_access();
    inner.signal_pending |= flag;
    0
}

pub fn sys_sigaction(
    signum: i32,
    action: *const SignalAction,
    old_action: *mut SignalAction,
) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!("kernel:pid[{}] sys_sigaction signum={}", pid, signum);
    }
    if signum <= 0
        || signum > MAX_SIG as i32
        || signum == SignalFlags::SIGKILL.bits().trailing_zeros() as i32
    {
        return errno(EINVAL);
    }
    let process = current_process();
    let mut inner = process.inner_exclusive_access();
    let idx = signum as usize;
    let token = inner.memory_set.token();
    if !old_action.is_null() {
        let old = inner.signal_actions.table[idx];
        let bytes = unsafe {
            core::slice::from_raw_parts(
                (&old as *const SignalAction) as *const u8,
                core::mem::size_of::<SignalAction>(),
            )
        };
        if let Err(err) = copy_to_user(token, old_action as *mut u8, bytes) {
            return err;
        }
    }
    if !action.is_null() {
        let new_action = match read_from_user::<SignalAction>(token, action) {
            Ok(v) => v,
            Err(err) => return err,
        };
        inner.signal_actions.table[idx] = new_action;
    }
    0
}

pub fn sys_sigprocmask(mask: u32) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!("kernel:pid[{}] sys_sigprocmask mask=0x{:x}", pid, mask);
    }
    let process = current_process();
    let mut inner = process.inner_exclusive_access();
    inner.signal_mask = SignalFlags::from_bits_truncate(mask);
    0
}

pub fn sys_sigreturn() -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!("kernel:pid[{}] sys_sigreturn", pid);
    }
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    let saved = match inner.signal_trap_cx.take() {
        Some(cx) => cx,
        None => return errno(EINVAL),
    };
    let saved_a0 = saved.x[10] as isize;
    *inner.get_trap_cx() = saved;
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    process_inner.signal_mask = inner.signal_mask_backup;
    saved_a0
}

/// Get user ID
/// rcore-lab is a single-user system, always returns 0
pub fn sys_getuid() -> isize {
    trace!("kernel:pid[{}] sys_getuid", current_process().pid.0);
    0
}

/// Get effective user ID
/// rcore-lab is a single-user system, always returns 0
pub fn sys_geteuid() -> isize {
    trace!("kernel:pid[{}] sys_geteuid", current_process().pid.0);
    0
}

/// Get group ID
/// rcore-lab is a single-user system, always returns 0
pub fn sys_getgid() -> isize {
    trace!("kernel:pid[{}] sys_getgid", current_process().pid.0);
    0
}

/// Get effective group ID
/// rcore-lab is a single-user system, always returns 0
pub fn sys_getegid() -> isize {
    trace!("kernel:pid[{}] sys_getegid", current_process().pid.0);
    0
}

/// Exit all threads in the process
/// In rcore-lab, exit_group behaves the same as exit since we terminate the entire process
pub fn sys_exit_group(exit_code: i32) -> ! {
    trace!(
        "kernel:pid[{}] sys_exit_group (exit_code={})",
        current_process().pid.0,
        exit_code
    );
    sys_exit(exit_code)
}

pub fn sys_shutdown() -> ! {
    trace!(
        "kernel:pid[{}] sys_shutdown",
        current_process().pid.0);
    shutdown();
}
