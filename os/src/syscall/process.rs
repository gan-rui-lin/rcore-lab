//! Process management syscalls
//!
use alloc::sync::Arc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::{
    fs::{open_file, OpenFlags},
    mm::{translated_ref, translated_refmut, translated_str},
    task::{
        add_task, current_task, current_user_token, exit_current_and_run_next,
        suspend_current_and_run_next,
    },
};

use super::errno::*;

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

pub fn sys_exit(exit_code: i32) -> ! {
    let pid = current_task().unwrap().pid.0;
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
    trace!("kernel: sys_getpid pid:{}", current_task().unwrap().pid.0);
    current_task().unwrap().pid.0 as isize
}

pub fn sys_fork() -> isize {
    let pid = current_task().unwrap().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!("kernel:pid[{}] sys_fork", pid);
    }
    let current_task = current_task().unwrap();
    let new_task = current_task.fork();
    let new_pid = new_task.pid.0;
    // modify trap context of new_task, because it returns immediately after switching
    let trap_cx = new_task.inner_exclusive_access().get_trap_cx();
    // we do not have to move to next instruction since we have done it before
    // for child process, fork returns 0
    trap_cx.x[10] = 0;
    // add new task to scheduler
    add_task(new_task);
    new_pid as isize
}

pub fn sys_exec(path: *const u8, mut argv: *const usize, mut envp: *const usize) -> isize {
    let pid = current_task().unwrap().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!("kernel:pid[{}] sys_exec", pid);
    }
    if path.is_null() {
        return errno(EFAULT);
    }
    let token = current_user_token();
    let path = translated_str(token, path);
    if crate::syscall::should_trace_syscall(pid) {
        trace!("kernel:pid[{}] sys_exec path={}", pid, path);
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
        args.push(path.clone());
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
    if let Some(app_inode) = open_file(path.as_str(), OpenFlags::RDONLY) {
        let all_data = app_inode.read_all();
        let task = current_task().unwrap();
        task.set_name_from_path(path.as_str());
        task.exec(all_data.as_slice(), args, envs);
        0
    } else {
        errno(ENOENT)
    }
}

/// If there is not a child process whose pid is same as given, return -ECHILD.
/// Else if there is a child process but it is still running, return -EAGAIN.
pub fn sys_waitpid(pid: isize, exit_code_ptr: *mut i32) -> isize {
    //trace!("kernel: sys_waitpid");
    let task = current_task().unwrap();
    // find a child process

    // ---- access current PCB exclusively
    let mut inner = task.inner_exclusive_access();
    if !inner
        .children
        .iter()
        .any(|p| pid == -1 || pid as usize == p.getpid())
    {
        return errno(ECHILD);
        // ---- release current PCB
    }
    let pair = inner.children.iter().enumerate().find(|(_, p)| {
        // ++++ temporarily access child PCB exclusively
        p.inner_exclusive_access().is_zombie() && (pid == -1 || pid as usize == p.getpid())
        // ++++ release child PCB
    });
    if let Some((idx, _)) = pair {
        let child = inner.children.remove(idx);
        // Child may still be referenced by other kernel structures; don't panic here.
        if Arc::strong_count(&child) > 1 {
            trace!(
                "kernel:pid[{}] waitpid: child pid {} has {} refs",
                task.getpid(),
                child.getpid(),
                Arc::strong_count(&child)
            );
        }
        let found_pid = child.getpid();
        // ++++ temporarily access child PCB exclusively
        let exit_code = child.inner_exclusive_access().exit_code;
        // ++++ release child PCB
        *translated_refmut(inner.memory_set.token(), exit_code_ptr) = exit_code;
        found_pid as isize
    } else {
        errno(EAGAIN)
    }
    // ---- release current PCB automatically
}

/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
pub fn sys_get_time(_ts: *mut TimeVal, _tz: usize) -> isize {
    trace!(
        "kernel:pid[{}] sys_get_time NOT IMPLEMENTED",
        current_task().unwrap().pid.0
    );
    errno(ENOSYS)
}

/// YOUR JOB: Implement mmap.
pub fn sys_mmap(_start: usize, _len: usize, _port: usize) -> isize {
    trace!(
        "kernel:pid[{}] sys_mmap NOT IMPLEMENTED",
        current_task().unwrap().pid.0
    );
    errno(ENOSYS)
}

/// YOUR JOB: Implement munmap.
pub fn sys_munmap(_start: usize, _len: usize) -> isize {
    trace!(
        "kernel:pid[{}] sys_munmap NOT IMPLEMENTED",
        current_task().unwrap().pid.0
    );
    errno(ENOSYS)
}

/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    let pid = current_task().unwrap().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!("kernel:pid[{}] sys_sbrk", pid);
    }
    if let Some(old_brk) = current_task().unwrap().change_program_brk(size) {
        old_brk as isize
    } else {
        errno(ENOMEM)
    }
}

/// YOUR JOB: Implement spawn.
/// HINT: fork + exec =/= spawn
pub fn sys_spawn(_path: *const u8) -> isize {
    trace!(
        "kernel:pid[{}] sys_spawn NOT IMPLEMENTED",
        current_task().unwrap().pid.0
    );
    errno(ENOSYS)
}

// YOUR JOB: Set task priority.
pub fn sys_set_priority(_prio: isize) -> isize {
    trace!(
        "kernel:pid[{}] sys_set_priority NOT IMPLEMENTED",
        current_task().unwrap().pid.0
    );
    errno(ENOSYS)
}
