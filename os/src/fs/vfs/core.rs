use crate::sync::UPIntrFreeCell;
use lazy_static::lazy_static;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::{Arc, Weak};
use alloc::vec;
use alloc::vec::Vec;

#[cfg(feature = "ext4")]
use super::ext4::Ext4Fs;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VfsNodeKind {
    File,
    Dir,
}

pub trait VfsInode: Send + Sync {
    fn kind(&self) -> VfsNodeKind;
    fn read_at(&self, offset: usize, buf: &mut [u8]) -> usize;
    fn write_at(&self, offset: usize, buf: &[u8]) -> usize;
    fn lookup(&self, name: &str) -> Option<Arc<dyn VfsInode>>;
    fn create(&self, name: &str) -> Option<Arc<dyn VfsInode>>;
    fn create_dir(&self, _name: &str) -> Option<Arc<dyn VfsInode>> {
        None
    }
    fn remove(&self, _name: &str, _is_dir: bool) -> bool {
        false
    }
    fn truncate(&self);
    fn list(&self) -> Vec<String>;
    fn size(&self) -> usize {
        0
    }

    fn is_dir(&self) -> bool {
        self.kind() == VfsNodeKind::Dir
    }
}

#[allow(dead_code)]
pub struct Dentry {
    name: String,
    parent: Option<Weak<Dentry>>,
    inode: Arc<dyn VfsInode>,
}

#[allow(dead_code)]
impl Dentry {
    pub fn new(name: &str, parent: Option<Arc<Dentry>>, inode: Arc<dyn VfsInode>) -> Arc<Self> {
        Arc::new(Self {
            name: name.to_string(),
            parent: parent.map(|p| Arc::downgrade(&p)),
            inode,
        })
    }

    pub fn inode(&self) -> Arc<dyn VfsInode> {
        self.inode.clone()
    }

    pub fn path(&self) -> String {
        if let Some(parent) = self.parent.as_ref().and_then(|p| p.upgrade()) {
            let parent_path = parent.path();
            if parent_path == "/" {
                parent_path + &self.name
            } else {
                parent_path + "/" + &self.name
            }
        } else {
            String::from("/")
        }
    }
}

struct MountPoint {
    path: String,
    root: Arc<dyn VfsInode>,
    #[cfg(feature = "ext4")]
    _ext4_guard: Option<Arc<Ext4Fs>>,
}

pub(crate) struct Vfs {
    mounts: Vec<MountPoint>,
}

impl Vfs {
    pub(crate) fn new() -> Self {
        Self {
            mounts: vec![MountPoint {
                path: String::from("/"),
                root: Arc::new(NullInode),
                #[cfg(feature = "ext4")]
                _ext4_guard: None,
            }],
        }
    }

    pub(crate) fn mount_root(&mut self, root: Arc<dyn VfsInode>) {
        if let Some(mount) = self.mounts.iter_mut().find(|m| m.path == "/") {
            mount.root = root;
        } else {
            self.mounts.push(MountPoint {
                path: String::from("/"),
                root,
                #[cfg(feature = "ext4")]
                _ext4_guard: None,
            });
        }
    }

    pub(crate) fn mount_at(&mut self, path: &str, root: Arc<dyn VfsInode>) {
        let path = normalize_path(path);
        if let Some(mount) = self.mounts.iter_mut().find(|m| m.path == path) {
            mount.root = root;
            #[cfg(feature = "ext4")]
            {
                mount._ext4_guard = None;
            }
        } else {
            self.mounts.push(MountPoint {
                path,
                root,
                #[cfg(feature = "ext4")]
                _ext4_guard: None,
            });
        }
    }

    #[cfg(feature = "ext4")]
    pub(crate) fn mount_root_ext4(&mut self, root: Arc<dyn VfsInode>, guard: Arc<Ext4Fs>) {
        if let Some(mount) = self.mounts.iter_mut().find(|m| m.path == "/") {
            mount.root = root;
            mount._ext4_guard = Some(guard);
        } else {
            self.mounts.push(MountPoint {
                path: String::from("/"),
                root,
                _ext4_guard: Some(guard),
            });
        }
    }

    fn resolve_mount<'a>(&'a self, path: &'a str) -> Option<(&'a MountPoint, &'a str)> {
        let mut best: Option<&MountPoint> = None;
        for mount in &self.mounts {
            let is_root = mount.path == "/";
            if is_root || path == mount.path || path.starts_with(&(mount.path.clone() + "/")) {
                if best
                    .as_ref()
                    .map(|m| m.path.len() < mount.path.len())
                    .unwrap_or(true)
                {
                    best = Some(mount);
                }
            }
        }
        let mount = best?;
        let rel = if mount.path == "/" {
            path.trim_start_matches('/')
        } else {
            path.get(mount.path.len()..)?.trim_start_matches('/')
        };
        Some((mount, rel))
    }

    fn resolve_inner(&self, path: &str, log_missing: bool) -> Option<Arc<dyn VfsInode>> {
        let path = normalize_path(path);
        let (mount, rel) = self.resolve_mount(&path)?;
        let mut current = mount.root.clone();
        if rel.is_empty() {
            return Some(current);
        }
        trace!("vfs: resolve path={} rel={}", path, rel);
        for comp in rel.split('/').filter(|s| !s.is_empty()) {
            match current.lookup(comp) {
                Some(next) => current = next,
                None => {
                    if log_missing {
                        error!("vfs: resolve failed at {} for {}", comp, path);
                    }
                    return None;
                }
            }
        }
        Some(current)
    }

    pub(crate) fn resolve(&self, path: &str) -> Option<Arc<dyn VfsInode>> {
        self.resolve_inner(path, true)
    }

    pub(crate) fn resolve_quiet(&self, path: &str) -> Option<Arc<dyn VfsInode>> {
        self.resolve_inner(path, false)
    }

    pub(crate) fn resolve_parent(&self, path: &str) -> Option<(Arc<dyn VfsInode>, String)> {
        let path = normalize_path(path);
        if path == "/" {
            return None;
        }
        let mut comps: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let name = comps.pop()?.to_string();
        let parent_path = if comps.is_empty() {
            String::from("/")
        } else {
            format!("/{}", comps.join("/"))
        };
        let parent = self.resolve(&parent_path)?;
        Some((parent, name))
    }

    pub(crate) fn root_inode(&self) -> Option<Arc<dyn VfsInode>> {
        self.resolve("/")
    }
}

lazy_static! {
    pub(crate) static ref ROOT_VFS: UPIntrFreeCell<Vfs> = unsafe { UPIntrFreeCell::new(Vfs::new()) };
}

struct NullInode;

impl VfsInode for NullInode {
    fn kind(&self) -> VfsNodeKind {
        VfsNodeKind::Dir
    }

    fn read_at(&self, _offset: usize, _buf: &mut [u8]) -> usize {
        0
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
}

pub(crate) fn normalize_path(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for comp in path.split('/') {
        match comp {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(comp),
        }
    }
    if parts.is_empty() {
        String::from("/")
    } else {
        format!("/{}", parts.join("/"))
    }
}
