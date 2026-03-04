#![allow(missing_docs)]

//! Futex (Fast Userspace muTEX) implementation.
//!
//! Provides FUTEX_WAIT and FUTEX_WAKE operations used by musl libc
//! for pthread mutexes, condition variables, and thread joining.
//!
//! The futex table is keyed by physical address so that shared memory
//! futexes work correctly across processes.

use alloc::collections::BTreeMap;
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use crate::mm::{PageTable, VirtAddr};
use crate::sync::UPIntrFreeCell;
use crate::task::{
    block_current_task, schedule, wakeup_task, TaskControlBlock,
    current_user_token, current_task,
};
use lazy_static::lazy_static;

/// Futex operation codes (Linux ABI)
pub const FUTEX_WAIT: u32 = 0;
pub const FUTEX_WAKE: u32 = 1;
pub const FUTEX_PRIVATE_FLAG: u32 = 128;
pub const FUTEX_WAIT_PRIVATE: u32 = FUTEX_WAIT | FUTEX_PRIVATE_FLAG;
pub const FUTEX_WAKE_PRIVATE: u32 = FUTEX_WAKE | FUTEX_PRIVATE_FLAG;

lazy_static! {
    /// Global futex wait queue table, keyed by physical address.
    static ref FUTEX_TABLE: UPIntrFreeCell<BTreeMap<usize, VecDeque<Arc<TaskControlBlock>>>> =
        unsafe { UPIntrFreeCell::new(BTreeMap::new()) };
}

/// Translate a user virtual address to physical address using the current
/// process page table. Returns None if unmapped.
fn uaddr_to_paddr(token: usize, uaddr: usize) -> Option<usize> {
    let page_table = PageTable::from_token(token);
    page_table.translate_va(VirtAddr::from(uaddr)).map(|pa| pa.into())
}

/// Read a u32 from user virtual address.
fn read_user_u32(token: usize, uaddr: usize) -> Option<u32> {
    let page_table = PageTable::from_token(token);
    let pa = page_table.translate_va(VirtAddr::from(uaddr))?;
    let ptr: *const u32 = pa.0 as *const u32;
    Some(unsafe { core::ptr::read_volatile(ptr) })
}

/// FUTEX_WAIT: if *uaddr == expected, block the current task.
///
/// Returns 0 on success (was woken), -EAGAIN if *uaddr != expected.
pub fn futex_wait(uaddr: usize, expected: u32) -> isize {
    let token = current_user_token();

    // Atomically check value and enqueue under the futex table lock
    let mut table = FUTEX_TABLE.exclusive_access();

    // Read the current value at uaddr
    let current_val = match read_user_u32(token, uaddr) {
        Some(v) => v,
        None => return -14, // EFAULT
    };

    // If value changed, return EAGAIN
    if current_val != expected {
        return -11; // EAGAIN
    }

    // Translate to physical address for the key
    let paddr = match uaddr_to_paddr(token, uaddr) {
        Some(pa) => pa,
        None => return -14, // EFAULT
    };

    // Enqueue current task
    let task = current_task().unwrap();
    let queue = table.entry(paddr).or_insert_with(VecDeque::new);
    queue.push_back(task);

    // Block the current task (sets status to Blocked, returns context pointer)
    // Must drop the table lock before scheduling
    drop(table);

    let task_cx_ptr = block_current_task();
    schedule(task_cx_ptr);

    0
}

/// FUTEX_WAKE: wake up to `num_wake` waiters on the given futex address.
///
/// Returns the number of waiters that were woken.
pub fn futex_wake(uaddr: usize, num_wake: u32) -> isize {
    let token = current_user_token();

    let paddr = match uaddr_to_paddr(token, uaddr) {
        Some(pa) => pa,
        None => return -14, // EFAULT
    };

    let mut table = FUTEX_TABLE.exclusive_access();
    let mut woken = 0u32;

    if let Some(queue) = table.get_mut(&paddr) {
        while woken < num_wake {
            if let Some(task) = queue.pop_front() {
                wakeup_task(task);
                woken += 1;
            } else {
                break;
            }
        }
        // Clean up empty queue
        if queue.is_empty() {
            table.remove(&paddr);
        }
    }

    woken as isize
}
