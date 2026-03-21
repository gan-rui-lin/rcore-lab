//! Task pid and user resource implementation.

use super::ProcessControlBlock;
use crate::config::{KERNEL_STACK_SIZE, PAGE_SIZE, USER_STACK_SIZE};
use crate::mm::{frame_alloc, FrameTracker};
use crate::mm::{MapPermission, PhysPageNum, VirtAddr};
use crate::sync::UPIntrFreeCell;
#[cfg(target_arch = "loongarch64")]
use alloc::vec;
use alloc::{
    sync::{Arc, Weak},
    vec,
    vec::Vec,
};
use lazy_static::*;

pub struct RecycleAllocator {
    current: usize,
    recycled: Vec<usize>,
}

impl RecycleAllocator {
    pub fn new() -> Self {
        RecycleAllocator {
            current: 0,
            recycled: Vec::new(),
        }
    }
    pub fn new_with_start(start: usize) -> Self {
        RecycleAllocator {
            current: start,
            recycled: Vec::new(),
        }
    }
    pub fn alloc(&mut self) -> usize {
        if let Some(id) = self.recycled.pop() {
            id
        } else {
            self.current += 1;
            self.current - 1
        }
    }
    pub fn dealloc(&mut self, id: usize) {
        assert!(id < self.current);
        assert!(
            !self.recycled.iter().any(|i| *i == id),
            "id {} has been deallocated!",
            id
        );
        self.recycled.push(id);
    }
}

lazy_static! {
    static ref PID_ALLOCATOR: UPIntrFreeCell<RecycleAllocator> =
        unsafe { UPIntrFreeCell::new(RecycleAllocator::new_with_start(1)) };
}

pub const IDLE_PID: usize = 0;
const KSTACK_GUARD_WORDS: usize = 4;
const KSTACK_GUARD_MAGIC: u128 = 0xDEAD_BEEF_DEAD_BEEF_DEAD_BEEF_DEAD_BEEF;
const KSTACK_FILL_MAGIC: u128 = 0xA5A5_A5A5_A5A5_A5A5_A5A5_A5A5_A5A5_A5A5;

pub struct PidHandle(pub usize);

pub fn pid_alloc() -> PidHandle {
    PidHandle(PID_ALLOCATOR.exclusive_access().alloc())
}

impl Drop for PidHandle {
    fn drop(&mut self) {
        PID_ALLOCATOR.exclusive_access().dealloc(self.0);
    }
}

/// Kernel stack for a process(task)
pub struct KernelStack {
    inner: Arc<Vec<u128>>,
}

/// allocate a new kernel stack
pub fn kstack_alloc() -> KernelStack {
    let mut words = vec![KSTACK_FILL_MAGIC; KERNEL_STACK_SIZE / core::mem::size_of::<u128>()];
    for (i, slot) in words.iter_mut().take(KSTACK_GUARD_WORDS).enumerate() {
        *slot = KSTACK_GUARD_MAGIC ^ (i as u128);
    }
    KernelStack {
        inner: Arc::new(words),
    }
}

impl KernelStack {
    #[inline]
    pub fn check_guard(&self) {
        for (i, slot) in self.inner.iter().take(KSTACK_GUARD_WORDS).enumerate() {
            let expected = KSTACK_GUARD_MAGIC ^ (i as u128);
            assert!(
                *slot == expected,
                "kernel stack overflow detected: guard[{}]={:#x}, expected={:#x}",
                i,
                *slot,
                expected
            );
        }
    }

    /// Push a variable of type T into the top of the KernelStack and return its raw pointer
    #[allow(unused)]
    pub fn push_on_top<T>(&self, value: T) -> *mut T
    where
        T: Sized,
    {
        let kernel_stack_top = self.get_top();
        let ptr_mut = (kernel_stack_top - core::mem::size_of::<T>()) as *mut T;
        unsafe {
            *ptr_mut = value;
        }
        ptr_mut
    }
    /// Get the top of the KernelStack
    pub fn get_top(&self) -> usize {
        self.inner.as_ptr() as usize + KERNEL_STACK_SIZE
    }
}

pub struct TaskUserRes {
    pub tid: usize,
    pub ustack_base: usize,
    pub process: Weak<ProcessControlBlock>,
    trap_cx_frame: Option<FrameTracker>,
}

fn ustack_bottom_from_tid(ustack_base: usize, tid: usize) -> usize {
    ustack_base + tid * (PAGE_SIZE + USER_STACK_SIZE)
}

impl TaskUserRes {
    pub fn new(
        process: Arc<ProcessControlBlock>,
        ustack_base: usize,
        alloc_user_res: bool,
    ) -> Self {
        let tid = process.inner_exclusive_access().alloc_tid();
        let mut task_user_res = Self {
            tid,
            ustack_base,
            process: Arc::downgrade(&process),
            trap_cx_frame: None,
        };
        task_user_res.alloc_trap_cx();
        if alloc_user_res {
            task_user_res.alloc_ustack();
        }
        task_user_res
    }

    pub fn alloc_user_res(&mut self) {
        self.alloc_trap_cx();
        self.alloc_ustack();
    }

    fn alloc_ustack(&self) {
        let process = self.process.upgrade().unwrap();
        let mut process_inner = process.inner_exclusive_access();
        let ustack_bottom = ustack_bottom_from_tid(self.ustack_base, self.tid);
        let ustack_top = ustack_bottom + USER_STACK_SIZE;
        process_inner.memory_set.insert_framed_area(
            ustack_bottom.into(),
            ustack_top.into(),
            MapPermission::R | MapPermission::W | MapPermission::U,
        );
        if let Some(pte) = process_inner
            .memory_set
            .translate(VirtAddr::from(ustack_bottom).floor())
        {
            trace!(
                "[ustack_alloc] tid={} bottom={:#x} top={:#x} pte_bits={:#x}",
                self.tid,
                ustack_bottom,
                ustack_top,
                pte.bits
            );
        }
    }

    fn alloc_trap_cx(&mut self) {
        if self.trap_cx_frame.is_none() {
            let frame = frame_alloc().expect("alloc trap_cx frame");
            self.trap_cx_frame = Some(frame);
        }
    }

    fn dealloc_user_res(&self) {
        let process = self.process.upgrade().unwrap();
        let mut process_inner = process.inner_exclusive_access();
        let ustack_bottom_va: VirtAddr = ustack_bottom_from_tid(self.ustack_base, self.tid).into();
        process_inner
            .memory_set
            .remove_area_with_start_vpn(ustack_bottom_va.into());
    }

    #[allow(unused)]
    pub fn alloc_tid(&mut self) {
        self.tid = self
            .process
            .upgrade()
            .unwrap()
            .inner_exclusive_access()
            .alloc_tid();
    }

    pub fn dealloc_tid(&self) {
        let process = self.process.upgrade().unwrap();
        let mut process_inner = process.inner_exclusive_access();
        process_inner.dealloc_tid(self.tid);
    }

    pub fn trap_cx_user_va(&self) -> usize {
        0
    }

    pub fn trap_cx_ppn(&self) -> PhysPageNum {
        self.trap_cx_frame
            .as_ref()
            .expect("trap_cx_frame not allocated")
            .ppn
    }

    pub fn ustack_base(&self) -> usize {
        self.ustack_base
    }
    pub fn ustack_top(&self) -> usize {
        ustack_bottom_from_tid(self.ustack_base, self.tid) + USER_STACK_SIZE
    }
}

impl Drop for TaskUserRes {
    fn drop(&mut self) {
        self.dealloc_tid();
        self.dealloc_user_res();
    }
}
