use super::id::TaskUserRes;
use super::{KernelStack, ProcessControlBlock, TaskContext, kstack_alloc};
use crate::trap::TrapContext;
use crate::sync::UPIntrFreeCell;
use alloc::sync::{Arc, Weak};
use crate::sync::UPIntrRefMut;

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
    pub signal_mask_backup: super::SignalFlags,
    pub clear_child_tid: usize,
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
                    task_cx: TaskContext::goto_trap_return(kstack_top),
                    task_status: TaskStatus::Ready,
                    exit_code: None,
                    signal_trap_cx: None,
                    signal_mask_backup: super::SignalFlags::empty(),
                    clear_child_tid: 0,
                })
            },
        }
    }
}

#[derive(Copy, Clone, PartialEq)]
pub enum TaskStatus {
    Ready,
    Running,
    Blocked,
}
