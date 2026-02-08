//! Types related to task management & Functions for completely changing TCB
use super::TaskContext;
use super::{kstack_alloc, pid_alloc, KernelStack, PidHandle};
use crate::config::TRAP_CONTEXT_BASE;
use crate::fs::{File, Stdin, Stdout};
use crate::mm::{translated_refmut, MemorySet, PhysPageNum, VirtAddr, KERNEL_SPACE};
use crate::sync::UPSafeCell;
use crate::trap::{trap_handler, TrapContext};
use alloc::sync::{Arc, Weak};
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefMut;
use riscv::register::sie;

/// Task control block structure
///
/// Directly save the contents that will not change during running
pub struct TaskControlBlock {
    // Immutable
    /// Process identifier
    pub pid: PidHandle,

    /// Kernel stack corresponding to PID
    pub kernel_stack: KernelStack,

    /// Mutable
    inner: UPSafeCell<TaskControlBlockInner>,
}

impl TaskControlBlock {
    fn build_user_stack(
        token: usize,
        mut user_sp: usize,
        mut args: Vec<String>,
        envs: Vec<String>,
    ) -> (usize, usize, usize, usize) {
        if args.is_empty() {
            args.push(String::new());
        }
        let mut env_addrs: Vec<usize> = Vec::new();
        for env in envs.iter() {
            user_sp -= env.len() + 1;
            let addr = user_sp;
            for (i, b) in env.as_bytes().iter().enumerate() {
                *translated_refmut(token, (addr + i) as *mut u8) = *b;
            }
            *translated_refmut(token, (addr + env.len()) as *mut u8) = 0;
            env_addrs.push(addr);
        }
        let mut arg_addrs: Vec<usize> = Vec::new();
        for arg in args.iter() {
            user_sp -= arg.len() + 1;
            let addr = user_sp;
            for (i, b) in arg.as_bytes().iter().enumerate() {
                *translated_refmut(token, (addr + i) as *mut u8) = *b;
            }
            *translated_refmut(token, (addr + arg.len()) as *mut u8) = 0;
            arg_addrs.push(addr);
        }
        user_sp &= !0xf;
        let word_size = core::mem::size_of::<usize>();
        user_sp -= (env_addrs.len() + 1) * word_size;
        let envp_base = user_sp;
        for (i, addr) in env_addrs.iter().enumerate() {
            *translated_refmut(token, (envp_base + i * word_size) as *mut usize) = *addr;
        }
        *translated_refmut(
            token,
            (envp_base + env_addrs.len() * word_size) as *mut usize,
        ) = 0;
        user_sp -= (arg_addrs.len() + 1) * word_size;
        let argv_base = user_sp;
        for (i, addr) in arg_addrs.iter().enumerate() {
            *translated_refmut(token, (argv_base + i * word_size) as *mut usize) = *addr;
        }
        *translated_refmut(
            token,
            (argv_base + arg_addrs.len() * word_size) as *mut usize,
        ) = 0;
        let argc = arg_addrs.len();
        user_sp = (user_sp - word_size) & !0xf;
        *translated_refmut(token, user_sp as *mut usize) = argc;
        (user_sp, argc, argv_base, envp_base)
    }

    /// Get the mutable reference of the inner TCB
    pub fn inner_exclusive_access(&self) -> RefMut<'_, TaskControlBlockInner> {
        self.inner.exclusive_access()
    }
    /// Get the address of app's page table
    pub fn get_user_token(&self) -> usize {
        let inner = self.inner_exclusive_access();
        inner.memory_set.token()
    }

    /// Get a copy of the current process name.
    pub fn name(&self) -> String {
        let inner = self.inner_exclusive_access();
        inner.name.clone()
    }

    /// Update the process name based on a path-like string.
    pub fn set_name_from_path(&self, path: &str) {
        let name = path
            .rsplit('/')
            .find(|part| !part.is_empty())
            .unwrap_or(path);
        let mut inner = self.inner_exclusive_access();
        inner.name = name.to_string();
    }
}

pub struct TaskControlBlockInner {
    /// The physical page number of the frame where the trap context is placed
    pub trap_cx_ppn: PhysPageNum,

    /// Application data can only appear in areas
    /// where the application address space is lower than base_size
    pub base_size: usize,

    /// Save task context
    pub task_cx: TaskContext,

    /// Maintain the execution status of the current process
    pub task_status: TaskStatus,

    /// Application address space
    pub memory_set: MemorySet,

    /// Parent process of the current process.
    /// Weak will not affect the reference count of the parent
    pub parent: Option<Weak<TaskControlBlock>>,

