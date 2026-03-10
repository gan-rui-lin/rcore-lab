//! Process management syscalls
//!
use alloc::format;
use alloc::sync::Arc;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use bitflags::bitflags;

use crate::{
    fs::{open_file, File, OpenFlags},
    mm::{translated_byte_buffer, translated_ref, translated_refmut, translated_str, MapPermission, PageTable, VirtAddr},
    task::{
        add_task, current_process, current_task, current_trap_cx, current_user_token,
        exit_current_and_run_next, futex_requeue, futex_remove_waiter, futex_remove_waiter_any,
        futex_wait, futex_wait_bitset, futex_wake, futex_wake_bitset, pid2process,
        suspend_current_and_run_next,
        FutexKey, RLimit, RLIMIT_NLIMITS, SignalAction, SignalFlags, TaskControlBlock,
        TaskStatus, UserContext, flags_to_user_mask, user_mask_to_flags,
        MAX_SIG, SIGKILL, SIGSTOP,
    },
    timer::{add_timer, get_time, get_time_ms, get_time_us, remove_timer},
};

use arch::TrapFrameArgs;

use super::errno::*;
use crate::config::{CLOCK_FREQ, PAGE_SIZE};

fn dump_user_bytes(tag: &str, token: usize, addr: usize, len: usize) {
    let page_table = PageTable::from_token(token);
    let end = addr.saturating_add(len);
    let mut cur = addr;
    while cur < end {
        let mut line = [0u8; 16];
        let mut any = false;
        let line_end = core::cmp::min(cur + 16, end);
        for i in 0..(line_end - cur) {
            let va = cur + i;
            if let Some(pa) = page_table.translate_va(VirtAddr::from(va)) {
                line[i] = *pa.get_ref::<u8>();
                any = true;
            }
        }
        if any {
            info!("[clone-tls] {} {:#x}: {:02x?}", tag, cur, &line[..(line_end - cur)]);
        } else {
            info!("[clone-tls] {} {:#x}: <unmapped>", tag, cur);
        }
        cur += 16;
    }
}

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
#[derive(Clone, Copy, Debug, Default)]
pub struct SysInfo {
    pub uptime: i64,
    pub loads: [u64; 3],
    pub totalram: u64,
    pub freeram: u64,
    pub sharedram: u64,
    pub bufferram: u64,
    pub totalswap: u64,
    pub freeswap: u64,
    pub procs: u16,
    pub pad: u16,
    pub totalhigh: u64,
    pub freehigh: u64,
    pub mem_unit: u32,
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

bitflags! {
    struct FutexCmd: u32 {
        const FUTEX_WAIT = 0;
        const FUTEX_WAKE = 1;
        const FUTEX_REQUEUE = 3;
        const FUTEX_WAIT_BITSET = 9;
        const FUTEX_WAKE_BITSET = 10;
    }
}

bitflags! {
    struct FutexOpt: u32 {
        const FUTEX_PRIVATE_FLAG = 0x80;
        const FUTEX_CLOCK_REALTIME = 0x100;
    }
}

bitflags! {
    pub struct CloneFlags: u64 {
        const VM = 0x0000_0100;
        const FS = 0x0000_0200;
        const FILES = 0x0000_0400;
        const SIGHAND = 0x0000_0800;
        const VFORK = 0x0000_4000;
        const THREAD = 0x0001_0000;
        const SYSVSEM = 0x0004_0000;
        const SETTLS = 0x0008_0000;
        const PARENT_SETTID = 0x0010_0000;
        const CHILD_CLEARTID = 0x0020_0000;
        const CHILD_SETTID = 0x0100_0000;
    }
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

pub fn sys_futex(
    uaddr1: *mut i32,
    futex_op: u32,
    val: i32,
    timeout: *const TimeSpec,
    uaddr2: *mut i32,
    val3: i32,
) -> isize {
    let pid_now = current_process().pid.0;
    if uaddr1.is_null() || (uaddr1 as usize) % core::mem::size_of::<i32>() != 0 {
        return errno(EINVAL);
    }
    let cmd = FutexCmd::from_bits_truncate(futex_op & 0x7f);
    let opt = FutexOpt::from_bits_truncate(futex_op);
    let token = current_user_token();
    let page_table = PageTable::from_token(token);
    let pa = match page_table.translate_va(VirtAddr::from(uaddr1 as usize)) {
        Some(pa) => pa,
        None => return errno(EFAULT),
    };
    let private = opt.contains(FutexOpt::FUTEX_PRIVATE_FLAG);
    let pid = if private { current_process().pid.0 } else { 0 };
    let key = FutexKey::new(pa, pid);
    let name = current_process().inner_exclusive_access().name.clone();
    if matches!(cmd, FutexCmd::FUTEX_WAIT | FutexCmd::FUTEX_WAIT_BITSET) && name == "entry-static.exe" {
        let tid = current_task()
            .and_then(|task| task.inner_exclusive_access().res.as_ref().map(|r| r.tid))
            .unwrap_or(0);
        info!(
            "[sys_futex] pid={} tid={} name={} cmd={:?} uaddr1={:#x} pa={:#x} private={} val={}",
            pid_now,
            tid,
            name,
            cmd,
            uaddr1 as usize,
            pa.0,
            private,
            val
        );
    }
    trace!(
        "[sys_futex] pid={} op={:#x} cmd={:?} private={} uaddr1={:#x} pa={:#x} val={} uaddr2={:#x} val3={} timeout={}",
        pid_now,
        futex_op,
        cmd,
        private,
        uaddr1 as usize,
        pa.0,
        val,
        uaddr2 as usize,
        val3,
        !timeout.is_null()
    );

    match cmd {
        FutexCmd::FUTEX_WAIT => {
            let futex_word = translated_ref(token, uaddr1);
            if *futex_word != val {
                trace!(
                    "[sys_futex] pid={} wait mismatch word={} val={}",
                    pid_now,
                    *futex_word,
                    val
                );
                return errno(EAGAIN);
            }
            let has_timeout = !timeout.is_null();
            if has_timeout {
                let spec = match read_from_user(token, timeout) {
                    Ok(value) => value,
                    Err(err) => return err,
                };
                if spec.tv_nsec >= 1_000_000_000 {
                    return errno(EINVAL);
                }
                let add_ms = spec
                    .tv_sec
                    .saturating_mul(1000)
                    .saturating_add(spec.tv_nsec / 1_000_000);
                let expire_ms = get_time_ms().saturating_add(add_ms);
                add_timer(expire_ms, current_task().unwrap());
            }
            trace!(
                "[sys_futex] pid={} wait sleep word={} timeout={}",
                pid_now,
                *futex_word,
                has_timeout
            );
            let ret = futex_wait(key.clone());
            if has_timeout {
                let task = current_task().unwrap();
                let timed_out = futex_remove_waiter(&key, &task);
                remove_timer(task);
                if timed_out {
                    trace!("[sys_futex] pid={} wait timed out", pid_now);
                    return errno(ETIMEDOUT);
                }
            }
            ret
        }
        FutexCmd::FUTEX_WAKE => {
            let woke = futex_wake(key, val as usize) as isize;
            trace!("[sys_futex] pid={} wake n={}", pid_now, woke);
            woke
        }
        FutexCmd::FUTEX_REQUEUE => {
            if uaddr2.is_null() {
                return errno(EINVAL);
            }
            let pa2 = match page_table.translate_va(VirtAddr::from(uaddr2 as usize)) {
                Some(pa) => pa,
                None => return errno(EFAULT),
            };
            let new_key = FutexKey::new(pa2, pid);
            let requeued = futex_requeue(key, val, new_key, val3) as isize;
            trace!(
                "[sys_futex] pid={} requeue n={} uaddr2={:#x} pa2={:#x}",
                pid_now,
                requeued,
                uaddr2 as usize,
                pa2.0
            );
            requeued
        }
        FutexCmd::FUTEX_WAIT_BITSET => {
            if val3 == 0 {
                return errno(EINVAL);
            }
            let futex_word = translated_ref(token, uaddr1);
            if *futex_word != val {
                trace!(
                    "[sys_futex] pid={} wait_bitset mismatch word={} val={}",
                    pid_now,
                    *futex_word,
                    val
                );
                return errno(EAGAIN);
            }
            let has_timeout = !timeout.is_null();
            if has_timeout {
                let spec = match read_from_user(token, timeout) {
                    Ok(value) => value,
                    Err(err) => return err,
                };
                if spec.tv_nsec >= 1_000_000_000 {
                    return errno(EINVAL);
                }
                let add_ms = spec
                    .tv_sec
                    .saturating_mul(1000)
                    .saturating_add(spec.tv_nsec / 1_000_000);
                let expire_ms = get_time_ms().saturating_add(add_ms);
                add_timer(expire_ms, current_task().unwrap());
            }
            trace!(
                "[sys_futex] pid={} wait_bitset sleep word={} bitset={:#x} timeout={}",
                pid_now,
                *futex_word,
                val3,
                has_timeout
            );
            let ret = futex_wait_bitset(key.clone(), val3);
            if has_timeout {
                let task = current_task().unwrap();
                let timed_out = futex_remove_waiter(&key, &task);
                remove_timer(task);
                if timed_out {
                    trace!("[sys_futex] pid={} wait_bitset timed out", pid_now);
                    return errno(ETIMEDOUT);
                }
            }
            ret
        }
        FutexCmd::FUTEX_WAKE_BITSET => {
            if val3 == 0 {
                return errno(EINVAL);
            }
            let woke = futex_wake_bitset(key, val as usize, val3) as isize;
            trace!("[sys_futex] pid={} wake_bitset n={} bitset={:#x}", pid_now, woke, val3);
            woke
        }
        _ => errno(ENOSYS),
    }
}

pub fn sys_exit(exit_code: i32) -> ! {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_exit", pid);
    }
    let name = current_process().inner_exclusive_access().name.clone();
    if pid == 4 || name == "sh" {
        trace!(
            "[sys_exit] pid={} name={} code={} sepc={:#x}",
            pid,
            name,
            exit_code,
            current_trap_cx().sepc
        );
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
        syscall!("kernel:pid[{}] sys_fork", pid);
    }
    let current_process = current_process();
    let new_process = current_process.fork();
    let new_pid = new_process.pid.0;
    let new_task = new_process.inner_exclusive_access().get_task(0);
    let parent_cx = *current_trap_cx();
    let clone_stack = parent_cx[TrapFrameArgs::ARG1];
    let new_task_inner = new_task.inner_exclusive_access();
    let trap_cx = new_task_inner.get_trap_cx();
    *trap_cx = parent_cx;
    trap_cx[TrapFrameArgs::RET] = 0;
    #[cfg(target_arch = "loongarch64")]
    if trap_cx[TrapFrameArgs::TLS] == 0 {
        trap_cx[TrapFrameArgs::TLS] = 0x7000_1000;
    }
    if clone_stack != 0 {
        trap_cx[TrapFrameArgs::SP] = clone_stack;
    }
    trap_cx.kernel_sp = new_task.kstack.get_top();
    new_pid as isize
}

pub fn sys_clone(
    flags: usize,
    stack: *const u8,
    ptid: *mut i32,
    tls: *mut i32,
    ctid: *mut i32,
) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!(
            "kernel:pid[{}] sys_clone flags={:#x} stack={:#x} ptid={:#x} tls={:#x} ctid={:#x}",
            pid,
            flags,
            stack as usize,
            ptid as usize,
            tls as usize,
            ctid as usize
        );
    }
    let exit_signal = (flags & 0xff) as i32;
    if exit_signal != 0 && (exit_signal <= 0 || exit_signal > MAX_SIG as i32) {
        return errno(EINVAL);
    }
    let clone_flags = CloneFlags::from_bits_truncate((flags as u64) & !0xff);
    info!(
        "[clone] flags={:#x} clone_flags={:#x} stack={:#x} ptid={:#x} tls={:#x} ctid={:#x}",
        flags,
        clone_flags.bits(),
        stack as usize,
        ptid as usize,
        tls as usize,
        ctid as usize
    );

    if !clone_flags.contains(CloneFlags::THREAD) {
        return sys_fork();
    }

    let task = current_task().unwrap();
    let process = task.process.upgrade().unwrap();
    let parent_cx = *current_trap_cx();

    let parent_task_inner = task.inner_exclusive_access();
    let ustack_base = parent_task_inner.res.as_ref().unwrap().ustack_base;
    let parent_signal_mask = parent_task_inner.signal_mask;
    drop(parent_task_inner);

    let alloc_user_stack = stack.is_null();
    let new_task = Arc::new(TaskControlBlock::new(
        Arc::clone(&process),
        ustack_base,
        alloc_user_stack,
    ));
    // 新线程继承父线程的信号掩码（Linux 语义：clone(CLONE_THREAD) 继承 signal_mask）
    {
        let mut inner = new_task.inner_exclusive_access();
        inner.signal_mask = parent_signal_mask;
    }
    add_task(Arc::clone(&new_task));

    let new_task_inner = new_task.inner_exclusive_access();
    let new_task_res = new_task_inner.res.as_ref().unwrap();
    let new_task_tid = new_task_res.tid;
    let mut process_inner = process.inner_exclusive_access();
    let tasks = &mut process_inner.tasks;
    while tasks.len() < new_task_tid + 1 {
        tasks.push(None);
    }
    tasks[new_task_tid] = Some(Arc::clone(&new_task));
    drop(process_inner);

    let new_trap_cx = new_task_inner.get_trap_cx();
    *new_trap_cx = parent_cx;
    new_trap_cx[TrapFrameArgs::RET] = 0;
    if !stack.is_null() {
        new_trap_cx[TrapFrameArgs::SP] = stack as usize;
    }
    if clone_flags.contains(CloneFlags::SETTLS) && !tls.is_null() {
        new_trap_cx[TrapFrameArgs::TLS] = tls as usize;
        let name = current_process().inner_exclusive_access().name.clone();
        if name == "entry-static.exe" {
            let token = current_user_token();
            let tls_addr = tls as usize;
            info!(
                "[clone-tls] pid={} tid={} tls={:#x} stack={:#x}",
                pid,
                new_task_tid,
                tls_addr,
                stack as usize
            );
            let base = tls_addr.saturating_sub(256);
            dump_user_bytes("tp-0x100", token, base, 128);
            dump_user_bytes("tp-0x80", token, tls_addr.saturating_sub(128), 128);
            dump_user_bytes("tp+0x0", token, tls_addr, 64);
        }
    }
    new_trap_cx.kernel_sp = new_task.kstack.get_top();
    drop(new_task_inner);
    if clone_flags.contains(CloneFlags::CHILD_CLEARTID) && !ctid.is_null() {
        let mut inner = new_task.inner_exclusive_access();
        inner.clear_child_tid = ctid as usize;
        info!(
            "[clone] pid={} tid={} child_cleartid={:#x}",
            pid,
            new_task_tid,
            ctid as usize
        );
    }

    if clone_flags.contains(CloneFlags::PARENT_SETTID) && !ptid.is_null() {
        let token = current_user_token();
        let ptid_addr = ptid as usize;
        let pt = PageTable::from_token(token);
        let vpn = VirtAddr::from(ptid_addr).floor();
        match pt.translate(vpn) {
            Some(pte) => info!(
                "[clone] ptid addr={:#x} vpn={:?} pte_valid={} flags={:?}",
                ptid_addr,
                vpn,
                pte.is_valid(),
                pte.flags()
            ),
            None => info!("[clone] ptid addr={:#x} vpn={:?} pte=None", ptid_addr, vpn),
        }
        *translated_refmut(token, ptid) = new_task_tid as i32;
    }
    if clone_flags.contains(CloneFlags::CHILD_SETTID) && !ctid.is_null() {
        let token = new_task.get_user_token();
        let ctid_addr = ctid as usize;
        let pt = PageTable::from_token(token);
        let vpn = VirtAddr::from(ctid_addr).floor();
        match pt.translate(vpn) {
            Some(pte) => info!(
                "[clone] ctid addr={:#x} vpn={:?} pte_valid={} flags={:?}",
                ctid_addr,
                vpn,
                pte.is_valid(),
                pte.flags()
            ),
            None => info!("[clone] ctid addr={:#x} vpn={:?} pte=None", ctid_addr, vpn),
        }
        *translated_refmut(token, ctid) = new_task_tid as i32;
    }

    new_task_tid as isize
}

