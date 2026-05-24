use super::id::TaskUserRes;
use super::{kstack_alloc, KernelStack, ProcessControlBlock, SignalFlags, TaskContext};
use crate::mm::PhysPageNum;
use crate::sync::UPIntrMutex;
use alloc::collections::BTreeMap;
use alloc::sync::{Arc, Weak};
use arch::TrapContext;
use core::sync::atomic::{AtomicUsize, Ordering};

pub fn live_task_count() -> usize {
    super::pid2process_snapshot()
        .iter()
        .map(|(_, process)| process.thread_count())
        .sum()
}

pub fn live_task_pid_summary() -> (usize, usize, usize, usize, usize, usize) {
    let mut pid_counts: BTreeMap<usize, usize> = BTreeMap::new();
    for (pid, process) in super::pid2process_snapshot() {
        let count = process.thread_count();
        if count > 0 {
            pid_counts.insert(pid, count);
        }
    }
    if pid_counts.is_empty() {
        return (0, 0, 0, 0, 0, 0);
    }
    let total: usize = pid_counts.values().sum();
    let unique_pids = pid_counts.len();
    let mut top_pid = 0usize;
    let mut top_count = 0usize;
    let mut min_pid = usize::MAX;
    let mut max_pid = 0usize;
    for (pid, count) in pid_counts.iter() {
        if *count > top_count {
            top_count = *count;
            top_pid = *pid;
        }
        if *pid < min_pid {
            min_pid = *pid;
        }
        if *pid > max_pid {
            max_pid = *pid;
        }
    }
    (total, unique_pids, top_pid, top_count, min_pid, max_pid)
}

pub struct TaskControlBlock {
    pub process: Weak<ProcessControlBlock>,
    pub kstack: KernelStack,
    user: UPIntrMutex<TaskUserState>,
    sched: UPIntrMutex<TaskSchedState>,
    signals: UPIntrMutex<TaskSignalState>,
    last_syscall: AtomicUsize,
    sigcancel_last_pc: AtomicUsize,
    sigcancel_loop_count: AtomicUsize,
    illegal_last_sepc: AtomicUsize,
    illegal_repeat_count: AtomicUsize,
}

impl TaskControlBlock {
    pub fn get_user_token(&self) -> usize {
        let process = self.process.upgrade().unwrap();
        process.get_user_token()
    }

    pub fn tid(&self) -> usize {
        self.user
            .lock()
            .res
            .as_ref()
            .map(|res| res.tid)
            .unwrap_or(0)
    }

    pub fn user_res_snapshot(&self) -> Option<TaskUserResSnapshot> {
        self.user.lock().res.as_ref().map(TaskUserResSnapshot::from)
    }

    pub fn ustack_base(&self) -> usize {
        self.user
            .lock()
            .res
            .as_ref()
            .map(|res| res.ustack_base())
            .unwrap_or(0)
    }

    pub fn ustack_top(&self) -> usize {
        self.user
            .lock()
            .res
            .as_ref()
            .map(|res| res.ustack_top())
            .unwrap_or(0)
    }

    pub fn trap_cx_user_va(&self) -> usize {
        self.user
            .lock()
            .res
            .as_ref()
            .map(|res| res.trap_cx_user_va())
            .unwrap_or(0)
    }

