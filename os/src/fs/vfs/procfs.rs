use super::core::{VfsInode, VfsNodeKind};
use crate::config::MEMORY_END;
use crate::task::{current_process, pid2process, pid2process_snapshot, TaskStatus};
use crate::timer::get_time_us;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

const MEMORY_START: usize = 0x8000_0000;
const SYSCALL_WAITPID: usize = 260;

struct ProcRootInode {
    entries: BTreeMap<String, Arc<dyn VfsInode>>,
}

impl ProcRootInode {
    fn new(entries: BTreeMap<String, Arc<dyn VfsInode>>) -> Arc<Self> {
        Arc::new(Self { entries })
    }
}

struct ProcStaticDirInode {
    entries: BTreeMap<String, Arc<dyn VfsInode>>,
}

impl ProcStaticDirInode {
    fn new(entries: BTreeMap<String, Arc<dyn VfsInode>>) -> Arc<Self> {
        Arc::new(Self { entries })
    }
}

impl VfsInode for ProcStaticDirInode {
    fn kind(&self) -> VfsNodeKind {
        VfsNodeKind::Dir
    }

    fn read_at(&self, _offset: usize, _buf: &mut [u8]) -> usize {
        0
    }

    fn write_at(&self, _offset: usize, buf: &[u8]) -> usize {
        buf.len()
    }

    fn lookup(&self, name: &str) -> Option<Arc<dyn VfsInode>> {
        self.entries.get(name).cloned()
    }

    fn create(&self, _name: &str) -> Option<Arc<dyn VfsInode>> {
        None
    }

    fn truncate(&self) {}

    fn list(&self) -> Vec<String> {
        let mut entries: Vec<String> = self.entries.keys().cloned().collect();
        entries.sort();
        entries
    }
}

impl VfsInode for ProcRootInode {
    fn kind(&self) -> VfsNodeKind {
        VfsNodeKind::Dir
    }

    fn read_at(&self, _offset: usize, _buf: &mut [u8]) -> usize {
        0
    }

    fn write_at(&self, _offset: usize, buf: &[u8]) -> usize {
        buf.len()
    }

    fn lookup(&self, name: &str) -> Option<Arc<dyn VfsInode>> {
        if let Some(node) = self.entries.get(name) {
            return Some(node.clone());
        }
        if name == "self" {
            return Some(ProcPidDirInode::new(current_process().getpid()));
        }
        let pid = name.parse::<usize>().ok()?;
        pid2process(pid)?;
        Some(ProcPidDirInode::new(pid))
    }

    fn create(&self, _name: &str) -> Option<Arc<dyn VfsInode>> {
        None
    }

    fn truncate(&self) {}

    fn list(&self) -> Vec<String> {
        let mut entries: Vec<String> = self.entries.keys().cloned().collect();
        entries.push(String::from("self"));
        for (pid, _) in pid2process_snapshot() {
            entries.push(format!("{}", pid));
        }
        entries.sort();
        entries
    }
}

struct ProcFileInode {
    content: Arc<dyn Fn() -> String + Send + Sync>,
}

impl ProcFileInode {
    fn new<F>(content: F) -> Arc<Self>
    where
        F: Fn() -> String + Send + Sync + 'static,
    {
        Arc::new(Self {
            content: Arc::new(content),
        })
    }
}

impl VfsInode for ProcFileInode {
    fn kind(&self) -> VfsNodeKind {
        VfsNodeKind::File
    }

    fn read_at(&self, offset: usize, buf: &mut [u8]) -> usize {
        let content = (self.content)();
        let bytes = content.as_bytes();
        if offset >= bytes.len() {
            return 0;
        }
        let n = core::cmp::min(buf.len(), bytes.len() - offset);
        buf[..n].copy_from_slice(&bytes[offset..offset + n]);
        n
    }

    fn write_at(&self, _offset: usize, buf: &[u8]) -> usize {
        buf.len()
    }

    fn lookup(&self, _name: &str) -> Option<Arc<dyn VfsInode>> {
        None
    }

    fn create(&self, _name: &str) -> Option<Arc<dyn VfsInode>> {
        None
    }

    fn truncate(&self) {}

    fn list(&self) -> Vec<String> {
        Vec::new()
    }

    fn size(&self) -> usize {
        (self.content)().as_bytes().len()
    }
}

struct ProcPidDirInode {
    pid: usize,
}

impl ProcPidDirInode {
    fn new(pid: usize) -> Arc<Self> {
        Arc::new(Self { pid })
    }
}

impl VfsInode for ProcPidDirInode {
    fn kind(&self) -> VfsNodeKind {
        VfsNodeKind::Dir
    }

