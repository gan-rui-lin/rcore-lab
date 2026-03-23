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

    fn write_at(&self, _offset: usize, _buf: &[u8]) -> usize {
        0
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

    fn write_at(&self, _offset: usize, _buf: &[u8]) -> usize {
        0
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

    fn write_at(&self, _offset: usize, _buf: &[u8]) -> usize {
        0
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

    fn write_at(&self, _offset: usize, _buf: &[u8]) -> usize {
        0
    }

    fn lookup(&self, name: &str) -> Option<Arc<dyn VfsInode>> {
        match name {
            "stat" => Some(ProcPidStatInode::new(self.pid)),
            "maps" => Some(ProcPidMapsInode::new(self.pid)),
            _ => None,
        }
    }

    fn create(&self, _name: &str) -> Option<Arc<dyn VfsInode>> {
        None
    }

    fn truncate(&self) {}

    fn list(&self) -> Vec<String> {
        vec![String::from("maps"), String::from("stat")]
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

    fn write_at(&self, _offset: usize, _buf: &[u8]) -> usize {
        0
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
        let mut state = if inner.is_zombie { 'Z' } else { 'S' };
        for task in inner.tasks.iter().filter_map(|task| task.as_ref()) {
            if let Some(task_inner) = task.try_inner_exclusive_access() {
                match task_inner.task_status {
                    TaskStatus::Running => {
                        state = 'R';
                        break;
                    }
                    TaskStatus::Blocked => {
                        state = 'S';
                    }
                    TaskStatus::Ready => {
                        if task_inner.last_syscall == SYSCALL_WAITPID {
                            state = 'S';
                        } else if state != 'Z' {
                            state = 'R';
                        }
                    }
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

    fn write_at(&self, _offset: usize, _buf: &[u8]) -> usize {
        0
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
    String::from("proc /proc proc rw 0 0\nrootfs / rootfs rw 0 0\n")
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

pub(in crate::fs::vfs) fn procfs_root() -> Arc<dyn VfsInode> {
    let mut entries: BTreeMap<String, Arc<dyn VfsInode>> = BTreeMap::new();
    let mut kernel_entries: BTreeMap<String, Arc<dyn VfsInode>> = BTreeMap::new();
    kernel_entries.insert(
        String::from("pid_max"),
        ProcFileInode::new(|| String::from("32768\n")),
    );
    let mut sys_entries: BTreeMap<String, Arc<dyn VfsInode>> = BTreeMap::new();
    sys_entries.insert(
        String::from("kernel"),
        ProcStaticDirInode::new(kernel_entries),
    );
    entries.insert(String::from("mounts"), ProcFileInode::new(proc_mounts));
    entries.insert(String::from("meminfo"), ProcFileInode::new(proc_meminfo));
    entries.insert(String::from("stat"), ProcFileInode::new(proc_stat));
    entries.insert(String::from("sys"), ProcStaticDirInode::new(sys_entries));
    entries.insert(String::from("uptime"), ProcFileInode::new(proc_uptime));
    ProcRootInode::new(entries)
}
