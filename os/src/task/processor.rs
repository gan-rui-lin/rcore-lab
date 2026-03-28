#![allow(missing_docs)]

use super::{ProcessControlBlock, TaskContext, TaskControlBlock, TaskStatus, fetch_task, ready_queue_snapshot};
use crate::sync::UPIntrFreeCell;
use arch::{TrapContext, TrapFrameArgs};
use alloc::sync::Arc;
use lazy_static::*;
use core::sync::atomic::{AtomicU64, Ordering};

pub struct Processor {
    current: Option<Arc<TaskControlBlock>>,
    idle_task_cx: TaskContext,
}

impl Processor {
    pub fn new() -> Self {
        Self {
            current: None,
            idle_task_cx: TaskContext::zero_init(),
        }
    }
    fn get_idle_task_cx_ptr(&mut self) -> *mut TaskContext {
        &mut self.idle_task_cx as *mut _
    }
    pub fn take_current(&mut self) -> Option<Arc<TaskControlBlock>> {
        self.current.take()
    }
    pub fn current(&self) -> Option<Arc<TaskControlBlock>> {
        self.current.as_ref().map(Arc::clone)
    }
}

lazy_static! {
    pub static ref PROCESSOR: UPIntrFreeCell<Processor> = unsafe { UPIntrFreeCell::new(Processor::new()) };
}

const RUN_TASKS_EMPTY_INTERVAL: u64 = 2000;
static RUN_TASKS_EMPTY_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn run_tasks() {
    loop {
        // Enable interrupts briefly so pending timer interrupts can fire.
        // This is critical: when all tasks are blocked in kernel-mode
        // syscalls (e.g., accept, recv, waitpid), sstatus.SIE remains 0
        // and timer interrupts are never taken. Without this, SIGALRM
        // from setitimer can never be delivered.
        arch::enable_interrupts();
        arch::disable_interrupts();

        let mut processor = PROCESSOR.exclusive_access();
        if let Some(task) = fetch_task() {
            task.kstack.check_guard();
            let idle_task_cx_ptr = processor.get_idle_task_cx_ptr();
            let mut task_inner = task.inner_exclusive_access();
            let next_task_cx_ptr = &task_inner.task_cx as *const TaskContext;
            task_inner.task_status = TaskStatus::Running;
            drop(task_inner);
            let pt_token = task.get_user_token();
            processor.current = Some(task);
            drop(processor);
            unsafe {
                arch::switch_to_task(idle_task_cx_ptr, next_task_cx_ptr, pt_token);
            }
        } else {
            let count = RUN_TASKS_EMPTY_COUNTER.fetch_add(1, Ordering::Relaxed);
            if count % RUN_TASKS_EMPTY_INTERVAL == 0 {
                let ready_len = ready_queue_snapshot().len();
                warn!("no tasks available in run_tasks (ready_len={})", ready_len);
            }
        }
    }
}

pub fn take_current_task() -> Option<Arc<TaskControlBlock>> {
    PROCESSOR.exclusive_access().take_current()
}

pub fn current_task() -> Option<Arc<TaskControlBlock>> {
    PROCESSOR.exclusive_access().current()
}

pub fn current_process() -> Arc<ProcessControlBlock> {
    current_task().unwrap().process.upgrade().unwrap()
}

pub fn current_user_token() -> usize {
    let task = current_task().unwrap();
    task.get_user_token()
}

pub fn current_trap_cx() -> &'static mut TrapContext {
    current_task()
        .unwrap()
        .inner_exclusive_access()
        .get_trap_cx()
}

pub fn current_trap_cx_user_va() -> usize {
    current_task()
        .unwrap()
        .inner_exclusive_access()
        .res
        .as_ref()
        .unwrap()
        .trap_cx_user_va()
}

pub fn current_kstack_top() -> usize {
    current_task().unwrap().kstack.get_top()
}