/// Maximum depth for shebang recursion to prevent infinite loops
const MAX_SHEBANG_DEPTH: usize = 4;

/// Parse shebang line and return (interpreter_path, optional_arg)
fn parse_shebang(data: &[u8]) -> Option<(String, Option<String>)> {
    // Check for shebang marker
    if data.len() < 2 || data[0] != b'#' || data[1] != b'!' {
        return None;
    }

    // Find the end of first line
    let line_end = data.iter().position(|&b| b == b'\n' || b == b'\r').unwrap_or(data.len());
    if line_end <= 2 {
        return None;
    }

    // Extract the shebang line (skip #!)
    let shebang_line = &data[2..line_end];

    // Convert to string and trim whitespace
    let shebang_str = core::str::from_utf8(shebang_line).ok()?.trim();
    if shebang_str.is_empty() {
        return None;
    }

    let mut parts = shebang_str.split_whitespace();
    let interpreter = String::from(parts.next()?);
    let arg = parts.next().map(String::from);
    Some((interpreter, arg))
}

fn resolve_relative_path(path: &str) -> String {
    let cwd = current_process().inner_exclusive_access().cwd.clone();
    if cwd == "/" {
        format!("/{}", path)
    } else {
        format!("{}/{}", cwd.trim_end_matches('/'), path)
    }
}

