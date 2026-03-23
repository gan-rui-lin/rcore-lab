#![allow(missing_docs)]

use super::{ProcessControlBlock, TaskContext, TaskControlBlock, TaskStatus, fetch_task, ready_queue_snapshot};
use crate::sync::UPIntrFreeCell;
use arch::TrapContext;
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

pub fn schedule(switched_task_cx_ptr: *mut TaskContext) {
    let mut processor = PROCESSOR.exclusive_access();
    let idle_task_cx_ptr = processor.get_idle_task_cx_ptr();
    drop(processor);
    unsafe {
        arch::switch_to_idle(switched_task_cx_ptr, idle_task_cx_ptr);
    }
}
