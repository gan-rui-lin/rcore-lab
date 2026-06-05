use super::id::RecycleAllocator;
use super::manager::insert_into_pid2process;
use super::{
    add_task, pid_alloc, PidHandle, SignalAction, SignalActions, SignalFlags, TaskControlBlock,
    TlsArea, MAX_SIG,
};
use crate::config::{PAGE_SIZE, USER_MMAP_TOP, USER_STACK_SIZE};
use crate::fs::{File, Stdin, Stdout};
use crate::mm::{
    translated_byte_buffer_checked, translated_ref, translated_refmut, translated_str, MemorySet,
};
use crate::sync::{Condvar, Mutex, Semaphore, UPIntrMutex, UPIntrRwLock};
use alloc::string::String;
use alloc::sync::{Arc, Weak};
use alloc::vec;
use alloc::vec::Vec;
use arch::{TrapContext, TrapFrameArgs};
use xmas_elf::sections::{SectionData, ShType};
use xmas_elf::symbol_table::Entry;
use xmas_elf::ElfFile;

fn align_up(value: usize, align: usize) -> usize {
    (value + align - 1) & !(align - 1)
}

#[cfg(target_arch = "loongarch64")]
const LOONGARCH_MIN_TCB_ADDR: usize = 0x7000_1000;

pub const RLIMIT_NLIMITS: usize = 16;
pub const RLIMIT_STACK: usize = 3;
pub const RLIMIT_NOFILE: usize = 7;
pub const RLIM_INFINITY: u64 = u64::MAX;
pub const DEFAULT_TIMER_SLACK_NS: usize = 50_000;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChildWaitEvent {
    Stopped(i32),
    Continued(i32),
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
    memory: UPIntrRwLock<ProcessMemory>,
    fs: UPIntrRwLock<ProcessFs>,
    identity: UPIntrRwLock<ProcessIdentity>,
    sync_objects: UPIntrMutex<ProcessSyncObjects>,
    limits: UPIntrRwLock<ProcessLimits>,
    timers: UPIntrRwLock<ProcessTimers>,
    threads: UPIntrMutex<ProcessThreads>,
    signals: UPIntrMutex<ProcessSignals>,
    family: UPIntrMutex<ProcessFamily>,
}

pub struct ProcessSyncObjects {
    pub mutex_list: Vec<Option<Arc<dyn Mutex>>>,
    pub semaphore_list: Vec<Option<Arc<Semaphore>>>,
    pub condvar_list: Vec<Option<Arc<Condvar>>>,
}

pub struct ProcessLimits {
    pub rlimits: [RLimit; RLIMIT_NLIMITS],
}

pub struct ProcessTimers {
    pub itimers: [IntervalTimerState; 3],
    /// ITIMER_REAL: absolute expire time in ms, 0 = inactive.
    pub itimer_real_expire_ms: usize,
    /// ITIMER_REAL: interval for repeating timer in ms, 0 = one-shot.
    pub itimer_real_interval_ms: usize,
    pub timer_slack_ns: usize,
    pub default_timer_slack_ns: usize,
}

pub struct ProcessMemory {
    pub memory_set: MemorySet,
    pub heap_bottom: usize,
    pub program_brk: usize,
    pub mmap_base: usize,
    pub tls_area: Option<TlsArea>,
}

pub struct ProcessSignals {
    pub signals: SignalFlags,
    pub signal_pending: SignalFlags,
    /// Lightweight siginfo metadata for process-level pending signals.
    /// Index `i` corresponds to signal number `i + 1`.
    pub pending_signal_sender_pid: [i32; MAX_SIG],
    pub pending_signal_si_code: [i32; MAX_SIG],
    pub signal_actions: SignalActions,
}

pub struct ProcessFamily {
    pub is_zombie: bool,
    pub parent: Option<Weak<ProcessControlBlock>>,
    pub children: Vec<Arc<ProcessControlBlock>>,
    pub exit_code: i32,
    /// Set by ptrace(PTRACE_TRACEME).
    pub ptrace_traceme: bool,
    /// Pending ptrace stop signal to be reported by waitpid.
    pub ptrace_stop_signal: Option<i32>,
    /// Pending non-exit child event to be reported by waitid(WSTOPPED/WCONTINUED).
    pub child_wait_event: Option<ChildWaitEvent>,
    /// True when the process is currently in a job-control stopped state.
    pub group_stopped: bool,
    /// Parent process waiting on clone(CLONE_VM|CLONE_VFORK) synchronization.
    pub vfork_vm_parent: Option<Weak<ProcessControlBlock>>,
}