    fn read_at(&self, _offset: usize, _buf: &mut [u8]) -> usize {
        0
    }

    fn write_at(&self, _offset: usize, buf: &[u8]) -> usize {
        buf.len()
    }

    fn lookup(&self, name: &str) -> Option<Arc<dyn VfsInode>> {
        match name {
            "stat" => Some(ProcPidStatInode::new(self.pid)),
            "task" => Some(ProcPidTaskDirInode::new(self.pid)),
            "maps" => Some(ProcPidMapsInode::new(self.pid)),
            // /proc/self/mounts, /proc/self/mountinfo, /proc/self/mountstats
            "mounts" => Some(ProcFileInode::new(proc_mounts)),
            "mountinfo" => Some(ProcFileInode::new(proc_mountinfo)),
            "mountstats" => Some(ProcFileInode::new(|| String::from("device rootfs mounted on / with fstype rootfs\n"))),
            // /proc/self/cgroup - needed by cgroup tests
            "cgroup" => Some(ProcFileInode::new(|| String::from("0::/\n"))),
            // /proc/self/status - needed by various tests
            "status" => Some(ProcPidStatusInode::new(self.pid)),
            // /proc/self/fd - file descriptor directory
            "fd" => Some(ProcStaticDirInode::new(BTreeMap::new())),
            // /proc/self/cmdline
            "cmdline" => Some(ProcFileInode::new(|| String::from("\0"))),
            // /proc/self/environ
            "environ" => Some(ProcFileInode::new(|| String::from("\0"))),
            _ => None,
        }
    }

    fn create(&self, _name: &str) -> Option<Arc<dyn VfsInode>> {
        None
    }

    fn truncate(&self) {}

    fn list(&self) -> Vec<String> {
        vec![
            String::from("maps"),
            String::from("stat"),
            String::from("task"),
            String::from("status"),
            String::from("mounts"),
            String::from("mountinfo"),
            String::from("mountstats"),
            String::from("cgroup"),
            String::from("fd"),
            String::from("cmdline"),
            String::from("environ"),
        ]
    }
}

struct ProcPidTaskDirInode {
    pid: usize,
}

impl ProcPidTaskDirInode {
    fn new(pid: usize) -> Arc<Self> {
        Arc::new(Self { pid })
    }
}

impl VfsInode for ProcPidTaskDirInode {
    fn kind(&self) -> VfsNodeKind {
        VfsNodeKind::Dir
    }

    fn read_at(&self, _offset: usize, _buf: &mut [u8]) -> usize {
        0
    }

    fn write_at(&self, _offset: usize, buf: &[u8]) -> usize {
        buf.len()
    }

    fn lookup(&self, name: &str) -> Option<Arc<dyn VfsInode>> {
        let tid = name.parse::<usize>().ok()?;
        let process = pid2process(self.pid)?;
        let inner = process.inner_exclusive_access();
        let (task_idx, _) = inner
            .tasks
            .iter()
            .enumerate()
            .find(|(idx, t)| {
                t.is_some() && ((if *idx == 0 { self.pid } else { self.pid + *idx }) == tid)
            })?;
        Some(ProcPidTaskTidDirInode::new(self.pid, task_idx))
    }

    fn create(&self, _name: &str) -> Option<Arc<dyn VfsInode>> {
        None
    }

    fn truncate(&self) {}

    fn list(&self) -> Vec<String> {
        let Some(process) = pid2process(self.pid) else {
            return Vec::new();
        };
        let inner = process.inner_exclusive_access();
        let mut out: Vec<String> = inner
            .tasks
            .iter()
            .enumerate()
            .filter_map(|(idx, t)| {
                if t.is_some() {
                    Some(if idx == 0 { self.pid } else { self.pid + idx })
                } else {
                    None
                }
            })
            .map(|tid| format!("{}", tid))
            .collect();
        out.sort();
        out
    }
}

struct ProcPidTaskTidDirInode {
    pid: usize,
    task_idx: usize,
}

impl ProcPidTaskTidDirInode {
    fn new(pid: usize, task_idx: usize) -> Arc<Self> {
        Arc::new(Self { pid, task_idx })
    }
}

impl VfsInode for ProcPidTaskTidDirInode {
    fn kind(&self) -> VfsNodeKind {
        VfsNodeKind::Dir
    }

    fn read_at(&self, _offset: usize, _buf: &mut [u8]) -> usize {
        0
    }

    fn write_at(&self, _offset: usize, buf: &[u8]) -> usize {
        buf.len()
    }

