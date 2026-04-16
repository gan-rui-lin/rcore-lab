#![allow(missing_docs)]

mod action;
mod auxv;
mod context;
mod futex;
mod id;
mod manager;
mod process;
mod processor;
mod signal;
#[allow(clippy::module_inception)]
#[allow(rustdoc::private_intra_doc_links)]
mod task;
mod tls;
use crate::config::USER_STACK_TOP as USER_ADDR_MAX;
#[cfg_attr(
    all(debug_assertions, target_arch = "riscv64"),
    path = "initproc_embed_riscv64_debug.rs"
)]
#[cfg_attr(
    all(not(debug_assertions), target_arch = "riscv64"),
    path = "initproc_embed_riscv64_release.rs"
)]
#[cfg_attr(target_arch = "loongarch64", path = "initproc_embed_loongarch64.rs")]
mod initproc_embed;
#[allow(unused_imports)]
use crate::fs::{open_file, OpenFlags};
use crate::mm::{translated_byte_buffer_checked, translated_refmut, PageTable, VirtAddr};
use crate::timer::remove_timer;
use alloc::sync::Arc;
#[allow(unused_imports)]
use alloc::vec::Vec;
use arch::{shutdown, TrapContext, TrapFrameArgs};
use core::sync::atomic::{AtomicU64, Ordering};
use lazy_static::*;
use manager::fetch_task;
use process::ProcessControlBlock;

pub use action::{SignalAction, SignalActions, SA_RESTART, SA_SIGINFO, SA_RESETHAND};
pub use auxv::AuxvInfo;
pub use context::TaskContext;
pub use futex::{
    futex_remove_waiter, futex_remove_waiter_any, futex_requeue, futex_wait, futex_wait_bitset,
    futex_wake, futex_wake_bitset, FutexKey,
};
pub use id::{kstack_alloc, pid_alloc, KernelStack, PidHandle, IDLE_PID};
pub use manager::{
    add_task, pid2process, pid2process_aggregate, pid2process_fdtable_summary, pid2process_len,
    pid2process_snapshot, ready_queue_len, ready_queue_snapshot, remove_from_pid2process,
    remove_task, wakeup_task, print_pid2process_top_by_fd, print_ready_queue_brief,
};
pub use process::{
    ChildWaitEvent, IntervalTimerState, RLimit, RLIMIT_NLIMITS, RLIMIT_NOFILE, RLIMIT_STACK,
    RLIM_INFINITY,
};
pub use processor::{
    current_kstack_top, current_process, current_task, current_trap_cx, current_trap_cx_user_va,
    current_task_context_for_alloc_trace, current_user_token, has_pending_unmasked_signal,
    print_current_task_brief_for_alloc_error, run_tasks, schedule, take_current_task,
};
pub use signal::{
    flags_to_user_mask, user_mask_to_flags, SigNumber, SignalFlags, MAX_SIG, SIGABRT, SIGALRM,
    SIGBUS, SIGCHLD, SIGCONT, SIGFPE, SIGHUP, SIGILL, SIGINT, SIGIO, SIGKILL, SIGPIPE, SIGPROF,
    SIGPWR, SIGQUIT, SIGSEGV, SIGSTKFLT, SIGSTOP, SIGSYS, SIGTERM, SIGTRAP, SIGTSTP, SIGTTIN,
    SIGTTOU, SIGURG, SIGUSR1, SIGUSR2, SIGVTALRM, SIGWINCH, SIGXCPU, SIGXFSZ,
};
pub use task::{
    live_task_count, live_task_pid_summary, TaskControlBlock, TaskControlBlockInner, TaskStatus,
};
pub use tls::{TlsArea, TlsInfo};

const DEBUG_DUMP_INTERVAL: u64 = 200;
static DEBUG_DUMP_COUNTER: AtomicU64 = AtomicU64::new(0);

const LINUX_SIGINFO_SIZE: usize = 128;
const LINUX_SIGSET_WORDS: usize = 16; // 128 bytes / 8

