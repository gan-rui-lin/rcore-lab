#![allow(missing_docs)]

mod action;
mod auxv;
mod context;
mod id;
mod manager;
mod process;
mod processor;
mod signal;
mod switch;
#[allow(clippy::module_inception)]
#[allow(rustdoc::private_intra_doc_links)]
mod task;
mod futex;
mod tls;

use crate::fs::{open_file, OpenFlags};
use crate::sbi::shutdown;
use crate::timer::remove_timer;
use alloc::sync::Arc;
use lazy_static::*;
#[allow(unused_imports)]
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use manager::fetch_task;
use process::ProcessControlBlock;
use switch::__switch;

pub use action::{SignalAction, SignalActions};
pub use auxv::AuxvInfo;
pub use context::TaskContext;
pub use id::{IDLE_PID, KernelStack, PidHandle, kstack_alloc, pid_alloc};
pub use manager::{
    add_task, pid2process, pid2process_snapshot, ready_queue_snapshot, remove_from_pid2process,
    remove_task, wakeup_task,
};
pub use processor::{
    current_kstack_top, current_process, current_task, current_trap_cx, current_trap_cx_user_va,
    current_user_token, run_tasks, schedule, take_current_task,
};
pub use process::{RLimit, RLIMIT_NLIMITS, RLIMIT_NOFILE, RLIMIT_STACK, RLIM_INFINITY};
pub use signal::{SignalFlags, SigNumber, MAX_SIG};
pub use task::{TaskControlBlock, TaskStatus};
pub use futex::{
    FutexKey, futex_requeue, futex_remove_waiter, futex_remove_waiter_any, futex_wait,
    futex_wait_bitset, futex_wake, futex_wake_bitset,
};
pub use tls::{TlsArea, TlsInfo};

const DEBUG_DUMP_INTERVAL: u64 = 200;
static DEBUG_DUMP_COUNTER: AtomicU64 = AtomicU64::new(0);

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
    // for debug
    let pid = process.getpid();
    let name = process.inner_exclusive_access().name.clone();
    info!(
        "[exit] pid={} tid={} name={} code={}",
        pid,
        tid,
        name,
        exit_code
    );
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
        let parent = process_inner.parent.as_ref().and_then(|p| p.upgrade());

        drop(process_inner);
        if let Some(parent) = parent {
            let mut parent_inner = parent.inner_exclusive_access();
            parent_inner.signal_pending |= SignalFlags::SIGCHLD;
        }
        let process_inner = process.inner_exclusive_access();
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

pub fn add_initproc() {
    let _initproc = INITPROC.clone();
}

pub fn current_add_signal(signal: SignalFlags) {
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    process_inner.signal_pending |= signal;
}

pub fn handle_signals() {
    let task = match current_task() {
        Some(task) => task,
        None => return,
    };
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let pending = process_inner.signal_pending & !process_inner.signal_mask;
    if pending.is_empty() {
        return;
    }
    let mut task_inner = task.inner_exclusive_access();
    if task_inner.task_status == TaskStatus::Blocked {
        futex_remove_waiter_any(&task);
        task_inner.task_status = TaskStatus::Ready;
        drop(task_inner);
        add_task(task);
        return;
    }
    if task_inner.signal_trap_cx.is_some() && !pending.contains(SignalFlags::SIGKILL) {
        return;
    }
    let (signum, flag) = if pending.contains(SignalFlags::SIGKILL) {
        let sigkill_num = SignalFlags::SIGKILL.bits().trailing_zeros() as usize;
        (sigkill_num, SignalFlags::SIGKILL)
    } else {
        let signum = pending.bits().trailing_zeros() as usize;
        let flag = match 1u64.checked_shl(signum as u32) {
            Some(bits) => SignalFlags::from_bits_truncate(bits),
            None => return,
        };
        (signum, flag)
    };
    process_inner.signal_pending.remove(flag);

    if signum == SignalFlags::SIGSTOP.bits().trailing_zeros() as usize {
        drop(task_inner);
        drop(process_inner);
        block_current_and_run_next();
        return;
    }

    if signum == SignalFlags::SIGKILL.bits().trailing_zeros() as usize {
        let pid = process.pid.0;
        let name = process_inner.name.clone();
        warn!("[signal] pid={} name={} killed by SIGKILL", pid, name);
        drop(task_inner);
        drop(process_inner);
        exit_current_and_run_next(-(SigNumber::SigKill as i32));
        return;
    }

    let action = process_inner.signal_actions.table[signum];
    if action.handler == 0 {
        let pid = process.pid.0;
        let name = process_inner.name.clone();
        warn!(
            "[signal] pid={} name={} default handler for signum {}",
            pid,
            name,
            signum
        );
        drop(task_inner);
        drop(process_inner);
        exit_current_and_run_next(-(signum as i32));
        return;
    }

    if task_inner.signal_trap_cx.is_none() {
        task_inner.signal_trap_cx = Some(*task_inner.get_trap_cx());
        task_inner.signal_mask_backup = process_inner.signal_mask;
        process_inner.signal_mask |= action.mask | flag;
    }
    let trap_cx = task_inner.get_trap_cx();
    trap_cx.sepc = action.handler;
}

