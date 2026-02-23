use alloc::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Weak},
    // vec::Vec,
};
use lazy_static::lazy_static;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::mm::PhysAddr;
use crate::task::{block_current_and_run_next, current_task, wakeup_task, TaskControlBlock};

const FUTEX_BITSET_MATCH_ANY: i32 = -1;
const FUTEX_TRACE_INTERVAL: u64 = 2000;
static FUTEX_WAIT_COUNTER: AtomicU64 = AtomicU64::new(0);
static FUTEX_WAKE_COUNTER: AtomicU64 = AtomicU64::new(0);
static FUTEX_REQUEUE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct FutexKey {
    paddr: PhysAddr,
    pid: usize,
}

impl FutexKey {
    pub fn new(paddr: PhysAddr, pid: usize) -> Self {
        Self { paddr, pid }
    }
}

struct FutexWaiter {
    task: Weak<TaskControlBlock>,
    bitset: i32,
}

type FutexBucket = VecDeque<FutexWaiter>;

lazy_static! {
    static ref FUTEX_Q: spin::Mutex<BTreeMap<FutexKey, FutexBucket>> =
        spin::Mutex::new(BTreeMap::new());
}

pub fn futex_wait(futex_key: FutexKey) -> isize {
    futex_wait_bitset(futex_key, FUTEX_BITSET_MATCH_ANY)
}

pub fn futex_wait_bitset(futex_key: FutexKey, bitset: i32) -> isize {
    let task = current_task().unwrap();
    let mut futex_q = FUTEX_Q.lock();
    let queue = futex_q
        .entry(futex_key.clone())
        .or_insert_with(VecDeque::new);
    queue.push_back(FutexWaiter {
            task: Arc::downgrade(&task),
            bitset,
        });
    let queue_len = queue.len();
    drop(futex_q);
    let count = FUTEX_WAIT_COUNTER.fetch_add(1, Ordering::Relaxed);
    if count % FUTEX_TRACE_INTERVAL == 0 {
        let (pid, tid) = task
            .process
            .upgrade()
            .map(|p| (p.pid.0, task.inner_exclusive_access().res.as_ref().map(|r| r.tid).unwrap_or(0)))
            .unwrap_or((0, 0));
        info!(
            "[futex] wait pid={} tid={} key=({:#x},{}) bitset={:#x} qlen={}",
            pid,
            tid,
            futex_key.paddr.0,
            futex_key.pid,
            bitset,
            queue_len
        );
    }
    drop(task);
    block_current_and_run_next();
    0
}

pub fn futex_wake(futex_key: FutexKey, max_size: usize) -> usize {
    futex_wake_bitset(futex_key, max_size, FUTEX_BITSET_MATCH_ANY)
}

pub fn futex_wake_bitset(futex_key: FutexKey, max_size: usize, bitset: i32) -> usize {
    let mut futex_q = FUTEX_Q.lock();
    let mut num = 0usize;
    let before_len = futex_q.get(&futex_key).map(|q| q.len()).unwrap_or(0);
    if let Some(queue) = futex_q.get_mut(&futex_key) {
        let mut i = 0usize;
        while i < queue.len() && num < max_size {
            let matches = queue[i].bitset & bitset != 0;
            if matches {
                if let Some(waiter) = queue.remove(i) {
                    if let Some(task) = waiter.task.upgrade() {
                        wakeup_task(task);
                        num += 1;
                    }
                }
            } else {
                i += 1;
            }
        }
        if queue.is_empty() {
            futex_q.remove(&futex_key);
        }
    }
    let count = FUTEX_WAKE_COUNTER.fetch_add(1, Ordering::Relaxed);
    if count % FUTEX_TRACE_INTERVAL == 0 {
        let (pid, tid) = current_task()
            .and_then(|task| task.process.upgrade().map(|p| {
                let tid = task.inner_exclusive_access().res.as_ref().map(|r| r.tid).unwrap_or(0);
                (p.pid.0, tid)
            }))
            .unwrap_or((0, 0));
        info!(
            "[futex] wake pid={} tid={} key=({:#x},{}) max={} bitset={:#x} woke={} qlen_before={}",
            pid,
            tid,
            futex_key.paddr.0,
            futex_key.pid,
            max_size,
            bitset,
            num,
            before_len
        );
    }
    num
}

pub fn futex_requeue(
    old_key: FutexKey,
    max_wake: i32,
    new_key: FutexKey,
    max_requeue: i32,
) -> usize {
    let mut futex_q = FUTEX_Q.lock();
    let mut woke = 0usize;
    let mut moved = 0usize;
    let mut requeued = VecDeque::new();
    if let Some(queue) = futex_q.get_mut(&old_key) {
        while let Some(waiter) = queue.pop_front() {
            if woke < max_wake as usize {
                if let Some(task) = waiter.task.upgrade() {
                    wakeup_task(task);
                    woke += 1;
                }
                continue;
            }
            if moved < max_requeue as usize {
                requeued.push_back(waiter);
                moved += 1;
            } else {
                queue.push_front(waiter);
                break;
            }
        }
        if queue.is_empty() {
            futex_q.remove(&old_key);
        }
    }
    if !requeued.is_empty() {
        futex_q
            .entry(new_key.clone())
            .or_insert_with(VecDeque::new)
            .extend(requeued);
    }
    let count = FUTEX_REQUEUE_COUNTER.fetch_add(1, Ordering::Relaxed);
    if count % FUTEX_TRACE_INTERVAL == 0 {
        let (pid, tid) = current_task()
            .and_then(|task| task.process.upgrade().map(|p| {
                let tid = task.inner_exclusive_access().res.as_ref().map(|r| r.tid).unwrap_or(0);
                (p.pid.0, tid)
            }))
            .unwrap_or((0, 0));
        info!(
            "[futex] requeue pid={} tid={} old=({:#x},{}) new=({:#x},{}) max_wake={} max_requeue={} woke={} moved={}",
            pid,
            tid,
            old_key.paddr.0,
            old_key.pid,
            new_key.paddr.0,
            new_key.pid.clone(),
            max_wake,
            max_requeue,
            woke,
            moved
        );
    }
    woke
}

pub fn futex_remove_waiter(futex_key: &FutexKey, task: &Arc<TaskControlBlock>) -> bool {
    let mut futex_q = FUTEX_Q.lock();
    if let Some(queue) = futex_q.get_mut(futex_key) {
        let mut i = 0usize;
        while i < queue.len() {
            let remove_entry = match queue[i].task.upgrade() {
                Some(waiter_task) => Arc::as_ptr(&waiter_task) == Arc::as_ptr(task),
                None => true,
            };
            if remove_entry {
                queue.remove(i);
                if queue.is_empty() {
                    futex_q.remove(futex_key);
                }
                return true;
            }
            i += 1;
        }
    }
    false
}

pub fn futex_remove_waiter_any(task: &Arc<TaskControlBlock>) -> bool {
    let mut futex_q = FUTEX_Q.lock();
    let mut empty_keys = alloc::vec::Vec::new();
    for (key, queue) in futex_q.iter_mut() {
        let mut i = 0usize;
        while i < queue.len() {
            let remove_entry = match queue[i].task.upgrade() {
                Some(waiter_task) => Arc::as_ptr(&waiter_task) == Arc::as_ptr(task),
                None => true,
            };
            if remove_entry {
                queue.remove(i);
                if queue.is_empty() {
                    empty_keys.push(key.clone());
                }
                return true;
            }
            i += 1;
        }
    }
    for key in empty_keys {
        futex_q.remove(&key);
    }
    false
}