    /// A vector containing TCBs of all child processes of the current process
    pub children: Vec<Arc<TaskControlBlock>>,

    /// It is set when active exit or execution error occurs
    pub exit_code: i32,
    pub fd_table: Vec<Option<Arc<dyn File + Send + Sync>>>,

    /// Process name, used for tracing/debugging.
    pub name: String,

    /// Heap bottom
    pub heap_bottom: usize,

    /// Program break
    pub program_brk: usize,
}

impl TaskControlBlockInner {
    pub fn get_trap_cx(&self) -> &'static mut TrapContext {
        self.trap_cx_ppn.get_mut()
    }
    pub fn get_user_token(&self) -> usize {
        self.memory_set.token()
    }
    fn get_status(&self) -> TaskStatus {
        self.task_status
    }
    pub fn is_zombie(&self) -> bool {
        self.get_status() == TaskStatus::Zombie
    }
    pub fn alloc_fd(&mut self) -> usize {
        if let Some(fd) = (0..self.fd_table.len()).find(|fd| self.fd_table[*fd].is_none()) {
            fd
        } else {
            self.fd_table.push(None);
            self.fd_table.len() - 1
        }
    }
}

impl TaskControlBlock {
    /// Create a new process
    ///
    /// At present, it is only used for the creation of initproc
    pub fn new(elf_data: &[u8]) -> Self {
        // memory_set with elf program headers/trampoline/trap context/user stack
        let (memory_set, user_stack_top, entry_point) = MemorySet::from_elf(elf_data);
        let token = memory_set.token();
        let (user_sp, argc, argv_base, envp_base) =
            Self::build_user_stack(token, user_stack_top, Vec::new(), Vec::new());
        let trap_cx_ppn = memory_set
            .translate(VirtAddr::from(TRAP_CONTEXT_BASE).into())
            .unwrap()
            .ppn();
        // alloc a pid and a kernel stack in kernel space
        let pid_handle = pid_alloc();
        let kernel_stack = kstack_alloc();
        let kernel_stack_top = kernel_stack.get_top();
        // push a task context which goes to trap_return to the top of kernel stack
        let task_control_block = Self {
            pid: pid_handle,
            kernel_stack,
            inner: unsafe {
                UPSafeCell::new(TaskControlBlockInner {
                    trap_cx_ppn,
                    base_size: user_stack_top,
                    task_cx: TaskContext::goto_trap_return(kernel_stack_top),
                    task_status: TaskStatus::Ready,
                    memory_set,
                    parent: None,
                    children: Vec::new(),
                    exit_code: 0,
                    fd_table: vec![
                        // 0 -> stdin
                        Some(Arc::new(Stdin)),
                        // 1 -> stdout
                        Some(Arc::new(Stdout)),
                        // 2 -> stderr
                        Some(Arc::new(Stdout)),
                    ],
                    name: String::from("initproc"),
                    heap_bottom: user_stack_top,
                    program_brk: user_stack_top,
                })
            },
        };
        // prepare TrapContext in user space
        let mut trap_cx = TrapContext::app_init_context(
            entry_point,
            user_sp,
            KERNEL_SPACE.exclusive_access().token(),
            kernel_stack_top,
            trap_handler as usize,
        );
        trap_cx.x[10] = argc;
        trap_cx.x[11] = argv_base;
        trap_cx.x[12] = envp_base;
        *task_control_block.inner_exclusive_access().get_trap_cx() = trap_cx;
        task_control_block
    }

    /// Load a new elf to replace the original application address space and start execution
    pub fn exec(&self, elf_data: &[u8], args: Vec<String>, envs: Vec<String>) {
        // memory_set with elf program headers/trampoline/trap context/user stack
        let (memory_set, user_stack_top, entry_point) = MemorySet::from_elf(elf_data);
        let token = memory_set.token();
        let (user_sp, argc, argv_base, envp_base) =
            Self::build_user_stack(token, user_stack_top, args.clone(), envs);
        let trap_cx_ppn = memory_set
            .translate(VirtAddr::from(TRAP_CONTEXT_BASE).into())
            .unwrap()
            .ppn();

        // **** access current TCB exclusively
        let mut inner = self.inner_exclusive_access();
        // substitute memory_set
        inner.memory_set = memory_set;
        // update trap_cx ppn
        inner.trap_cx_ppn = trap_cx_ppn;
        inner.base_size = user_stack_top;
        inner.heap_bottom = user_stack_top;
        inner.program_brk = user_stack_top;
        if let Some(arg0) = args.first() {
            let name = arg0
                .rsplit('/')
                .find(|part| !part.is_empty())
                .unwrap_or(arg0.as_str());
            inner.name = name.to_string();
        }
        // initialize trap_cx
        let mut trap_cx = TrapContext::app_init_context(
            entry_point,
            user_sp,
            KERNEL_SPACE.exclusive_access().token(),
            self.kernel_stack.get_top(),
            trap_handler as usize,
        );
        trap_cx.x[10] = argc;
        trap_cx.x[11] = argv_base;
        trap_cx.x[12] = envp_base;
        *inner.get_trap_cx() = trap_cx;
        // **** release current PCB
    }

