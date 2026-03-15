use arch::{TrapContext, TrapFrameArgs};
use crate::{
    mm::kernel_token,
    syscall::errno::{errno, EAGAIN, ECHILD},
    task::{TaskControlBlock, add_task, current_process, current_task},
};
use crate::trap::user_trap_entry;
use alloc::sync::Arc;

pub fn sys_thread_create(entry: usize, arg: usize) -> isize {
    let task = current_task().unwrap();
    let process = task.process.upgrade().unwrap();
    // create a new thread
    let new_task = Arc::new(TaskControlBlock::new(
        Arc::clone(&process),
        task.inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .ustack_base,
        true,
    ));
    let new_task_inner = new_task.inner_exclusive_access();
    let new_task_res = new_task_inner.res.as_ref().unwrap();
    let new_task_tid = new_task_res.tid;
    let mut process_inner = process.inner_exclusive_access();
    // add new thread to current process
    let tasks = &mut process_inner.tasks;
    while tasks.len() < new_task_tid + 1 {
        tasks.push(None);
    }
    tasks[new_task_tid] = Some(Arc::clone(&new_task));
    let new_task_trap_cx = new_task_inner.get_trap_cx();
    *new_task_trap_cx = TrapContext::app_init_context(
        entry,
        new_task_res.ustack_top(),
        kernel_token(),
        new_task.kstack.get_top(),
        user_trap_entry as usize,
    );
    new_task_trap_cx[TrapFrameArgs::ARG0] = arg;
    // Queue the thread after its initial trap context is ready.
    add_task(Arc::clone(&new_task));
    new_task_tid as isize
}

pub fn sys_gettid() -> isize {
    current_task()
        .unwrap()
        .inner_exclusive_access()
        .res
        .as_ref()
        .unwrap()
        .tid as isize
}

pub fn sys_set_tid_address(tidptr: *mut i32) -> isize {
    let task = current_task().unwrap();
    let mut task_inner = task.inner_exclusive_access();
    let tid = task_inner.res.as_ref().unwrap().tid as i32;
    task_inner.clear_child_tid = tidptr as usize;
    drop(task_inner);
    // Linux set_tid_address: only saves the pointer for clear_child_tid on exit,
    // and returns the caller's TID. It does NOT write to *tidptr.
    // Writing TID to a user pointer is CLONE_CHILD_SETTID's job in clone().
    info!(
        "[sys_set_tid_address] pid={} tid={} tidptr={:#x}",
        current_process().pid.0, tid, tidptr as usize
    );
    tid as isize
}

/// thread does not exist, return -ECHILD
/// thread has not exited yet, return -EAGAIN
/// otherwise, return thread's exit code
pub fn sys_waittid(tid: usize) -> isize {
    let task = current_task().unwrap();
    let process = task.process.upgrade().unwrap();
    let pid = process.pid.0;

    let task_inner = task.inner_exclusive_access();
    let calling_tid = task_inner.res.as_ref().map(|r| r.tid).unwrap_or(0);

    // Log to verify if pthread_join uses sys_waittid
    if pid == 34 || pid == 36 {
        info!(
            "[sys_waittid] pid={} caller_tid={} waiting_for_tid={}",
            pid, calling_tid, tid
        );
    }

    let mut process_inner = process.inner_exclusive_access();
    // a thread cannot wait for itself
    if task_inner.res.as_ref().unwrap().tid == tid {
        return errno(ECHILD);
    }
    let mut exit_code: Option<i32> = None;
    if tid >= process_inner.tasks.len() {
        return errno(ECHILD);
    }
    let waited_task = process_inner.tasks[tid].as_ref();
    if let Some(waited_task) = waited_task {
        if let Some(waited_exit_code) = waited_task.inner_exclusive_access().exit_code {
            exit_code = Some(waited_exit_code);
        }
    } else {
        // waited thread does not exist
        return errno(ECHILD);
    }
    if let Some(exit_code) = exit_code {
        // dealloc the exited thread
        process_inner.tasks[tid] = None;
        if pid == 34 || pid == 36 {
            info!(
                "[sys_waittid] pid={} tid={} -> exit_code={}",
                pid, calling_tid, exit_code
            );
        }
        exit_code as isize
    } else {
        // waited thread has not exited
        errno(EAGAIN)
    }
}
