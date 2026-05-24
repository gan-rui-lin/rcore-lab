//! Timer management: hardware timer + task scheduling queue.

#![allow(missing_docs)]

pub use arch::{get_time, get_time_ms, get_time_us, set_next_trigger};

use crate::sync::UPIntrFreeCell;
use crate::task::{wakeup_task, TaskControlBlock};
use alloc::collections::BinaryHeap;
use alloc::sync::Arc;
use core::cmp::Ordering;
use core::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use lazy_static::lazy_static;

const TIMER_DEBUG_INTERVAL: u64 = 500;
static CHECK_TIMER_COUNT: AtomicU64 = AtomicU64::new(0);

pub struct TimerCondVar {
    pub expire_ms: usize,
    pub task: Arc<TaskControlBlock>,
}

impl PartialEq for TimerCondVar {
    fn eq(&self, other: &Self) -> bool {
        self.expire_ms == other.expire_ms
    }
}

impl Eq for TimerCondVar {}

impl PartialOrd for TimerCondVar {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        let a = -(self.expire_ms as isize);
        let b = -(other.expire_ms as isize);
        Some(a.cmp(&b))
    }
}

impl Ord for TimerCondVar {
    fn cmp(&self, other: &Self) -> Ordering {
        self.partial_cmp(other).unwrap()
    }
}

lazy_static! {
    static ref TIMERS: UPIntrFreeCell<BinaryHeap<TimerCondVar>> =
        unsafe { UPIntrFreeCell::new(BinaryHeap::<TimerCondVar>::new()) };
}

pub fn add_timer(expire_ms: usize, task: Arc<TaskControlBlock>) {
    let mut timers = TIMERS.exclusive_access();
    timers.push(TimerCondVar { expire_ms, task });
}

pub fn remove_timer(task: Arc<TaskControlBlock>) {
    let mut timers = TIMERS.exclusive_access();
    let mut temp = BinaryHeap::<TimerCondVar>::new();
    for condvar in timers.drain() {
        if Arc::as_ptr(&task) != Arc::as_ptr(&condvar.task) {
            temp.push(condvar);
        }
    }
    timers.clear();
    timers.append(&mut temp);
}

pub fn check_timer() {
    let count = CHECK_TIMER_COUNT.fetch_add(1, AtomicOrdering::Relaxed) + 1;
    if count % TIMER_DEBUG_INTERVAL == 0 {
        info!(
            "[timer] check_timer count={} time_ms={}",
            count,
            get_time_ms()
        );
    }
    let current_ms = get_time_ms();
    let mut timers = TIMERS.exclusive_access();
    while let Some(timer) = timers.peek() {
        if timer.expire_ms <= current_ms {
            wakeup_task(Arc::clone(&timer.task));
            timers.pop();
        } else {
            break;
        }
    }
    drop(timers);
    // Poll network stack from timer interrupt so loopback TCP packets
    // get delivered even when all user processes are blocked in recv().
    crate::net::poll_net_if_available();
    // Check ITIMER_REAL for all processes
    check_itimers(current_ms);
}

/// Check and fire ITIMER_REAL timers across all processes.
fn check_itimers(current_ms: usize) {
    use crate::task::{pid2process_snapshot, wakeup_task, SignalFlags, TaskStatus};
    let procs = pid2process_snapshot();
    for (_pid, process) in procs {
        // Use try_inner_exclusive_access to avoid deadlocking if the timer
        // interrupt fires while someone holds this process's inner lock.
        let mut inner = match process.try_inner_exclusive_access() {
            Some(inner) => inner,
            None => continue,
        };
        let expire = inner.itimer_real_expire_ms;
        if expire != 0 && expire <= current_ms {
            // Fire SIGALRM
            log::warn!(
                "[itimer] pid={} SIGALRM fired, expire={} now={}",
                _pid,
                expire,
                current_ms
            );
            // Reload interval or disarm
            if inner.itimer_real_interval_ms > 0 {
                inner.itimer_real_expire_ms = current_ms + inner.itimer_real_interval_ms;
            } else {
                inner.itimer_real_expire_ms = 0;
            }
            drop(inner);
            process.insert_process_signal(SignalFlags::SIGALRM, 0, 0);

            // Wake up any blocked tasks in this process so they can handle the signal
            for task in process.tasks_snapshot() {
                if task.status() == TaskStatus::Blocked {
                    task.mark_interrupted();
                    task.set_status(TaskStatus::Ready);
                    wakeup_task(task.clone());
                    log::info!("[itimer] pid={} woke blocked task", _pid);
                }
            }
        }
    }
}

pub fn timer_len() -> usize {
    TIMERS.exclusive_access().len()
}
