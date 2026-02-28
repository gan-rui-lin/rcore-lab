//! Process management syscalls
//!
use alloc::format;
use alloc::sync::Arc;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::{
    arch::shutdown,
    fs::{open_file, File, OpenFlags},
    mm::{translated_byte_buffer, translated_ref, translated_refmut, translated_str, MapPermission, VirtAddr},
    task::{
        current_process, current_task, current_trap_cx, current_user_token, exit_current_and_run_next,
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
    let name = current_process().inner_exclusive_access().name.clone();
    if pid == 4 || name == "sh" {
        #[cfg(target_arch = "riscv64")]
        let sepc = current_trap_cx().sepc;
        #[cfg(target_arch = "loongarch64")]
        let sepc = current_trap_cx().era;
        trace!(
            "[sys_exit] pid={} name={} code={} sepc={:#x}",
            pid,
            name,
            exit_code,
            sepc
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
        trace!("kernel:pid[{}] sys_fork", pid);
    }
    let current_process = current_process();
    let new_process = current_process.fork();
    let new_pid = new_process.pid.0;
    let new_task = new_process.inner_exclusive_access().get_task(0);
    let parent_cx = *current_trap_cx();
    let clone_stack = parent_cx.x[11];
    let new_task_inner = new_task.inner_exclusive_access();
    let trap_cx = new_task_inner.get_trap_cx();
    *trap_cx = parent_cx;
    trap_cx.x[10] = 0;
    if clone_stack != 0 {
        trap_cx.x[2] = clone_stack;
    }
    trap_cx.kernel_sp = new_task.kstack.get_top();
    new_pid as isize
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
        // 检查文件内容，如果是脚本则解析 shebang 进行递归 exec
        // 如果是 ELF 则正常执行，注意动态链接的 ELF 需要先加载解释器再由解释器加载主程序
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
        if all_data.len() < 4 || &all_data[..4] != b"\x7fELF" {
            return errno(ENOEXEC);
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
                    // ! 实际已经先 ensure_flink 了
                    if interp_path == "/lib/ld-linux-riscv64-lp64d.so.1" {
                        let musl_loader = "/musl/lib/libc.so";
                        let glibc_loader = "/glibc/lib/ld-linux-riscv64-lp64d.so.1";
                        if open_file(musl_loader, OpenFlags::empty()).is_some() {
                            interp_path = String::from(musl_loader);
                            info!("[sys_exec] prefer musl loader: {}", interp_path);
                        } else if open_file(glibc_loader, OpenFlags::empty()).is_some() {
                            interp_path = String::from(glibc_loader);
                            info!("[sys_exec] fallback glibc loader: {}", interp_path);
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
    const MAP_PRIVATE: usize = 0x02;
    const MAP_FIXED: usize = 0x10;
    const MAP_ANON: usize = 0x20;

    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!("kernel:pid[{}] sys_mmap", pid);
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
        // let ra = cx.x[1];
        // let gp = cx.x[3];
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
        trace!("kernel:pid[{}] sys_munmap", pid);
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
        trace!("kernel:pid[{}] sys_sbrk", pid);
    }
    #[cfg(target_arch = "riscv64")]
    let sepc = current_trap_cx().sepc;
    #[cfg(target_arch = "loongarch64")]
    let sepc = current_trap_cx().era;
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
    let pid = current_process().pid.0;
    let name = current_process().inner_exclusive_access().name.clone();
    if pid == 4 || name == "sh" {
        #[cfg(target_arch = "riscv64")]
        let sepc = current_trap_cx().sepc;
        #[cfg(target_arch = "loongarch64")]
        let sepc = current_trap_cx().era;
        trace!(
            "[sys_exit_group] pid={} name={} code={} sepc={:#x}",
            pid,
            name,
            exit_code,
            sepc
        );
    }
    sys_exit(exit_code)
}

pub fn sys_shutdown() -> ! {
    trace!(
        "kernel:pid[{}] sys_shutdown",
        current_process().pid.0);
    shutdown();
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
        trace!("kernel:pid[{}] sys_mprotect addr=0x{:x} len=0x{:x} prot=0x{:x}",
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
///
/// Note: This is a simplified implementation that doesn't actually block.
/// A full implementation would need to:
/// 1. Block the current task until a signal arrives
/// 2. Handle timeout properly
/// 3. Fill in the siginfo structure with signal details
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

    // For now, return EAGAIN to indicate no signal is pending
    // A full implementation would:
    // 1. Check if any signals in the set are pending
    // 2. If yes, dequeue the signal and return its number
    // 3. If no, block until a signal arrives or timeout occurs

    // Read the signal set from user space (for validation)
    let token = current_user_token();
    let _sigset = translated_ref(token, set);

    // Read timeout if provided
    if !timeout.is_null() {
        let _ts = translated_ref(token, timeout);
        // Would use timeout to set up a timer
    }

    // Check if info pointer is provided
    if !info.is_null() {
        // Would fill in siginfo structure here
        let _info_ref = translated_refmut(token, info);
    }

    // For simplicity, return EAGAIN (no signal available)
    errno(EAGAIN)
}