pub fn debug_dump_tasks() {
    let count = DEBUG_DUMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    if count % DEBUG_DUMP_INTERVAL != 0 {
        return;
    }
    let processes = pid2process_snapshot();
    let ready = ready_queue_snapshot();
    info!("[debug] process count={} ready_queue={}", processes.len(), ready.len());
    for (idx, task) in ready.iter().enumerate() {
        let pid = task.process.upgrade().map(|p| p.getpid()).unwrap_or(0);
        if let Some(task_inner) = task.try_inner_exclusive_access() {
            let tid = task_inner.res.as_ref().map(|r| r.tid).unwrap_or(0);
            let trap_cx = task_inner.get_trap_cx();
            info!(
                "[debug] ready[{}] pid={} tid={} status={:?} last_syscall={} sepc={:#x} sp={:#x} ra={:#x}",
                idx,
                pid,
                tid,
                task_inner.task_status,
                task_inner.last_syscall,
                trap_cx.sepc,
                trap_cx.x[2],
                trap_cx.x[1]
            );
        } else {
            info!("[debug] ready[{}] pid={} <busy>", idx, pid);
        }
    }
    for (pid, process) in processes {
        let inner = process.inner_exclusive_access();
        info!(
            "[debug] pid={} name={} zombie={} tasks={} pending={:?} mask={:?}",
            pid,
            inner.name,
            inner.is_zombie,
            inner.tasks.len(),
            inner.signal_pending,
            inner.signal_mask
        );
        for (child_idx, child) in inner.children.iter().enumerate() {
            let child_inner = child.inner_exclusive_access();
            info!(
                "[debug]   child[{}] pid={} name={} zombie={}",
                child_idx,
                child.getpid(),
                child_inner.name,
                child_inner.is_zombie
            );
        }
        for (tid, task) in inner.tasks.iter().enumerate() {
            let Some(task) = task else {
                info!("[debug]   tid={} <none>", tid);
                continue;
            };
            if let Some(task_inner) = task.try_inner_exclusive_access() {
                let trap_cx = task_inner.get_trap_cx();
                info!(
                    "[debug]   tid={} status={:?} exit={:?} last_syscall={} sepc={:#x} sp={:#x} ra={:#x}",
                    tid,
                    task_inner.task_status,
                    task_inner.exit_code,
                    task_inner.last_syscall,
                    trap_cx.sepc,
                    trap_cx.x[2],
                    trap_cx.x[1]
                );
            } else {
                info!("[debug]   tid={} <busy>", tid);
            }
        }
    }
}

pub fn remove_inactive_task(task: Arc<TaskControlBlock>) {
    remove_task(Arc::clone(&task));
    remove_timer(Arc::clone(&task));
}

pub fn block_and_yield() {
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    process_inner.signal_pending.remove(SignalFlags::SIGCONT);
    drop(process_inner);
    block_current_and_run_next();

}