#[cfg_attr(target_arch = "riscv64", path = "user_context_riscv64.rs")]
#[cfg_attr(target_arch = "loongarch64", path = "user_context_loongarch64.rs")]
mod user_context;
pub use user_context::UserContext;

#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxSigInfo {
    si_signo: i32,
    si_errno: i32,
    si_code: i32,
    // Keep 8-byte alignment for the payload union so si_pid is at +16.
    _align_pad: i32,
    _pad: [u8; LINUX_SIGINFO_SIZE - 16],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct StackT {
    pub ss_sp: usize,
    pub ss_flags: i32,
    pub _pad: i32,
    pub ss_size: usize,
}

const USER_UCONTEXT_SIZE: usize = core::mem::size_of::<UserContext>();
// musl cancel_handler reads/writes uc_mcontext.gregs[0]/pc at ucontext+176.
const _USER_UCONTEXT_LAYOUT_CHECK: [(); core::mem::offset_of!(UserContext, uc_mcontext)] =
    [(); 176];

impl Default for LinuxSigInfo {
    fn default() -> Self {
        Self {
            si_signo: 0,
            si_errno: 0,
            si_code: 0,
            _align_pad: 0,
            _pad: [0u8; LINUX_SIGINFO_SIZE - 16],
        }
    }
}

fn copy_to_user(token: usize, dst: *mut u8, data: &[u8]) -> Result<(), ()> {
    if dst.is_null() {
        return Err(());
    }
    if data.is_empty() {
        return Ok(());
    }
    let mut offset = 0usize;
    let Some(slices) = translated_byte_buffer_checked(token, dst, data.len(), true) else {
        return Err(());
    };
    for slice in slices {
        let len = slice.len().min(data.len() - offset);
        slice[..len].copy_from_slice(&data[offset..offset + len]);
        offset += len;
        if offset >= data.len() {
            break;
        }
    }
    if offset == data.len() {
        Ok(())
    } else {
        Err(())
    }
}