fn build_exec_candidates(exec_path: &str, envs: &[String]) -> Vec<String> {
    // 绝对路径直接返回
    if exec_path.starts_with('/') {
        return vec![String::from(exec_path)];
    }
    // 相对路径需要基于当前工作目录拼接
    if exec_path.contains('/') {
        return vec![resolve_relative_path(exec_path)];
    }
    let mut candidates = Vec::new();
    // 根据 PATH 环境变量拼接候选路径，PATH 以冒号分隔多个目录，空目录表示当前目录
    if let Some(path_env) = envs.iter().find(|env| env.starts_with("PATH=")) {
        let path_value = &path_env[5..];
        for dir in path_value.split(':') {
            if dir.is_empty() {
                continue;
            }
            let candidate = if dir == "/" {
                format!("/{}", exec_path)
            } else {
                format!("{}/{}", dir.trim_end_matches('/'), exec_path)
            };
            candidates.push(candidate);
        }
    }
    if candidates.is_empty() {
        candidates.push(resolve_relative_path(exec_path));
    }
    candidates
}

fn trace_exec_resolution(
    name: &str,
    exec_path: &str,
    exec_path_resolved: &str,
    args: &[String],
) {
    if name == "busybox"
        && (exec_path.contains("run-all.sh")
            || exec_path.contains("/basic/")
            || exec_path.starts_with("./"))
    {
        let argv0 = args.get(0).cloned().unwrap_or_default();
        let argv1 = args.get(1).cloned().unwrap_or_default();
        trace!(
            "[sys_exec] pid={} name={} raw={} resolved={} argv0={} argv1={}",
            current_process().pid.0,
            name,
            exec_path,
            exec_path_resolved,
            argv0,
            argv1
        );
    }
}

fn trace_entry_bytes(exec_path_resolved: &str, all_data: &[u8], app: &Arc<dyn File>) {
    if exec_path_resolved != "/bin/sh" && exec_path_resolved != "/musl/busybox" {
        return;
    }
    if let Ok(elf) = xmas_elf::ElfFile::new(all_data) {
        let entry = elf.header.pt2.entry_point() as usize;
        let ph_count = elf.header.pt2.ph_count() as usize;
        for idx in 0..ph_count {
            if let Ok(ph) = elf.program_header(idx as u16) {
                if ph.get_type().ok() != Some(xmas_elf::program::Type::Load) {
                    continue;
                }
                let vaddr = ph.virtual_addr() as usize;
                let filesz = ph.file_size() as usize;
                if entry < vaddr || entry >= vaddr.saturating_add(filesz) {
                    continue;
                }
                let offset = ph.offset() as usize;
                let file_off = offset + (entry - vaddr);
                let end = (file_off + 8).min(all_data.len());
                if end > file_off {
                    let label = if exec_path_resolved == "/bin/sh" {
                        "/bin/sh"
                    } else {
                        "/musl/busybox"
                    };
                    let entry_bytes = &all_data[file_off..end];
                    trace!("[sys_exec] {} entry bytes={:02x?}", label, entry_bytes);
                    if exec_path_resolved == "/musl/busybox"
                        && entry_bytes.iter().all(|b| *b == 0)
                    {
                        let head_len = all_data.len().min(16);
                        trace!(
                            "[sys_exec] busybox read_all len={} head={:02x?}",
                            all_data.len(),
                            &all_data[..head_len]
                        );
                        if let Some(inode) = app.inode() {
                            let mut buf = [0u8; 8];
                            let n = inode.read_at(file_off, &mut buf);
                            trace!(
                                "[sys_exec] busybox inode.read_at off={:#x} n={} bytes={:02x?}",
                                file_off,
                                n,
                                &buf[..n]
                            );
                            trace!("[sys_exec] busybox inode.size={}", inode.size());
                        } else {
                            trace!("[sys_exec] busybox inode missing");
                        }
                    }
                }
                break;
            }
        }
    }
}

fn trace_run_all_head(name: &str, exec_path: &str, all_data: &[u8]) {
    if name == "busybox" && exec_path.contains("run-all.sh") {
        let head_len = all_data.len().min(16);
        let head = &all_data[..head_len];
        trace!("[sys_exec] run-all.sh head={:02x?} len={}", head, all_data.len());
    }
}

