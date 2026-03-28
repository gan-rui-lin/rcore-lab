#![allow(missing_docs)]

use super::{ProcessControlBlock, TaskControlBlock, TaskStatus};
use crate::sync::UPIntrFreeCell;
use alloc::collections::{BTreeMap, VecDeque};
#[allow(unused_imports)]
use alloc::vec::Vec;
use alloc::sync::Arc;
use arch::TrapFrameArgs;
use lazy_static::*;

pub struct TaskManager {
    ready_queue: VecDeque<Arc<TaskControlBlock>>,
}

impl TaskManager {
    pub fn new() -> Self {
        Self {
            ready_queue: VecDeque::new(),
        }
    }
    pub fn add(&mut self, task: Arc<TaskControlBlock>) {
        self.ready_queue.push_back(task);
    }
    pub fn fetch(&mut self) -> Option<Arc<TaskControlBlock>> {
        self.ready_queue.pop_front()
    }
    pub fn remove(&mut self, task: Arc<TaskControlBlock>) {
        if let Some((id, _)) = self
            .ready_queue
            .iter()
            .enumerate()
            .find(|(_, t)| Arc::as_ptr(t) == Arc::as_ptr(&task))
        {
            self.ready_queue.remove(id);
        }
    }
    #[allow(dead_code)]
    pub fn ready_snapshot(&self) -> Vec<Arc<TaskControlBlock>> {
        self.ready_queue.iter().cloned().collect()
    }

    pub fn ready_len(&self) -> usize {
        self.ready_queue.len()
    }
}

lazy_static! {
    pub static ref TASK_MANAGER: UPIntrFreeCell<TaskManager> =
        unsafe { UPIntrFreeCell::new(TaskManager::new()) };
    pub static ref PID2PCB: UPIntrFreeCell<BTreeMap<usize, Arc<ProcessControlBlock>>> =
        unsafe { UPIntrFreeCell::new(BTreeMap::new()) };
}

pub fn add_task(task: Arc<TaskControlBlock>) {
    TASK_MANAGER.exclusive_access().add(task);
}

pub fn wakeup_task(task: Arc<TaskControlBlock>) {
    let mut task_inner = task.inner_exclusive_access();
    task_inner.task_status = TaskStatus::Ready;
    drop(task_inner);
    add_task(task);
}

pub fn remove_task(task: Arc<TaskControlBlock>) {
    TASK_MANAGER.exclusive_access().remove(task);
}

pub fn fetch_task() -> Option<Arc<TaskControlBlock>> {
    TASK_MANAGER.exclusive_access().fetch()
}

pub fn pid2process(pid: usize) -> Option<Arc<ProcessControlBlock>> {
    let map = PID2PCB.exclusive_access();
    map.get(&pid).map(Arc::clone)
}

pub fn pid2process_snapshot() -> Vec<(usize, Arc<ProcessControlBlock>)> {
    PID2PCB
        .exclusive_access()
        .iter()
        .map(|(pid, pcb)| (*pid, Arc::clone(pcb)))
        .collect()
}

pub fn insert_into_pid2process(pid: usize, process: Arc<ProcessControlBlock>) {
    PID2PCB.exclusive_access().insert(pid, process);
}

pub fn remove_from_pid2process(pid: usize) {
    let mut map = PID2PCB.exclusive_access();
    if map.remove(&pid).is_none() {
        panic!("cannot find pid {} in pid2process!", pid);
    }
}

pub fn ready_queue_snapshot() -> Vec<Arc<TaskControlBlock>> {
    TASK_MANAGER.exclusive_access().ready_snapshot()
}

pub fn ready_queue_len() -> usize {
    TASK_MANAGER.exclusive_access().ready_len()
}

pub fn pid2process_len() -> usize {
    PID2PCB.exclusive_access().len()
}

pub fn pid2process_aggregate(
) -> (
    usize,
    usize,
    usize,
    usize,
    usize,
    usize,
    usize,
    usize,
    usize,
    usize,
    usize,
) {
    let map = PID2PCB.exclusive_access();
    let total_processes = map.len();
    let mut sampled_processes = 0usize;
    let mut skipped_processes = 0usize;
    let mut total_children = 0usize;
    let mut total_tasks = 0usize;
    let total_exited_threads = 0usize;
    let mut max_children = 0usize;
    let mut max_children_pid = 0usize;
    let total_mutex_waiters = 0usize;
    let total_sem_waiters = 0usize;
    let total_cond_waiters = 0usize;
    for (pid, process) in map.iter() {
        if let Some(inner) = process.try_inner_exclusive_access() {
            sampled_processes += 1;
            let child_len = inner.children.len();
            total_children += child_len;
            total_tasks += inner.tasks.iter().filter(|t| t.is_some()).count();
            if child_len > max_children {
                max_children = child_len;
                max_children_pid = *pid;
            }
        } else {
            skipped_processes += 1;
        }
    }
    (
        total_processes,
        sampled_processes,
        skipped_processes,
        total_children,
        total_tasks,
        total_exited_threads,
        max_children_pid,
        max_children,
        total_mutex_waiters,
        total_sem_waiters,
        total_cond_waiters,
    )
}

