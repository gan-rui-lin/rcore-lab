#![allow(missing_docs)]

mod action;
mod auxv;
mod context;
mod id;
mod manager;
mod process;
mod processor;
mod sigframe;
mod signal;
mod switch;
#[allow(clippy::module_inception)]
#[allow(rustdoc::private_intra_doc_links)]
mod task;
mod tls;

use crate::fs::{open_file, OpenFlags};
use crate::arch::shutdown;
use crate::timer::remove_timer;
use alloc::sync::Arc;
use lazy_static::*;
#[allow(unused_imports)]
use alloc::vec::Vec;
use manager::fetch_task;
use process::ProcessControlBlock;
use switch::__switch;

pub use action::{
    SignalAction, SignalActions, KSigAction,
    SIG_DFL, SIG_IGN,
    SA_NOCLDSTOP, SA_NOCLDWAIT, SA_SIGINFO, SA_ONSTACK,
    SA_RESTART, SA_NODEFER, SA_RESETHAND, SA_RESTORER,
};
pub use auxv::AuxvInfo;
pub use context::TaskContext;
pub use id::{IDLE_PID, KernelStack, PidHandle, kstack_alloc, pid_alloc};
pub use manager::{
    add_task, pid2process, remove_from_pid2process, remove_task, wakeup_task,
};
pub use processor::{
    current_kstack_top, current_process, current_task, current_trap_cx, current_trap_cx_user_va,
    current_user_token, run_tasks, schedule, take_current_task,
};
pub use sigframe::restore_signal_frame;
pub use signal::{SigNumber, SigSet, SigDefaultAction, sig_default_action, MAX_SIG};
pub use task::{TaskControlBlock, TaskStatus};
pub use tls::{TlsArea, TlsInfo};

pub fn suspend_current_and_run_next() {
    let task = take_current_task().unwrap();
    let mut task_inner = task.inner_exclusive_access();
    let task_cx_ptr = &mut task_inner.task_cx as *mut TaskContext;
    task_inner.task_status = TaskStatus::Ready;
    drop(task_inner);
    add_task(task);
    schedule(task_cx_ptr);
}

/// This function must be followed by a schedule
pub fn block_current_task() -> *mut TaskContext {
    let task = take_current_task().unwrap();
    let mut task_inner = task.inner_exclusive_access();
    task_inner.task_status = TaskStatus::Blocked;
    &mut task_inner.task_cx as *mut TaskContext
}

pub fn block_current_and_run_next() {
    let task_cx_ptr = block_current_task();
    schedule(task_cx_ptr);
}

pub fn exit_current_and_run_next(exit_code: i32) {
    let task = take_current_task().unwrap();
    let mut task_inner = task.inner_exclusive_access();
    let process = task.process.upgrade().unwrap();
    let tid = task_inner.res.as_ref().unwrap().tid;
    task_inner.exit_code = Some(exit_code);
    task_inner.res = None;
    drop(task_inner);
    drop(task);
    if tid == 0 {
        let pid = process.getpid();
        if pid == IDLE_PID {
            shutdown();
        }
        remove_from_pid2process(pid);
        let mut process_inner = process.inner_exclusive_access();
        process_inner.is_zombie = true;
        process_inner.exit_code = exit_code;
        {
            let mut initproc_inner = INITPROC.inner_exclusive_access();
            for child in process_inner.children.iter() {
                child.inner_exclusive_access().parent = Some(Arc::downgrade(&INITPROC));
                initproc_inner.children.push(child.clone());
            }
        }
        let mut recycle_res = alloc::vec::Vec::new();
        for task in process_inner.tasks.iter().filter(|t| t.is_some()) {
            let task = task.as_ref().unwrap();
            remove_inactive_task(Arc::clone(&task));
            let mut task_inner = task.inner_exclusive_access();
            if let Some(res) = task_inner.res.take() {
                recycle_res.push(res);
            }
        }
        drop(process_inner);
        recycle_res.clear();
        let mut process_inner = process.inner_exclusive_access();
        process_inner.children.clear();
        process_inner.memory_set.recycle_data_pages();
        process_inner.fd_table.clear();
        while process_inner.tasks.len() > 1 {
            process_inner.tasks.pop();
        }
    }
    drop(process);
    let mut _unused = TaskContext::zero_init();
    schedule(&mut _unused as *mut _);
}

lazy_static! {
    pub static ref INITPROC: Arc<ProcessControlBlock> = {
        let v = open_file("initcode", OpenFlags::empty())
            .or_else(|| open_file("ch6b_initproc", OpenFlags::empty()))
            .map(|inode| inode.read_all())
            .unwrap_or_else(|| INITPROC_EMBED.to_vec());
        ProcessControlBlock::new(v.as_slice())
    };
}

#[cfg(all(debug_assertions, target_arch = "riscv64"))]
const INITPROC_EMBED: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../user/target/riscv64gc-unknown-none-elf/debug/initcode"
));