fn sys_exec_internal(path: *const u8, argv: *const usize, envp: *const usize, depth: usize) -> isize {
    if path.is_null() {
        return errno(EFAULT);
    }
    let token = current_user_token();
    let mut exec_path = translated_str(token, path);
    let mut args: Vec<String> = Vec::new();
    if !argv.is_null() {
        let mut argv = argv;
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
        args.push(exec_path.clone());
    }
    let mut envs: Vec<String> = Vec::new();
    if !envp.is_null() {
        let mut envp = envp;
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

    let mut depth = depth;
    loop {
        if exec_path == "/bin/sh" {
            // 应该已经提前做了硬链接
            if open_file("/bin/sh", OpenFlags::empty()).is_none() {
                if open_file("/musl/busybox", OpenFlags::empty()).is_some() {
                    info!("[sys_exec] pid={} exec /bin/sh fallback to /musl/busybox", current_process().pid.0);
                    exec_path = String::from("/musl/busybox");
                } else {
                    error!("[sys_exec] pid={} exec /bin/sh fallback busybox also not found", current_process().pid.0);
                    return errno(ENOENT);
                }
            } else {
                trace!("[sys_exec] /bin/sh ready");
            }

        }
        let mut resolved_path = None;
        let mut app = None;
        // 根据 exec_path 和 PATH 环境变量构建候选路径列表，并尝试打开找到第一个存在的文件
        for candidate in build_exec_candidates(exec_path.as_str(), &envs) {
            if let Some(found) = open_file(candidate.as_str(), OpenFlags::empty()) {
                resolved_path = Some(candidate);
                app = Some(found);
                break;
            }
        }
        // 如果没有找到任何候选文件，返回 ENOENT
        let Some(app) = app else {
            let name = current_process().inner_exclusive_access().name.clone();
            if name == "busybox" && exec_path.starts_with("./") {
                trace!(
                    "[sys_exec] pid={} name={} raw={} -> ENOENT (no candidate)",
                    current_process().pid.0,
                    name,
                    exec_path
                );
            }
            return errno(ENOENT);
        };
        // 如果找到的候选文件路径和原始 exec_path 不同，说明是通过 PATH 环境变量解析得到的，打印解析信息
        let exec_path_resolved = resolved_path.unwrap_or_else(|| exec_path.clone());
        let name = current_process().inner_exclusive_access().name.clone();
        trace_exec_resolution(&name, &exec_path, &exec_path_resolved, &args);
        let all_data = app.read_all();
        trace_entry_bytes(&exec_path_resolved, &all_data, &app);
        trace_run_all_head(&name, &exec_path, &all_data);
        // Prefer ELF execution; only try shebang (or /bin/sh fallback) for non-ELF.
        let is_elf = all_data.len() >= 4 && &all_data[..4] == b"\x7fELF";
        if !is_elf {
            // Check for shebang scripts first.
            if let Some((interpreter, opt_arg)) = parse_shebang(&all_data) {
                trace!(
                    "[sys_exec] shebang interp={} opt_arg={:?} script={}",
                    interpreter,
                    opt_arg,
                    exec_path_resolved
                );
                if depth >= MAX_SHEBANG_DEPTH {
                    return errno(ELOOP);
                }
                depth += 1;
                trace!("[sys_exec] shebang interpreter ready: {}", interpreter);
                let interp_basename = interpreter
                    .rsplit('/')
                    .find(|part| !part.is_empty())
                    .unwrap_or(interpreter.as_str());
                let mut new_args: Vec<String> = Vec::new();
                if interp_basename == "sh" {
                    new_args.push(String::from("sh"));
                } else {
                    new_args.push(interpreter.clone());
                }
                if let Some(arg) = opt_arg {
                    new_args.push(arg);
                }
                new_args.push(exec_path_resolved.clone());
                if args.len() > 1 {
                    new_args.extend(args.into_iter().skip(1));
                }
                args = new_args;
                exec_path = interpreter;
                continue;
            }
            // Non-ELF without shebang: fall back to /bin/sh (execlp-like behavior).
            if depth >= MAX_SHEBANG_DEPTH {
                return errno(ELOOP);
            }
            depth += 1;
            let mut new_args: Vec<String> = Vec::new();
            new_args.push(String::from("sh"));
            new_args.push(exec_path_resolved.clone());
            if args.len() > 1 {
                new_args.extend(args.into_iter().skip(1));
            }
            args = new_args;
            exec_path = String::from("/bin/sh");
            warn!(
                "[sys_exec] non-ELF without shebang, fallback to /bin/sh for {} with args {:?}. BE CAREFUL OF IT.for run-static.sh/run-dynamic.sh, it's fine",
                exec_path_resolved, args
            );
            continue;
        }
        let mut interp_data: Option<Vec<u8>> = None;
        if let Ok(elf) = xmas_elf::ElfFile::new(all_data.as_slice()) {
            let mut interp: Option<String> = None;
            for i in 0..elf.header.pt2.ph_count() {
                let ph = elf.program_header(i).unwrap();
                if ph.get_type().unwrap() == xmas_elf::program::Type::Interp {
                    let interp_start = ph.offset() as usize;
                    let interp_end = interp_start + ph.file_size() as usize;
                    if interp_end <= all_data.len() {
                        if let Ok(interp_str) = core::str::from_utf8(&all_data[interp_start..interp_end]) {
                            interp = Some(String::from(interp_str.trim_end_matches('\0')));
                        }
                    }
                    break;
                }
            }
            if let Some(mut interp_path) = interp {
                if open_file(interp_path.as_str(), OpenFlags::empty()).is_none() {
                    // Fallback: try musl/glibc loader for known interpreter paths
                    let musl_loader = "/musl/lib/libc.so";
                    let glibc_loader = "/glibc/lib/ld-linux-riscv64-lp64d.so.1";
                    if interp_path == "/lib/ld-linux-riscv64-lp64d.so.1"
                        || interp_path.contains("ld-musl-riscv64")
                    {
                        if open_file(musl_loader, OpenFlags::empty()).is_some() {
                            info!("[sys_exec] interp {} not found, fallback to musl loader: {}", interp_path, musl_loader);
                            interp_path = String::from(musl_loader);
                        } else if open_file(glibc_loader, OpenFlags::empty()).is_some() {
                            info!("[sys_exec] interp {} not found, fallback to glibc loader: {}", interp_path, glibc_loader);
                            interp_path = String::from(glibc_loader);
                        }
                    }
                }
                if open_file(interp_path.as_str(), OpenFlags::empty()).is_none() {
                    trace!("[sys_exec] interp not found: {}", interp_path);
                    return errno(ENOENT);
                }
                if let Some(interp_file) = open_file(interp_path.as_str(), OpenFlags::empty()) {
                    interp_data = Some(interp_file.read_all());
                } else {
                    error!("[sys_exec] interp open failed: {}", interp_path);
                    return errno(ENOENT);
                }
            }
        }
        let process = current_process();
        {
            let mut inner = process.inner_exclusive_access();
            let name = exec_path
                .rsplit('/')
                .find(|part| !part.is_empty())
                .unwrap_or(exec_path.as_str());
            inner.name = String::from(name);
            trace!(
                "[sys_exec] set process name to {} for path {}",
                name,
                exec_path
            );
        }
        // if exec_path == "/bin/sh" {
        //     let argv0 = args.get(0).cloned().unwrap_or_default();
        //     let argv1 = args.get(1).cloned().unwrap_or_default();
        //     trace!("[sys_exec] /bin/sh argv0={} argv1={}", argv0, argv1);
        // }
        process.exec_with_interp(all_data.as_slice(), interp_data.as_deref(), args, envs);
        let after_name = current_process().inner_exclusive_access().name.clone();
        trace!("[sys_exec] after exec name={}", after_name);
        return 0;
    }
}

pub fn sys_exec(path: *const u8, argv: *const usize, envp: *const usize) -> isize {
    sys_exec_internal(path, argv, envp, 0)
}

/// If there is not a child process whose pid is same as given, return -ECHILD.
/// Else if there is a child process but it is still running, return -EAGAIN.
pub fn sys_waitpid(pid: isize, exit_code_ptr: *mut i32) -> isize {
    loop {
        let process = current_process();
        let mut inner = process.inner_exclusive_access();
        let _trace_pid = process.getpid();
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
        // let child_count = inner.children.len();
        // let pending = inner.signal_pending;
        // let mask = inner.signal_mask;
        // let name = inner.name.clone();
        drop(inner);
        // if crate::syscall::should_trace_syscall(trace_pid) {
        //     trace!(
        //         "[sys_waitpid] pid={} name={} child_count={} pending={:?} mask={:?} -> yield",
        //         trace_pid,
        //         name,
        //         child_count,
        //         pending,
        //         mask
        //     );
        //     crate::task::debug_dump_tasks();
        // }
        suspend_current_and_run_next();
    }
}

/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
pub fn sys_get_time(_ts: *mut TimeVal, _tz: usize) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_get_time", pid);
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

pub fn sys_clock_gettime(clock_id: usize, ts: *mut TimeSpec) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_clock_gettime", pid);
    }
    if ts.is_null() {
        return errno(EFAULT);
    }
    match clock_id {
        0 | 1 => {
            let us = get_time_us();
            let spec = TimeSpec {
                tv_sec: us / 1_000_000,
                tv_nsec: (us % 1_000_000) * 1_000,
            };
            let bytes = unsafe {
                core::slice::from_raw_parts(
                    (&spec as *const TimeSpec) as *const u8,
                    core::mem::size_of::<TimeSpec>(),
                )
            };
            let token = current_user_token();
            match copy_to_user(token, ts as *mut u8, bytes) {
                Ok(_) => 0,
                Err(err) => err,
            }
        }
        _ => errno(EINVAL),
    }
}