/// Return lightweight current-task context for allocator tracing.
/// (pid, tid, last_syscall, sampled)
pub fn current_task_context_for_alloc_trace() -> (usize, usize, usize, bool) {
    let Some(processor) = PROCESSOR.try_exclusive_access() else {
        return (0, 0, 0, false);
    };
    let Some(task) = processor.current() else {
        return (0, 0, 0, false);
    };
    drop(processor);

    let pid = task.process.upgrade().map(|p| p.getpid()).unwrap_or(0);
    let result = if let Some(task_inner) = task.try_inner_exclusive_access() {
        let tid = task_inner.res.as_ref().map(|r| r.tid).unwrap_or(0);
        (pid, tid, task_inner.last_syscall, true)
    } else {
        (pid, 0, 0, false)
    };
    result
}

/// Print current task/process context for alloc_error diagnostics.
pub fn print_current_task_brief_for_alloc_error() {
    let task = PROCESSOR.exclusive_access().current();
    let Some(task) = task else {
        println!("[kernel] alloc_error current: <no-current-task>");
        return;
    };
    let Some(process) = task.process.upgrade() else {
        println!("[kernel] alloc_error current: <task-without-process>");
        return;
    };
    let pid = process.getpid();
    if let Some(task_inner) = task.try_inner_exclusive_access() {
        let tid = task_inner.res.as_ref().map(|r| r.tid).unwrap_or(0);
        let trap_cx = task_inner.get_trap_cx();
        if let Some(proc_inner) = process.try_inner_exclusive_access() {
            println!(
                "[kernel] alloc_error current: pid={} tid={} name={} zombie={} status={:?} last_syscall={} sepc={:#x} sp={:#x} ra={:#x} sampled=true",
                pid,
                tid,
                proc_inner.name.as_str(),
                proc_inner.is_zombie,
                task_inner.task_status,
                task_inner.last_syscall,
                trap_cx.sepc,
                trap_cx[TrapFrameArgs::SP],
                trap_cx[TrapFrameArgs::RA]
            );
        } else {
            println!(
                "[kernel] alloc_error current: pid={} tid={} name=<proc_busy> status={:?} last_syscall={} sepc={:#x} sp={:#x} ra={:#x} sampled=false",
                pid,
                tid,
                task_inner.task_status,
                task_inner.last_syscall,
                trap_cx.sepc,
                trap_cx[TrapFrameArgs::SP],
                trap_cx[TrapFrameArgs::RA]
            );
        }
    } else if let Some(proc_inner) = process.try_inner_exclusive_access() {
        println!(
            "[kernel] alloc_error current: pid={} name={} zombie={} <task_busy> sampled=true",
            pid,
            proc_inner.name.as_str(),
            proc_inner.is_zombie
        );
    } else {
        println!(
            "[kernel] alloc_error current: pid={} name=<proc_busy> <task_busy> sampled=false",
            pid
        );
    };
}

/// Check if the current task has pending unmasked signals.
///
/// When `ignore_sigchld` is true, a pending SIGCHLD alone will not count as
/// "having a pending signal". This prevents blocking I/O (pipe, socket) from
/// being spuriously interrupted when a child exits, which would break wait()
/// scenarios.
pub fn has_pending_unmasked_signal(ignore_sigchld: bool) -> bool {
    let Some(task) = current_task() else {
        return false;
    };
    let process = match task.process.upgrade() {
        Some(p) => p,
        None => return false,
    };
    let process_inner = process.inner_exclusive_access();
    let task_inner = task.inner_exclusive_access();
    let mut pending =
        (process_inner.signal_pending | task_inner.signal_pending) & !task_inner.signal_mask;
    if ignore_sigchld {
        pending &= !super::SignalFlags::SIGCHLD;
    }
    !pending.is_empty()
}

pub fn schedule(switched_task_cx_ptr: *mut TaskContext) {
    let mut processor = PROCESSOR.exclusive_access();
    let idle_task_cx_ptr = processor.get_idle_task_cx_ptr();
    drop(processor);
    unsafe {
        arch::switch_to_idle(switched_task_cx_ptr, idle_task_cx_ptr);
    }
}
