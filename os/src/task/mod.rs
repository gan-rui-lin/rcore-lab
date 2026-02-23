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
use crate::mm::{translated_byte_buffer, translated_refmut, PageTable, VirtAddr};
use crate::sbi::shutdown;
use crate::trap::TrapContext;
use crate::timer::remove_timer;
use alloc::sync::Arc;
use lazy_static::*;
#[allow(unused_imports)]
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use manager::fetch_task;
use process::ProcessControlBlock;
use switch::__switch;

pub use action::{SignalAction, SignalActions, SA_SIGINFO};
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
pub use signal::{SignalFlags, SigNumber, MAX_SIG, flags_to_user_mask, user_mask_to_flags};
pub use task::{TaskControlBlock, TaskStatus};
pub use futex::{
    FutexKey, futex_requeue, futex_remove_waiter, futex_remove_waiter_any, futex_wait,
    futex_wait_bitset, futex_wake, futex_wake_bitset,
};
pub use tls::{TlsArea, TlsInfo};

const DEBUG_DUMP_INTERVAL: u64 = 200;
static DEBUG_DUMP_COUNTER: AtomicU64 = AtomicU64::new(0);

const SIGNAL_TRACE_INTERVAL: u64 = 2000;
static SIGNAL_TRACE_COUNTER: AtomicU64 = AtomicU64::new(0);

const LINUX_SIGINFO_SIZE: usize = 128;
const RISCV_FPU_STATE_SIZE: usize = 528;

#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxSigInfo {
    si_signo: i32,
    si_errno: i32,
    si_code: i32,
    _pad: [u8; LINUX_SIGINFO_SIZE - 12],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct StackT {
    pub ss_sp: usize,
    pub ss_flags: i32,
    pub _pad: i32,
    pub ss_size: usize,
}