pub fn sys_nanosleep(req: *const TimeSpec, rem: *mut TimeSpec) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_nanosleep", pid);
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
        syscall!("kernel:pid[{}] sys_times", pid);
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
        syscall!("kernel:pid[{}] sys_uname", pid);
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

pub fn sys_syslog(_log_type: usize, buf: *mut u8, len: usize) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_syslog", pid);
    }
    if len == 0 {
        return 0;
    }
    if buf.is_null() {
        return errno(EFAULT);
    }
    let token = current_user_token();
    let out = [0u8; 1];
    match copy_to_user(token, buf, &out) {
        Ok(_) => 0,
        Err(err) => err,
    }
}

pub fn sys_sysinfo(info: *mut SysInfo) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_sysinfo", pid);
    }
    if info.is_null() {
        return errno(EFAULT);
    }
    let us = get_time_us();
    let data = SysInfo {
        uptime: (us / 1_000_000) as i64,
        loads: [0, 0, 0],
        totalram: 0,
        freeram: 0,
        sharedram: 0,
        bufferram: 0,
        totalswap: 0,
        freeswap: 0,
        procs: 1,
        pad: 0,
        totalhigh: 0,
        freehigh: 0,
        mem_unit: 1,
    };
    let bytes = unsafe {
        core::slice::from_raw_parts(
            (&data as *const SysInfo) as *const u8,
            core::mem::size_of::<SysInfo>(),
        )
    };
    let token = current_user_token();
    match copy_to_user(token, info as *mut u8, bytes) {
        Ok(_) => 0,
        Err(err) => err,
    }
}

