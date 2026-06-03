//! Timer management: hardware timer + task scheduling queue.

#![allow(missing_docs)]

pub use arch::{get_time, get_time_ms, get_time_us, set_next_trigger};

use crate::sync::UPIntrRwLock;
use crate::task::{wakeup_task, TaskControlBlock};
use alloc::collections::BinaryHeap;
use alloc::sync::Arc;
use alloc::vec::Vec;
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
    static ref TIMERS: UPIntrRwLock<BinaryHeap<TimerCondVar>> =
        unsafe { UPIntrRwLock::new(BinaryHeap::<TimerCondVar>::new()) };
}

fn with_timers_read<R>(f: impl FnOnce(&BinaryHeap<TimerCondVar>) -> R) -> R {
    let timers = TIMERS.read();
    f(&timers)
}

fn with_timers_write<R>(f: impl FnOnce(&mut BinaryHeap<TimerCondVar>) -> R) -> R {
    let mut timers = TIMERS.write();
    f(&mut timers)
}

pub fn add_timer(expire_ms: usize, task: Arc<TaskControlBlock>) {
    with_timers_write(|timers| {
        timers.push(TimerCondVar { expire_ms, task });
    });
}

pub fn remove_timer(task: Arc<TaskControlBlock>) {
    with_timers_write(|timers| {
        let mut temp = BinaryHeap::<TimerCondVar>::new();
        for condvar in timers.drain() {
            if Arc::as_ptr(&task) != Arc::as_ptr(&condvar.task) {
                temp.push(condvar);
            }
        }
        timers.clear();
        timers.append(&mut temp);
    });
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
    let mut expired = Vec::new();
    with_timers_write(|timers| {
        while let Some(timer) = timers.peek() {
            if timer.expire_ms <= current_ms {
                expired.push(Arc::clone(&timer.task));
                timers.pop();
            } else {
                break;
            }
        }
    });
    for task in expired {
        wakeup_task(task);
    }
    // Poll network stack from timer interrupt so loopback TCP packets
    // get delivered even when all user processes are blocked in recv().
    crate::net::poll_net_if_available();
    // Check ITIMER_REAL for all processes
    check_itimers(current_ms);
    // Check POSIX timers (timer_create/timer_settime)
    check_posix_timers(current_ms);
}

/// Check and fire ITIMER_REAL timers across all processes.
fn check_itimers(current_ms: usize) {
    use crate::task::{pid2process_snapshot, wakeup_task, SignalFlags, TaskStatus};
    let procs = pid2process_snapshot();
    for (_pid, process) in procs {
        // Use try_with_inner_mut to avoid deadlocking if the timer
        // interrupt fires while someone holds this process's inner lock.
        let expire = match process.try_with_timers_mut(|timers| {
            let expire = timers.itimer_real_expire_ms;
            if expire != 0 && expire <= current_ms {
                // Reload interval or disarm
                if timers.itimer_real_interval_ms > 0 {
                    timers.itimer_real_expire_ms = current_ms + timers.itimer_real_interval_ms;
                } else {
                    timers.itimer_real_expire_ms = 0;
                }
                Some(expire)
            } else {
                None
            }
        }) {
            Some(Some(expire)) => expire,
            _ => continue,
        };
        // Fire SIGALRM
        log::warn!(
            "[itimer] pid={} SIGALRM fired, expire={} now={}",
            _pid,
            expire,
            current_ms
        );
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

/// Check and fire POSIX timers (timer_create / timer_settime).
fn check_posix_timers(current_ms: usize) {
    use crate::task::{pid2process_snapshot, wakeup_task, SignalFlags, TaskStatus};
    use crate::syscall::posix_timers_check_expired;

    let current_us = current_ms as u64 * 1000;
    let expired_list = posix_timers_check_expired(current_us);

    if expired_list.is_empty() {
        return;
    }

    let procs = pid2process_snapshot();
    for (pid, signo) in expired_list {
        if let Some((_p, process)) = procs.iter().find(|(p, _)| *p == pid) {
            let sig_flag = if signo > 0 && signo <= 64 {
                SignalFlags::from_bits_truncate(1u64 << signo as u32)
            } else {
                SignalFlags::SIGALRM
            };
            process.insert_process_signal(sig_flag, 0i32, 0i32);
            for task in process.tasks_snapshot() {
                if task.status() == TaskStatus::Blocked {
                    task.mark_interrupted();
                    task.set_status(TaskStatus::Ready);
                    wakeup_task(task.clone());
                }
            }
        }
    }
}

pub fn timer_len() -> usize {
    with_timers_read(|timers| timers.len())
}
