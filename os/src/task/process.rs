use super::id::RecycleAllocator;
use super::manager::insert_into_pid2process;
use super::{PidHandle, SignalActions, SignalFlags, TaskControlBlock, TlsArea, add_task, pid_alloc};
use crate::config::{USER_STACK_SIZE};
use crate::fs::{File, Stdin, Stdout};
use crate::mm::{KERNEL_SPACE, MemorySet, translated_refmut};
use crate::sync::{Condvar, Mutex, Semaphore, UPSafeCell};
use crate::trap::{TrapContext, trap_handler};
use alloc::string::String;
use alloc::sync::{Arc, Weak};
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefMut;

const DEFAULT_MMAP_BASE: usize = 0x4000_0000;

pub struct ProcessControlBlock {
    pub pid: PidHandle,
    inner: UPSafeCell<ProcessControlBlockInner>,
}

pub struct ProcessControlBlockInner {
    pub is_zombie: bool,
    pub memory_set: MemorySet,
    pub parent: Option<Weak<ProcessControlBlock>>,
    pub children: Vec<Arc<ProcessControlBlock>>,
    pub exit_code: i32,
    pub fd_table: Vec<Option<Arc<dyn File + Send + Sync>>>,
    pub signals: SignalFlags,
    pub signal_pending: SignalFlags,
    pub signal_mask: SignalFlags,
    pub signal_actions: SignalActions,
    pub tasks: Vec<Option<Arc<TaskControlBlock>>>,
    pub task_res_allocator: RecycleAllocator,
    pub mutex_list: Vec<Option<Arc<dyn Mutex>>>,
    pub semaphore_list: Vec<Option<Arc<Semaphore>>>,
    pub condvar_list: Vec<Option<Arc<Condvar>>>,
    pub name: String,
    pub cwd: String,
    pub heap_bottom: usize,
    pub program_brk: usize,
    pub mmap_base: usize,
    pub tls_area: Option<TlsArea>,
}

impl ProcessControlBlockInner {
    pub fn get_user_token(&self) -> usize {
        self.memory_set.token()
    }

    pub fn alloc_fd(&mut self) -> usize {
        if let Some(fd) = (0..self.fd_table.len()).find(|fd| self.fd_table[*fd].is_none()) {
            fd
        } else {
            self.fd_table.push(None);
            self.fd_table.len() - 1
        }
    }

    pub fn alloc_tid(&mut self) -> usize {
        self.task_res_allocator.alloc()
    }

    pub fn dealloc_tid(&mut self, tid: usize) {
        self.task_res_allocator.dealloc(tid)
    }

    pub fn thread_count(&self) -> usize {
        self.tasks.len()
    }

    pub fn get_task(&self, tid: usize) -> Arc<TaskControlBlock> {
        self.tasks[tid].as_ref().unwrap().clone()
    }
}