    fn lookup(&self, name: &str) -> Option<Arc<dyn VfsInode>> {
        match name {
            "stat" => Some(ProcPidTaskStatInode::new(self.pid, self.task_idx)),
            _ => None,
        }
    }

    fn create(&self, _name: &str) -> Option<Arc<dyn VfsInode>> {
        None
    }

    fn truncate(&self) {}

    fn list(&self) -> Vec<String> {
        vec![String::from("stat")]
    }
}

struct ProcPidTaskStatInode {
    pid: usize,
    task_idx: usize,
}

impl ProcPidTaskStatInode {
    fn new(pid: usize, task_idx: usize) -> Arc<Self> {
        Arc::new(Self { pid, task_idx })
    }

    fn render(&self) -> String {
        let Some(process) = pid2process(self.pid) else {
            return String::new();
        };
        let inner = process.inner_exclusive_access();
        let comm = inner.name.clone();
        let mut state = if inner.is_zombie { 'Z' } else { 'R' };
        if !inner.is_zombie {
            if let Some(Some(task)) = inner.tasks.get(self.task_idx) {
                if let Some(task_inner) = task.try_inner_exclusive_access() {
                state = match task_inner.task_status {
                    TaskStatus::Blocked => 'S',
                    TaskStatus::Running => 'R',
                    TaskStatus::Ready => {
                        if task_inner.last_syscall == SYSCALL_WAITPID {
                            'S'
                        } else {
                            'R'
                        }
                    }
                };
                }
            }
        }
        let tid = if self.task_idx == 0 {
            self.pid
        } else {
            self.pid + self.task_idx
        };
        format!(
            "{} ({}) {} 0 0 0 0 0 0 0 0 0 0 1 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n",
            tid, comm, state
        )
    }
}

impl VfsInode for ProcPidTaskStatInode {
    fn kind(&self) -> VfsNodeKind {
        VfsNodeKind::File
    }

    fn read_at(&self, offset: usize, buf: &mut [u8]) -> usize {
        let content = self.render();
        let bytes = content.as_bytes();
        if offset >= bytes.len() {
            return 0;
        }
        let n = core::cmp::min(buf.len(), bytes.len() - offset);
        buf[..n].copy_from_slice(&bytes[offset..offset + n]);
        n
    }

    fn write_at(&self, _offset: usize, buf: &[u8]) -> usize {
        buf.len()
    }

    fn lookup(&self, _name: &str) -> Option<Arc<dyn VfsInode>> {
        None
    }

    fn create(&self, _name: &str) -> Option<Arc<dyn VfsInode>> {
        None
    }

    fn truncate(&self) {}

    fn list(&self) -> Vec<String> {
        Vec::new()
    }

    fn size(&self) -> usize {
        self.render().as_bytes().len()
    }
}

struct ProcPidMapsInode {
    pid: usize,
}

impl ProcPidMapsInode {
    fn new(pid: usize) -> Arc<Self> {
        Arc::new(Self { pid })
    }

    fn render(&self) -> String {
        let Some(process) = pid2process(self.pid) else {
            return String::new();
        };
        let inner = process.inner_exclusive_access();
        inner
            .memory_set
            .render_proc_maps(&inner.name, inner.heap_bottom, inner.program_brk)
    }
}

impl VfsInode for ProcPidMapsInode {
    fn kind(&self) -> VfsNodeKind {
        VfsNodeKind::File
    }

    fn read_at(&self, offset: usize, buf: &mut [u8]) -> usize {
        let content = self.render();
        let bytes = content.as_bytes();
        if offset >= bytes.len() {
            return 0;
        }
        let n = core::cmp::min(buf.len(), bytes.len() - offset);
        buf[..n].copy_from_slice(&bytes[offset..offset + n]);
        n
    }

    fn write_at(&self, _offset: usize, buf: &[u8]) -> usize {
        buf.len()
    }

    fn lookup(&self, _name: &str) -> Option<Arc<dyn VfsInode>> {
        None
    }

    fn create(&self, _name: &str) -> Option<Arc<dyn VfsInode>> {
        None
    }

    fn truncate(&self) {}

    fn list(&self) -> Vec<String> {
        Vec::new()
    }

    fn size(&self) -> usize {
        self.render().as_bytes().len()
    }
}

struct ProcPidStatInode {
    pid: usize,
}

impl ProcPidStatInode {
    fn new(pid: usize) -> Arc<Self> {
        Arc::new(Self { pid })
    }