/// YOUR JOB: Implement mmap.
pub fn sys_mmap(start: usize, len: usize, prot: usize, flags: usize, fd: usize, offset: usize) -> isize {
    const PROT_READ: usize = 0x1;
    const PROT_WRITE: usize = 0x2;
    const PROT_EXEC: usize = 0x4;
    const MAP_PRIVATE: usize = 0x02;
    const MAP_FIXED: usize = 0x10;
    const MAP_ANON: usize = 0x20;

    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_mmap", pid);
    }

    let mut len = len;
    if len == 0 {
        warn!(
            "kernel:pid[{}] sys_mmap with zero length, treating as 1 page",
            pid
        );
        //// 针对 musl busybox 的问题排查代码
        // let cx = current_trap_cx();
        // let sepc = cx.sepc;
        // let ra = cx[TrapFrameArgs::RA];
        // let gp = cx.gp();
        // let process = current_process();
        // let inner = process.inner_exclusive_access();
        // let name = inner.name.clone();
        // let heap_bottom = inner.heap_bottom;
        // let program_brk = inner.program_brk;
        // let mmap_base = inner.mmap_base;
        // drop(inner);
        // let token = current_user_token();
        // let heap_struct = gp.wrapping_add(0x688);
        // let heap_align = *translated_ref(token, (heap_struct + 0x38) as *const usize);
        // let heap_min = *translated_ref(token, (heap_struct + 0x10) as *const usize);
        // let malloc_shift = *translated_ref(token, (gp + 0x140) as *const u32);
        if (flags & MAP_ANON) != 0 && (flags & MAP_PRIVATE) != 0 {
            // trace!(
            //     "[sys_mmap] pid={} name={} sepc={:#x} ra={:#x} gp={:#x} req={:#x} len=0 prot={:#x} flags={:#x} fd={} off={:#x} hb={:#x} brk={:#x} mmap_base={:#x} heap_align={:#x} heap_min={:#x} malloc_shift={} -> compat map 1 page",
            //     pid,
            //     name,
            //     sepc,
            //     ra,
            //     gp,
            //     start,
            //     prot,
            //     flags,
            //     fd,
            //     offset,
            //     heap_bottom,
            //     program_brk,
            //     mmap_base,
            //     heap_align,
            //     heap_min,
            //     malloc_shift
            // );
            len = PAGE_SIZE;
        } else {
            // trace!(
            //     "[sys_mmap] pid={} name={} sepc={:#x} ra={:#x} gp={:#x} req={:#x} len=0 prot={:#x} flags={:#x} fd={} off={:#x} hb={:#x} brk={:#x} mmap_base={:#x} heap_align={:#x} heap_min={:#x} malloc_shift={} -> EINVAL",
            //     pid,
            //     name,
            //     sepc,
            //     ra,
            //     gp,
            //     start,
            //     prot,
            //     flags,
            //     fd,
            //     offset,
            //     heap_bottom,
            //     program_brk,
            //     mmap_base,
            //     heap_align,
            //     heap_min,
            //     malloc_shift
            // );
            return errno(EINVAL);
        }
    }

    if start % PAGE_SIZE != 0 && (flags & MAP_FIXED) != 0 {
        error!(
            "start addr should be page aligned when MAP_FIXED is set, but got {:#x} in pid {}",
            pid, start
        );
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
    // 获取页对齐的长度，如果 len 已经是页大小的整数倍，则保持不变；否则向上调整到下一个页边界。
    let len = (len + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);

    // 获取 mmap 的起始地址，如果是固定映射且提供了非零的起始地址，则使用该地址；否则根据当前进程的 mmap_base 来分配一个合适的地址，并更新 mmap_base。
    let process = current_process();
    let mut inner = process.inner_exclusive_access();
    let req_start = start;
    let is_fixed = (flags & MAP_FIXED) != 0 && req_start != 0;
    let start = if is_fixed {
        req_start
    } else if req_start != 0 {
        let base = (req_start + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        if base + len > inner.mmap_base {
            inner.mmap_base = base + len;
        }
        base
    } else {
        let base = (inner.mmap_base + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        inner.mmap_base = base + len;
        base
    };
    if inner.name == "busybox" || inner.name == "ld-linux-riscv64-lp64d.so.1" {
        let overlap = inner
            .memory_set
            .overlap_count(VirtAddr(start), VirtAddr(start + len));
        trace!(
            "[sys_mmap] pid={} name={} req={:#x} len={:#x} flags={:#x} -> start={:#x} overlap={} fixed={}",
            pid,
            inner.name,
            req_start,
            len,
            flags,
            start,
            overlap,
            is_fixed
        );
        if is_fixed && overlap > 0 {
            let ranges = inner
                .memory_set
                .overlap_ranges(VirtAddr(start), VirtAddr(start + len));
            for (idx, (r_start, r_end)) in ranges.into_iter().enumerate() {
                trace!(
                    "[sys_mmap] pid={} fixed overlap[{}]=[{:#x},{:#x})",
                    pid,
                    idx,
                    r_start.0,
                    r_end.0
                );
            }
        }
    }
    trace!(
        "[sys_mmap] pid={} req={:#x} len={:#x} flags={:#x} -> start={:#x}",
        pid,
        req_start,
        len,
        flags,
        start
    );

    let overlap = inner
        .memory_set
        .overlap_count(VirtAddr(start), VirtAddr(start + len));
    if overlap > 0 {
        if inner.name == "busybox" || inner.name == "ld-linux-riscv64-lp64d.so.1" {
            let ranges = inner
                .memory_set
                .overlap_ranges(VirtAddr(start), VirtAddr(start + len));
            for (idx, (r_start, r_end)) in ranges.into_iter().enumerate() {
                trace!(
                    "[sys_mmap] pid={} overlap[{}]=[{:#x},{:#x})", pid, idx, r_start.0, r_end.0
                );
            }
        }
        return errno(ENOMEM);
    }

    // 在进程的内存空间里插入一个新的映射区域，起始地址为 start，长度为 len，权限为 map_perm。
    inner
        .memory_set
        .insert_framed_area(VirtAddr(start), VirtAddr(start + len), map_perm);
    drop(inner);

    // 文件映射填充部分，在“不是匿名映射、而且有有效 fd”的情况下，把文件内容读进映射的页里。
    // TODO: 懒分配/写时复制等优化
    if (flags & MAP_ANON) == 0 && fd != usize::MAX {
        // offset 参数必须是页大小的整数倍，否则返回 -EINVAL。
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
pub fn sys_munmap(start: usize, len: usize) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_munmap", pid);
    }
    if start % PAGE_SIZE != 0 || len == 0 {
        return errno(EINVAL);
    }
    let process = current_process();
    let mut inner = process.inner_exclusive_access();
    // if inner.name == "busybox" || inner.name == "ld-linux-riscv64-lp64d.so.1" {
    //     let before = inner
    //         .memory_set
    //         .overlap_count(VirtAddr(start), VirtAddr(start + len));
    //     trace!(
    //         "[sys_munmap] pid={} name={} start={:#x} len={:#x} overlap_before={}",
    //         pid,
    //         inner.name,
    //         start,
    //         len,
    //         before
    //     );
    //     if before > 0 {
    //         let ranges = inner
    //             .memory_set
    //             .overlap_ranges(VirtAddr(start), VirtAddr(start + len));
    //         for (idx, (r_start, r_end)) in ranges.into_iter().enumerate() {
    //             trace!(
    //                 "[sys_munmap] pid={} overlap[{}]=[{:#x},{:#x})",
    //                 pid,
    //                 idx,
    //                 r_start.0,
    //                 r_end.0
    //             );
    //         }
    //     }
    // }
    inner
        .memory_set
        .remove_area_with_start_vpn(VirtAddr(start).floor());
    // if inner.name == "busybox" || inner.name == "ld-linux-riscv64-lp64d.so.1" {
    //     let after = inner
    //         .memory_set
    //         .overlap_count(VirtAddr(start), VirtAddr(start + len));
    //     trace!(
    //         "[sys_munmap] pid={} name={} start={:#x} len={:#x} overlap_after={}",
    //         pid,
    //         inner.name,
    //         start,
    //         len,
    //         after
    //     );
    // }
    0
}

/// change data segment size
pub fn sys_sbrk(arg: isize) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_sbrk", pid);
    }
    let sepc = current_trap_cx().sepc;
    let name = current_process().inner_exclusive_access().name.clone();
    let process = current_process();
    let mut inner = process.inner_exclusive_access();
    let current_brk = inner.program_brk;
    let heap_bottom = inner.heap_bottom;
    if arg == 0 {
        // if name == "busybox" {
        //     trace!(
        //         "[sys_sbrk] pid={} name={} sepc={:#x} arg=0 cur={:#x} heap_bottom={:#x}",
        //         pid,
        //         name,
        //         sepc,
        //         current_brk,
        //         heap_bottom
        //     );
        // }
        return current_brk as isize;
    }
    let is_abs = (arg as usize) >= heap_bottom;
    let delta = if is_abs {
        arg - current_brk as isize
    } else {
        // sbrk(delta) is equivalent to brk(current_brk + delta)
        arg
    };
    let new_brk = (current_brk as isize + delta) as usize;
    if new_brk < heap_bottom {
        // new_brk is below the heap bottom, which is invalid
        error!(
            "[sys_sbrk] pid={} name={} sepc={:#x} arg={} cur={:#x} heap_bottom={:#x} new={:#x} -> ENOMEM",
            pid,
            name,
            sepc,
            arg,
            current_brk,
            heap_bottom,
            new_brk
        );
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
        trace!(
            "[sys_sbrk] pid={} name={} sepc={:#x} arg={} cur={:#x} heap_bottom={:#x} new={:#x} ok",
            pid,
            name,
            sepc,
            arg,
            current_brk,
            heap_bottom,
            new_brk
        );
        if is_abs {
            current_brk as isize + delta
        } else {
            current_brk as isize
        }
    } else {
        error!(
            "[sys_sbrk] pid={} name={} sepc={:#x} arg={} cur={:#x} heap_bottom={:#x} new={:#x} -> ENOMEM",
            pid,
            name,
            sepc,
            arg,
            current_brk,
            heap_bottom,
            new_brk
        );
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
        syscall!("kernel:pid[{}] sys_kill pid={} signum={}", pid_now, pid, signum);
    }
    if signum <= 0 || signum > MAX_SIG as i32 {
        return errno(EINVAL);
    }
    // bit (signum-1) 对应 signum：signal 1 → bit 0, signal 9 → bit 8
    let flag = match 1u64.checked_shl((signum - 1) as u32) {
        Some(bits) => SignalFlags::from_bits_truncate(bits),
        None => return errno(EINVAL),
    };
    if flag.is_empty() {
        return errno(EINVAL);
    }
    let process = match pid2process(pid) {
        Some(process) => process,
        None => return errno(ESRCH),
    };
    let mut inner = process.inner_exclusive_access();
    inner.signal_pending |= flag;
    // Wake blocked tasks so pending signals can be handled.
    // 需要设置 interrupted_by_signal 并从 futex 队列移除，
    // 否则 futex_wait 不会返回 EINTR。
    for task in inner.tasks.iter().filter_map(|t| t.as_ref()) {
        let mut task_inner = task.inner_exclusive_access();
        if task_inner.task_status == TaskStatus::Blocked {
            futex_remove_waiter_any(task);
            task_inner.interrupted_by_signal = true;
            task_inner.task_status = TaskStatus::Ready;
            drop(task_inner);
            add_task(task.clone());
        }
    }
    0
}