#[derive(Clone)]
pub struct ProcessFs {
    pub fd_table: Vec<Option<Arc<dyn File + Send + Sync>>>,
    pub cwd: String,
    pub root_dir: String,
}

#[derive(Clone)]
pub struct ProcessIdentity {
    pub name: String,
    pub real_uid: u32,
    pub effective_uid: u32,
    pub saved_uid: u32,
    pub fs_uid: u32,
    pub real_gid: u32,
    pub effective_gid: u32,
    pub saved_gid: u32,
    pub fs_gid: u32,
    pub supplementary_groups: Vec<u32>,
    /// Nice value in Linux range [-20, 19], default 0.
    pub nice: i32,
    /// Capability sets (bit mask, Linux kernel format).
    pub cap_permitted: u64,
    pub cap_effective: u64,
    pub cap_inheritable: u64,
    pub cap_bounding: u64,
    pub session_id: usize,
    pub pgid: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct ProcessCredentials {
    pub real_uid: u32,
    pub effective_uid: u32,
    pub saved_uid: u32,
    pub fs_uid: u32,
    pub real_gid: u32,
    pub effective_gid: u32,
    pub saved_gid: u32,
    pub fs_gid: u32,
    pub cap_permitted: u64,
    pub cap_effective: u64,
    pub cap_inheritable: u64,
    pub cap_bounding: u64,
}

pub struct ProcessThreads {
    pub tasks: Vec<Option<Arc<TaskControlBlock>>>,
    pub task_res_allocator: RecycleAllocator,
}

impl ProcessSignals {
    #[inline]
    pub fn set_pending_signal_siginfo(&mut self, signum: usize, sender_pid: i32, si_code: i32) {
        if signum == 0 || signum > MAX_SIG {
            return;
        }
        let idx = signum - 1;
        self.pending_signal_sender_pid[idx] = sender_pid;
        self.pending_signal_si_code[idx] = si_code;
    }

    #[inline]
    pub fn get_pending_signal_siginfo(&self, signum: usize) -> (i32, i32) {
        if signum == 0 || signum > MAX_SIG {
            return (0, 0);
        }
        let idx = signum - 1;
        (
            self.pending_signal_sender_pid[idx],
            self.pending_signal_si_code[idx],
        )
    }

    #[inline]
    pub fn clear_pending_signal_siginfo(&mut self, signum: usize) {
        if signum == 0 || signum > MAX_SIG {
            return;
        }
        let idx = signum - 1;
        self.pending_signal_sender_pid[idx] = 0;
        self.pending_signal_si_code[idx] = 0;
    }
}

impl ProcessFs {
    /// Allocate a new file descriptor. Returns None if RLIMIT_NOFILE is reached.
    pub fn alloc_fd(&mut self, limit: usize) -> Option<usize> {
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
}

impl ProcessIdentity {
    pub fn credentials_snapshot(&self) -> ProcessCredentials {
        ProcessCredentials {
            real_uid: self.real_uid,
            effective_uid: self.effective_uid,
            saved_uid: self.saved_uid,
            fs_uid: self.fs_uid,
            real_gid: self.real_gid,
            effective_gid: self.effective_gid,
            saved_gid: self.saved_gid,
            fs_gid: self.fs_gid,
            cap_permitted: self.cap_permitted,
            cap_effective: self.cap_effective,
            cap_inheritable: self.cap_inheritable,
            cap_bounding: self.cap_bounding,
        }
    }
}

impl ProcessThreads {
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

    pub fn tasks_snapshot(&self) -> Vec<Arc<TaskControlBlock>> {
        self.tasks
            .iter()
            .filter_map(|task| task.as_ref().map(Arc::clone))
            .collect()
    }

    pub fn insert_task(&mut self, tid: usize, task: Arc<TaskControlBlock>) {
        while self.tasks.len() < tid + 1 {
            self.tasks.push(None);
        }
        self.tasks[tid] = Some(task);
    }

    pub fn remove_task(&mut self, tid: usize) -> Option<Arc<TaskControlBlock>> {
        self.tasks.get_mut(tid).and_then(Option::take)
    }

