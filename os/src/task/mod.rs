//! Task management implementation
//!
//! Everything about task management, like starting and switching tasks is
//! implemented here.
//!
//! A single global instance of [`TaskManager`] called `TASK_MANAGER` controls
//! all the tasks in the whole operating system.
//!
//! A single global instance of [`Processor`] called `PROCESSOR` monitors running
//! task(s) for each core.
//!
//! A single global instance of `PID_ALLOCATOR` allocates pid for user apps.
//!
//! Be careful when you see `__switch` ASM function in `switch.S`. Control flow around this function
//! might not be what you expect.
mod context;
mod id;
mod manager;
mod processor;
mod switch;
mod action;
mod signal;
#[allow(clippy::module_inception)]
#[allow(rustdoc::private_intra_doc_links)]
mod task;

use crate::fs::{open_file, OpenFlags};
use alloc::sync::Arc;
pub use context::TaskContext;
use lazy_static::*;
pub use manager::{fetch_task, TaskManager};
use switch::__switch;
pub use action::{SignalAction, SignalActions};
pub use signal::{SignalFlags, MAX_SIG, SigNumber};
pub use task::{TaskControlBlock, TaskStatus};

pub use id::{kstack_alloc, pid_alloc, KernelStack, PidHandle};
pub use manager::add_task;
pub use processor::{
    current_task, current_trap_cx, current_user_token, run_tasks, schedule, take_current_task,
    Processor,
};

/// Find a task by pid, searching current task first, then the ready queue.
pub fn find_task_by_pid(pid: usize) -> Option<Arc<TaskControlBlock>> {
    if let Some(task) = current_task() {
        if task.getpid() == pid {
            return Some(task);
        }
    }
    manager::TASK_MANAGER
        .exclusive_access()
        .find_by_pid(pid)
}
/// Suspend the current 'Running' task and run the next task in task list.
pub fn suspend_current_and_run_next() {
    // There must be an application running.
    let task = take_current_task().unwrap();

    // ---- access current TCB exclusively
    let mut task_inner = task.inner_exclusive_access();
    let task_cx_ptr = &mut task_inner.task_cx as *mut TaskContext;
    // Change status to Ready
    if task_inner.is_stopped() {
        task_inner.task_status = TaskStatus::Stopped;
    } else {
        task_inner.task_status = TaskStatus::Ready;
    }
    drop(task_inner);
    // ---- release current PCB

    // push back to ready queue.
    add_task(task);
    // jump to scheduling cycle
    schedule(task_cx_ptr);
}

/// pid of usertests app in make run TEST=1
pub const IDLE_PID: usize = 0;

/// Exit the current 'Running' task and run the next task in task list.
pub fn exit_current_and_run_next(exit_code: i32) {
    // take from Processor
    let task = take_current_task().unwrap();

    let pid = task.getpid();
    if pid == IDLE_PID {
        println!(
            "[kernel] Idle process exit with exit_code {} ...",
            exit_code
        );
        panic!("All applications completed!");
    }

    // **** access current TCB exclusively
    let mut inner = task.inner_exclusive_access();
    // Change status to Zombie
    inner.task_status = TaskStatus::Zombie;
    // Record exit code
    inner.exit_code = exit_code;
    // do not move to its parent but under initproc

    // ++++++ access initproc TCB exclusively
    {
        let mut initproc_inner = INITPROC.inner_exclusive_access();
        for child in inner.children.iter() {
            child.inner_exclusive_access().parent = Some(Arc::downgrade(&INITPROC));
            initproc_inner.children.push(child.clone());
        }
    }
    // ++++++ release parent PCB

    inner.children.clear();
    // deallocate user space
    inner.memory_set.recycle_data_pages();
    // drop file descriptors
    inner.fd_table.clear();
    drop(inner);
    // **** release current PCB
    // drop task manually to maintain rc correctly
    drop(task);
    // we do not have to save task context
    let mut _unused = TaskContext::zero_init();
    schedule(&mut _unused as *mut _);
}

/// Check pending signals for current task and deliver one if needed.
pub fn handle_signals() {
    let task = match current_task() {
        Some(task) => task,
        None => return,
    };
    let mut inner = task.inner_exclusive_access();
    let pending = inner.signal_pending & !inner.signal_mask;
    if pending.is_empty() {
        return;
    }
    if inner.signal_trap_cx.is_some() && !pending.contains(SignalFlags::SIGKILL) {
        return;
    }
    let (signum, flag) = if pending.contains(SignalFlags::SIGKILL) {
        let sigkill_num = SignalFlags::SIGKILL.bits().trailing_zeros() as usize;
        (sigkill_num, SignalFlags::SIGKILL)
    } else {
        let signum = pending.bits().trailing_zeros() as usize;
        let flag = match SignalFlags::from_bits(1u32 << signum) {
            Some(flag) => flag,
            None => return,
        };
        (signum, flag)
    };
    inner.signal_pending.remove(flag);

    if signum == SignalFlags::SIGCONT.bits().trailing_zeros() as usize {
        inner.signal_stopped = false;
        inner.task_status = TaskStatus::Ready;
        return;
    }

    if signum == SignalFlags::SIGSTOP.bits().trailing_zeros() as usize {
        inner.signal_stopped = true;
        inner.task_status = TaskStatus::Stopped;
        drop(inner);
        suspend_current_and_run_next();
        return;
    }

    if signum == SignalFlags::SIGKILL.bits().trailing_zeros() as usize {
        drop(inner);
        exit_current_and_run_next(-(SigNumber::SigKill as i32));
        return;
    }

    let action = inner.signal_actions.table[signum];
    if action.handler == 0 {
        drop(inner);
        exit_current_and_run_next(-(signum as i32));
        return;
    }

    if inner.signal_trap_cx.is_none() {
        inner.signal_trap_cx = Some(*inner.get_trap_cx());
        inner.signal_mask_backup = inner.signal_mask;
        inner.signal_mask |= action.mask | flag;
    }
    let trap_cx = inner.get_trap_cx();
    trap_cx.sepc = action.handler;
}

lazy_static! {
    /// Creation of initial process
    ///
    /// the name "initproc" may be changed to any other app name like "usertests",
    /// but we have user_shell, so we don't need to change it.
    pub static ref INITPROC: Arc<TaskControlBlock> = Arc::new({
        let v = open_file("initcode", OpenFlags::empty())
            .or_else(|| open_file("ch6b_initproc", OpenFlags::empty()))
            .map(|inode| inode.read_all())
            .unwrap_or_else(|| INITPROC_EMBED.to_vec());
        TaskControlBlock::new(v.as_slice())
    });
}

#[cfg(debug_assertions)]
const INITPROC_EMBED: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../user/target/riscv64gc-unknown-none-elf/debug/initcode"
));
#[cfg(not(debug_assertions))]
const INITPROC_EMBED: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../user/target/riscv64gc-unknown-none-elf/release/initcode"
));

///Add init process to the manager
pub fn add_initproc() {
    add_task(INITPROC.clone());
}