    pub fn trap_cx(&self) -> &'static mut TrapContext {
        let ppn = self.user.lock().trap_cx_ppn;
        ppn.get_mut()
    }

    pub fn with_trap_cx_mut<R>(&self, f: impl FnOnce(&mut TrapContext) -> R) -> R {
        let ppn = self.user.lock().trap_cx_ppn;
        f(ppn.get_mut())
    }

    pub fn set_trap_cx_ppn_from_res(&self) {
        let mut user = self.user.lock();
        user.trap_cx_ppn = user.res.as_ref().unwrap().trap_cx_ppn();
    }

    pub fn take_user_res(&self) -> Option<TaskUserRes> {
        self.user.lock().res.take()
    }

    pub fn set_ustack_base_and_refresh_trap_cx_ppn(&self, ustack_base: usize) {
        let mut user = self.user.lock();
        if let Some(res) = user.res.as_mut() {
            res.ustack_base = ustack_base;
        }
        user.trap_cx_ppn = user.res.as_ref().unwrap().trap_cx_ppn();
    }

    pub fn status(&self) -> TaskStatus {
        self.sched.lock().task_status
    }

    pub fn set_status(&self, status: TaskStatus) {
        self.sched.lock().task_status = status;
    }

    pub fn exit_code(&self) -> Option<i32> {
        self.sched.lock().exit_code
    }

    pub fn set_exit_code(&self, exit_code: Option<i32>) {
        self.sched.lock().exit_code = exit_code;
    }

    pub fn task_cx_ptr_mut_for_switch(&self, status: TaskStatus) -> *mut TaskContext {
        let mut sched = self.sched.lock();
        sched.task_status = status;
        &mut sched.task_cx as *mut TaskContext
    }

    pub fn task_cx_ptr_for_switch(&self, status: TaskStatus) -> *const TaskContext {
        let mut sched = self.sched.lock();
        sched.task_status = status;
        &sched.task_cx as *const TaskContext
    }

    pub fn with_sched<R>(&self, f: impl FnOnce(&TaskSchedState) -> R) -> R {
        let sched = self.sched.lock();
        f(&sched)
    }

    pub fn with_sched_mut<R>(&self, f: impl FnOnce(&mut TaskSchedState) -> R) -> R {
        let mut sched = self.sched.lock();
        f(&mut sched)
    }

    pub fn signal_mask(&self) -> SignalFlags {
        self.signals.lock().signal_mask
    }

    pub fn set_signal_mask(&self, mask: SignalFlags) {
        self.signals.lock().signal_mask = mask;
    }

    pub fn update_signal_mask(&self, f: impl FnOnce(&mut SignalFlags)) {
        let mut signals = self.signals.lock();
        f(&mut signals.signal_mask);
        signals
            .signal_mask
            .remove(SignalFlags::SIGKILL | SignalFlags::SIGSTOP);
    }

    pub fn pending_signal(&self) -> SignalFlags {
        self.signals.lock().signal_pending
    }

    pub fn insert_pending_signal(&self, signal: SignalFlags) {
        self.signals.lock().signal_pending.insert(signal);
    }

    pub fn remove_pending_signal(&self, signal: SignalFlags) {
        self.signals.lock().signal_pending.remove(signal);
    }

    pub fn mark_interrupted(&self) {
        self.signals.lock().interrupted_by_signal = true;
    }

    pub fn take_interrupted(&self) -> bool {
        let mut signals = self.signals.lock();
        let interrupted = signals.interrupted_by_signal;
        signals.interrupted_by_signal = false;
        interrupted
    }

    pub fn set_interrupted(&self, interrupted: bool) {
        self.signals.lock().interrupted_by_signal = interrupted;
    }

    pub fn interrupted_by_signal(&self) -> bool {
        self.signals.lock().interrupted_by_signal
    }

    pub fn handling_sig(&self) -> isize {
        self.signals.lock().handling_sig
    }

    pub fn set_handling_sig(&self, signum: isize) {
        self.signals.lock().handling_sig = signum;
    }

    pub fn clear_child_tid(&self) -> usize {
        self.signals.lock().clear_child_tid
    }

    pub fn set_clear_child_tid(&self, clear_child_tid: usize) {
        self.signals.lock().clear_child_tid = clear_child_tid;
    }

    pub fn save_signal_frame(&self) -> bool {
        let saved_cx = *self.trap_cx();
        let mut signals = self.signals.lock();
        if signals.signal_trap_cx.is_some() {
            return false;
        }
        signals.signal_trap_cx = Some(saved_cx);
        signals.signal_mask_backup = signals.signal_mask;
        true
    }

    pub fn take_signal_frame(&self) -> Option<TrapContext> {
        self.signals.lock().signal_trap_cx.take()
    }

    pub fn with_signals<R>(&self, f: impl FnOnce(&TaskSignalState) -> R) -> R {
        let signals = self.signals.lock();
        f(&signals)
    }

    pub fn with_signals_mut<R>(&self, f: impl FnOnce(&mut TaskSignalState) -> R) -> R {
        let mut signals = self.signals.lock();
        f(&mut signals)
    }

    pub fn try_debug_snapshot(&self) -> Option<TaskDebugSnapshot> {
        let user = self.user.try_lock()?;
        let sched = self.sched.try_lock()?;
        let trap_cx: &mut TrapContext = user.trap_cx_ppn.get_mut();
        Some(TaskDebugSnapshot {
            tid: user.res.as_ref().map(|r| r.tid).unwrap_or(0),
            status: sched.task_status,
            exit_code: sched.exit_code,
            last_syscall: self.last_syscall(),
            sepc: trap_cx.sepc,
            sp: trap_cx[arch::TrapFrameArgs::SP],
            ra: trap_cx[arch::TrapFrameArgs::RA],
        })
    }

    pub fn last_syscall(&self) -> usize {
        self.last_syscall.load(Ordering::Relaxed)
    }

    pub fn set_last_syscall(&self, syscall_id: usize) {
        self.last_syscall.store(syscall_id, Ordering::Relaxed);
    }

    pub fn record_illegal_instruction(&self, sepc: usize) -> usize {
        if self.illegal_last_sepc.load(Ordering::Relaxed) == sepc {
            self.illegal_repeat_count
                .fetch_add(1, Ordering::Relaxed)
                .saturating_add(1)
        } else {
            self.illegal_last_sepc.store(sepc, Ordering::Relaxed);
            self.illegal_repeat_count.store(1, Ordering::Relaxed);
            1
        }
    }

    pub fn sigcancel_last_pc(&self) -> usize {
        self.sigcancel_last_pc.load(Ordering::Relaxed)
    }

    pub fn set_sigcancel_last_pc(&self, pc: usize) {
        self.sigcancel_last_pc.store(pc, Ordering::Relaxed);
    }

    pub fn sigcancel_loop_count(&self) -> usize {
        self.sigcancel_loop_count.load(Ordering::Relaxed)
    }

    pub fn set_sigcancel_loop_count(&self, count: usize) {
        self.sigcancel_loop_count.store(count, Ordering::Relaxed);
    }
}

