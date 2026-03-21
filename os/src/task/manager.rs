#![allow(missing_docs)]

use super::{ProcessControlBlock, TaskControlBlock, TaskStatus};
use crate::sync::UPIntrFreeCell;
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