    fn render(&self) -> String {
        let Some(process) = pid2process(self.pid) else {
            return String::new();
        };
        let inner = process.inner_exclusive_access();
        let comm = inner.name.clone();
        // /proc/<pid>/stat should reflect the thread-group leader state.
        // LTP's TST_PROCESS_STATE_WAIT() relies on this for parent process sleep detection.
        let mut state = if inner.is_zombie { 'Z' } else { 'R' };
        if !inner.is_zombie {
            if let Some(Some(leader)) = inner.tasks.get(0) {
                if let Some(task_inner) = leader.try_inner_exclusive_access() {
                    state = match task_inner.task_status {
                        TaskStatus::Blocked => 'S',
                        TaskStatus::Running => 'R',
                        TaskStatus::Ready => {
                            if task_inner.last_syscall == SYSCALL_WAITPID {
                                'S'
                            } else {
                                'R'
                            }
                        }
                    };
                }
            }
        }
        format!(
            "{} ({}) {} 0 0 0 0 0 0 0 0 0 0 1 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n",
            self.pid, comm, state
        )
    }
}

impl VfsInode for ProcPidStatInode {
    fn kind(&self) -> VfsNodeKind {
        VfsNodeKind::File
    }

    fn read_at(&self, offset: usize, buf: &mut [u8]) -> usize {
        let content = self.render();
        let bytes = content.as_bytes();
        if offset >= bytes.len() {
            return 0;
        }
        let n = core::cmp::min(buf.len(), bytes.len() - offset);
        buf[..n].copy_from_slice(&bytes[offset..offset + n]);
        n
    }

    fn write_at(&self, _offset: usize, buf: &[u8]) -> usize {
        buf.len()
    }

    fn lookup(&self, _name: &str) -> Option<Arc<dyn VfsInode>> {
        None
    }

    fn create(&self, _name: &str) -> Option<Arc<dyn VfsInode>> {
        None
    }

    fn truncate(&self) {}

    fn list(&self) -> Vec<String> {
        Vec::new()
    }

    fn size(&self) -> usize {
        self.render().as_bytes().len()
    }
}

struct ProcPidStatusInode {
    pid: usize,
}

impl ProcPidStatusInode {
    fn new(pid: usize) -> Arc<Self> {
        Arc::new(Self { pid })
    }

    fn render(&self) -> String {
        let Some(process) = pid2process(self.pid) else {
            return String::new();
        };
        let inner = process.inner_exclusive_access();
        let name = inner.name.clone();
        let pid = self.pid;
        let uid = inner.effective_uid;
        drop(inner);
        format!(
            "Name:\t{name}\n\
             Pid:\t{pid}\n\
             PPid:\t0\n\
             Uid:\t{uid}\t{uid}\t{uid}\t{uid}\n\
             Gid:\t0\t0\t0\t0\n\
             VmRSS:\t0 kB\n\
             Threads:\t1\n"
        )
    }
}

impl VfsInode for ProcPidStatusInode {
    fn kind(&self) -> VfsNodeKind {
        VfsNodeKind::File
    }

    fn read_at(&self, offset: usize, buf: &mut [u8]) -> usize {
        let content = self.render();
        let bytes = content.as_bytes();
        if offset >= bytes.len() {
            return 0;
        }
        let n = core::cmp::min(buf.len(), bytes.len() - offset);
        buf[..n].copy_from_slice(&bytes[offset..offset + n]);
        n
    }

    fn write_at(&self, _offset: usize, buf: &[u8]) -> usize {
        buf.len()
    }

    fn lookup(&self, _name: &str) -> Option<Arc<dyn VfsInode>> {
        None
    }

    fn create(&self, _name: &str) -> Option<Arc<dyn VfsInode>> {
        None
    }

    fn truncate(&self) {}

    fn list(&self) -> Vec<String> {
        Vec::new()
    }

    fn size(&self) -> usize {
        self.render().as_bytes().len()
    }
}

fn proc_mounts() -> String {
    String::from(
        "rootfs / rootfs rw 0 0\n\
         proc /proc proc rw,nosuid,nodev,noexec,relatime 0 0\n",
    )
}

fn proc_mountinfo() -> String {
    String::from(
        "1 0 0:1 / / rw,relatime shared:1 - rootfs rootfs rw\n\
         2 1 0:4 / /proc rw,nosuid,nodev,noexec,relatime shared:2 - proc proc rw\n",
    )
}

fn proc_meminfo() -> String {
    let total_kb = MEMORY_END.saturating_sub(MEMORY_START) / 1024;
    let free_kb = total_kb / 2;
    let mut out = String::new();
    out.push_str(&format!("MemTotal: {} kB\n", total_kb));
    out.push_str(&format!("MemFree: {} kB\n", free_kb));
    out.push_str(&format!("MemAvailable: {} kB\n", free_kb));
    out.push_str("Buffers: 0 kB\n");
    out.push_str("Cached: 0 kB\n");
    out
}

