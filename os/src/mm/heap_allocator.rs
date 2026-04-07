//! The global allocator
use crate::config::KERNEL_HEAP_SIZE;
use buddy_system_allocator::LockedHeap;

#[global_allocator]
/// heap allocator instance
static HEAP_ALLOCATOR: LockedHeap = LockedHeap::empty();

#[alloc_error_handler]
/// panic when heap allocation error occurs
pub fn handle_alloc_error(layout: core::alloc::Layout) -> ! {
    println!(
        "[kernel] alloc_error layout: size={} align={}",
        layout.size(),
        layout.align()
    );
    let proc_len = crate::task::pid2process_len();
    let ready_len = crate::task::ready_queue_len();
    let live_tasks = crate::task::live_task_count();
    let (tracked_tasks, unique_task_pids, top_pid, top_pid_count, min_pid, max_pid) =
        crate::task::live_task_pid_summary();
    let timer_len = crate::timer::timer_len();
    let (
        map_len,
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
    ) = crate::task::pid2process_aggregate();
    let (total_fd_slots, max_fd_slots, max_fd_pid) = crate::task::pid2process_fdtable_summary();
    println!(
        "[kernel] alloc_error diag: pid2pcb_len={} ready_queue_len={} timer_len={} live_tasks={}",
        proc_len, ready_len, timer_len, live_tasks
    );
    println!(
        "[kernel] alloc_error diag2: map_len={} sampled={} skipped={} total_children={} total_tasks={} total_exited_threads={} max_children_pid={} max_children={}",
        map_len,
        sampled_processes,
        skipped_processes,
        total_children,
        total_tasks,
        total_exited_threads,
        max_children_pid,
        max_children
    );
    println!(
        "[kernel] alloc_error diag3: mutex_waiters={} sem_waiters={} cond_waiters={}",
        total_mutex_waiters, total_sem_waiters, total_cond_waiters
    );
    println!(
        "[kernel] alloc_error diag4: tracked_tasks={} unique_task_pids={} top_pid={} top_pid_count={} min_pid={} max_pid={}",
        tracked_tasks,
        unique_task_pids,
        top_pid,
        top_pid_count,
        min_pid,
        max_pid
    );
    println!(
        "[kernel] alloc_error diag5: total_fd_slots={} max_fd_slots={} max_fd_pid={}",
        total_fd_slots,
        max_fd_slots,
        max_fd_pid
    );
    panic!("Heap allocation error, layout = {:?}", layout);
}
/// heap space ([u8; KERNEL_HEAP_SIZE])
static mut HEAP_SPACE: [u8; KERNEL_HEAP_SIZE] = [0; KERNEL_HEAP_SIZE];
/// initiate heap allocator
pub fn init_heap() {
    unsafe {
        HEAP_ALLOCATOR
            .lock()
            .init(HEAP_SPACE.as_ptr() as usize, KERNEL_HEAP_SIZE);
    }
}

#[allow(unused)]
pub fn heap_test() {
    use alloc::boxed::Box;
    use alloc::vec::Vec;
    extern "C" {
        fn sbss();
        fn ebss();
    }
    let bss_range = sbss as usize..ebss as usize;
    let a = Box::new(5);
    assert_eq!(*a, 5);
    assert!(bss_range.contains(&(a.as_ref() as *const _ as usize)));
    drop(a);
    let mut v: Vec<usize> = Vec::new();
    for i in 0..500 {
        v.push(i);
    }
    for (i, val) in v.iter().take(500).enumerate() {
        assert_eq!(*val, i);
    }
    assert!(bss_range.contains(&(v.as_ptr() as usize)));
    drop(v);
    println!("heap_test passed!");
}