impl ProcessControlBlock {
    pub fn inner_exclusive_access(&self) -> RefMut<'_, ProcessControlBlockInner> {
        self.inner.exclusive_access()
    }

    /// Try to borrow the process inner state; returns None if already borrowed.
    pub fn try_inner_exclusive_access(&self) -> Option<RefMut<'_, ProcessControlBlockInner>> {
        self.inner.try_exclusive_access()
    }

    pub fn new(elf_data: &[u8]) -> Arc<Self> {
        let (mut memory_set, user_stack_top, entry_point, tls_info, _auxv_info) = MemorySet::from_elf(elf_data);
        let ustack_base = user_stack_top.saturating_sub(USER_STACK_SIZE);

        // Initialize TLS if PT_TLS segment exists
        let tls_area = tls_info.map(|info| {
            TlsArea::new(&info, &mut memory_set, elf_data)
        });
        let pid_handle = pid_alloc();
        let process = Arc::new(Self {
            pid: pid_handle,
            inner: unsafe {
                UPSafeCell::new(ProcessControlBlockInner {
                    is_zombie: false,
                    memory_set,
                    parent: None,
                    children: Vec::new(),
                    exit_code: 0,
                    fd_table: vec![
                        Some(Arc::new(Stdin)),
                        Some(Arc::new(Stdout)),
                        Some(Arc::new(Stdout)),
                    ],
                    signals: SignalFlags::empty(),
                    signal_pending: SignalFlags::empty(),
                    signal_mask: SignalFlags::empty(),
                    signal_actions: SignalActions::default(),
                    tasks: Vec::new(),
                    task_res_allocator: RecycleAllocator::new(),
                    mutex_list: Vec::new(),
                    semaphore_list: Vec::new(),
                    condvar_list: Vec::new(),
                    name: String::from("initproc"),
                    cwd: String::from("/"),
                    heap_bottom: user_stack_top,
                    program_brk: user_stack_top,
                    mmap_base: DEFAULT_MMAP_BASE,
                    tls_area: tls_area.clone(),
                })
            },
        });
        let task = Arc::new(TaskControlBlock::new(Arc::clone(&process), ustack_base, false));
        let task_inner = task.inner_exclusive_access();
        let trap_cx = task_inner.get_trap_cx();
        let ustack_top = user_stack_top;
        let kstack_top = task.kstack.get_top();
        drop(task_inner);
        let mut trap_cx_value = TrapContext::app_init_context(
            entry_point,
            ustack_top,
            KERNEL_SPACE.exclusive_access().token(),
            kstack_top,
            trap_handler as usize,
        );

        // Set tp register if TLS is present
        if let Some(ref tls) = tls_area {
            trap_cx_value.x[4] = tls.tp_value;  // tp = x4
            info!("[kernel] TLS initialized: tp = {:#x}", tls.tp_value);
        }

        *trap_cx = trap_cx_value;
        let mut process_inner = process.inner_exclusive_access();
        process_inner.tasks.push(Some(Arc::clone(&task)));
        drop(process_inner);
        insert_into_pid2process(process.getpid(), Arc::clone(&process));
        add_task(task);
        process
    }

    pub fn exec(self: &Arc<Self>, elf_data: &[u8], args: Vec<String>, envs: Vec<String>) {
        assert_eq!(self.inner_exclusive_access().thread_count(), 1);
        let (mut memory_set, user_stack_top, entry_point, tls_info, auxv_info) = MemorySet::from_elf(elf_data);

        // Initialize TLS if PT_TLS segment exists
        let tls_area = tls_info.map(|info| {
            TlsArea::new(&info, &mut memory_set, elf_data)
        });

        let new_token = memory_set.token();
        let ustack_base = user_stack_top.saturating_sub(USER_STACK_SIZE);
        {
            let mut inner = self.inner_exclusive_access();
            inner.memory_set = memory_set;
            inner.heap_bottom = user_stack_top;
            inner.program_brk = user_stack_top;
            inner.mmap_base = DEFAULT_MMAP_BASE;
            inner.tls_area = tls_area.clone();
        }
        let task = self.inner_exclusive_access().get_task(0);
        let mut task_inner = task.inner_exclusive_access();
        if let Some(res) = task_inner.res.as_mut() {
            res.ustack_base = ustack_base;
        }
        task_inner.trap_cx_ppn = task_inner.res.as_ref().unwrap().trap_cx_ppn();
        let mut user_sp = user_stack_top;
        // End marker (NULL) at the top of stack region.
        user_sp -= 1;
        *translated_refmut(new_token, user_sp as *mut u8) = 0;
        let mut env_addrs: Vec<usize> = Vec::new();
        for env in envs.iter() {
            user_sp -= env.len() + 1;
            let addr = user_sp;
            for (i, b) in env.as_bytes().iter().enumerate() {
                *translated_refmut(new_token, (addr + i) as *mut u8) = *b;
            }
            *translated_refmut(new_token, (addr + env.len()) as *mut u8) = 0;
            env_addrs.push(addr);
        }
        let mut arg_addrs: Vec<usize> = Vec::new();
        for arg in args.iter() {
            user_sp -= arg.len() + 1;
            let addr = user_sp;
            for (i, b) in arg.as_bytes().iter().enumerate() {
                *translated_refmut(new_token, (addr + i) as *mut u8) = *b;
            }
            *translated_refmut(new_token, (addr + arg.len()) as *mut u8) = 0;
            arg_addrs.push(addr);
        }
        user_sp &= !0xf;
        let word_size = core::mem::size_of::<usize>();

        // Push auxiliary vectors FIRST (before envp/argv)
        // Use simple auxv for programs without PT_TLS (like busybox)
        // Use complete auxv for programs with PT_TLS
        if tls_area.is_some() {
            // Push 16 random bytes for AT_RANDOM
            user_sp -= 16;
            user_sp &= !0xf;  // Align to 16 bytes
            let random_addr = user_sp;
            // Write some pseudo-random bytes (TODO: use proper RNG)
            for i in 0..16 {
                *translated_refmut(new_token, (random_addr + i) as *mut u8) = (i * 17) as u8;
            }

            // Push complete auxiliary vectors with proper AT_RANDOM
            let mut auxv_entries = auxv_info.to_entries(crate::config::PAGE_SIZE);
            // Update AT_RANDOM to point to our random bytes
            for entry in &mut auxv_entries {
                if entry.0 == crate::task::auxv::auxv_type::AT_RANDOM {
                    entry.1 = random_addr;
                }
            }

            user_sp -= auxv_entries.len() * 2 * word_size;  // Each entry is 2 words (type, value)
            let auxv_base = user_sp;
            for (i, (aux_type, aux_val)) in auxv_entries.iter().enumerate() {
                *translated_refmut(new_token, (auxv_base + i * 2 * word_size) as *mut usize) = *aux_type;
                *translated_refmut(new_token, (auxv_base + i * 2 * word_size + word_size) as *mut usize) = *aux_val;
            }
            info!("[kernel] exec: Pushed {} auxv entries at {:#x}, AT_RANDOM={:#x}",
                auxv_entries.len(), auxv_base, random_addr);
        } else {
            // Simple auxv for programs without PT_TLS (master branch style)
            const AT_NULL: usize = 0;
            const AT_PHDR: usize = 3;
            const AT_PHENT: usize = 4;
            const AT_PHNUM: usize = 5;
            const AT_PAGESZ: usize = 6;
            const AT_ENTRY: usize = 9;
            let simple_auxv = [
                (AT_ENTRY, auxv_info.entry),
                (AT_PHDR, auxv_info.phdr_addr),
                (AT_PHENT, auxv_info.phent_size),
                (AT_PHNUM, auxv_info.phnum),
                (AT_PAGESZ, crate::config::PAGE_SIZE),
                (AT_NULL, 0),
            ];
            for (key, val) in simple_auxv.iter().rev() {
                user_sp -= 2 * word_size;
                *translated_refmut(new_token, user_sp as *mut usize) = *key;
                *translated_refmut(new_token, (user_sp + word_size) as *mut usize) = *val;
            }
            info!("[kernel] exec: Pushed {} simple auxv entries (no PT_TLS)", simple_auxv.len());
        }

        // Now push envp and argv arrays
        user_sp -= (env_addrs.len() + 1) * word_size;
        let envp_base = user_sp;
        for (i, addr) in env_addrs.iter().enumerate() {
            *translated_refmut(new_token, (envp_base + i * word_size) as *mut usize) = *addr;
        }
        *translated_refmut(
            new_token,
            (envp_base + env_addrs.len() * word_size) as *mut usize,
        ) = 0;
        user_sp -= (arg_addrs.len() + 1) * word_size;
        let argv_base = user_sp;
        for (i, addr) in arg_addrs.iter().enumerate() {
            *translated_refmut(new_token, (argv_base + i * word_size) as *mut usize) = *addr;
        }
        *translated_refmut(
            new_token,
            (argv_base + arg_addrs.len() * word_size) as *mut usize,
        ) = 0;

        let argc = arg_addrs.len();
        user_sp = (user_sp - word_size) & !0xf;
        *translated_refmut(new_token, user_sp as *mut usize) = argc;

        let mut trap_cx = TrapContext::app_init_context(
            entry_point,
            user_sp,
            KERNEL_SPACE.exclusive_access().token(),
            task.kstack.get_top(),
            trap_handler as usize,
        );
        trap_cx.x[10] = argc;
        trap_cx.x[11] = argv_base;
        trap_cx.x[12] = envp_base;

        // Set tp register only if TLS segment is present
        if let Some(ref tls) = tls_area {
            trap_cx.x[4] = tls.tp_value;  // tp = x4
            info!("[kernel] exec: TLS initialized: tp = {:#x}", tls.tp_value);
        }
        // Note: If no PT_TLS, we don't set tp - let userspace libc initialize it

        *task_inner.get_trap_cx() = trap_cx;
    }

    pub fn fork(self: &Arc<Self>) -> Arc<Self> {
        let mut parent = self.inner_exclusive_access();
        assert_eq!(parent.thread_count(), 1);
        let mut memory_set = MemorySet::from_existed_user(&parent.memory_set);

        // Copy TLS from parent if it exists
        let tls_area = parent.tls_area.as_ref().map(|parent_tls| {
            TlsArea::new_from_parent(parent_tls, &parent.memory_set, &mut memory_set)
        });

        let pid = pid_alloc();
        let mut new_fd_table: Vec<Option<Arc<dyn File + Send + Sync>>> = Vec::new();
        for fd in parent.fd_table.iter() {
            if let Some(file) = fd {
                new_fd_table.push(Some(file.clone()));
            } else {
                new_fd_table.push(None);
            }
        }
        let child = Arc::new(Self {
            pid,
            inner: unsafe {
                UPSafeCell::new(ProcessControlBlockInner {
                    is_zombie: false,
                    memory_set,
                    parent: Some(Arc::downgrade(self)),
                    children: Vec::new(),
                    exit_code: 0,
                    fd_table: new_fd_table,
                    signals: SignalFlags::empty(),
                    signal_pending: SignalFlags::empty(),
                    signal_mask: parent.signal_mask,
                    signal_actions: parent.signal_actions.clone(),
                    tasks: Vec::new(),
                    task_res_allocator: RecycleAllocator::new(),
                    mutex_list: Vec::new(),
                    semaphore_list: Vec::new(),
                    condvar_list: Vec::new(),
                    name: parent.name.clone(),
                    cwd: parent.cwd.clone(),
                    heap_bottom: parent.heap_bottom,
                    program_brk: parent.program_brk,
                    mmap_base: parent.mmap_base,
                    tls_area,
                })
            },
        });
        parent.children.push(Arc::clone(&child));
        let ustack_base = parent
            .get_task(0)
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .ustack_base;
        let task = Arc::new(TaskControlBlock::new(
            Arc::clone(&child),
            ustack_base,
            false,
        ));
        let mut child_inner = child.inner_exclusive_access();
        child_inner.tasks.push(Some(Arc::clone(&task)));
        drop(child_inner);
        let task_inner = task.inner_exclusive_access();
        let trap_cx = task_inner.get_trap_cx();
        trap_cx.kernel_sp = task.kstack.get_top();
        drop(task_inner);
        insert_into_pid2process(child.getpid(), Arc::clone(&child));
        add_task(task);
        child
    }

    pub fn getpid(&self) -> usize {
        self.pid.0
    }
}
