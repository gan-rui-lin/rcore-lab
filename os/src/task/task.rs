use super::id::TaskUserRes;
use super::{KernelStack, ProcessControlBlock, TaskContext, kstack_alloc};
use arch::TrapContext;
use crate::sync::UPIntrFreeCell;
use alloc::collections::BTreeMap;
use alloc::sync::{Arc, Weak};
use crate::sync::UPIntrRefMut;

pub fn live_task_count() -> usize {
    super::pid2process_snapshot()
        .iter()
        .filter_map(|(_, process)| process.try_inner_exclusive_access())
        .map(|inner| inner.tasks.iter().filter(|t| t.is_some()).count())
        .sum()
}

pub fn live_task_pid_summary() -> (usize, usize, usize, usize, usize, usize) {
    let mut pid_counts: BTreeMap<usize, usize> = BTreeMap::new();
    for (pid, process) in super::pid2process_snapshot() {
        if let Some(inner) = process.try_inner_exclusive_access() {
            let count = inner.tasks.iter().filter(|t| t.is_some()).count();
            if count > 0 {
                pid_counts.insert(pid, count);
            }
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
    inner: UPIntrFreeCell<TaskControlBlockInner>,
}

impl TaskControlBlock {
    pub fn inner_exclusive_access(&self) -> UPIntrRefMut<'_, TaskControlBlockInner> {
        self.inner.exclusive_access()
    }

    /// Try to borrow the task inner state; returns None if already borrowed.
    pub fn try_inner_exclusive_access(&self) -> Option<UPIntrRefMut<'_, TaskControlBlockInner>> {
        self.inner.try_exclusive_access()
    }

    pub fn get_user_token(&self) -> usize {
        let process = self.process.upgrade().unwrap();
        let inner = process.inner_exclusive_access();
        inner.memory_set.token()
    }
}

pub struct TaskControlBlockInner {
    pub res: Option<TaskUserRes>,
    pub trap_cx_ppn: crate::mm::PhysPageNum,
    pub task_cx: TaskContext,
    pub task_status: TaskStatus,
    pub exit_code: Option<i32>,
    pub signal_trap_cx: Option<TrapContext>,
    /// 当前线程的信号掩码（Linux 中 signal_mask 是 per-thread 的）
    pub signal_mask: super::SignalFlags,
    pub signal_mask_backup: super::SignalFlags,
    pub signal_pending: super::SignalFlags,
    pub signal_ucontext_ptr: usize,
    pub clear_child_tid: usize,
    pub interrupted_by_signal: bool,
    /// 当前正在处理的信号编号（-1 表示未处理信号，用于防止信号重入）
    pub handling_sig: isize,
    // for debug
    pub last_syscall: usize,
    // SIGCANCEL loop detection to prevent pthread_cancel hanging
    pub sigcancel_last_pc: usize,
    pub sigcancel_loop_count: usize,
}

impl TaskControlBlockInner {
    pub fn get_trap_cx(&self) -> &'static mut TrapContext {
        self.trap_cx_ppn.get_mut()
    }

    #[allow(unused)]
    fn get_status(&self) -> TaskStatus {
        self.task_status
    }
}

impl TaskControlBlock {
    pub fn new(process: Arc<ProcessControlBlock>, ustack_base: usize, alloc_user_res: bool) -> Self {
        let res = TaskUserRes::new(Arc::clone(&process), ustack_base, alloc_user_res);
        let trap_cx_ppn = res.trap_cx_ppn();
        let kstack = kstack_alloc();
        let kstack_top = kstack.get_top();
        Self {
            process: Arc::downgrade(&process),
            kstack,
            inner: unsafe {
                UPIntrFreeCell::new(TaskControlBlockInner {
                    res: Some(res),
                    trap_cx_ppn,
                    task_cx: TaskContext::goto_trap_return(kstack_top, {
                        #[cfg(target_arch = "riscv64")]
                        { arch::kernel_text_addr(crate::trap::do_trap_return as usize) }
                        #[cfg(target_arch = "loongarch64")]
                        { crate::trap::task_entry as usize }
                    }),
                    task_status: TaskStatus::Ready,
                    exit_code: None,
                    signal_trap_cx: None,
                    signal_mask: super::SignalFlags::empty(),
                    signal_mask_backup: super::SignalFlags::empty(),
                    signal_pending: super::SignalFlags::empty(),
                    signal_ucontext_ptr: 0,
                    clear_child_tid: 0,
                    interrupted_by_signal: false,
                    handling_sig: -1, // -1 表示未处理信号
                    last_syscall: 0,
                    sigcancel_last_pc: 0,
                    sigcancel_loop_count: 0,
                })
            },
        }
    }
}

#[derive(Copy, Clone, PartialEq, Debug)]
pub enum TaskStatus {
    Ready,
    Running,
    Blocked,
}