pub struct TaskUserState {
    pub res: Option<TaskUserRes>,
    pub trap_cx_ppn: PhysPageNum,
}

#[derive(Clone, Copy)]
pub struct TaskUserResSnapshot {
    pub tid: usize,
    pub ustack_base: usize,
    pub ustack_top: usize,
    pub trap_cx_user_va: usize,
}

impl From<&TaskUserRes> for TaskUserResSnapshot {
    fn from(res: &TaskUserRes) -> Self {
        Self {
            tid: res.tid,
            ustack_base: res.ustack_base(),
            ustack_top: res.ustack_top(),
            trap_cx_user_va: res.trap_cx_user_va(),
        }
    }
}

pub struct TaskSchedState {
    pub task_cx: TaskContext,
    pub task_status: TaskStatus,
    pub exit_code: Option<i32>,
}

pub struct TaskSignalState {
    pub signal_trap_cx: Option<TrapContext>,
    /// 当前线程的信号掩码（Linux 中 signal_mask 是 per-thread 的）
    pub signal_mask: SignalFlags,
    pub signal_mask_backup: SignalFlags,
    pub signal_pending: SignalFlags,
    pub signal_ucontext_ptr: usize,
    pub signal_canary_ptr: usize,
    pub clear_child_tid: usize,
    pub interrupted_by_signal: bool,
    /// 当前正在处理的信号编号（-1 表示未处理信号，用于防止信号重入）
    pub handling_sig: isize,
}

pub struct TaskDebugSnapshot {
    pub tid: usize,
    pub status: TaskStatus,
    pub exit_code: Option<i32>,
    pub last_syscall: usize,
    pub sepc: usize,
    pub sp: usize,
    pub ra: usize,
}

impl TaskControlBlock {
    pub fn new(
        process: Arc<ProcessControlBlock>,
        ustack_base: usize,
        alloc_user_res: bool,
    ) -> Self {
        let res = TaskUserRes::new(Arc::clone(&process), ustack_base, alloc_user_res);
        let trap_cx_ppn = res.trap_cx_ppn();
        let kstack = kstack_alloc();
        let kstack_top = kstack.get_top();
        Self {
            process: Arc::downgrade(&process),
            kstack,
            user: unsafe {
                UPIntrMutex::new(TaskUserState {
                    res: Some(res),
                    trap_cx_ppn,
                })
            },
            sched: unsafe {
                UPIntrMutex::new(TaskSchedState {
                    task_cx: TaskContext::for_user_trap_loop(
                        kstack_top,
                        arch::kernel_text_addr(crate::trap::user_trap_loop as usize),
                    ),
                    task_status: TaskStatus::Ready,
                    exit_code: None,
                })
            },
            signals: unsafe {
                UPIntrMutex::new(TaskSignalState {
                    signal_trap_cx: None,
                    signal_mask: SignalFlags::empty(),
                    signal_mask_backup: SignalFlags::empty(),
                    signal_pending: SignalFlags::empty(),
                    signal_ucontext_ptr: 0,
                    signal_canary_ptr: 0,
                    clear_child_tid: 0,
                    interrupted_by_signal: false,
                    handling_sig: -1, // -1 表示未处理信号
                })
            },
            last_syscall: AtomicUsize::new(0),
            sigcancel_last_pc: AtomicUsize::new(0),
            sigcancel_loop_count: AtomicUsize::new(0),
            illegal_last_sepc: AtomicUsize::new(0),
            illegal_repeat_count: AtomicUsize::new(0),
        }
    }
}

#[derive(Copy, Clone, PartialEq, Debug)]
pub enum TaskStatus {
    Ready,
    Running,
    Blocked,
}