pub fn suspend_current_and_run_next() {
    let task = take_current_task().unwrap();
    task.kstack.check_guard();
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
    task.kstack.check_guard();
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
    task.kstack.check_guard();
    let mut task_inner = task.inner_exclusive_access();
    let process = task.process.upgrade().unwrap();
    let tid = task_inner.res.as_ref().unwrap().tid;
    // for debug
    let pid = process.getpid();
    let name = process.inner_exclusive_access().name.clone();
    info!(
        "[exit] pid={} tid={} name={} code={}",
        pid, tid, name, exit_code
    );
    let clear_child_tid = task_inner.clear_child_tid;
    // Linux processes clear_child_tid for ALL threads, including the main thread (tid=0).
    // musl's __init_tls calls set_tid_address for the main thread.
    if clear_child_tid != 0 {
        info!(
            "[exit] pid={} tid={} clear_child_tid={:#x}",
            pid, tid, clear_child_tid
        );
        let token = process.inner_exclusive_access().memory_set.token();
        let page_table = PageTable::from_token(token);
        if let Some(pa) = page_table.translate_va(VirtAddr::from(clear_child_tid)) {
            *translated_refmut(token, clear_child_tid as *mut i32) = 0;

            // 唤醒进程内共享的 futex（FUTEX_PRIVATE_FLAG=1）
            let thread_shared_key = FutexKey::new(pa, pid);
            let woke_private = futex_wake(thread_shared_key, 1);

            // 唤醒进程间共享的 futex（FUTEX_PRIVATE_FLAG=0）
            // 这确保无论 musl/glibc 使用哪种 futex 模式都能被正确唤醒
            let process_shared_key = FutexKey::new(pa, 0);
            let woke_shared = futex_wake(process_shared_key, 1);

            info!(
                "[exit] pid={} tid={} clear_child_tid wake addr={:#x} pa={:#x} woke_private={} woke_shared={}",
                pid,
                tid,
                clear_child_tid,
                pa.0,
                woke_private,
                woke_shared
            );
        } else {
            warn!(
                "[exit] pid={} tid={} clear_child_tid addr={:#x} not mapped",
                pid, tid, clear_child_tid
            );
        }
    }
    task_inner.exit_code = Some(exit_code);
    task_inner.res = None;
    drop(task_inner);
    drop(task);
    let pid = process.getpid();
    let all_threads_exited = {
        let process_inner = process.inner_exclusive_access();
        process_inner
            .tasks
            .iter()
            .filter_map(|t| t.as_ref())
            .all(|t| t.inner_exclusive_access().exit_code.is_some())
    };
    if tid == 0 || all_threads_exited {
        if pid == IDLE_PID {
            shutdown();
        }
        crate::syscall::cleanup_shm_for_process_exit(pid);
        let mut process_inner = process.inner_exclusive_access();
        process_inner.is_zombie = true;
        process_inner.exit_code = exit_code;
        let parent = process_inner.parent.as_ref().and_then(|p| p.upgrade());

        drop(process_inner);
        if let Some(parent) = parent {
            let mut parent_inner = parent.inner_exclusive_access();
            parent_inner.signal_pending |= SignalFlags::SIGCHLD;
            parent_inner.set_pending_signal_siginfo(SIGCHLD, pid as i32, 0);
        }
        let process_inner = process.inner_exclusive_access();
        if let Some(vfork_parent) = process_inner.vfork_vm_parent.as_ref().and_then(|p| p.upgrade()) {
            let mut parent_inner = vfork_parent.inner_exclusive_access();
            let copied = parent_inner
                .memory_set
                .sync_user_writable_from(&process_inner.memory_set);
            trace!(
                "[vfork] sync child pid={} -> parent pid={} copied_pages={}",
                pid,
                vfork_parent.pid.0,
                copied
            );
        }
        if Arc::ptr_eq(&process, &INITPROC) {
            warn!("[exit] initproc is exiting; skip reparent-to-initproc");
            for child in process_inner.children.iter() {
                if Arc::ptr_eq(child, &process) {
                    continue;
                }
                child.inner_exclusive_access().parent = None;
            }
        } else {
            let mut initproc_inner = INITPROC.inner_exclusive_access();
            for child in process_inner.children.iter() {
                if Arc::ptr_eq(child, &INITPROC) {
                    continue;
                }
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

const INITPROC_EMBED: &[u8] = initproc_embed::INITPROC_EMBED;

pub fn add_initproc() {
    let _initproc = INITPROC.clone();
}

pub fn current_add_signal(signal: SignalFlags) {
    let process = current_process();
    let pid = process.getpid() as i32;
    let mut process_inner = process.inner_exclusive_access();
    process_inner.signal_pending |= signal;
    let mut bits = signal.bits();
    while bits != 0 {
        let idx = bits.trailing_zeros() as usize;
        process_inner.set_pending_signal_siginfo(idx + 1, pid, 0);
        bits &= bits - 1;
    }
}

fn wake_process_for_signal(process: &Arc<ProcessControlBlock>) {
    let inner = process.inner_exclusive_access();
    for task in inner.tasks.iter().filter_map(|t| t.as_ref()) {
        let mut task_inner = task.inner_exclusive_access();
        if task_inner.task_status == TaskStatus::Blocked {
            futex_remove_waiter_any(task);
            task_inner.interrupted_by_signal = true;
            task_inner.task_status = TaskStatus::Ready;
            drop(task_inner);
            add_task(task.clone());
        }
    }
}

fn itimer_signal(which: usize) -> SignalFlags {
    match which {
        0 => SignalFlags::SIGALRM,
        1 => SignalFlags::SIGVTALRM,
        2 => SignalFlags::SIGPROF,
        _ => SignalFlags::empty(),
    }
}

fn tick_timer_state(state: &mut IntervalTimerState, delta_us: usize) -> bool {
    if state.remaining_us == 0 {
        return false;
    }
    if state.remaining_us > delta_us {
        state.remaining_us -= delta_us;
        return false;
    }
    if state.interval_us > 0 {
        state.remaining_us = state.interval_us;
    } else {
        state.remaining_us = 0;
    }
    true
}

pub fn process_interval_timers(on_user_tick: bool) {
    const TIMER_TICK_US: usize = 10_000;

    for (_pid, process) in pid2process_snapshot() {
        let should_signal = {
            let mut inner = process.inner_exclusive_access();
            tick_timer_state(&mut inner.itimers[0], TIMER_TICK_US)
        };
        if should_signal {
            let mut inner = process.inner_exclusive_access();
            inner.signal_pending |= SignalFlags::SIGALRM;
            inner.set_pending_signal_siginfo(signal::SIGALRM, 0, 0);
            drop(inner);
            wake_process_for_signal(&process);
        }
    }

    if !on_user_tick {
        return;
    }

    let Some(process) = current_task().and_then(|task| task.process.upgrade()) else {
        return;
    };
    let mut pending = SignalFlags::empty();
    {
        let mut inner = process.inner_exclusive_access();
        for which in [1usize, 2usize] {
            if tick_timer_state(&mut inner.itimers[which], TIMER_TICK_US) {
                pending |= itimer_signal(which);
            }
        }
        if !pending.is_empty() {
            inner.signal_pending |= pending;
            let mut bits = pending.bits();
            while bits != 0 {
                let idx = bits.trailing_zeros() as usize;
                inner.set_pending_signal_siginfo(idx + 1, 0, 0);
                bits &= bits - 1;
            }
        }
    }
    if !pending.is_empty() {
        wake_process_for_signal(&process);
    }
}

/// 查找第一个待处理的信号
/// 返回：(signum, flag, from_process)
/// 注意：signum 是信号编号（1-64），对应 bit 位置是 signum-1
fn find_pending_signal(
    process_pending: SignalFlags,
    task_pending: SignalFlags,
) -> Option<(usize, SignalFlags, bool)> {
    // 优先处理 SIGKILL
    if process_pending.contains(SignalFlags::SIGKILL) {
        return Some((signal::SIGKILL, SignalFlags::SIGKILL, true));
    }

    // 处理任务级信号
    if !task_pending.is_empty() {
        let bit_pos = task_pending.bits().trailing_zeros() as usize;
        let signum = bit_pos + 1; // bit N 对应 signum N+1
        let flag = SignalFlags::from_bits_truncate(1u64 << bit_pos);
        return Some((signum, flag, false));
    }

    // 处理进程级信号
    if !process_pending.is_empty() {
        let bit_pos = process_pending.bits().trailing_zeros() as usize;
        let signum = bit_pos + 1; // bit N 对应 signum N+1
        let flag = SignalFlags::from_bits_truncate(1u64 << bit_pos);
        return Some((signum, flag, true));
    }

    None
}

// SIGCANCEL 循环检测已统一移至 sys_sigreturn 中处理。
// sigreturn 能判断 handler 是否修改了 PC（成功取消 vs 失败），
// 在失败时重新注入 SIG33 并在多次失败后强制退出。

/// 设置用户态信号栈（UserContext + LinuxSigInfo + canary）
fn setup_signal_stack(
    signum: usize,
    sender_pid: i32,
    si_code: i32,
    trap_cx: &mut TrapContext,
    saved_cx: &TrapContext,
    signal_mask_backup: SignalFlags,
    token: usize,
    need_siginfo: bool,
) -> (usize, usize) {
    let mut user_sp = trap_cx[TrapFrameArgs::SP] & !0xf;

    if need_siginfo {
        // 压入 UserContext
        user_sp = user_sp.saturating_sub(USER_UCONTEXT_SIZE);
        let ucontext_ptr = user_sp;
        let ucontext = UserContext::from_trap(saved_cx, signal_mask_backup);
        let ucontext_bytes = unsafe {
            core::slice::from_raw_parts(
                (&ucontext as *const UserContext) as *const u8,
                core::mem::size_of::<UserContext>(),
            )
        };
        let _ = copy_to_user(token, ucontext_ptr as *mut u8, ucontext_bytes);

        // 压入 LinuxSigInfo
        user_sp = user_sp.saturating_sub(core::mem::size_of::<LinuxSigInfo>());
        let info_ptr = user_sp;
        let mut siginfo = LinuxSigInfo::default();
        siginfo.si_signo = signum as i32;
        siginfo.si_code = si_code;
        siginfo._pad[0..4].copy_from_slice(&sender_pid.to_ne_bytes()); // si_pid at offset 16
        // glibc sigcancel_handler expects SI_TKILL for SIG32/SIG33.
        if (signum == 32 || signum == 33) && siginfo.si_code == 0 {
            siginfo.si_code = -6; // SI_TKILL
        }
        let siginfo_bytes = unsafe {
            core::slice::from_raw_parts(
                (&siginfo as *const LinuxSigInfo) as *const u8,
                core::mem::size_of::<LinuxSigInfo>(),
            )
        };
        let _ = copy_to_user(token, info_ptr as *mut u8, siginfo_bytes);

        // 设置参数寄存器
        trap_cx[TrapFrameArgs::ARG0] = signum;
        trap_cx[TrapFrameArgs::ARG1] = info_ptr;
        trap_cx[TrapFrameArgs::ARG2] = ucontext_ptr;

        // 压入 canary
        user_sp = user_sp.saturating_sub(core::mem::size_of::<usize>());
        let canary: usize = 0x11451415;
        let canary_bytes = canary.to_le_bytes();
        let _ = copy_to_user(token, user_sp as *mut u8, &canary_bytes);

        trap_cx[TrapFrameArgs::SP] = user_sp;
        (ucontext_ptr, user_sp) // 返回 (ucontext_ptr, canary_ptr)
    } else {
        // 没有 siginfo，只设置参数
        trap_cx[TrapFrameArgs::ARG0] = signum;

        // 压入 canary
        user_sp = user_sp.saturating_sub(core::mem::size_of::<usize>());
        let canary: usize = 0x11451415;
        let canary_bytes = canary.to_le_bytes();
        let _ = copy_to_user(token, user_sp as *mut u8, &canary_bytes);

        trap_cx[TrapFrameArgs::SP] = user_sp;
        (0, user_sp) // 没有 ucontext_ptr
    }
}

pub fn handle_signals() {
    // 1. 获取当前任务和进程
    let task = match current_task() {
        Some(task) => task,
        None => return,
    };
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let mut task_inner = task.inner_exclusive_access();

    // 2. 检查信号重入
    if task_inner.handling_sig != -1 {
        debug!(
            "[handle_signals] pid={} tid={} already handling signal {}, deferring",
            process.pid.0,
            task_inner.res.as_ref().map(|r| r.tid).unwrap_or(0),
            task_inner.handling_sig
        );
        return;
    }

    // 3. 计算待处理信号（signal_mask 是 per-thread 的）
    let process_pending = process_inner.signal_pending & !task_inner.signal_mask;
    let task_pending = task_inner.signal_pending & !task_inner.signal_mask;

    if (process_pending | task_pending).is_empty() {
        return;
    }

    // 4. 查找第一个待处理的信号
    let (signum, flag, from_process) = match find_pending_signal(process_pending, task_pending) {
        Some(result) => result,
        None => return,
    };

    let pid = process.pid.0;
    let _tid = task_inner.res.as_ref().map(|r| r.tid).unwrap_or(0);
    if signum == 32 || signum == 33 {
        let tp = task_inner.get_trap_cx()[TrapFrameArgs::TLS];
        info!(
            "[handle_signals] pid={} tid={} sig{} tp={:#x} mask={:?} task_pending={:?} proc_pending={:?}",
            pid,
            _tid,
            signum,
            tp,
            task_inner.signal_mask,
            task_pending,
            process_pending
        );
    }
    // 5. 如果任务被阻塞，唤醒它
    if task_inner.task_status == TaskStatus::Blocked {
        if signum == 32 || signum == 33 {
            let trap_cx = task_inner.get_trap_cx();
            info!(
                "[signal-flow] route=blocked_wakeup pid={} tid={} sig{} from_process={} handling_sig={} sepc={:#x} sp={:#x} ra={:#x} mask={:?} task_pending={:?} proc_pending={:?}",
                pid,
                _tid,
                signum,
                from_process,
                task_inner.handling_sig,
                trap_cx.sepc,
                trap_cx[TrapFrameArgs::SP],
                trap_cx[TrapFrameArgs::RA],
                task_inner.signal_mask,
                task_pending,
                process_pending
            );
        }
        futex_remove_waiter_any(&task);
        task_inner.task_status = TaskStatus::Ready;
        task_inner.interrupted_by_signal = true;
        drop(task_inner);
        add_task(task);
        return;
    }

    // 7. 如果已经在处理其他信号，跳过（除了 SIGKILL）
    if task_inner.signal_trap_cx.is_some() && signum != signal::SIGKILL {
        if signum == 32 || signum == 33 {
            info!(
                "[signal-flow] route=defer_nested pid={} tid={} sig{} handling_sig={} saved_ctx_present=true",
                pid,
                _tid,
                signum,
                task_inner.handling_sig
            );
        }
        return;
    }

    let (sender_pid, si_code) = if from_process {
        process_inner.get_pending_signal_siginfo(signum)
    } else {
        (0, 0)
    };

    // 8. 清除 pending 信号
    if from_process {
        process_inner.signal_pending.remove(flag);
        process_inner.clear_pending_signal_siginfo(signum);
    } else {
        task_inner.signal_pending.remove(flag);
    }

    // For ptrace(PTRACE_TRACEME), signal delivery stops the tracee and
    // reports WIFSTOPPED to parent waitpid(), except for SIGKILL.
    if process_inner.ptrace_traceme && signum != signal::SIGKILL {
        process_inner.ptrace_stop_signal = Some(signum as i32);
        drop(task_inner);
        drop(process_inner);
        block_current_and_run_next();
        return;
    }

    // 9. 处理内核信号（SIGSTOP、SIGKILL）
    if signum == signal::SIGSTOP {
        process_inner.child_wait_event = Some(ChildWaitEvent::Stopped(signal::SIGSTOP as i32));
        process_inner.group_stopped = true;
        drop(task_inner);
        drop(process_inner);
        block_current_and_run_next();
        return;
    }

    if signum == signal::SIGKILL {
        warn!(
            "[signal] pid={} name={} killed by SIGKILL",
            pid, process_inner.name
        );
        // SIGKILL 必须终止进程内所有线程，不仅仅是当前线程
        // 向其他线程也注入 SIGKILL，确保它们在下次调度时退出
        for other_task in process_inner.tasks.iter().filter_map(|t| t.as_ref()) {
            if !Arc::ptr_eq(other_task, &task) {
                let mut other_inner = other_task.inner_exclusive_access();
                other_inner.signal_pending.insert(SignalFlags::SIGKILL);
                if other_inner.task_status == TaskStatus::Blocked {
                    futex_remove_waiter_any(other_task);
                    other_inner.interrupted_by_signal = true;
                    other_inner.task_status = TaskStatus::Ready;
                    drop(other_inner);
                    add_task(other_task.clone());
                }
            }
        }
        drop(task_inner);
        drop(process_inner);
        exit_current_and_run_next(-(signal::SIGKILL as i32));
        return;
    }

    // 10. 获取信号处理动作
    let mut action = process_inner.signal_actions.table[signum];
    if signum == SIGCHLD && action.handler != 0 {
        let page_table = PageTable::from_token(process_inner.memory_set.token());
        let handler_va = VirtAddr::from(action.handler);
        let invalid = match page_table.translate(handler_va.floor()) {
            Some(pte) => !pte.is_valid(),
            None => true,
        };
        if invalid {
            warn!(
                "[signal] pid={} sigchld invalid handler={:#x}, resetting to default",
                pid, action.handler
            );
            action = SignalAction::default();
            process_inner.signal_actions.table[signum] = action;
        }
    }
    if action.handler == 1 {
        debug!("[signal] pid={} signum={} handler=SIG_IGN", pid, signum);
        return;
    }
    if action.handler >= USER_ADDR_MAX && action.handler > 1 {
        error!(
            "[signal] pid={} signum={} invalid handler={:#x}, falling back to SIG_DFL",
            pid, signum, action.handler
        );
        action.handler = 0;
    }
    if signum == 32 || signum == 33 {
        info!(
            "[handle_signals] pid={} tid={} sig{} handler={:#x} flags={:#x} restorer={:#x}",
            pid,
            _tid,
            signum,
            action.handler,
            action.flags,
            action.restorer()
        );
    }
    if signum == SIGCHLD {
        trace!(
            "[handle_signals] pid={} tid={} sigchld handler={:#x} flags={:#x} restorer={:#x} mask={:?}",
            pid,
            _tid,
            action.handler,
            action.flags,
            action.restorer(),
            task_inner.signal_mask
        );
    }

    // 11. 默认处理（SIG_DFL）
    if action.handler == 0 {
        match signum {
            // 默认忽略的信号
            signal::SIGCHLD | signal::SIGURG | signal::SIGWINCH => {
                debug!("[signal] pid={} signum={} default=ignore", pid, signum);
                return;
            }
            // SIGCONT: 恢复被停止的进程（目前简单忽略）
            signal::SIGCONT => {
                process_inner.child_wait_event = Some(ChildWaitEvent::Continued(signal::SIGCONT as i32));
                process_inner.group_stopped = false;
                debug!("[signal] pid={} SIGCONT default=continue", pid);
                return;
            }
            // 默认停止的信号
            signal::SIGTSTP | signal::SIGTTIN | signal::SIGTTOU => {
                process_inner.child_wait_event = Some(ChildWaitEvent::Stopped(signum as i32));
                process_inner.group_stopped = true;
                debug!("[signal] pid={} signum={} default=stop", pid, signum);
                drop(task_inner);
                drop(process_inner);
                block_current_and_run_next();
                return;
            }
            // 其他信号默认终止进程
            _ => {
                warn!(
                    "[signal] pid={} name={} default handler for signal {} -> terminate",
                    pid, process_inner.name, signum
                );
                // Fatal default signals should terminate the whole thread group.
                // Otherwise one thread can die while siblings keep waiting (e.g. futex join),
                // and the process hangs instead of exiting by signal.
                for other_task in process_inner.tasks.iter().filter_map(|t| t.as_ref()) {
                    if !Arc::ptr_eq(other_task, &task) {
                        let mut other_inner = other_task.inner_exclusive_access();
                        other_inner.signal_pending.insert(flag);
                        other_inner.signal_mask.remove(flag);
                        if other_inner.task_status == TaskStatus::Blocked {
                            futex_remove_waiter_any(other_task);
                            other_inner.interrupted_by_signal = true;
                            other_inner.task_status = TaskStatus::Ready;
                            drop(other_inner);
                            add_task(other_task.clone());
                        }
                    }
                }
                drop(task_inner);
                drop(process_inner);
                exit_current_and_run_next(-(signum as i32));
                return;
            }
        }
    }

    // 12. 保存 trap context（如果尚未保存）
    if task_inner.signal_trap_cx.is_none() {
        task_inner.signal_trap_cx = Some(*task_inner.get_trap_cx());
        task_inner.signal_mask_backup = task_inner.signal_mask;
        task_inner.signal_mask |= action.mask | flag;
        task_inner.handling_sig = signum as isize;
    }

    // 13. 设置 trap context 调用用户态 handler
    let old_pc = task_inner.get_trap_cx().sepc;
    let old_sp = task_inner.get_trap_cx()[TrapFrameArgs::SP];
    let old_ra = task_inner.get_trap_cx()[TrapFrameArgs::RA];
    let trap_cx = task_inner.get_trap_cx();
    trap_cx.sepc = action.handler;
    // LoongArch: no SA_RESTORER, always use kernel trampoline for sigreturn
    #[cfg(target_arch = "loongarch64")]
    {
        if let Some(res) = task_inner.res.as_ref() {
            let tramp_base = res.ustack_base().saturating_sub(crate::config::PAGE_SIZE);
            let tramp_offset = arch::sigtrx::sigreturn_trampoline_offset();
            let tramp = tramp_base + tramp_offset;
            trap_cx[TrapFrameArgs::RA] = tramp;
            trace!(
                "[signal] pid={} signum={} tramp_base={:#x} tramp={:#x} offset={:#x} sp={:#x}",
                pid,
                signum,
                tramp_base,
                tramp,
                tramp_offset,
                trap_cx[TrapFrameArgs::SP]
            );
        }
    }
    // RISC-V: use sa_restorer if valid and looks like a real mapped address;
    // otherwise fallback to fixed SIG_RETURN_ADDR stub.
    // glibc dynamic binaries may have unrelocated sa_restorer values
    // (e.g., raw libc offset 0x2000 instead of libc_base + 0x2000).
    // Our kernel trampoline does the same thing (calls rt_sigreturn).
    #[cfg(not(target_arch = "loongarch64"))]
    {
        // Heuristic: a valid restorer should be above 0x10000 (PIE load base or
        // mmap region). Values below 0x10000 are clearly unrelocated offsets from
        // glibc shared libraries (e.g., raw offset 0x200 instead of libc_base + 0x200).
        let use_restorer = action.restorer != 0
            && action.restorer < USER_ADDR_MAX
            && action.restorer >= 0x10000;
        if use_restorer {
            trap_cx[TrapFrameArgs::RA] = action.restorer;
        } else {
            if action.restorer != 0 && action.restorer < 0x10000 {
                warn!(
                    "[signal] pid={} signum={} restorer={:#x} looks unrelocated, using kernel trampoline",
                    pid, signum, action.restorer
                );
            }
            trap_cx[TrapFrameArgs::RA] =
                arch::SIG_RETURN_ADDR + arch::sigtrx::sigreturn_trampoline_offset();
        }
    }

    // 14. 设置信号栈
    let saved_cx = task_inner.signal_trap_cx.as_ref().unwrap();
    let token = process_inner.memory_set.token();
    let need_siginfo = (action.flags & SA_SIGINFO) != 0 || signum == 32 || signum == 33;

    let (ucontext_ptr, canary_ptr) = setup_signal_stack(
        signum,
        sender_pid,
        si_code,
        trap_cx,
        saved_cx,
        task_inner.signal_mask_backup,
        token,
        need_siginfo,
    );

    task_inner.signal_ucontext_ptr = ucontext_ptr;
    task_inner.signal_canary_ptr = canary_ptr;
    if signum == 32 || signum == 33 {
        let trap_cx = task_inner.get_trap_cx();
        info!(
            "[signal-flow] route=deliver pid={} tid={} sig{} from_process={} handler={:#x} old_pc={:#x} new_pc={:#x} old_sp={:#x} new_sp={:#x} old_ra={:#x} new_ra={:#x} ucontext_ptr={:#x} need_siginfo={} mask_backup={:?} mask_now={:?}",
            pid,
            _tid,
            signum,
            from_process,
            action.handler,
            old_pc,
            trap_cx.sepc,
            old_sp,
            trap_cx[TrapFrameArgs::SP],
            old_ra,
            trap_cx[TrapFrameArgs::RA],
            ucontext_ptr,
            need_siginfo,
            task_inner.signal_mask_backup,
            task_inner.signal_mask
        );
    }
}

pub fn debug_dump_tasks() {
    let count = DEBUG_DUMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    if count % DEBUG_DUMP_INTERVAL != 0 {
        return;
    }
    let processes = pid2process_snapshot();
    let ready = ready_queue_snapshot();
    info!(
        "[debug] process count={} ready_queue={}",
        processes.len(),
        ready.len()
    );
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
                trap_cx[TrapFrameArgs::SP],
                trap_cx[TrapFrameArgs::RA]
            );
        } else {
            info!("[debug] ready[{}] pid={} <busy>", idx, pid);
        }
    }
    for (pid, process) in processes {
        let inner = process.inner_exclusive_access();
        info!(
            "[debug] pid={} name={} zombie={} tasks={} pending={:?}",
            pid,
            inner.name,
            inner.is_zombie,
            inner.tasks.len(),
            inner.signal_pending
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
                    trap_cx[TrapFrameArgs::SP],
                    trap_cx[TrapFrameArgs::RA]
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
    process_inner.clear_pending_signal_siginfo(SIGCONT);
    drop(process_inner);
    block_current_and_run_next();
}