#[cfg(all(not(debug_assertions), target_arch = "riscv64"))]
const INITPROC_EMBED: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../user/target/riscv64gc-unknown-none-elf/release/initcode"
));

#[cfg(all(debug_assertions, target_arch = "loongarch64"))]
const INITPROC_EMBED: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../user/target/loongarch64-unknown-none/debug/initcode"
));

#[cfg(all(not(debug_assertions), target_arch = "loongarch64"))]
const INITPROC_EMBED: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../user/target/loongarch64-unknown-none/release/initcode"
));

pub fn add_initproc() {
    let _initproc = INITPROC.clone();
}

/// Add a signal to the process-level pending set (for kill() etc.)
pub fn current_add_signal(signum: usize) {
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    process_inner.signal_pending.add_sig(signum);
}

/// Check and deliver pending signals to the current task.
/// Called at trap exit before returning to user space.
pub fn handle_signals() {
    let task = match current_task() {
        Some(task) => task,
        None => return,
    };
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let mut task_inner = task.inner_exclusive_access();

    // Merge process-level and thread-level pending signals
    let all_pending = process_inner.signal_pending.or(task_inner.signal_pending);

    // SIGKILL and SIGSTOP are never maskable
    let effective_mask = task_inner.signal_mask.and_not(SigSet::unmaskable());
    let deliverable = all_pending.and_not(effective_mask);

    if deliverable.is_empty() {
        return;
    }

    // Pick the lowest signal number
    let signum = deliverable.lowest_signal().unwrap(); // 1-indexed

    // Dequeue from the correct source (prefer thread-level)
    if task_inner.signal_pending.contains_sig(signum) {
        task_inner.signal_pending.remove_sig(signum);
    } else {
        process_inner.signal_pending.remove_sig(signum);
    }

    // SIGSTOP: block the task
    if signum == 19 {
        drop(task_inner);
        drop(process_inner);
        block_current_and_run_next();
        return;
    }

    // SIGKILL: terminate immediately
    if signum == 9 {
        let pid = process.pid.0;
        let name = process_inner.name.clone();
        warn!("[signal] pid={} name={} killed by SIGKILL", pid, name);
        drop(task_inner);
        drop(process_inner);
        exit_current_and_run_next(-(SigNumber::SigKill as i32));
        return;
    }

    let action = process_inner.signal_actions.table[signum];

    // SIG_DFL (handler == 0): default action
    if action.handler == SIG_DFL {
        use SigDefaultAction::*;
        match sig_default_action(signum) {
            Term | Core => {
                let pid = process.pid.0;
                let name = process_inner.name.clone();
                warn!(
                    "[signal] pid={} name={} default handler for signum {} (terminate)",
                    pid, name, signum
                );
                drop(task_inner);
                drop(process_inner);
                exit_current_and_run_next(-(signum as i32));
                return;
            }
            Stop => {
                drop(task_inner);
                drop(process_inner);
                block_current_and_run_next();
                return;
            }
            Cont => {
                // Continue: just return to user space
                return;
            }
            Ignore => {
                // Ignored signal: discard
                return;
            }
        }
    }

    // SIG_IGN (handler == 1): ignore
    if action.handler == SIG_IGN {
        return;
    }

    // User handler: push signal frame to user stack
    let old_mask = task_inner.signal_mask;
    let token = process_inner.memory_set.token();
    let trap_cx = task_inner.get_trap_cx();

    // Save kernel-side backup for LoongArch64 fallback (no real sigframe yet)
    #[cfg(target_arch = "loongarch64")]
    {
        task_inner.signal_trap_cx = Some(*trap_cx);
        task_inner.signal_mask_backup = old_mask;
    }

    if sigframe::setup_signal_frame(token, trap_cx, signum, &action, old_mask).is_none() {
        // Failed to set up frame (e.g. stack overflow) — kill with SIGSEGV
        warn!("[signal] failed to setup signal frame for signum={}, killing", signum);
        drop(task_inner);
        drop(process_inner);
        exit_current_and_run_next(-(SigNumber::SigSegv as i32));
        return;
    }

    // Update signal mask: block sa_mask + the signal itself (unless SA_NODEFER)
    task_inner.signal_mask.union(action.mask);
    if action.flags & SA_NODEFER == 0 {
        task_inner.signal_mask.add_sig(signum);
    }
    task_inner.signal_mask.sanitize_mask();

    // SA_RESETHAND: reset handler to SIG_DFL after delivery
    if action.flags & SA_RESETHAND != 0 {
        process_inner.signal_actions.table[signum] = SignalAction::default();
    }
}

pub fn remove_inactive_task(task: Arc<TaskControlBlock>) {
    remove_task(Arc::clone(&task));
    remove_timer(Arc::clone(&task));
}

pub fn block_and_yield() {
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    process_inner.signal_pending.remove_sig(18); // SIGCONT
    drop(process_inner);
    block_current_and_run_next();
}
