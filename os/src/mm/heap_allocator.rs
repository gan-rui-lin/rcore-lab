//! The global allocator
use crate::config::KERNEL_HEAP_SIZE;
use buddy_system_allocator::LockedHeap;
use core::alloc::{GlobalAlloc, Layout};
use core::cmp::max;
use core::mem::size_of;
use core::sync::atomic::{AtomicUsize, Ordering};
use spin::Mutex;

const ALLOC_TRACE_CAP: usize = 64;
const ALLOC_TRACE_MIN_SIZE: usize = 4096;

#[derive(Clone, Copy)]
struct AllocTraceEntry {
    seq: usize,
    req_size: usize,
    req_align: usize,
    rounded_size: usize,
    rounded_class: usize,
    pid: usize,
    tid: usize,
    last_syscall: usize,
    sampled: bool,
    ok: bool,
    ptr: usize,
}

impl AllocTraceEntry {
    const fn empty() -> Self {
        Self {
            seq: 0,
            req_size: 0,
            req_align: 0,
            rounded_size: 0,
            rounded_class: 0,
            pid: 0,
            tid: 0,
            last_syscall: 0,
            sampled: false,
            ok: false,
            ptr: 0,
        }
    }
}

struct AllocTraceRing {
    next: usize,
    entries: [AllocTraceEntry; ALLOC_TRACE_CAP],
}

impl AllocTraceRing {
    const fn new() -> Self {
        Self {
            next: 0,
            entries: [AllocTraceEntry::empty(); ALLOC_TRACE_CAP],
        }
    }

    fn push(&mut self, entry: AllocTraceEntry) {
        self.entries[self.next] = entry;
        self.next = (self.next + 1) % ALLOC_TRACE_CAP;
    }
}

struct TracedLockedHeap(LockedHeap);

impl TracedLockedHeap {
    const fn empty() -> Self {
        Self(LockedHeap::empty())
    }
}

static ALLOC_TRACE_SEQ: AtomicUsize = AtomicUsize::new(0);
static ALLOC_TRACE_RING: Mutex<AllocTraceRing> = Mutex::new(AllocTraceRing::new());

unsafe impl GlobalAlloc for TracedLockedHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = self.0.alloc(layout);
        maybe_record_alloc_trace(layout, ptr);
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        self.0.dealloc(ptr, layout);
    }
}

#[global_allocator]
/// heap allocator instance
static HEAP_ALLOCATOR: TracedLockedHeap = TracedLockedHeap::empty();

#[alloc_error_handler]
/// panic when heap allocation error occurs
pub fn handle_alloc_error(layout: core::alloc::Layout) -> ! {
    let rounded = rounded_alloc_size(layout);
    let rounded_class = rounded.trailing_zeros() as usize;
    println!(
        "[kernel] alloc_error layout: size={} align={}",
        layout.size(),
        layout.align()
    );
    println!(
        "[kernel] alloc_error layout2: rounded_size={} class={} word_size={}",
        rounded,
        rounded_class,
        size_of::<usize>()
    );
    if let Some(heap) = HEAP_ALLOCATOR.0.try_lock() {
        let user = heap.stats_alloc_user();
        let actual = heap.stats_alloc_actual();
        let total = heap.stats_total_bytes();
        let free_bytes = total.saturating_sub(actual);
        let overhead = actual.saturating_sub(user);
        println!(
            "[kernel] alloc_error heap: user={} actual={} overhead={} total={} free={} used_pct={}%",
            user,
            actual,
            overhead,
            total,
            free_bytes,
            if total == 0 {
                0
            } else {
                actual.saturating_mul(100) / total
            }
        );
    } else {
        println!("[kernel] alloc_error heap: <heap_lock_busy>");
    }
    let (fa_cur, fa_end, fa_recycled, fa_avail) = crate::mm::frame_allocator_stats();
    println!(
        "[kernel] alloc_error frames: current={:#x} end={:#x} recycled={} available_est={}",
        fa_cur,
        fa_end,
        fa_recycled,
        fa_avail
    );
    crate::task::print_current_task_brief_for_alloc_error();
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
        proc_len,
        ready_len,
        timer_len,
        live_tasks
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
        total_mutex_waiters,
        total_sem_waiters,
        total_cond_waiters
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
    crate::task::print_ready_queue_brief(8);
    crate::task::print_pid2process_top_by_fd(5);
    print_recent_alloc_trace(24);
    panic!("Heap allocation error, layout = {:?}", layout);
}

#[inline]
fn rounded_alloc_size(layout: core::alloc::Layout) -> usize {
    let size_pow2 = layout.size().checked_next_power_of_two().unwrap_or(usize::MAX);
    max(
        size_pow2,
        max(layout.align(), size_of::<usize>()),
    )
}

#[inline]
fn maybe_record_alloc_trace(layout: Layout, ptr: *mut u8) {
    if layout.size() < ALLOC_TRACE_MIN_SIZE && !ptr.is_null() {
        return;
    }
    let rounded = rounded_alloc_size(layout);
    let rounded_class = rounded.trailing_zeros() as usize;
    let (pid, tid, last_syscall, sampled) = crate::task::current_task_context_for_alloc_trace();
    let entry = AllocTraceEntry {
        seq: ALLOC_TRACE_SEQ.fetch_add(1, Ordering::Relaxed) + 1,
        req_size: layout.size(),
        req_align: layout.align(),
        rounded_size: rounded,
        rounded_class,
        pid,
        tid,
        last_syscall,
        sampled,
        ok: !ptr.is_null(),
        ptr: ptr as usize,
    };
    if let Some(mut ring) = ALLOC_TRACE_RING.try_lock() {
        ring.push(entry);
    }
}

fn print_recent_alloc_trace(limit: usize) {
    let Some(ring) = ALLOC_TRACE_RING.try_lock() else {
        println!("[kernel] alloc_error recent_alloc: <ring_lock_busy>");
        return;
    };
    println!(
        "[kernel] alloc_error recent_alloc: cap={} next={} min_size={}",
        ALLOC_TRACE_CAP,
        ring.next,
        ALLOC_TRACE_MIN_SIZE
    );
    let mut printed = 0usize;
    for step in 0..ALLOC_TRACE_CAP {
        if printed >= limit {
            break;
        }
        let idx = (ring.next + ALLOC_TRACE_CAP - 1 - step) % ALLOC_TRACE_CAP;
        let entry = ring.entries[idx];
        if entry.seq == 0 {
            continue;
        }
        println!(
            "[kernel] alloc_error recent_alloc[{}]: seq={} pid={} tid={} syscall={} sampled={} req={} align={} rounded={} class={} ok={} ptr={:#x}",
            printed,
            entry.seq,
            entry.pid,
            entry.tid,
            entry.last_syscall,
            entry.sampled,
            entry.req_size,
            entry.req_align,
            entry.rounded_size,
            entry.rounded_class,
            entry.ok,
            entry.ptr
        );
        printed += 1;
    }
}
/// heap space ([u8; KERNEL_HEAP_SIZE])
static mut HEAP_SPACE: [u8; KERNEL_HEAP_SIZE] = [0; KERNEL_HEAP_SIZE];
/// initiate heap allocator
pub fn init_heap() {
    unsafe {
        HEAP_ALLOCATOR
            .0
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
