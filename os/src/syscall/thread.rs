use crate::{
    syscall::errno::{errno, EAGAIN, ECHILD},
    task::{add_task, current_process, current_task, TaskControlBlock},
};
use alloc::sync::Arc;
use arch::{TrapContext, TrapFrameArgs};

pub fn sys_thread_create(entry: usize, arg: usize) -> isize {
    let task = current_task().unwrap();
    let process = task.process.upgrade().unwrap();
    // create a new thread
    let new_task = Arc::new(TaskControlBlock::new(
        Arc::clone(&process),
        task.ustack_base(),
        true,
    ));
    let new_task_res = new_task.user_res_snapshot().unwrap();
    let new_task_tid = new_task_res.tid;
    // add new thread to current process
    process.insert_task(new_task_tid, Arc::clone(&new_task));
    new_task.with_trap_cx_mut(|new_task_trap_cx| {
        *new_task_trap_cx = TrapContext::app_init_context(entry, new_task_res.ustack_top);
        new_task_trap_cx[TrapFrameArgs::ARG0] = arg;
    });
    // Queue the thread after its initial trap context is ready.
    add_task(Arc::clone(&new_task));
    new_task_tid as isize
}

/// Convert (process_pid, internal_tid) to the Linux-visible TID.
///   - Main thread (internal_tid == 0): TID = process_pid
///   - Non-main thread: TID = process_pid + internal_tid
/// Guarantees: TID > 0, unique per thread, main thread TID == TGID.
pub fn to_user_tid(process_pid: usize, internal_tid: usize) -> usize {
    if internal_tid == 0 {
        process_pid
    } else {
        process_pid + internal_tid
    }
}

/// Check if `target_tid` (Linux-visible) matches (process_pid, internal_tid).
pub fn match_user_tid(process_pid: usize, internal_tid: usize, target_tid: usize) -> bool {
    to_user_tid(process_pid, internal_tid) == target_tid
}

pub fn sys_gettid() -> isize {
    let task = current_task().unwrap();
    let process = current_process();
    let tid = task.tid();
    to_user_tid(process.pid.0, tid) as isize
}

pub fn sys_set_tid_address(tidptr: *mut i32) -> isize {
    let task = current_task().unwrap();
    let process = current_process();
    let internal_tid = task.tid();
    let tid = to_user_tid(process.pid.0, internal_tid);
    task.set_clear_child_tid(tidptr as usize);
    info!(
        "[sys_set_tid_address] pid={} tid={} tidptr={:#x}",
        process.pid.0, tid, tidptr as usize
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

    let calling_tid = task.tid();

    // Log to verify if pthread_join uses sys_waittid
    if pid == 34 || pid == 36 {
        info!(
            "[sys_waittid] pid={} caller_tid={} waiting_for_tid={}",
            pid, calling_tid, tid
        );
    }

    // a thread cannot wait for itself
    if calling_tid == tid {
        return errno(ECHILD);
    }
    let mut exit_code: Option<i32> = None;
    let waited_task = process.with_threads(|threads| {
        threads
            .tasks
            .get(tid)
            .and_then(|slot| slot.as_ref().map(Arc::clone))
    });
    if let Some(waited_task) = waited_task {
        if let Some(waited_exit_code) = waited_task.exit_code() {
            exit_code = Some(waited_exit_code);
        }
    } else {
        // waited thread does not exist
        return errno(ECHILD);
    }
    if let Some(exit_code) = exit_code {
        // dealloc the exited thread
        process.remove_task(tid);
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