#[allow(dead_code)]
pub fn pid2process_fdtable_summary() -> (usize, usize, usize) {
    let map = PID2PCB.exclusive_access();
    let mut total_fd_slots = 0usize;
    let mut max_fd_slots = 0usize;
    let mut max_fd_pid = 0usize;
    for (pid, process) in map.iter() {
        if let Some(inner) = process.try_inner_exclusive_access() {
            let fd_len = inner.fd_table.len();
            total_fd_slots += fd_len;
            if fd_len > max_fd_slots {
                max_fd_slots = fd_len;
                max_fd_pid = *pid;
            }
        }
    }
    (total_fd_slots, max_fd_slots, max_fd_pid)
}

#[derive(Clone, Copy, Debug)]
struct TopFdEntry {
    valid: bool,
    pid: usize,
    fd_slots: usize,
    fd_used: usize,
    tasks_alive: usize,
    children: usize,
    is_zombie: bool,
}

impl TopFdEntry {
    const fn empty() -> Self {
        Self {
            valid: false,
            pid: 0,
            fd_slots: 0,
            fd_used: 0,
            tasks_alive: 0,
            children: 0,
            is_zombie: false,
        }
    }
}

/// Print top processes ordered by fd_table length.
/// Intended for alloc_error diagnostics, so it avoids heap allocation.
pub fn print_pid2process_top_by_fd(limit: usize) {
    const MAX_TOP: usize = 8;
    let limit = limit.min(MAX_TOP);
    if limit == 0 {
        return;
    }
    let mut top = [TopFdEntry::empty(); MAX_TOP];
    let map = PID2PCB.exclusive_access();
    for (pid, process) in map.iter() {
        let Some(inner) = process.try_inner_exclusive_access() else {
            continue;
        };
        let fd_slots = inner.fd_table.len();
        let fd_used = inner.fd_table.iter().filter(|fd| fd.is_some()).count();
        let tasks_alive = inner.tasks.iter().filter(|t| t.is_some()).count();
        let children = inner.children.len();
        let is_zombie = inner.is_zombie;
        let candidate = TopFdEntry {
            valid: true,
            pid: *pid,
            fd_slots,
            fd_used,
            tasks_alive,
            children,
            is_zombie,
        };

        // Insertion-sort into fixed-size descending array by fd_slots.
        for idx in 0..limit {
            if !top[idx].valid || candidate.fd_slots > top[idx].fd_slots {
                for j in (idx + 1..limit).rev() {
                    top[j] = top[j - 1];
                }
                top[idx] = candidate;
                break;
            }
        }
    }
    drop(map);

    for (idx, entry) in top.iter().take(limit).enumerate() {
        if !entry.valid {
            break;
        }
        if let Some(process) = pid2process(entry.pid) {
            if let Some(inner) = process.try_inner_exclusive_access() {
                println!(
                    "[kernel] alloc_error top_fd[{}]: pid={} name={} zombie={} tasks_alive={} children={} fd_used={} fd_slots={} sampled=true",
                    idx,
                    entry.pid,
                    inner.name.as_str(),
                    entry.is_zombie,
                    entry.tasks_alive,
                    entry.children,
                    entry.fd_used,
                    entry.fd_slots
                );
            } else {
                println!(
                    "[kernel] alloc_error top_fd[{}]: pid={} name=<busy> zombie={} tasks_alive={} children={} fd_used={} fd_slots={} sampled=false",
                    idx,
                    entry.pid,
                    entry.is_zombie,
                    entry.tasks_alive,
                    entry.children,
                    entry.fd_used,
                    entry.fd_slots
                );
            }
        } else {
            println!(
                "[kernel] alloc_error top_fd[{}]: pid={} name=<gone> zombie={} tasks_alive={} children={} fd_used={} fd_slots={} sampled=false",
                idx,
                entry.pid,
                entry.is_zombie,
                entry.tasks_alive,
                entry.children,
                entry.fd_used,
                entry.fd_slots
            );
        }
    }
}

/// Print brief ready-queue context for alloc_error diagnostics.
pub fn print_ready_queue_brief(limit: usize) {
    let mgr = TASK_MANAGER.exclusive_access();
    for (idx, task) in mgr.ready_queue.iter().take(limit).enumerate() {
        let pid = task.process.upgrade().map(|p| p.getpid()).unwrap_or(0);
        if let Some(task_inner) = task.try_inner_exclusive_access() {
            let tid = task_inner.res.as_ref().map(|r| r.tid).unwrap_or(0);
            let trap_cx = task_inner.get_trap_cx();
            println!(
                "[kernel] alloc_error ready[{}]: pid={} tid={} status={:?} last_syscall={} sepc={:#x} sp={:#x} ra={:#x}",
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
            println!(
                "[kernel] alloc_error ready[{}]: pid={} <task_busy>",
                idx,
                pid
            );
        }
    }
}
