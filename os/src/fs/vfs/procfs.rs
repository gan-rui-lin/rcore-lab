use super::core::{VfsInode, VfsNodeKind};
use crate::config::MEMORY_END;
use crate::timer::get_time_us;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

const MEMORY_START: usize = 0x8000_0000;

struct ProcDirInode {
    entries: BTreeMap<String, Arc<dyn VfsInode>>,
}

impl ProcDirInode {
    fn new(entries: BTreeMap<String, Arc<dyn VfsInode>>) -> Arc<Self> {
        Arc::new(Self { entries })
    }
}

impl VfsInode for ProcDirInode {
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
        self.entries.keys().cloned().collect()
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

/// Build /proc/sys/kernel/ subtree with sched_rt_runtime_us etc.
fn proc_sys_kernel() -> Arc<dyn VfsInode> {
    let mut entries: BTreeMap<String, Arc<dyn VfsInode>> = BTreeMap::new();
    // cyclictest reads this to check if RT scheduling is available
    entries.insert(
        String::from("sched_rt_runtime_us"),
        ProcFileInode::new(|| String::from("950000\n")),
    );
    entries.insert(
        String::from("sched_rt_period_us"),
        ProcFileInode::new(|| String::from("1000000\n")),
    );
    ProcDirInode::new(entries)
}

fn proc_sys() -> Arc<dyn VfsInode> {
    let mut entries: BTreeMap<String, Arc<dyn VfsInode>> = BTreeMap::new();
    entries.insert(String::from("kernel"), proc_sys_kernel());
    ProcDirInode::new(entries)
}

/// Generate /proc/self/smaps content
fn proc_self_smaps() -> String {
    let process = crate::task::current_process();
    let inner = process.inner_exclusive_access();
    inner.memory_set.generate_smaps()
}

/// Build /proc/self/ subtree
fn proc_self_dir() -> Arc<dyn VfsInode> {
    let mut entries: BTreeMap<String, Arc<dyn VfsInode>> = BTreeMap::new();
    entries.insert(
        String::from("status"),
        ProcFileInode::new(|| {
            let pid = crate::task::current_process().pid.0;
            format!(
                "Name:\tunknown\nState:\tR (running)\nPid:\t{}\nPPid:\t1\nThreads:\t1\n",
                pid
            )
        }),
    );
    entries.insert(
        String::from("smaps"),
        ProcFileInode::new(proc_self_smaps),
    );
    ProcDirInode::new(entries)
}

pub(in crate::fs::vfs) fn procfs_root() -> Arc<dyn VfsInode> {
    let mut entries: BTreeMap<String, Arc<dyn VfsInode>> = BTreeMap::new();
    entries.insert(String::from("mounts"), ProcFileInode::new(proc_mounts));
    entries.insert(String::from("meminfo"), ProcFileInode::new(proc_meminfo));
    entries.insert(String::from("stat"), ProcFileInode::new(proc_stat));
    entries.insert(String::from("uptime"), ProcFileInode::new(proc_uptime));
    entries.insert(String::from("sys"), proc_sys());
    entries.insert(String::from("self"), proc_self_dir());
    ProcDirInode::new(entries)
}
