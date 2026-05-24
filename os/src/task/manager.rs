#![allow(missing_docs)]

use super::{ProcessControlBlock, TaskControlBlock, TaskStatus};
use crate::sync::UPIntrRwLock;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::sync::Arc;
#[allow(unused_imports)]
use alloc::vec::Vec;
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
    pub static ref TASK_MANAGER: UPIntrRwLock<TaskManager> =
        unsafe { UPIntrRwLock::new(TaskManager::new()) };
    pub static ref PID2PCB: UPIntrRwLock<BTreeMap<usize, Arc<ProcessControlBlock>>> =
        unsafe { UPIntrRwLock::new(BTreeMap::new()) };
}

fn with_task_manager_read<R>(f: impl FnOnce(&TaskManager) -> R) -> R {
    let manager = TASK_MANAGER.read();
    f(&manager)
}

fn with_task_manager_write<R>(f: impl FnOnce(&mut TaskManager) -> R) -> R {
    let mut manager = TASK_MANAGER.write();
    f(&mut manager)
}

fn with_pid2pcb_read<R>(f: impl FnOnce(&BTreeMap<usize, Arc<ProcessControlBlock>>) -> R) -> R {
    let map = PID2PCB.read();
    f(&map)
}

fn with_pid2pcb_write<R>(f: impl FnOnce(&mut BTreeMap<usize, Arc<ProcessControlBlock>>) -> R) -> R {
    let mut map = PID2PCB.write();
    f(&mut map)
}

pub fn add_task(task: Arc<TaskControlBlock>) {
    with_task_manager_write(|manager| manager.add(task));
}

pub fn wakeup_task(task: Arc<TaskControlBlock>) {
    task.set_status(TaskStatus::Ready);
    add_task(task);
}

pub fn remove_task(task: Arc<TaskControlBlock>) {
    with_task_manager_write(|manager| manager.remove(task));
}

pub fn fetch_task() -> Option<Arc<TaskControlBlock>> {
    with_task_manager_write(|manager| manager.fetch())
}

pub fn pid2process(pid: usize) -> Option<Arc<ProcessControlBlock>> {
    with_pid2pcb_read(|map| map.get(&pid).map(Arc::clone))
}

pub fn pid2process_snapshot() -> Vec<(usize, Arc<ProcessControlBlock>)> {
    with_pid2pcb_read(|map| {
        map.iter()
            .map(|(pid, pcb)| (*pid, Arc::clone(pcb)))
            .collect()
    })
}

pub fn insert_into_pid2process(pid: usize, process: Arc<ProcessControlBlock>) {
    with_pid2pcb_write(|map| {
        map.insert(pid, process);
    });
}

pub fn remove_from_pid2process(pid: usize) {
    let removed = with_pid2pcb_write(|map| map.remove(&pid));
    if removed.is_none() {
        panic!("cannot find pid {} in pid2process!", pid);
    }
}

pub fn ready_queue_snapshot() -> Vec<Arc<TaskControlBlock>> {
    with_task_manager_read(|manager| manager.ready_snapshot())
}

pub fn ready_queue_len() -> usize {
    with_task_manager_read(|manager| manager.ready_len())
}

pub fn pid2process_len() -> usize {
    with_pid2pcb_read(|map| map.len())
}

pub fn pid2process_aggregate() -> (
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
    let map = PID2PCB.read();
    let total_processes = map.len();
    let mut sampled_processes = 0usize;
    let skipped_processes = 0usize;
    let mut total_children = 0usize;
    let mut total_tasks = 0usize;
    let total_exited_threads = 0usize;
    let mut max_children = 0usize;
    let mut max_children_pid = 0usize;
    let total_mutex_waiters = 0usize;
    let total_sem_waiters = 0usize;
    let total_cond_waiters = 0usize;
    for (pid, process) in map.iter() {
        sampled_processes += 1;
        let child_len = process.child_count();
        total_children += child_len;
        total_tasks += process.thread_count();
        if child_len > max_children {
            max_children = child_len;
            max_children_pid = *pid;
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
    let map = PID2PCB.read();
    let mut total_fd_slots = 0usize;
    let mut max_fd_slots = 0usize;
    let mut max_fd_pid = 0usize;
    for (pid, process) in map.iter() {
        let fd_len = process.with_fs(|fs| fs.fd_table.len());
        total_fd_slots += fd_len;
        if fd_len > max_fd_slots {
            max_fd_slots = fd_len;
            max_fd_pid = *pid;
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
    let map = PID2PCB.read();
    for (pid, process) in map.iter() {
        let (fd_slots, fd_used) = process.with_fs(|fs| {
            (
                fs.fd_table.len(),
                fs.fd_table.iter().filter(|fd| fd.is_some()).count(),
            )
        });
        let tasks_alive = process.thread_count();
        let children = process.child_count();
        let is_zombie = process.is_zombie();
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
            if process.try_with_sync_objects_mut(|_| ()).is_some() {
                println!(
                    "[kernel] alloc_error top_fd[{}]: pid={} name={} zombie={} tasks_alive={} children={} fd_used={} fd_slots={} sampled=true",
                    idx,
                    entry.pid,
                    process.name().as_str(),
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
    let mgr = TASK_MANAGER.read();
    for (idx, task) in mgr.ready_queue.iter().take(limit).enumerate() {
        let pid = task.process.upgrade().map(|p| p.getpid()).unwrap_or(0);
        if let Some(snapshot) = task.try_debug_snapshot() {
            println!(
                "[kernel] alloc_error ready[{}]: pid={} tid={} status={:?} last_syscall={} sepc={:#x} sp={:#x} ra={:#x}",
                idx,
                pid,
                snapshot.tid,
                snapshot.status,
                snapshot.last_syscall,
                snapshot.sepc,
                snapshot.sp,
                snapshot.ra
            );
        } else {
            println!(
                "[kernel] alloc_error ready[{}]: pid={} <task_busy>",
                idx, pid
            );
        }
    }
}
