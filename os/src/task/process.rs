use super::id::RecycleAllocator;
use super::manager::insert_into_pid2process;
use super::{
    add_task, pid_alloc, PidHandle, SignalAction, SignalActions, SignalFlags, TaskControlBlock,
    TlsArea,
};
use crate::config::{USER_MMAP_TOP, USER_STACK_SIZE};
use crate::fs::{File, Stdin, Stdout};
use crate::mm::{
    translated_byte_buffer, translated_ref, translated_refmut, translated_str, MemorySet,
};
use crate::sync::UPIntrRefMut;
use crate::sync::{Condvar, Mutex, Semaphore, UPIntrFreeCell};
use alloc::string::String;
use alloc::sync::{Arc, Weak};
use alloc::vec;
use alloc::vec::Vec;
use arch::{TrapContext, TrapFrameArgs};
use xmas_elf::sections::{SectionData, ShType};
use xmas_elf::symbol_table::Entry;
use xmas_elf::ElfFile;

#[cfg(target_arch = "loongarch64")]
const LOONGARCH_MIN_TCB_ADDR: usize = 0x7000_1000;

pub const RLIMIT_NLIMITS: usize = 16;
pub const RLIMIT_STACK: usize = 3;
pub const RLIMIT_NOFILE: usize = 7;
pub const RLIM_INFINITY: u64 = u64::MAX;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RLimit {
    pub rlim_cur: u64,
    pub rlim_max: u64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct IntervalTimerState {
    pub interval_us: usize,
    pub remaining_us: usize,
}

fn default_rlimits() -> [RLimit; RLIMIT_NLIMITS] {
    let mut limits = [RLimit {
        rlim_cur: RLIM_INFINITY,
        rlim_max: RLIM_INFINITY,
    }; RLIMIT_NLIMITS];
    let stack = USER_STACK_SIZE as u64;
    limits[RLIMIT_STACK] = RLimit {
        rlim_cur: stack,
        rlim_max: stack,
    };
    limits[RLIMIT_NOFILE] = RLimit {
        rlim_cur: 1024,
        rlim_max: 1024,
    };
    limits
}

fn find_global_pointer(elf_data: &[u8]) -> Option<usize> {
    let elf = ElfFile::new(elf_data).ok()?;
    for section in elf.section_iter() {
        let Ok(section_type) = section.get_type() else {
            continue;
        };
        if section_type != ShType::SymTab && section_type != ShType::DynSym {
            continue;
        }
        if let Ok(SectionData::SymbolTable64(entries)) = section.get_data(&elf) {
            for entry in entries {
                if let Ok(name) = entry.get_name(&elf) {
                    if name == "__global_pointer$" {
                        return Some(entry.value() as usize);
                    }
                }
            }
        } else if let Ok(SectionData::DynSymbolTable64(entries)) = section.get_data(&elf) {
            for entry in entries {
                if let Ok(name) = entry.get_name(&elf) {
                    if name == "__global_pointer$" {
                        return Some(entry.value() as usize);
                    }
                }
            }
        }
    }
    None
}

pub struct ProcessControlBlock {
    pub pid: PidHandle,
    inner: UPIntrFreeCell<ProcessControlBlockInner>,
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
    pub signal_actions: SignalActions,
    pub tasks: Vec<Option<Arc<TaskControlBlock>>>,
    pub task_res_allocator: RecycleAllocator,
    pub mutex_list: Vec<Option<Arc<dyn Mutex>>>,
    pub semaphore_list: Vec<Option<Arc<Semaphore>>>,
    pub condvar_list: Vec<Option<Arc<Condvar>>>,
    pub name: String,
    pub cwd: String,
    pub root_dir: String,
    pub real_uid: u32,
    pub effective_uid: u32,
    pub saved_uid: u32,
    pub fs_uid: u32,
    pub real_gid: u32,
    pub effective_gid: u32,
    pub saved_gid: u32,
    pub fs_gid: u32,
    /// Nice value in Linux range [-20, 19], default 0.
    pub nice: i32,
    /// Capability sets (bit mask, Linux kernel format)
    pub cap_permitted: u64,
    pub cap_effective: u64,
    pub cap_inheritable: u64,
    pub cap_bounding: u64,
    pub heap_bottom: usize,
    pub program_brk: usize,
    pub mmap_base: usize,
    pub tls_area: Option<TlsArea>,
    pub rlimits: [RLimit; RLIMIT_NLIMITS],
    pub itimers: [IntervalTimerState; 3],
    pub session_id: usize,
    pub pgid: usize,
    /// Set by ptrace(PTRACE_TRACEME).
    pub ptrace_traceme: bool,
    /// Pending ptrace stop signal to be reported by waitpid.
    pub ptrace_stop_signal: Option<i32>,
    /// ITIMER_REAL: absolute expire time in ms, 0 = inactive.
    pub itimer_real_expire_ms: usize,
    /// ITIMER_REAL: interval for repeating timer in ms, 0 = one-shot.
    pub itimer_real_interval_ms: usize,
    /// Parent process waiting on clone(CLONE_VM|CLONE_VFORK) synchronization.
    pub vfork_vm_parent: Option<Weak<ProcessControlBlock>>,
}

impl ProcessControlBlockInner {
    pub fn get_user_token(&self) -> usize {
        self.memory_set.token()
    }

    /// Allocate a new file descriptor. Returns None if RLIMIT_NOFILE is reached.
    pub fn alloc_fd(&mut self) -> Option<usize> {
        let limit = self.rlimits[RLIMIT_NOFILE].rlim_cur as usize;
        if let Some(fd) = (0..self.fd_table.len()).find(|fd| self.fd_table[*fd].is_none()) {
            if fd < limit {
                Some(fd)
            } else {
                None
            }
        } else {
            let new_fd = self.fd_table.len();
            if new_fd >= limit {
                return None;
            }
            self.fd_table.push(None);
            Some(new_fd)
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
    #[cfg(target_arch = "loongarch64")]
    fn alloc_minimal_tcb(memory_set: &mut MemorySet) -> usize {
        let base = LOONGARCH_MIN_TCB_ADDR;
        let end = base + crate::config::PAGE_SIZE;
        memory_set.insert_framed_area(
            base.into(),
            end.into(),
            crate::mm::MapPermission::R | crate::mm::MapPermission::W | crate::mm::MapPermission::U,
        );
        let token = memory_set.token();
        *translated_refmut(token, base as *mut usize) = 0;
        *translated_refmut(token, (base + 8) as *mut usize) = base;
        base
    }

    #[cfg(target_arch = "loongarch64")]
    fn alloc_minimal_tcb_if_needed(
        memory_set: &mut MemorySet,
        tls_area: &Option<TlsArea>,
    ) -> Option<usize> {
        if tls_area.is_none() {
            Some(Self::alloc_minimal_tcb(memory_set))
        } else {
            None
        }
    }

    #[cfg(not(target_arch = "loongarch64"))]
    fn alloc_minimal_tcb_if_needed(
        _memory_set: &mut MemorySet,
        _tls_area: &Option<TlsArea>,
    ) -> Option<usize> {
        None
    }

    #[cfg(target_arch = "loongarch64")]
    fn fallback_tcb_addr_if_no_tls(
        _token: usize,
        _ustack_top: usize,
        minimal_tcb: Option<usize>,
    ) -> Option<usize> {
        minimal_tcb
    }

    #[cfg(not(target_arch = "loongarch64"))]
    fn fallback_tcb_addr_if_no_tls(
        token: usize,
        ustack_top: usize,
        _minimal_tcb: Option<usize>,
    ) -> Option<usize> {
        let tcb_addr = ustack_top - 16;
        *translated_refmut(token, tcb_addr as *mut usize) = 0;
        *translated_refmut(token, (tcb_addr + 8) as *mut usize) = tcb_addr;
        Some(tcb_addr)
    }

    pub fn inner_exclusive_access(&self) -> UPIntrRefMut<'_, ProcessControlBlockInner> {
        self.inner.exclusive_access()
    }

    /// Try to borrow the process inner state; returns None if already borrowed.
    pub fn try_inner_exclusive_access(&self) -> Option<UPIntrRefMut<'_, ProcessControlBlockInner>> {
        self.inner.try_exclusive_access()
    }

    pub fn new(elf_data: &[u8]) -> Arc<Self> {
        let (mut memory_set, heap_bottom, user_stack_top, entry_point, tls_info, _auxv_info) =
            MemorySet::from_elf(elf_data);
        let ustack_base = user_stack_top.saturating_sub(USER_STACK_SIZE);
        let load_base = if let Ok(elf) = ElfFile::new(elf_data) {
            entry_point.saturating_sub(elf.header.pt2.entry_point() as usize)
        } else {
            0
        };
        let gp = find_global_pointer(elf_data)
            .map(|v| v + load_base)
            .unwrap_or(0);

        // Initialize TLS if PT_TLS segment exists
        let tls_area = tls_info.map(|info| TlsArea::new(&info, &mut memory_set, elf_data));
        let minimal_tcb = Self::alloc_minimal_tcb_if_needed(&mut memory_set, &tls_area);

        let pid_handle = pid_alloc();
        let process = Arc::new(Self {
            pid: pid_handle,
            inner: unsafe {
                UPIntrFreeCell::new(ProcessControlBlockInner {
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
                    signal_actions: SignalActions::default(),
                    tasks: Vec::new(),
                    task_res_allocator: RecycleAllocator::new(),
                    mutex_list: Vec::new(),
                    semaphore_list: Vec::new(),
                    condvar_list: Vec::new(),
                    name: String::from("initproc"),
                    cwd: String::from("/"),
                    root_dir: String::from("/"),
                    real_uid: 0,
                    effective_uid: 0,
                    saved_uid: 0,
                    fs_uid: 0,
                    real_gid: 0,
                    effective_gid: 0,
                    saved_gid: 0,
                    fs_gid: 0,
                    nice: 0,
                    cap_permitted: u64::MAX,
                    cap_effective: u64::MAX,
                    cap_inheritable: 0,
                    cap_bounding: u64::MAX,
                    heap_bottom,
                    program_brk: heap_bottom,
                    mmap_base: USER_MMAP_TOP,
                    tls_area: tls_area.clone(),
                    rlimits: default_rlimits(),
                    itimers: [IntervalTimerState::default(); 3],
                    session_id: 0,
                    pgid: 0,
                    ptrace_traceme: false,
                    ptrace_stop_signal: None,
                    itimer_real_expire_ms: 0,
                    itimer_real_interval_ms: 0,
                    vfork_vm_parent: None,
                })
            },
        });
        // Init process is its own session leader and process group leader
        {
            let mut inner = process.inner_exclusive_access();
            inner.session_id = process.pid.0;
            inner.pgid = process.pid.0;
        }
        let task = Arc::new(TaskControlBlock::new(Arc::clone(&process), ustack_base, false));
        let task_inner = task.inner_exclusive_access();
        let trap_cx = task_inner.get_trap_cx();
        let ustack_top = user_stack_top;
        drop(task_inner);
        let mut trap_cx_value = TrapContext::app_init_context(entry_point, ustack_top);

        if gp != 0 {
            trap_cx_value.set_gp(gp);
            info!("[kernel] GP initialized: gp = {:#x}", gp);
        }
        if let Some(ref tls) = tls_area {
            trap_cx_value[TrapFrameArgs::TLS] = tls.tp_value;
            info!("[kernel] TLS initialized: tp = {:#x}", tls.tp_value);
        } else {
            let token = process.inner_exclusive_access().memory_set.token();
            let tcb_addr =
                Self::fallback_tcb_addr_if_no_tls(token, ustack_top, minimal_tcb).unwrap_or(0);
            trap_cx_value[TrapFrameArgs::TLS] = tcb_addr;
            info!(
                "[kernel] Minimal TCB initialized (no PT_TLS): tp = {:#x}",
                tcb_addr
            );
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
        self.exec_with_interp(elf_data, None, args, envs);
    }

    pub fn exec_with_interp(
        self: &Arc<Self>,
        elf_data: &[u8],
        interp_data: Option<&[u8]>,
        args: Vec<String>,
        envs: Vec<String>,
    ) {
        assert_eq!(self.inner_exclusive_access().thread_count(), 1);
        let exec_name = self.inner_exclusive_access().name.clone();
        debug!(
            "[kernel] exec: process name={} argc={}",
            exec_name,
            args.len()
        );
        let (
            mut memory_set,
            heap_bottom,
            user_stack_top,
            main_entry,
            tls_info,
            auxv_info,
            interp_base,
            interp_entry,
        ) = if let Some(interp_data) = interp_data {
            let (
                memory_set,
                heap_bottom,
                user_stack_top,
                entry_point,
                tls_info,
                auxv_info,
                interp_base,
                interp_entry,
            ) = MemorySet::from_elf_with_interp(elf_data, interp_data);
            (
                memory_set,
                heap_bottom,
                user_stack_top,
                entry_point,
                tls_info,
                auxv_info,
                Some(interp_base),
                Some(interp_entry),
            )
        } else {
            let (memory_set, heap_bottom, user_stack_top, entry_point, tls_info, auxv_info) =
                MemorySet::from_elf(elf_data);
            (
                memory_set,
                heap_bottom,
                user_stack_top,
                entry_point,
                tls_info,
                auxv_info,
                None,
                None,
            )
        };
        let entry_point = interp_entry.unwrap_or(main_entry);
        let load_base = if let Ok(elf) = ElfFile::new(elf_data) {
            main_entry.saturating_sub(elf.header.pt2.entry_point() as usize)
        } else {
            0
        };
        let gp = if let Some(interp_data) = interp_data {
            if let Ok(_interp_elf) = ElfFile::new(interp_data) {
                let interp_load_base = interp_base.unwrap_or(0);
                find_global_pointer(interp_data)
                    .map(|v| v + interp_load_base)
                    .unwrap_or(0)
            } else {
                0
            }
        } else {
            find_global_pointer(elf_data)
                .map(|v| v + load_base)
                .unwrap_or(0)
        };
        debug!(
            "[kernel] exec: entry={:#x} heap_bottom={:#x} user_stack_top={:#x} auxv_phdr={:#x} auxv_entry={:#x}",
            main_entry,
            heap_bottom,
            user_stack_top,
            auxv_info.phdr_addr,
            auxv_info.entry
        );

        // Initialize TLS if PT_TLS segment exists
        let tls_area = tls_info.map(|info| TlsArea::new(&info, &mut memory_set, elf_data));

        let minimal_tcb = Self::alloc_minimal_tcb_if_needed(&mut memory_set, &tls_area);

        let new_token = memory_set.token();
        if exec_name == "sh" || exec_name == "busybox" {
            let mut bytes = [0u8; 8];
            let mut offset = 0usize;
            let debug_entry = main_entry;
            for slice in translated_byte_buffer(new_token, debug_entry as *const u8, bytes.len()) {
                let len = slice.len().min(bytes.len() - offset);
                bytes[offset..offset + len].copy_from_slice(&slice[..len]);
                offset += len;
                if offset >= bytes.len() {
                    break;
                }
            }
            debug!("[kernel] exec: entry mem bytes={:02x?}", bytes);
        }
        let ustack_base = user_stack_top.saturating_sub(USER_STACK_SIZE);
        debug!(
            "[kernel] exec: token={:#x} ustack_base={:#x} tls={}",
            new_token,
            ustack_base,
            tls_area.is_some()
        );
        {
            let mut inner = self.inner_exclusive_access();
            inner.memory_set = memory_set;
            inner.heap_bottom = heap_bottom;
            inner.program_brk = heap_bottom;
            // Exec resets signal dispositions to default, except SIG_IGN.
            for action in inner.signal_actions.table.iter_mut().skip(1) {
                if action.handler != 1 {
                    *action = SignalAction::default();
                }
            }
            inner.mmap_base = USER_MMAP_TOP;
            inner.tls_area = tls_area.clone();
            inner.itimers = [IntervalTimerState::default(); 3];
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
        for (idx, addr) in arg_addrs.iter().take(4).enumerate() {
            debug!("[kernel] exec: argv[{}] ptr={:#x}", idx, addr);
        }
        for (idx, addr) in env_addrs.iter().take(4).enumerate() {
            debug!("[kernel] exec: envp[{}] ptr={:#x}", idx, addr);
        }
        user_sp &= !0xf;
        let word_size = core::mem::size_of::<usize>();

        // Reserve 16 random bytes for AT_RANDOM (needed by musl malloc).
        user_sp = user_sp.saturating_sub(16);
        user_sp &= !0xf;
        let random_addr = user_sp;
        for i in 0..16 {
            *translated_refmut(new_token, (random_addr + i) as *mut u8) = (i * 17) as u8;
        }

        // Allocate a minimal TCB for programs without PT_TLS so tp is valid.
        let tp_value: Option<usize> = if tls_area.is_none() {
            #[cfg(not(target_arch = "loongarch64"))]
            {
                user_sp = user_sp.saturating_sub(16);
                user_sp &= !0xf;
            }
            Self::fallback_tcb_addr_if_no_tls(new_token, user_sp, minimal_tcb)
        } else {
            None
        };

        // Prepare auxiliary vectors to be placed right after envp.
        let mut auxv_entries = if tls_area.is_some() {
            auxv_info.to_entries(crate::config::PAGE_SIZE)
        } else {
            use crate::task::auxv::auxv_type::*;
            vec![
                (AT_ENTRY, auxv_info.entry),
                (AT_PHDR, auxv_info.phdr_addr),
                (AT_PHENT, auxv_info.phent_size),
                (AT_PHNUM, auxv_info.phnum),
                (AT_PAGESZ, crate::config::PAGE_SIZE),
                (AT_UID, 0),
                (AT_EUID, 0),
                (AT_GID, 0),
                (AT_EGID, 0),
                (AT_SECURE, 0),
                (AT_RANDOM, 0),
                (AT_NULL, 0),
            ]
        };
        if let Some(base) = interp_base {
            let at_base = crate::task::auxv::auxv_type::AT_BASE;
            if let Some(pos) = auxv_entries
                .iter()
                .position(|(k, _)| *k == crate::task::auxv::auxv_type::AT_NULL)
            {
                auxv_entries.insert(pos, (at_base, base));
            } else {
                auxv_entries.push((at_base, base));
            }
        }
        for entry in &mut auxv_entries {
            if entry.0 == crate::task::auxv::auxv_type::AT_RANDOM {
                entry.1 = random_addr;
            }
        }

        // Linux ABI stack layout (from low address/high stack):
        // [argc][argv...][NULL][envp...][NULL][auxv...]
        // sp points to argc.
        let argc = arg_addrs.len();
        let argv_size = (argc + 1) * word_size;
        let envp_size = (env_addrs.len() + 1) * word_size;
        let auxv_size = auxv_entries.len() * 2 * word_size;
        let total_size = word_size + argv_size + envp_size + auxv_size;
        user_sp = user_sp.saturating_sub(total_size);
        user_sp &= !0xf;

        // Now write from low address to high
        let mut current_sp = user_sp;

        // Write argc
        *translated_refmut(new_token, current_sp as *mut usize) = argc;
        current_sp += word_size;

        // Write argv pointers
        let argv_base = current_sp;
        for addr in arg_addrs.iter() {
            *translated_refmut(new_token, current_sp as *mut usize) = *addr;
            current_sp += word_size;
        }
        *translated_refmut(new_token, current_sp as *mut usize) = 0; // argv NULL terminator
        current_sp += word_size;

        // Write envp pointers
        let envp_base = current_sp;
        for addr in env_addrs.iter() {
            *translated_refmut(new_token, current_sp as *mut usize) = *addr;
            current_sp += word_size;
        }
        *translated_refmut(new_token, current_sp as *mut usize) = 0; // envp NULL terminator
        current_sp += word_size;

        // Write auxv entries immediately after envp NULL terminator.
        let auxv_base = current_sp;
        for (i, (aux_type, aux_val)) in auxv_entries.iter().enumerate() {
            *translated_refmut(new_token, (auxv_base + i * 2 * word_size) as *mut usize) =
                *aux_type;
            *translated_refmut(
                new_token,
                (auxv_base + i * 2 * word_size + word_size) as *mut usize,
            ) = *aux_val;
        }

        let argc_mem = *translated_ref(new_token, user_sp as *const usize);
        let argv0_mem = *translated_ref(new_token, argv_base as *const usize);
        let envp0_mem = *translated_ref(new_token, envp_base as *const usize);
        if args.get(0).map(|s| s.as_str()) == Some("sh") {
            let argv1_mem = *translated_ref(new_token, (argv_base + word_size) as *const usize);
            let argv0_str = translated_str(new_token, argv0_mem as *const u8);
            let argv1_str = if argv1_mem == 0 {
                String::from("<null>")
            } else {
                translated_str(new_token, argv1_mem as *const u8)
            };
            debug!(
                "[kernel] exec: argv0_str={} argv1_str={} argc={}",
                argv0_str, argv1_str, argc
            );
        }
        info!(
            "[kernel] exec: sp={:#x}, argc={}, argv_base={:#x}, envp_base={:#x}",
            user_sp, argc, argv_base, envp_base
        );
        info!(
            "[kernel] exec: auxv_base={:#x}, auxv_entries={}",
            auxv_base,
            auxv_entries.len()
        );
        info!(
            "[kernel] exec: argc@sp={} argv0@argv_base={:#x} envp0@envp_base={:#x}",
            argc_mem, argv0_mem, envp0_mem
        );
        info!(
            "[kernel] exec: argv[0]={:#x}, argv[1]={:#x}",
            if argc > 0 { arg_addrs[0] } else { 0 },
            if argc > 1 { arg_addrs[1] } else { 0 }
        );

        let mut trap_cx = TrapContext::app_init_context(
            entry_point,
            user_sp, // sp should point to argc
        );
        trap_cx[TrapFrameArgs::ARG0] = argc;
        trap_cx[TrapFrameArgs::ARG1] = argv_base;
        trap_cx[TrapFrameArgs::ARG2] = envp_base;

        if gp != 0 {
            trap_cx.set_gp(gp);
            info!("[kernel] exec: GP initialized: gp = {:#x}", gp);
        }
        // Set tp register
        if let Some(ref tls) = tls_area {
            trap_cx[TrapFrameArgs::TLS] = tls.tp_value;
            info!("[kernel] exec: TLS initialized: tp = {:#x}", tls.tp_value);
        } else if let Some(tcb_addr) = tp_value {
            trap_cx[TrapFrameArgs::TLS] = tcb_addr;
            info!(
                "[kernel] exec: Minimal TCB initialized (no PT_TLS): tp = {:#x}",
                tcb_addr
            );
        }

        *task_inner.get_trap_cx() = trap_cx;
    }

    pub fn fork(self: &Arc<Self>) -> Arc<Self> {
        let mut parent = self.inner_exclusive_access();
        if parent.thread_count() != 1 {
            warn!(
                "[fork-stage] pid={} fork from multi-thread process (thread_count={}), continuing with single-thread child",
                self.pid.0,
                parent.thread_count()
            );
        }
        // info!(
        //     "[fork-stage] pid={} start: children={} tasks={} fd_table_len={}",
        //     self.pid.0,
        //     parent.children.len(),
        //     parent.tasks.iter().filter(|t| t.is_some()).count(),
        //     parent.fd_table.len()
        // );
        // info!("[fork-stage] pid={} before memory_set clone", self.pid.0);
        let memory_set = MemorySet::from_existed_user(&mut parent.memory_set);
        // info!("[fork-stage] pid={} after memory_set clone", self.pid.0);

        // TLS pages are already cloned via MemorySet::from_existed_user.
        let tls_area = parent.tls_area.clone();

        let pid = pid_alloc();
        info!("[fork-stage] pid={} before fd_table clone", self.pid.0);
        let mut new_fd_table: Vec<Option<Arc<dyn File + Send + Sync>>> = Vec::new();
        for fd in parent.fd_table.iter() {
            if let Some(file) = fd {
                new_fd_table.push(Some(file.clone()));
            } else {
                new_fd_table.push(None);
            }
        }
        info!(
            "[fork-stage] pid={} after fd_table clone new_len={}",
            self.pid.0,
            new_fd_table.len()
        );
        let _new_pid_value = pid.0;
        let child = Arc::new(Self {
            pid,
            inner: unsafe {
                UPIntrFreeCell::new(ProcessControlBlockInner {
                    is_zombie: false,
                    memory_set,
                    parent: Some(Arc::downgrade(self)),
                    children: Vec::new(),
                    exit_code: 0,
                    fd_table: new_fd_table,
                    signals: SignalFlags::empty(),
                    signal_pending: SignalFlags::empty(),
                    signal_actions: parent.signal_actions.clone(),
                    tasks: Vec::new(),
                    task_res_allocator: RecycleAllocator::new(),
                    mutex_list: Vec::new(),
                    semaphore_list: Vec::new(),
                    condvar_list: Vec::new(),
                    name: parent.name.clone(),
                    cwd: parent.cwd.clone(),
                    root_dir: parent.root_dir.clone(),
                    real_uid: parent.real_uid,
                    effective_uid: parent.effective_uid,
                    saved_uid: parent.saved_uid,
                    fs_uid: parent.fs_uid,
                    real_gid: parent.real_gid,
                    effective_gid: parent.effective_gid,
                    saved_gid: parent.saved_gid,
                    fs_gid: parent.fs_gid,
                    nice: parent.nice,
                    cap_permitted: parent.cap_permitted,
                    cap_effective: parent.cap_effective,
                    cap_inheritable: parent.cap_inheritable,
                    cap_bounding: parent.cap_bounding,
                    heap_bottom: parent.heap_bottom,
                    program_brk: parent.program_brk,
                    mmap_base: parent.mmap_base,
                    tls_area,
                    rlimits: parent.rlimits,
                    itimers: [IntervalTimerState::default(); 3],
                    session_id: parent.session_id,
                    pgid: parent.pgid,
                    ptrace_traceme: false,
                    ptrace_stop_signal: None,
                    itimer_real_expire_ms: 0,
                    itimer_real_interval_ms: 0,
                    vfork_vm_parent: None,
                })
            },
        });
        // info!("[fork-stage] pid={} child pcb allocated new_pid={}", self.pid.0, new_pid_value);
        parent.children.push(Arc::clone(&child));
        info!("[fork-stage] pid={} child linked to parent", self.pid.0);
        let ustack_base = parent
            .get_task(0)
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .ustack_base;
        // 获取父线程的 signal_mask，用于子进程继承
        let parent_signal_mask = parent.get_task(0).inner_exclusive_access().signal_mask;
        let task = Arc::new(TaskControlBlock::new(
            Arc::clone(&child),
            ustack_base,
            false,
        ));
        // info!("[fork-stage] pid={} child task allocated", self.pid.0);
        let mut child_inner = child.inner_exclusive_access();
        child_inner.tasks.push(Some(Arc::clone(&task)));
        // info!("[fork-stage] pid={} child task linked", self.pid.0);
        drop(child_inner);
        let mut task_inner = task.inner_exclusive_access();
        // 子进程继承父进程的信号掩码（Linux 语义：fork 继承 signal_mask）
        task_inner.signal_mask = parent_signal_mask;
        drop(task_inner);
        insert_into_pid2process(child.getpid(), Arc::clone(&child));
        // info!("[fork-stage] pid={} child inserted pid2process", self.pid.0);
        add_task(task);
        // info!("[fork-stage] pid={} child added ready queue", self.pid.0);
        child
    }

    pub fn getpid(&self) -> usize {
        self.pid.0
    }
}