pub fn sys_tkill(tid: isize, signum: i32) -> isize {
    let pid_now = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid_now) {
        syscall!("kernel:pid[{}] sys_tkill tid={} signum={}", pid_now, tid, signum);
    }
    if tid <= 0 {
        return errno(EINVAL);
    }
    if signum <= 0 || signum > MAX_SIG as i32 {
        return errno(EINVAL);
    }
    // bit (signum-1) 对应 signum：signal 1 → bit 0, signal 33 → bit 32
    let flag = match 1u64.checked_shl((signum - 1) as u32) {
        Some(bits) => SignalFlags::from_bits_truncate(bits),
        None => return errno(EINVAL),
    };
    if flag.is_empty() {
        return errno(EINVAL);
    }
    let process = current_process();
    let inner = process.inner_exclusive_access();
    let tid = tid as usize;
    if tid >= inner.tasks.len() || inner.tasks[tid].is_none() {
        return errno(ESRCH);
    }
    if let Some(task) = inner.tasks[tid].as_ref() {
        let mut task_inner = task.inner_exclusive_access();
        if signum == 33 {
            let target_tid = task_inner.res.as_ref().map(|r| r.tid).unwrap_or(0);
            let tp = task_inner.get_trap_cx()[TrapFrameArgs::TLS];
            info!(
                "[tkill] pid={} target_tid={} tp={:#x} mask={:?} pending_before={:?}",
                pid_now,
                target_tid,
                tp,
                task_inner.signal_mask,
                task_inner.signal_pending
            );
        }
        task_inner.signal_pending |= flag;
        if signum == 33 {
            info!(
                "[tkill] pid={} target_tid={} pending_after={:?}",
                pid_now,
                task_inner.res.as_ref().map(|r| r.tid).unwrap_or(0),
                task_inner.signal_pending
            );
        }
        if task_inner.task_status == TaskStatus::Blocked {
            futex_remove_waiter_any(task);
            task_inner.interrupted_by_signal = true;
            task_inner.task_status = TaskStatus::Ready;
            drop(task_inner);
            add_task(task.clone());
        }
    }
    0
}

pub fn sys_sigaction(
    signum: i32,
    action: *const SignalAction,
    old_action: *mut SignalAction,
) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_sigaction signum={}", pid, signum);
    }
    if signum <= 0
        || signum > MAX_SIG as i32
        || signum == SIGKILL as i32
        || signum == SIGSTOP as i32
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
        if signum == 33 {
            info!(
                "[sigaction] pid={} signum=33 handler={:#x} flags={:#x} restorer={:#x} mask={:?}",
                pid,
                new_action.handler,
                new_action.flags,
                new_action.restorer,
                new_action.mask
            );
        }
        inner.signal_actions.table[idx] = new_action;
    }
    0
}

pub fn sys_sigprocmask(
    how: usize,
    set: *const usize,
    oldset: *mut usize,
    sigsetsize: usize,
) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!(
            "kernel:pid[{}] sys_sigprocmask how=0x{:x} set={:?} oldset={:?} size={}",
            pid,
            how,
            set,
            oldset,
            sigsetsize
        );
    }
    if sigsetsize != core::mem::size_of::<usize>() {
        return errno(EINVAL);
    }
    let token = current_user_token();
    let task = current_task().unwrap();
    let mut task_inner = task.inner_exclusive_access();

    if !oldset.is_null() {
        // 使用 flags_to_user_mask 简化转换
        let user_mask = flags_to_user_mask(task_inner.signal_mask) as usize;
        let bytes = unsafe {
            core::slice::from_raw_parts(
                (&user_mask as *const usize) as *const u8,
                core::mem::size_of::<usize>(),
            )
        };
        if let Err(err) = copy_to_user(token, oldset as *mut u8, bytes) {
            return err;
        }
    }

    if !set.is_null() {
        let user_mask = match read_from_user::<usize>(token, set) {
            Ok(v) => v,
            Err(err) => return err,
        };
        let mut new_flags = user_mask_to_flags(user_mask as u64);
        new_flags.remove(SignalFlags::SIGKILL | SignalFlags::SIGSTOP);
        match how {
            0 => task_inner.signal_mask |= new_flags,    // SIG_BLOCK
            1 => task_inner.signal_mask &= !new_flags,   // SIG_UNBLOCK
            2 => task_inner.signal_mask = new_flags,     // SIG_SETMASK
            _ => return errno(EINVAL),
        }
        task_inner
            .signal_mask
            .remove(SignalFlags::SIGKILL | SignalFlags::SIGSTOP);
    }
    0
}

pub fn sys_sigreturn() -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_sigreturn", pid);
    }
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();

    let saved = match inner.signal_trap_cx.take() {
        Some(cx) => cx,
        None => return errno(EINVAL),
    };

    // 检查栈底的 canary 值，防止栈溢出
    let current_sp = inner.get_trap_cx()[TrapFrameArgs::SP];
    let token = current_user_token();
    if let Ok(canary) = read_from_user::<usize>(token, current_sp as *const _) {
        if canary != 0x11451415 {
            error!(
                "[sigreturn] Stack canary corrupted! pid={} sp={:#x} canary={:#x} (expected 0x11451415)",
                pid, current_sp, canary
            );
            // 栈已被破坏，返回错误（后续会导致 SIGSEGV）
            return errno(EFAULT);
        }
    }
    let saved_a0 = saved[TrapFrameArgs::RET] as isize;
    let ucontext_ptr = inner.signal_ucontext_ptr;
    inner.signal_ucontext_ptr = 0;

    // 恢复 trap context 和 signal_mask
    let return_pc;
    let saved_pc = saved.sepc; // handler 修改前的原始 PC
    let ret_a0;
    if ucontext_ptr != 0 {
        let token = current_user_token();
        let ucontext = match read_from_user::<UserContext>(token, ucontext_ptr as *const _) {
            Ok(value) => value,
            Err(err) => return err,
        };
        info!(
            "[sigreturn] pid={} ucontext_ptr={:#x} saved_pc={:#x} ucontext_pc={:#x} sigmask={:#x}",
            pid,
            ucontext_ptr,
            saved.sepc,
            ucontext.uc_mcontext.gregs[0],
            ucontext.uc_sigmask[0]
        );
        let mut restored = saved;
        restored.restore_from_ucontext_gregs(&ucontext.uc_mcontext.gregs);
        *inner.get_trap_cx() = restored;
        // 从 ucontext 恢复信号掩码（per-thread）
        let mut new_mask = user_mask_to_flags(ucontext.uc_sigmask[0]);
        new_mask.remove(SignalFlags::SIGKILL | SignalFlags::SIGSTOP);
        inner.signal_mask = new_mask;
        return_pc = restored.sepc;
        ret_a0 = restored[TrapFrameArgs::RET] as isize;
    } else {
        *inner.get_trap_cx() = saved;
        // 从 backup 恢复信号掩码（per-thread）
        inner.signal_mask = inner.signal_mask_backup;
        return_pc = saved.sepc;
        ret_a0 = saved_a0;
    }

    // SA_RESETHAND 处理
    let current_sig = inner.handling_sig;
    inner.handling_sig = -1;

    if current_sig >= 0 && (current_sig as usize) <= crate::task::MAX_SIG {
        let process = current_process();
        let mut process_inner = process.inner_exclusive_access();
        let action = process_inner.signal_actions.table[current_sig as usize];
        if (action.flags & crate::task::SA_RESETHAND) != 0 {
            info!(
                "[sigreturn] SA_RESETHAND set for signal {}, resetting handler to SIG_DFL",
                current_sig
            );
            process_inner.signal_actions.table[current_sig as usize] = SignalAction::default();
        }
    }

    // SIGCANCEL (signal 33) 处理说明：
    // musl 的 cancel_handler 总是把 SIG33 加入 uc_sigmask 防止重复投递。
    // 取消通过两种机制工作：
    //   1. 异步取消(cancelasync=1)：handler 直接调用 pthread_exit，不会到达 sigreturn
    //   2. 延迟取消：handler 修改 ucontext PC 指向 __cancel，或者不做任何事
    //      后续 musl 的 __syscall_cp_asm 会在每个系统调用入口检查 cancel 标志
    //
    // 不要重新注入 SIG33！musl 的 cancel flag 机制会在下次系统调用时自动生效。
    // 如果 handler 修改了 PC（重定向到 __cancel），确保 SIG33 不阻塞后续行为。
    if current_sig == 33 && return_pc != saved_pc {
        // handler 成功修改了 PC，确保 SIG33 不被掩码
        inner.signal_mask.remove(SignalFlags::SIG33);
    }

    if current_sig == 33 {
        let tid = inner.res.as_ref().map(|r| r.tid).unwrap_or(0);
        let tp = inner.get_trap_cx()[TrapFrameArgs::TLS];
        info!(
            "[sigreturn] pid={} tid={} sig33 tp={:#x} saved_pc={:#x} return_pc={:#x} mask={:?}",
            pid,
            tid,
            tp,
            saved_pc,
            return_pc,
            inner.signal_mask
        );
    }

    ret_a0
}