#[repr(C, align(16))]
#[derive(Clone, Copy)]
pub struct MContext {
    pub gregs: [usize; 32],
    pub fpregs: [u8; RISCV_FPU_STATE_SIZE],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct UserContext {
    pub uc_flags: usize,
    pub uc_link: usize,
    pub uc_stack: StackT,
    pub uc_sigmask: u64,
    pub uc_mcontext: MContext,
}

const USER_UCONTEXT_SIZE: usize = core::mem::size_of::<UserContext>();

impl UserContext {
    fn from_trap(trap_cx: &TrapContext, sigmask: SignalFlags) -> Self {
        let mut gregs = [0usize; 32];
        gregs[0] = trap_cx.sepc;
        gregs[1..].copy_from_slice(&trap_cx.x[1..]);
        Self {
            uc_flags: 0,
            uc_link: 0,
            uc_stack: StackT {
                ss_sp: 0,
                ss_flags: 0,
                _pad: 0,
                ss_size: 0,
            },
            uc_sigmask: signal::flags_to_user_mask(sigmask),
            uc_mcontext: MContext {
                gregs,
                fpregs: [0u8; RISCV_FPU_STATE_SIZE],
            },
        }
    }
}

impl Default for LinuxSigInfo {
    fn default() -> Self {
        Self {
            si_signo: 0,
            si_errno: 0,
            si_code: 0,
            _pad: [0u8; LINUX_SIGINFO_SIZE - 12],
        }
    }
}

fn copy_to_user(token: usize, dst: *mut u8, data: &[u8]) -> Result<(), ()> {
    if dst.is_null() {
        return Err(());
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
    let clear_child_tid = task_inner.clear_child_tid;
    if tid != 0 && clear_child_tid != 0 {
        info!(
            "[exit] pid={} tid={} clear_child_tid={:#x}",
            pid,
            tid,
            clear_child_tid
        );
        let token = process.inner_exclusive_access().memory_set.token();
        let page_table = PageTable::from_token(token);
        if let Some(pa) = page_table.translate_va(VirtAddr::from(clear_child_tid)) {
            *translated_refmut(token, clear_child_tid as *mut i32) = 0;
            let key = FutexKey::new(pa, pid);
            let woke = futex_wake(key, 1);
            info!(
                "[exit] pid={} tid={} clear_child_tid wake addr={:#x} pa={:#x} woke={}",
                pid,
                tid,
                clear_child_tid,
                pa.0,
                woke
            );
        } else {
            warn!(
                "[exit] pid={} tid={} clear_child_tid addr={:#x} not mapped",
                pid,
                tid,
                clear_child_tid
            );
        }
    }
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
    let mut task_inner = task.inner_exclusive_access();
    let process_pending = process_inner.signal_pending & !process_inner.signal_mask;
    let task_pending = task_inner.signal_pending & !process_inner.signal_mask;
    let pending = process_pending | task_pending;
    if pending.is_empty() {
        return;
    }
    let signal_trace_count = SIGNAL_TRACE_COUNTER.fetch_add(1, Ordering::Relaxed);
    if signal_trace_count % SIGNAL_TRACE_INTERVAL == 0 {
        info!(
            "[signal] pending pid={} tid={} status={:?} proc_pending={:?} task_pending={:?} mask={:?} has_trap={}",
            process.pid.0,
            task_inner.res.as_ref().map(|r| r.tid).unwrap_or(0),
            task_inner.task_status,
            process_pending,
            task_pending,
            process_inner.signal_mask,
            task_inner.signal_trap_cx.is_some()
        );
    }
    let (signum, flag, from_process) = if process_pending.contains(SignalFlags::SIGKILL) {
        let sigkill_num = SignalFlags::SIGKILL.bits().trailing_zeros() as usize;
        (sigkill_num, SignalFlags::SIGKILL, true)
    } else if !task_pending.is_empty() {
        let signum = task_pending.bits().trailing_zeros() as usize;
        let flag = match 1u64.checked_shl(signum as u32) {
            Some(bits) => SignalFlags::from_bits_truncate(bits),
            None => return,
        };
        (signum, flag, false)
    } else {
        let signum = process_pending.bits().trailing_zeros() as usize;
        let flag = match 1u64.checked_shl(signum as u32) {
            Some(bits) => SignalFlags::from_bits_truncate(bits),
            None => return,
        };
        (signum, flag, true)
    };
    if task_inner.task_status == TaskStatus::Blocked {
        let tid = task_inner.res.as_ref().map(|r| r.tid).unwrap_or(0);
        futex_remove_waiter_any(&task);
        task_inner.task_status = TaskStatus::Ready;
        drop(task_inner);
        add_task(task);
        if signal_trace_count % SIGNAL_TRACE_INTERVAL == 0 {
            info!(
                "[signal] wake blocked pid={} tid={} signum={} from_process={}",
                process.pid.0,
                tid,
                signum,
                from_process
            );
        }
        return;
    }
    if task_inner.signal_trap_cx.is_some()
        && signum != SignalFlags::SIGKILL.bits().trailing_zeros() as usize
    {
        if signal_trace_count % SIGNAL_TRACE_INTERVAL == 0 {
            info!(
                "[signal] skip deliver pid={} tid={} signum={} reason=trap_cx_set",
                process.pid.0,
                task_inner.res.as_ref().map(|r| r.tid).unwrap_or(0),
                signum
            );
        }
        return;
    }
    if from_process {
        process_inner.signal_pending.remove(flag);
    } else {
        task_inner.signal_pending.remove(flag);
    }

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
    if signum == 33 {
        info!(
            "[signal] sig=33 pid={} tid={} handler={:#x} flags={:#x} mask={:?} sepc={:#x} sp={:#x}",
            process.pid.0,
            task_inner.res.as_ref().map(|r| r.tid).unwrap_or(0),
            action.handler,
            action.flags,
            action.mask,
            task_inner.get_trap_cx().sepc,
            task_inner.get_trap_cx().x[2]
        );
    }
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
    if action.restorer != 0 {
        trap_cx.x[1] = action.restorer;
    }
    let need_siginfo = (action.flags & SA_SIGINFO) != 0 || signum == 33;
    if need_siginfo {
        let token = process_inner.memory_set.token();
        let mut user_sp = trap_cx.x[2] & !0xf;

        user_sp = user_sp.saturating_sub(USER_UCONTEXT_SIZE);
        let ucontext_ptr = user_sp as *mut u8;
        if let Some(saved) = task_inner.signal_trap_cx.as_ref() {
            let ucontext = UserContext::from_trap(saved, task_inner.signal_mask_backup);
            let ucontext_bytes = unsafe {
                core::slice::from_raw_parts(
                    (&ucontext as *const UserContext) as *const u8,
                    core::mem::size_of::<UserContext>(),
                )
            };
            let _ = copy_to_user(token, ucontext_ptr, ucontext_bytes);
            task_inner.signal_ucontext_ptr = ucontext_ptr as usize;
        }

        user_sp = user_sp.saturating_sub(core::mem::size_of::<LinuxSigInfo>());
        let info_ptr = user_sp as *mut u8;
        let mut siginfo = LinuxSigInfo::default();
        siginfo.si_signo = signum as i32;
        let siginfo_bytes = unsafe {
            core::slice::from_raw_parts(
                (&siginfo as *const LinuxSigInfo) as *const u8,
                core::mem::size_of::<LinuxSigInfo>(),
            )
        };
        let _ = copy_to_user(token, info_ptr, siginfo_bytes);

        trap_cx.x[10] = signum;
        trap_cx.x[11] = info_ptr as usize;
        trap_cx.x[12] = ucontext_ptr as usize;
        trap_cx.x[2] = user_sp;
    } else {
        task_inner.signal_ucontext_ptr = 0;
    }
    if signum == 33 && trap_cx.x[11] == 0 {
        let token = process_inner.memory_set.token();
        let base_sp = trap_cx.x[2] & !0xf;
        let info_ptr = base_sp.saturating_sub(LINUX_SIGINFO_SIZE) as *mut u8;
        let ucontext_ptr = (info_ptr as usize)
            .saturating_sub(USER_UCONTEXT_SIZE) as *mut u8;
        let mut siginfo = LinuxSigInfo::default();
        siginfo.si_signo = signum as i32;
        let siginfo_bytes = unsafe {
            core::slice::from_raw_parts(
                (&siginfo as *const LinuxSigInfo) as *const u8,
                core::mem::size_of::<LinuxSigInfo>(),
            )
        };
        let _ = copy_to_user(token, info_ptr, siginfo_bytes);
        if let Some(saved) = task_inner.signal_trap_cx.as_ref() {
            let ucontext = UserContext::from_trap(saved, task_inner.signal_mask_backup);
            let ucontext_bytes = unsafe {
                core::slice::from_raw_parts(
                    (&ucontext as *const UserContext) as *const u8,
                    core::mem::size_of::<UserContext>(),
                )
            };
            let _ = copy_to_user(token, ucontext_ptr, ucontext_bytes);
            task_inner.signal_ucontext_ptr = ucontext_ptr as usize;
        }
        trap_cx.x[11] = info_ptr as usize;
        trap_cx.x[12] = ucontext_ptr as usize;
    }
    trap_cx.x[10] = signum;
    if signum == 33 {
        trace!(
            "[signal] deliver sig=33 pid={} tid={} a0={:#x} a1={:#x} a2={:#x} sp={:#x}",
            process.pid.0,
            task_inner.res.as_ref().map(|r| r.tid).unwrap_or(0),
            trap_cx.x[10],
            trap_cx.x[11],
            trap_cx.x[12],
            trap_cx.x[2]
        );
        if task_inner.signal_ucontext_ptr != 0 {
            let user_mask = signal::flags_to_user_mask(task_inner.signal_mask_backup);
            trace!(
                "[signal] sig=33 ucontext_ptr={:#x} mask={:#x}",
                task_inner.signal_ucontext_ptr,
                user_mask
            );
        }
    }
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