    /// parent process fork the child process
    pub fn fork(self: &Arc<TaskControlBlock>) -> Arc<TaskControlBlock> {
        trace!("task::fork: parent pid={}", self.pid.0);
        // Avoid kernel-mode timer interrupts during heavy cloning.
        unsafe { sie::clear_stimer() };
        // ---- hold parent PCB lock
        let mut parent_inner = self.inner_exclusive_access();
        // copy user space(include trap context)
        trace!("task::fork: copy user space");
        parent_inner.memory_set.debug_area_ranges();
        let memory_set = MemorySet::from_existed_user(&parent_inner.memory_set);
        let trap_cx_ppn = memory_set
            .translate(VirtAddr::from(TRAP_CONTEXT_BASE).into())
            .unwrap()
            .ppn();
        // alloc a pid and a kernel stack in kernel space
        trace!("task::fork: alloc pid/kstack");
        let pid_handle = pid_alloc();
        let kernel_stack = kstack_alloc();
        let kernel_stack_top = kernel_stack.get_top();
        // copy fd table
        trace!("task::fork: copy fd table");
        let mut new_fd_table: Vec<Option<Arc<dyn File + Send + Sync>>> = Vec::new();
        for fd in parent_inner.fd_table.iter() {
            if let Some(file) = fd {
                new_fd_table.push(Some(file.clone()));
            } else {
                new_fd_table.push(None);
            }
        }
        trace!("task::fork: build child tcb");
        let task_control_block = Arc::new(TaskControlBlock {
            pid: pid_handle,
            kernel_stack,
            inner: unsafe {
                UPSafeCell::new(TaskControlBlockInner {
                    trap_cx_ppn,
                    base_size: parent_inner.base_size,
                    task_cx: TaskContext::goto_trap_return(kernel_stack_top),
                    task_status: TaskStatus::Ready,
                    memory_set,
                    parent: Some(Arc::downgrade(self)),
                    children: Vec::new(),
                    exit_code: 0,
                    fd_table: new_fd_table,
                    name: parent_inner.name.clone(),
                    heap_bottom: parent_inner.heap_bottom,
                    program_brk: parent_inner.program_brk,
                })
            },
        });
        // add child
        trace!("task::fork: link child pid={}", task_control_block.pid.0);
        parent_inner.children.push(task_control_block.clone());
        // modify kernel_sp in trap_cx
        // **** access child PCB exclusively
        let trap_cx = task_control_block.inner_exclusive_access().get_trap_cx();
        trap_cx.kernel_sp = kernel_stack_top;
        // Re-enable timer interrupts after cloning.
        unsafe { sie::set_stimer() };
        // return
        trace!("task::fork: done parent pid={}, child pid={}", self.pid.0, task_control_block.pid.0);
        task_control_block
        // **** release child PCB
        // ---- release parent PCB
    }

    /// get pid of process
    pub fn getpid(&self) -> usize {
        self.pid.0
    }

    /// change the location of the program break. return None if failed.
    pub fn change_program_brk(&self, size: i32) -> Option<usize> {
        let mut inner = self.inner_exclusive_access();
        let heap_bottom = inner.heap_bottom;
        let old_break = inner.program_brk;
        let new_brk = inner.program_brk as isize + size as isize;
        if new_brk < heap_bottom as isize {
            return None;
        }
        let result = if size < 0 {
            inner
                .memory_set
                .shrink_to(VirtAddr(heap_bottom), VirtAddr(new_brk as usize))
        } else {
            inner
                .memory_set
                .append_to(VirtAddr(heap_bottom), VirtAddr(new_brk as usize))
        };
        if result {
            inner.program_brk = new_brk as usize;
            Some(old_break)
        } else {
            None
        }
    }
}

#[derive(Copy, Clone, PartialEq)]
/// task status: UnInit, Ready, Running, Exited
pub enum TaskStatus {
    /// uninitialized
    UnInit,
    /// ready to run
    Ready,
    /// running
    Running,
    /// exited
    Zombie,
}