/// Get user ID
/// rcore-lab is a single-user system, always returns 0
pub fn sys_getuid() -> isize {
    syscall!("kernel:pid[{}] sys_getuid", current_process().pid.0);
    0
}

/// Get effective user ID
/// rcore-lab is a single-user system, always returns 0
pub fn sys_geteuid() -> isize {
    syscall!("kernel:pid[{}] sys_geteuid", current_process().pid.0);
    0
}

/// Get group ID
/// rcore-lab is a single-user system, always returns 0
pub fn sys_getgid() -> isize {
    syscall!("kernel:pid[{}] sys_getgid", current_process().pid.0);
    0
}

/// Get effective group ID
/// rcore-lab is a single-user system, always returns 0
pub fn sys_getegid() -> isize {
    syscall!("kernel:pid[{}] sys_getegid", current_process().pid.0);
    0
}

pub fn sys_getrlimit(resource: usize, rlim: *mut RLimit) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_getrlimit resource={}", pid, resource);
    }
    if rlim.is_null() {
        return errno(EFAULT);
    }
    if resource >= RLIMIT_NLIMITS {
        return errno(EINVAL);
    }
    let process = current_process();
    let inner = process.inner_exclusive_access();
    let limit = inner.rlimits[resource];
    let bytes = unsafe {
        core::slice::from_raw_parts(
            (&limit as *const RLimit) as *const u8,
            core::mem::size_of::<RLimit>(),
        )
    };
    let token = current_user_token();
    match copy_to_user(token, rlim as *mut u8, bytes) {
        Ok(_) => 0,
        Err(err) => err,
    }
}

pub fn sys_prlimit64(
    pid: usize,
    resource: usize,
    new_limit: *const RLimit,
    old_limit: *mut RLimit,
) -> isize {
    let current_pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(current_pid) {
        trace!(
            "kernel:pid[{}] sys_prlimit64 pid={} resource={}",
            current_pid,
            pid,
            resource
        );
    }
    if resource >= RLIMIT_NLIMITS {
        return errno(EINVAL);
    }
    if pid != 0 && pid != current_pid {
        return errno(ESRCH);
    }

    let token = current_user_token();
    let process = current_process();
    let mut inner = process.inner_exclusive_access();
    let old = inner.rlimits[resource];
    if !old_limit.is_null() {
        let bytes = unsafe {
            core::slice::from_raw_parts(
                (&old as *const RLimit) as *const u8,
                core::mem::size_of::<RLimit>(),
            )
        };
        if let Err(err) = copy_to_user(token, old_limit as *mut u8, bytes) {
            return err;
        }
    }
    if !new_limit.is_null() {
        let new_val = read_from_user::<RLimit>(token, new_limit).unwrap();
        if new_val.rlim_cur > new_val.rlim_max {
            return errno(EINVAL);
        }
        inner.rlimits[resource] = new_val;
    }
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
    let pid = current_process().pid.0;
    let name = current_process().inner_exclusive_access().name.clone();
    if pid == 4 || name == "sh" {
        trace!(
            "[sys_exit_group] pid={} name={} code={} sepc={:#x}",
            pid,
            name,
            exit_code,
            current_trap_cx().sepc
        );
    }
    sys_exit(exit_code)
}

pub fn sys_shutdown() -> ! {
    trace!(
        "kernel:pid[{}] sys_shutdown",
        current_process().pid.0);
    arch::shutdown();
}

/// mprotect - change memory region protection
///
/// # Arguments
/// * `addr` - starting address of memory region (must be page-aligned)
/// * `len` - length of memory region
/// * `prot` - new protection flags (PROT_READ | PROT_WRITE | PROT_EXEC)
///
/// # Returns
/// * On success: 0
/// * On error: -errno
pub fn sys_mprotect(addr: usize, len: usize, prot: usize) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_mprotect addr=0x{:x} len=0x{:x} prot=0x{:x}",
               pid, addr, len, prot);
    }

    const PROT_READ: usize = 0x1;
    const PROT_WRITE: usize = 0x2;
    const PROT_EXEC: usize = 0x4;

    use crate::config::PAGE_SIZE;

    // Check alignment
    if addr % PAGE_SIZE != 0 {
        return errno(EINVAL);
    }

    if len == 0 {
        return 0;
    }

    // Convert prot flags to MapPermission
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

    let process = current_process();
    let mut inner = process.inner_exclusive_access();

    // Round up length to page boundary
    let page_count = (len + PAGE_SIZE - 1) / PAGE_SIZE;
    let end_addr = addr + page_count * PAGE_SIZE;

    // Change protection for the memory region
    let result = inner.memory_set.change_protection(
        VirtAddr(addr),
        VirtAddr(end_addr),
        map_perm,
    );

    if result {
        0
    } else {
        errno(EINVAL)
    }
}

/// rt_sigtimedwait - Wait for signal
///
/// # Arguments
/// - set: pointer to signal set to wait for
/// - info: pointer to siginfo_t structure (output)
/// - timeout: pointer to timespec structure (timeout)
/// - sigsetsize: size of signal set
///
/// # Returns
/// - Success: signal number
/// - Failure: -errno
pub fn sys_rt_sigtimedwait(
    set: *const usize,
    info: *mut usize,
    timeout: *const TimeSpec,
    _sigsetsize: usize,
) -> isize {
    // Validate pointers
    if set.is_null() {
        return errno(EFAULT);
    }

    // Read the signal set from user space
    let token = current_user_token();
    let sigset = *translated_ref(token, set);
    if sigset == 0 {
        return errno(EINVAL);
    }

    // Read timeout if provided
    let timeout_us = if !timeout.is_null() {
        let ts = read_from_user::<TimeSpec>(token, timeout).unwrap();
        if ts.tv_nsec >= 1_000_000_000 {
            return errno(EINVAL);
        }
        Some(
            ts.tv_sec
                .saturating_mul(1_000_000)
                .saturating_add(ts.tv_nsec / 1_000),
        )
    } else {
        None
    };

    let start = get_time_us();
    let deadline = timeout_us.map(|delta| start.saturating_add(delta));

    loop {
        let mut found = None;
        {
            let process = current_process();
            let mut inner = process.inner_exclusive_access();
            for signum in 1..=MAX_SIG {
                // SIGKILL 和 SIGSTOP 不能被等待
                if signum == SIGKILL || signum == SIGSTOP {
                    continue;
                }
                if (sigset & (1usize << (signum - 1))) == 0 {
                    continue;
                }
                // bit (signum-1) 对应 signum
                let flag = SignalFlags::from_bits_truncate(1u64 << (signum - 1));
                if inner.signal_pending.contains(flag) {
                    inner.signal_pending.remove(flag);
                    found = Some(signum as isize);
                    break;
                }
            }
        }

        if let Some(signum) = found {
            if !info.is_null() {
                let _info_ref = translated_refmut(token, info);
            }
            return signum;
        }

        if let Some(dl) = deadline {
            if get_time_us() >= dl {

                return errno(EAGAIN);
            }
        }
        suspend_current_and_run_next();
    }
}