    pub fn all_tasks_exited(&self) -> bool {
        self.tasks
            .iter()
            .filter_map(|task| task.as_ref())
            .all(|task| task.exit_code().is_some())
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

    pub fn with_sync_objects<R>(&self, f: impl FnOnce(&ProcessSyncObjects) -> R) -> R {
        let sync_objects = self.sync_objects.lock();
        f(&sync_objects)
    }

    pub fn with_sync_objects_mut<R>(&self, f: impl FnOnce(&mut ProcessSyncObjects) -> R) -> R {
        let mut sync_objects = self.sync_objects.lock();
        f(&mut sync_objects)
    }

    pub fn try_with_sync_objects_mut<R>(
        &self,
        f: impl FnOnce(&mut ProcessSyncObjects) -> R,
    ) -> Option<R> {
        let mut sync_objects = self.sync_objects.try_lock()?;
        Some(f(&mut sync_objects))
    }

    pub fn with_limits<R>(&self, f: impl FnOnce(&ProcessLimits) -> R) -> R {
        let limits = self.limits.read();
        f(&limits)
    }

    pub fn with_limits_mut<R>(&self, f: impl FnOnce(&mut ProcessLimits) -> R) -> R {
        let mut limits = self.limits.write();
        f(&mut limits)
    }

    pub fn with_timers<R>(&self, f: impl FnOnce(&ProcessTimers) -> R) -> R {
        let timers = self.timers.read();
        f(&timers)
    }

    pub fn with_timers_mut<R>(&self, f: impl FnOnce(&mut ProcessTimers) -> R) -> R {
        let mut timers = self.timers.write();
        f(&mut timers)
    }

    pub fn try_with_timers_mut<R>(&self, f: impl FnOnce(&mut ProcessTimers) -> R) -> Option<R> {
        let mut timers = self.timers.try_write()?;
        Some(f(&mut timers))
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
        crate::syscall::sched_reset_process_state(pid_handle.0);
        let process = Arc::new(Self {
            pid: pid_handle,
            memory: unsafe {
                UPIntrRwLock::new(ProcessMemory {
                    memory_set,
                    heap_bottom,
                    program_brk: heap_bottom,
                    mmap_base: USER_MMAP_TOP,
                    tls_area: tls_area.clone(),
                })
            },
            fs: unsafe {
                UPIntrRwLock::new(ProcessFs {
                    fd_table: vec![
                        Some(Arc::new(Stdin)),
                        Some(Arc::new(Stdout)),
                        Some(Arc::new(Stdout)),
                    ],
                    cwd: String::from("/"),
                    root_dir: String::from("/"),
                })
            },
            identity: unsafe {
                UPIntrRwLock::new(ProcessIdentity {
                    name: String::from("initproc"),
                    real_uid: 0,
                    effective_uid: 0,
                    saved_uid: 0,
                    fs_uid: 0,
                    real_gid: 0,
                    effective_gid: 0,
                    saved_gid: 0,
                    fs_gid: 0,
                    supplementary_groups: vec![0],
                    nice: 0,
                    cap_permitted: u64::MAX,
                    cap_effective: u64::MAX,
                    cap_inheritable: 0,
                    cap_bounding: u64::MAX,
                    session_id: 0,
                    pgid: 0,
                })
            },
            sync_objects: unsafe {
                UPIntrMutex::new(ProcessSyncObjects {
                    mutex_list: Vec::new(),
                    semaphore_list: Vec::new(),
                    condvar_list: Vec::new(),
                })
            },
            limits: unsafe {
                UPIntrRwLock::new(ProcessLimits {
                    rlimits: default_rlimits(),
                })
            },
            timers: unsafe {
                UPIntrRwLock::new(ProcessTimers {
                    itimers: [IntervalTimerState::default(); 3],
                    itimer_real_expire_ms: 0,
                    itimer_real_interval_ms: 0,
                    timer_slack_ns: DEFAULT_TIMER_SLACK_NS,
                    default_timer_slack_ns: DEFAULT_TIMER_SLACK_NS,
                })
            },
            threads: unsafe {
                UPIntrMutex::new(ProcessThreads {
                    tasks: Vec::new(),
                    task_res_allocator: RecycleAllocator::new(),
                })
            },
            signals: unsafe {
                UPIntrMutex::new(ProcessSignals {
                    signals: SignalFlags::empty(),
                    signal_pending: SignalFlags::empty(),
                    pending_signal_sender_pid: [0; MAX_SIG],
                    pending_signal_si_code: [0; MAX_SIG],
                    signal_actions: SignalActions::default(),
                })
            },
            family: unsafe {
                UPIntrMutex::new(ProcessFamily {
                    is_zombie: false,
                    parent: None,
                    children: Vec::new(),
                    exit_code: 0,
                    ptrace_traceme: false,
                    ptrace_stop_signal: None,
                    child_wait_event: None,
                    group_stopped: false,
                    vfork_vm_parent: None,
                })
            },
        });
        // Init process: session_id=0, pgid=0 (matches Linux /proc/1/stat behavior)
        {
            let mut identity = process.identity.write();
            identity.session_id = 0;
            identity.pgid = 0;
        }
        let task = Arc::new(TaskControlBlock::new(
            Arc::clone(&process),
            ustack_base,
            false,
        ));
        let ustack_top = user_stack_top;
        let mut trap_cx_value = TrapContext::app_init_context(entry_point, ustack_top);

        if gp != 0 {
            trap_cx_value.set_gp(gp);
            info!("[kernel] GP initialized: gp = {:#x}", gp);
        }
        if let Some(ref tls) = tls_area {
            trap_cx_value[TrapFrameArgs::TLS] = tls.tp_value;
            info!("[kernel] TLS initialized: tp = {:#x}", tls.tp_value);
        } else {
            let token = process.get_user_token();
            let tcb_addr =
                Self::fallback_tcb_addr_if_no_tls(token, ustack_top, minimal_tcb).unwrap_or(0);
            trap_cx_value[TrapFrameArgs::TLS] = tcb_addr;
            info!(
                "[kernel] Minimal TCB initialized (no PT_TLS): tp = {:#x}",
                tcb_addr
            );
        }

        task.with_trap_cx_mut(|trap_cx| *trap_cx = trap_cx_value);
        process.insert_task(0, Arc::clone(&task));
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
        assert_eq!(self.thread_count(), 1);
        let exec_name = self.name();
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
            let debug_entry = main_entry;
            if let Some(slices) = translated_byte_buffer_checked(
                new_token,
                debug_entry as *const u8,
                bytes.len(),
                false,
            ) {
                let mut offset = 0usize;
                for slice in slices {
                    let len = slice.len().min(bytes.len() - offset);
                    bytes[offset..offset + len].copy_from_slice(&slice[..len]);
                    offset += len;
                    if offset >= bytes.len() {
                        break;
                    }
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
        self.reset_memory_after_exec(memory_set, heap_bottom, tls_area.clone());
        self.with_timers_mut(|timers| timers.itimers = [IntervalTimerState::default(); 3]);
        {
            let mut signals = self.signals.lock();
            // Exec resets signal dispositions to default, except SIG_IGN.
            for action in signals.signal_actions.table.iter_mut().skip(1) {
                if action.handler != 1 {
                    *action = SignalAction::default();
                }
            }
        }
        let task = self.get_task(0);
        task.set_ustack_base_and_refresh_trap_cx_ppn(ustack_base);
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

        task.with_trap_cx_mut(|task_trap_cx| *task_trap_cx = trap_cx);
    }

    pub fn fork(self: &Arc<Self>) -> Arc<Self> {
        let thread_count = self.thread_count();
        if thread_count != 1 {
            warn!(
                "[fork-stage] pid={} fork from multi-thread process (thread_count={}), continuing with single-thread child",
                self.pid.0,
                thread_count
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
        let parent_signal_actions =
            self.with_process_signals(|signals| signals.signal_actions.clone());
        let (memory_set, heap_bottom, program_brk, mmap_base, tls_area) =
            self.clone_memory_for_fork();
        let rlimits = self.with_limits(|limits| limits.rlimits);
        // info!("[fork-stage] pid={} after memory_set clone", self.pid.0);

        let pid = pid_alloc();
        crate::syscall::sched_reset_process_state(pid.0);
        info!("[fork-stage] pid={} before fd_table clone", self.pid.0);
        let parent_fs = self.fs.read().clone();
        let parent_identity = self.identity.read().clone();
        let parent_timer_slack_ns = self.with_timers(|timers| timers.timer_slack_ns);
        let new_fd_table = parent_fs.fd_table.clone();
        info!(
            "[fork-stage] pid={} after fd_table clone new_len={}",
            self.pid.0,
            new_fd_table.len()
        );
        let _new_pid_value = pid.0;
        let child = Arc::new(Self {
            pid,
            memory: unsafe {
                UPIntrRwLock::new(ProcessMemory {
                    memory_set,
                    heap_bottom,
                    program_brk,
                    mmap_base,
                    tls_area,
                })
            },
            fs: unsafe {
                UPIntrRwLock::new(ProcessFs {
                    fd_table: new_fd_table,
                    cwd: parent_fs.cwd,
                    root_dir: parent_fs.root_dir,
                })
            },
            identity: unsafe { UPIntrRwLock::new(parent_identity) },
            sync_objects: unsafe {
                UPIntrMutex::new(ProcessSyncObjects {
                    mutex_list: Vec::new(),
                    semaphore_list: Vec::new(),
                    condvar_list: Vec::new(),
                })
            },
            limits: unsafe { UPIntrRwLock::new(ProcessLimits { rlimits }) },
            timers: unsafe {
                UPIntrRwLock::new(ProcessTimers {
                    itimers: [IntervalTimerState::default(); 3],
                    itimer_real_expire_ms: 0,
                    itimer_real_interval_ms: 0,
                    timer_slack_ns: parent_timer_slack_ns,
                    default_timer_slack_ns: parent_timer_slack_ns,
                })
            },
            threads: unsafe {
                UPIntrMutex::new(ProcessThreads {
                    tasks: Vec::new(),
                    task_res_allocator: RecycleAllocator::new(),
                })
            },
            signals: unsafe {
                UPIntrMutex::new(ProcessSignals {
                    signals: SignalFlags::empty(),
                    signal_pending: SignalFlags::empty(),
                    pending_signal_sender_pid: [0; MAX_SIG],
                    pending_signal_si_code: [0; MAX_SIG],
                    signal_actions: parent_signal_actions,
                })
            },
            family: unsafe {
                UPIntrMutex::new(ProcessFamily {
                    is_zombie: false,
                    parent: Some(Arc::downgrade(self)),
                    children: Vec::new(),
                    exit_code: 0,
                    ptrace_traceme: false,
                    ptrace_stop_signal: None,
                    child_wait_event: None,
                    group_stopped: false,
                    vfork_vm_parent: None,
                })
            },
        });
        // info!("[fork-stage] pid={} child pcb allocated new_pid={}", self.pid.0, new_pid_value);
        self.push_child(Arc::clone(&child));
        crate::syscall::inherit_shm_for_process_fork(self.pid.0, child.pid.0);
        crate::syscall::sched_inherit_for_fork(self.pid.0, child.pid.0);
        info!("[fork-stage] pid={} child linked to parent", self.pid.0);
        let parent_task = self.get_task(0);
        let ustack_base = parent_task.ustack_base();
        // 获取父线程的 signal_mask，用于子进程继承
        let parent_signal_mask = parent_task.signal_mask();
        let task = Arc::new(TaskControlBlock::new(
            Arc::clone(&child),
            ustack_base,
            false,
        ));
        // info!("[fork-stage] pid={} child task allocated", self.pid.0);
        child.insert_task(0, Arc::clone(&task));
        // info!("[fork-stage] pid={} child task linked", self.pid.0);
        // 子进程继承父进程的信号掩码（Linux 语义：fork 继承 signal_mask）
        task.set_signal_mask(parent_signal_mask);
        insert_into_pid2process(child.getpid(), Arc::clone(&child));
        // info!("[fork-stage] pid={} child inserted pid2process", self.pid.0);
        add_task(task);
        // info!("[fork-stage] pid={} child added ready queue", self.pid.0);
        child
    }

    pub fn getpid(&self) -> usize {
        self.pid.0
    }

    #[inline]
    pub fn get_user_token(&self) -> usize {
        self.memory.read().memory_set.token()
    }

    #[inline]
    pub fn thread_count(&self) -> usize {
        self.threads.lock().thread_count()
    }

    #[inline]
    pub fn get_task(&self, tid: usize) -> Arc<TaskControlBlock> {
        self.threads.lock().get_task(tid)
    }

    pub fn tasks_snapshot(&self) -> Vec<Arc<TaskControlBlock>> {
        self.threads.lock().tasks_snapshot()
    }

    #[inline]
    pub fn alloc_tid(&self) -> usize {
        self.threads.lock().alloc_tid()
    }

    #[inline]
    pub fn dealloc_tid(&self, tid: usize) {
        self.threads.lock().dealloc_tid(tid);
    }

    pub fn insert_task(&self, tid: usize, task: Arc<TaskControlBlock>) {
        self.threads.lock().insert_task(tid, task);
    }

    pub fn remove_task(&self, tid: usize) -> Option<Arc<TaskControlBlock>> {
        self.threads.lock().remove_task(tid)
    }

    pub fn all_tasks_exited(&self) -> bool {
        self.threads.lock().all_tasks_exited()
    }

    #[inline]
    pub fn name(&self) -> String {
        self.identity.read().name.clone()
    }

    pub fn set_name(&self, name: String) {
        self.identity.write().name = name;
    }

    #[inline]
    pub fn cwd(&self) -> String {
        self.fs.read().cwd.clone()
    }

    pub fn set_cwd(&self, cwd: String) {
        self.fs.write().cwd = cwd;
    }

    #[inline]
    pub fn root_dir(&self) -> String {
        self.fs.read().root_dir.clone()
    }

    pub fn set_root_dir(&self, root_dir: String) {
        self.fs.write().root_dir = root_dir;
    }

    pub fn credentials_snapshot(&self) -> ProcessCredentials {
        self.identity.read().credentials_snapshot()
    }

    pub fn effective_uid(&self) -> u32 {
        self.identity.read().effective_uid
    }

    pub fn effective_gid(&self) -> u32 {
        self.identity.read().effective_gid
    }

    pub fn real_uid(&self) -> u32 {
        self.identity.read().real_uid
    }

    pub fn real_gid(&self) -> u32 {
        self.identity.read().real_gid
    }

    pub fn pgid(&self) -> usize {
        self.identity.read().pgid
    }

    pub fn set_pgid(&self, pgid: usize) {
        self.identity.write().pgid = pgid;
    }

    pub fn session_id(&self) -> usize {
        self.identity.read().session_id
    }

    pub fn set_session_id(&self, session_id: usize) {
        self.identity.write().session_id = session_id;
    }

    pub fn get_file(&self, fd: usize) -> Option<Arc<dyn File + Send + Sync>> {
        self.fs
            .read()
            .fd_table
            .get(fd)
            .and_then(|slot| slot.as_ref().map(Arc::clone))
    }

    pub fn alloc_fd(&self) -> Option<usize> {
        let limit = self.with_limits(|limits| limits.rlimits[RLIMIT_NOFILE].rlim_cur as usize);
        self.fs.write().alloc_fd(limit)
    }

    pub fn install_file(&self, file: Arc<dyn File + Send + Sync>) -> Option<usize> {
        let limit = self.with_limits(|limits| limits.rlimits[RLIMIT_NOFILE].rlim_cur as usize);
        let mut fs = self.fs.write();
        let fd = fs.alloc_fd(limit)?;
        fs.fd_table[fd] = Some(file);
        Some(fd)
    }

    pub fn install_file_at(&self, fd: usize, file: Arc<dyn File + Send + Sync>) -> bool {
        let limit = self.with_limits(|limits| limits.rlimits[RLIMIT_NOFILE].rlim_cur as usize);
        if fd >= limit {
            return false;
        }
        let mut fs = self.fs.write();
        if fd >= fs.fd_table.len() {
            fs.fd_table.resize_with(fd + 1, || None);
        }
        fs.fd_table[fd] = Some(file);
        true
    }

    pub fn replace_file(
        &self,
        fd: usize,
        file: Option<Arc<dyn File + Send + Sync>>,
    ) -> Option<Arc<dyn File + Send + Sync>> {
        let mut fs = self.fs.write();
        if fd >= fs.fd_table.len() {
            return None;
        }
        core::mem::replace(&mut fs.fd_table[fd], file)
    }

    pub fn set_fd(&self, fd: usize, file: Option<Arc<dyn File + Send + Sync>>) -> bool {
        let mut fs = self.fs.write();
        if fd >= fs.fd_table.len() {
            return false;
        }
        fs.fd_table[fd] = file;
        true
    }

    pub fn take_fd(&self, fd: usize) -> Option<Arc<dyn File + Send + Sync>> {
        let mut fs = self.fs.write();
        if fd >= fs.fd_table.len() {
            return None;
        }
        fs.fd_table[fd].take()
    }

    pub fn fd_snapshot(&self) -> Vec<Option<Arc<dyn File + Send + Sync>>> {
        self.fs.read().fd_table.clone()
    }

    pub fn fd_files_snapshot(&self) -> Vec<Arc<dyn File + Send + Sync>> {
        self.fs
            .read()
            .fd_table
            .iter()
            .filter_map(|file| file.as_ref().map(Arc::clone))
            .collect()
    }

    pub fn with_fs<R>(&self, f: impl FnOnce(&ProcessFs) -> R) -> R {
        let fs = self.fs.read();
        f(&fs)
    }

    pub fn with_fs_mut<R>(&self, f: impl FnOnce(&mut ProcessFs) -> R) -> R {
        let mut fs = self.fs.write();
        f(&mut fs)
    }

    pub fn with_identity<R>(&self, f: impl FnOnce(&ProcessIdentity) -> R) -> R {
        let identity = self.identity.read();
        f(&identity)
    }

    pub fn with_identity_mut<R>(&self, f: impl FnOnce(&mut ProcessIdentity) -> R) -> R {
        let mut identity = self.identity.write();
        f(&mut identity)
    }

    pub fn with_threads<R>(&self, f: impl FnOnce(&ProcessThreads) -> R) -> R {
        let threads = self.threads.lock();
        f(&threads)
    }

    pub fn with_threads_mut<R>(&self, f: impl FnOnce(&mut ProcessThreads) -> R) -> R {
        let mut threads = self.threads.lock();
        f(&mut threads)
    }

    pub fn with_process_signals<R>(&self, f: impl FnOnce(&ProcessSignals) -> R) -> R {
        let signals = self.signals.lock();
        f(&signals)
    }

    pub fn with_process_signals_mut<R>(&self, f: impl FnOnce(&mut ProcessSignals) -> R) -> R {
        let mut signals = self.signals.lock();
        f(&mut signals)
    }

    pub fn pending_signal(&self) -> SignalFlags {
        self.signals.lock().signal_pending
    }

    pub fn signal_pending_snapshot(&self) -> SignalFlags {
        self.pending_signal()
    }

    pub fn insert_process_signal(&self, flag: SignalFlags, sender_pid: i32, si_code: i32) {
        let mut signals = self.signals.lock();
        signals.signal_pending |= flag;
        let mut bits = flag.bits();
        while bits != 0 {
            let idx = bits.trailing_zeros() as usize;
            signals.set_pending_signal_siginfo(idx + 1, sender_pid, si_code);
            bits &= bits - 1;
        }
    }

    pub fn remove_process_signal(&self, flag: SignalFlags) {
        self.signals.lock().signal_pending.remove(flag);
    }

    pub fn take_process_signal(&self, signum: usize) -> Option<(SignalFlags, i32, i32)> {
        if signum == 0 || signum > MAX_SIG {
            return None;
        }
        let flag = SignalFlags::from_bits_truncate(1u64 << (signum - 1));
        let mut signals = self.signals.lock();
        if !signals.signal_pending.contains(flag) {
            return None;
        }
        let (sender_pid, si_code) = signals.get_pending_signal_siginfo(signum);
        signals.signal_pending.remove(flag);
        signals.clear_pending_signal_siginfo(signum);
        Some((flag, sender_pid, si_code))
    }

    pub fn process_signal_siginfo(&self, signum: usize) -> (i32, i32) {
        self.signals.lock().get_pending_signal_siginfo(signum)
    }

    pub fn with_signal_action<R>(&self, signum: usize, f: impl FnOnce(SignalAction) -> R) -> R {
        let signals = self.signals.lock();
        f(signals.signal_actions.table[signum])
    }

    pub fn update_signal_action<R>(
        &self,
        signum: usize,
        f: impl FnOnce(&mut SignalAction) -> R,
    ) -> R {
        let mut signals = self.signals.lock();
        f(&mut signals.signal_actions.table[signum])
    }

    pub fn with_family<R>(&self, f: impl FnOnce(&ProcessFamily) -> R) -> R {
        let family = self.family.lock();
        f(&family)
    }

    pub fn with_family_mut<R>(&self, f: impl FnOnce(&mut ProcessFamily) -> R) -> R {
        let mut family = self.family.lock();
        f(&mut family)
    }

    pub fn parent(&self) -> Option<Arc<ProcessControlBlock>> {
        self.family
            .lock()
            .parent
            .as_ref()
            .and_then(|parent| parent.upgrade())
    }

    pub fn parent_weak(&self) -> Option<Weak<ProcessControlBlock>> {
        self.family.lock().parent.clone()
    }

    pub fn set_parent(&self, parent: Option<Weak<ProcessControlBlock>>) {
        self.family.lock().parent = parent;
    }

    pub fn children_snapshot(&self) -> Vec<Arc<ProcessControlBlock>> {
        self.family.lock().children.clone()
    }

    pub fn child_count(&self) -> usize {
        self.family.lock().children.len()
    }

    pub fn push_child(&self, child: Arc<ProcessControlBlock>) {
        self.family.lock().children.push(child);
    }

    pub fn remove_child_by_pid(&self, pid: usize) -> Option<Arc<ProcessControlBlock>> {
        let mut family = self.family.lock();
        let idx = family
            .children
            .iter()
            .position(|child| child.getpid() == pid)?;
        Some(family.children.remove(idx))
    }

    pub fn is_zombie(&self) -> bool {
        self.family.lock().is_zombie
    }

    pub fn set_zombie(&self, exit_code: i32) {
        let mut family = self.family.lock();
        family.is_zombie = true;
        family.exit_code = exit_code;
    }

    pub fn exit_code(&self) -> i32 {
        self.family.lock().exit_code
    }

    pub fn child_wait_event(&self) -> Option<ChildWaitEvent> {
        self.family.lock().child_wait_event
    }

    pub fn set_child_wait_event(&self, event: Option<ChildWaitEvent>) {
        self.family.lock().child_wait_event = event;
    }

    pub fn take_child_wait_event(&self) -> Option<ChildWaitEvent> {
        self.family.lock().child_wait_event.take()
    }

    pub fn group_stopped(&self) -> bool {
        self.family.lock().group_stopped
    }

    pub fn set_group_stopped(&self, stopped: bool) {
        self.family.lock().group_stopped = stopped;
    }

    pub fn ptrace_traceme(&self) -> bool {
        self.family.lock().ptrace_traceme
    }

    pub fn set_ptrace_traceme(&self, traceme: bool) {
        self.family.lock().ptrace_traceme = traceme;
    }

    pub fn ptrace_stop_signal(&self) -> Option<i32> {
        self.family.lock().ptrace_stop_signal
    }

    pub fn set_ptrace_stop_signal(&self, signum: Option<i32>) {
        self.family.lock().ptrace_stop_signal = signum;
    }

    pub fn take_ptrace_stop_signal(&self) -> Option<i32> {
        self.family.lock().ptrace_stop_signal.take()
    }

    pub fn vfork_parent(&self) -> Option<Arc<ProcessControlBlock>> {
        self.family
            .lock()
            .vfork_vm_parent
            .as_ref()
            .and_then(|parent| parent.upgrade())
    }

    pub fn set_vfork_parent(&self, parent: Option<Weak<ProcessControlBlock>>) {
        self.family.lock().vfork_vm_parent = parent;
    }

    pub fn with_memory<R>(&self, f: impl FnOnce(&ProcessMemory) -> R) -> R {
        let memory = self.memory.read();
        f(&memory)
    }

    pub fn with_memory_mut<R>(&self, f: impl FnOnce(&mut ProcessMemory) -> R) -> R {
        let mut memory = self.memory.write();
        f(&mut memory)
    }

    pub fn with_memory_set<R>(&self, f: impl FnOnce(&MemorySet) -> R) -> R {
        self.with_memory(|memory| f(&memory.memory_set))
    }

    pub fn with_memory_set_mut<R>(&self, f: impl FnOnce(&mut MemorySet) -> R) -> R {
        self.with_memory_mut(|memory| f(&mut memory.memory_set))
    }

    pub fn memory_snapshot_for_proc_maps(&self, name: &str) -> String {
        self.with_memory(|memory| {
            memory
                .memory_set
                .render_proc_maps(name, memory.heap_bottom, memory.program_brk)
        })
    }

    pub fn brk(&self) -> usize {
        self.memory.read().program_brk
    }

    pub fn set_brk(&self, program_brk: usize) {
        self.memory.write().program_brk = program_brk;
    }

    pub fn heap_bottom(&self) -> usize {
        self.memory.read().heap_bottom
    }

    pub fn mmap_base(&self) -> usize {
        self.memory.read().mmap_base
    }

    pub fn alloc_mmap_base(&self, len: usize) -> Result<usize, isize> {
        let mut memory = self.memory.write();
        let base = align_up(memory.mmap_base, PAGE_SIZE);
        let Some(next_base) = base.checked_add(len) else {
            return Err(-12);
        };
        memory.mmap_base = next_base;
        Ok(base)
    }

    pub fn reset_memory_after_exec(
        &self,
        memory_set: MemorySet,
        heap_bottom: usize,
        tls_area: Option<TlsArea>,
    ) {
        let mut memory = self.memory.write();
        memory.memory_set = memory_set;
        memory.heap_bottom = heap_bottom;
        memory.program_brk = heap_bottom;
        memory.mmap_base = USER_MMAP_TOP;
        memory.tls_area = tls_area;
    }

    pub fn clone_memory_for_fork(&self) -> (MemorySet, usize, usize, usize, Option<TlsArea>) {
        let mut parent = self.memory.write();
        let memory_set = MemorySet::from_existed_user(&mut parent.memory_set);
        (
            memory_set,
            parent.heap_bottom,
            parent.program_brk,
            parent.mmap_base,
            parent.tls_area.clone(),
        )
    }

    pub fn handle_cow_or_demand_fault(&self, addr: usize) -> bool {
        let mut memory = self.memory.write();
        memory.memory_set.handle_cow_fault(addr) || memory.memory_set.handle_demand_fault(addr)
    }

    pub fn recycle_user_memory(&self) {
        self.memory.write().memory_set.recycle_data_pages();
    }

    pub fn sync_vfork_memory_to_parent(&self, parent: &Arc<ProcessControlBlock>) -> usize {
        if self.pid.0 < parent.pid.0 {
            let child_memory = self.memory.read();
            let mut parent_memory = parent.memory.write();
            parent_memory
                .memory_set
                .sync_user_writable_from(&child_memory.memory_set)
        } else {
            let mut parent_memory = parent.memory.write();
            let child_memory = self.memory.read();
            parent_memory
                .memory_set
                .sync_user_writable_from(&child_memory.memory_set)
        }
    }
}