fn proc_stat() -> String {
    String::from(
        "cpu 0 0 0 0 0 0 0 0 0 0\n\
        intr 0\n\
        ctxt 0\n\
        btime 0\n\
        processes 1\n\
        procs_running 1\n\
        procs_blocked 0\n",
    )
}

fn proc_uptime() -> String {
    let us = get_time_us();
    let sec = us / 1_000_000;
    let frac = (us % 1_000_000) / 10_000;
    format!("{}.{} 0.00\n", sec, frac)
}

/// Generate /proc/cpuinfo content (architecture-specific)
fn proc_cpuinfo() -> String {
    #[cfg(target_arch = "riscv64")]
    {
        // RISC-V cpuinfo format (single core)
        String::from(
            "processor\t: 0\n\
             hart\t\t: 0\n\
             isa\t\t: rv64imafdc\n\
             mmu\t\t: sv39\n\
             uarch\t\t: qemu,virt\n\
             \n",
        )
    }
    #[cfg(target_arch = "loongarch64")]
    {
        // LoongArch cpuinfo format (single core)
        String::from(
            "system type\t: generic-loongson-machine\n\
             processor\t: 0\n\
             package\t\t: 0\n\
             core\t\t: 0\n\
             cpu family\t: Loongson-64bit\n\
             model name\t: Loongson-3A5000-QEMU\n\
             CPU MHz\t\t: 2000.00\n\
             BogoMIPS\t: 4000.00\n\
             tlb_entries\t: 2112\n\
             address sizes\t: 48 bits physical, 48 bits virtual\n\
             isa\t\t: loongarch64\n\
             features\t: cpucfg lam ual fpu\n\
             \n",
        )
    }
}

/// Build /proc/sys/kernel/ subtree with sched_rt_runtime_us etc.
fn proc_sys_kernel() -> Arc<dyn VfsInode> {
    let mut entries: BTreeMap<String, Arc<dyn VfsInode>> = BTreeMap::new();
    entries.insert(
        String::from("pid_max"),
        ProcFileInode::new(|| String::from("32768\n")),
    );
    // cyclictest reads this to check if RT scheduling is available
    entries.insert(
        String::from("sched_rt_runtime_us"),
        ProcFileInode::new(|| String::from("950000\n")),
    );
    entries.insert(
        String::from("sched_rt_period_us"),
        ProcFileInode::new(|| String::from("1000000\n")),
    );
    // waitid10 reads /proc/sys/kernel/core_pattern during setup.
    // Return a plain filename pattern (not a pipe handler) so tests
    // expecting non-dumped CLD_KILLED behavior can proceed.
    entries.insert(
        String::from("core_pattern"),
        ProcFileInode::new(|| String::from("core\n")),
    );
    ProcStaticDirInode::new(entries)
}

fn proc_sys() -> Arc<dyn VfsInode> {
    let mut entries: BTreeMap<String, Arc<dyn VfsInode>> = BTreeMap::new();
    entries.insert(String::from("kernel"), proc_sys_kernel());
    ProcStaticDirInode::new(entries)
}

pub(in crate::fs::vfs) fn procfs_root() -> Arc<dyn VfsInode> {
    let mut entries: BTreeMap<String, Arc<dyn VfsInode>> = BTreeMap::new();
    entries.insert(String::from("mounts"), ProcFileInode::new(proc_mounts));
    entries.insert(String::from("mountinfo"), ProcFileInode::new(proc_mountinfo));
    entries.insert(String::from("meminfo"), ProcFileInode::new(proc_meminfo));
    entries.insert(String::from("stat"), ProcFileInode::new(proc_stat));
    entries.insert(String::from("uptime"), ProcFileInode::new(proc_uptime));
    entries.insert(String::from("cpuinfo"), ProcFileInode::new(proc_cpuinfo));
    // /proc/cgroups - needed by cgroup tests (empty = no cgroup controllers)
    entries.insert(
        String::from("cgroups"),
        ProcFileInode::new(|| {
            String::from("#subsys_name\thierarchy\tnum_cgroups\tenabled\n")
        }),
    );
    // /proc/filesystems - needed by various tests
    entries.insert(
        String::from("filesystems"),
        ProcFileInode::new(|| {
            String::from(
                "nodev\tsysfs\n\
                 nodev\ttmpfs\n\
                 nodev\tproc\n\
                 \text4\n\
                 nodev\tdevtmpfs\n",
            )
        }),
    );
    entries.insert(String::from("sys"), proc_sys());
    ProcRootInode::new(entries)
}
