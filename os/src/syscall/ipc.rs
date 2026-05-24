//! System V IPC (Inter-Process Communication) implementation
//!
//! This module implements System V IPC mechanisms:
//! - Message queues
//! - Shared memory segments
//!
//! Reference: Linux man pages for msgget(2), msgsnd(2), msgrcv(2), msgctl(2),
//!            shmget(2), shmat(2), shmdt(2), shmctl(2)

#![allow(dead_code)]

use super::errno::*;
use super::user_mem::{self, UserReadPolicy, UserWritePolicy};
use crate::{
    config::PAGE_SIZE,
    mm::{frame_alloc, FrameTracker, MapPermission, VirtAddr},
    task::{current_process, current_user_token},
    timer::get_time_us,
};
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};
use spin::Mutex;

/// IPC key type (used to identify IPC objects)
pub type IpcKey = i32;

/// IPC identifier type (returned by xxxget() functions)
pub type IpcId = i32;

/// Special IPC key value for private IPC objects
pub const IPC_PRIVATE: IpcKey = 0;

/// IPC flags
pub const IPC_CREAT: i32 = 0o1000; // Create if key doesn't exist
pub const IPC_EXCL: i32 = 0o2000; // Fail if key exists
pub const IPC_NOWAIT: i32 = 0o4000; // Return immediately if would block
pub const MSG_NOERROR: i32 = 0o10000;
pub const MSG_EXCEPT: i32 = 0o20000;
pub const MSG_COPY: i32 = 0o40000;

/// IPC control commands
pub const IPC_RMID: i32 = 0; // Remove identifier
pub const IPC_SET: i32 = 1; // Set options
pub const IPC_STAT: i32 = 2; // Get options
pub const IPC_INFO: i32 = 3; // Get info
pub const MSG_STAT: i32 = 11;
pub const MSG_INFO: i32 = 12;

/// Message queue limits
const MSGMAX: usize = 8192; // Max message size
const MSGMNB: usize = 16384; // Max queue size in bytes
const MSGMNI_DEFAULT: usize = 16; // Conservative default keeps msgstress within heap budget.

static MSGMNI_LIMIT: AtomicUsize = AtomicUsize::new(MSGMNI_DEFAULT);

/// Shared memory limits
const SHMMAX: usize = 32 * 1024 * 1024; // Max segment size (32MB)
const SHMMIN: usize = 1; // Min segment size
const SHMMNI: usize = 128; // Max number of segments
const SHMSEG: usize = 128; // Max segments per process

#[repr(C)]
#[derive(Clone, Copy, Default)]
#[cfg(target_arch = "loongarch64")]
struct IpcPermUser {
    key: i32,
    uid: u32,
    gid: u32,
    cuid: u32,
    cgid: u32,
    mode: u32,
    seq: i32,
    pad2: u16,
    pad3: u16,
    unused1: u64,
    unused2: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
#[cfg(not(target_arch = "loongarch64"))]
struct IpcPermUser {
    key: i32,
    uid: u32,
    gid: u32,
    cuid: u32,
    cgid: u32,
    mode: u16,
    pad1: u16,
    seq: u16,
    pad2: u16,
    unused1: u64,
    unused2: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct MsqidDsUser {
    msg_perm: IpcPermUser,
    msg_stime: i64,
    msg_rtime: i64,
    msg_ctime: i64,
    msg_cbytes: u64,
    msg_qnum: u64,
    msg_qbytes: u64,
    msg_lspid: i32,
    msg_lrpid: i32,
    unused4: u64,
    unused5: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct MsgInfoUser {
    msgpool: i32,
    msgmap: i32,
    msgmax: i32,
    msgmnb: i32,
    msgmni: i32,
    msgssz: i32,
    msgtql: i32,
    msgseg: i32,
}

fn current_ipc_identity() -> (u32, u32, usize, bool) {
    let process = current_process();
    let pid = process.pid.0;
    let creds = process.credentials_snapshot();
    let euid = creds.effective_uid;
    let egid = creds.effective_gid;
    (euid, egid, pid, euid == 0)
}

fn now_epoch_sec() -> i64 {
    (get_time_us() / 1_000_000) as i64
}

fn requested_rw_bits(msgflg: i32) -> u16 {
    let raw = (msgflg & 0o777) as u16;
    if raw == 0 {
        0
    } else if raw & 0o700 != 0 {
        (raw >> 6) & 0o7
    } else if raw & 0o070 != 0 {
        (raw >> 3) & 0o7
    } else {
        raw & 0o7
    }
}

fn msgq_to_user_ds(msgq: &MessageQueue) -> MsqidDsUser {
    MsqidDsUser {
        msg_perm: make_ipc_perm_user(msgq),
        msg_stime: msgq.stime,
        msg_rtime: msgq.rtime,
        msg_ctime: msgq.ctime,
        msg_cbytes: msgq.total_bytes as u64,
        msg_qnum: msgq.messages.len() as u64,
        msg_qbytes: msgq.max_bytes as u64,
        msg_lspid: msgq.lspid,
        msg_lrpid: msgq.lrpid,
        unused4: 0,
        unused5: 0,
    }
}

#[cfg(target_arch = "loongarch64")]
fn make_ipc_perm_user(msgq: &MessageQueue) -> IpcPermUser {
    IpcPermUser {
        key: msgq.key,
        uid: msgq.uid,
        gid: msgq.gid,
        cuid: msgq.cuid,
        cgid: msgq.cgid,
        mode: (msgq.permissions & 0o777) as u32,
        seq: 0,
        pad2: 0,
        pad3: 0,
        unused1: 0,
        unused2: 0,
    }
}

#[cfg(not(target_arch = "loongarch64"))]
fn make_ipc_perm_user(msgq: &MessageQueue) -> IpcPermUser {
    IpcPermUser {
        key: msgq.key,
        uid: msgq.uid,
        gid: msgq.gid,
        cuid: msgq.cuid,
        cgid: msgq.cgid,
        mode: (msgq.permissions & 0o777) as u16,
        pad1: 0,
        seq: 0,
        pad2: 0,
        unused1: 0,
        unused2: 0,
    }
}

/// Message structure for message queues
#[derive(Clone)]
pub struct Message {
    pub mtype: isize,   // Message type (must be > 0)
    pub mtext: Vec<u8>, // Message data
}

/// Message queue structure
pub struct MessageQueue {
    pub key: IpcKey,
    pub id: IpcId,
    pub messages: Vec<Message>,
    pub total_bytes: usize,
    pub max_bytes: usize,
    pub permissions: u32,
    pub uid: u32,
    pub gid: u32,
    pub cuid: u32,
    pub cgid: u32,
    pub ctime: i64,
    pub stime: i64,
    pub rtime: i64,
    pub lspid: i32,
    pub lrpid: i32,
}

impl MessageQueue {
    pub fn new(key: IpcKey, id: IpcId, permissions: u32, uid: u32, gid: u32, now_sec: i64) -> Self {
        Self {
            key,
            id,
            messages: Vec::new(),
            total_bytes: 0,
            max_bytes: MSGMNB,
            permissions,
            uid,
            gid,
            cuid: uid,
            cgid: gid,
            ctime: now_sec,
            stime: 0,
            rtime: 0,
            lspid: 0,
            lrpid: 0,
        }
    }

    pub fn access_bits_for(&self, uid: u32, gid: u32) -> u16 {
        if uid == self.uid || uid == self.cuid {
            ((self.permissions >> 6) & 0o7) as u16
        } else if gid == self.gid || gid == self.cgid {
            ((self.permissions >> 3) & 0o7) as u16
        } else {
            (self.permissions & 0o7) as u16
        }
    }

    pub fn has_access(&self, uid: u32, gid: u32, req: u16) -> bool {
        if req == 0 {
            return true;
        }
        let granted = self.access_bits_for(uid, gid);
        (granted & req) == req
    }

    pub fn can_ctl(&self, uid: u32) -> bool {
        uid == self.uid || uid == self.cuid
    }

    pub fn send(&mut self, msg: Message, pid: usize, now_sec: i64) -> Result<(), isize> {
        let msg_size = msg.mtext.len();

        if msg_size > MSGMAX {
            return Err(errno(EINVAL));
        }

        if self.total_bytes + msg_size > self.max_bytes {
            return Err(errno(EAGAIN));
        }

        self.messages.push(msg);
        self.total_bytes += msg_size;
        self.stime = now_sec;
        self.lspid = pid as i32;
        Ok(())
    }

    pub fn receive(
        &mut self,
        msgtyp: isize,
        msgsz: usize,
        msgflg: i32,
        pid: usize,
        now_sec: i64,
    ) -> Result<Message, isize> {
        let msg_noerror = msgflg & MSG_NOERROR != 0;
        let msg_except = msgflg & MSG_EXCEPT != 0;
        let msg_copy = msgflg & MSG_COPY != 0;

        if msg_copy {
            if msgflg & IPC_NOWAIT == 0 || msg_except || msgtyp < 0 {
                return Err(errno(EINVAL));
            }
            let idx = msgtyp as usize;
            let Some(msg) = self.messages.get(idx) else {
                return Err(errno(ENOMSG));
            };
            if msg.mtext.len() > msgsz && !msg_noerror {
                return Err(errno(E2BIG));
            }
            return Ok(msg.clone());
        }

        let pos = if msgtyp == 0 {
            if self.messages.is_empty() {
                return Err(errno(ENOMSG));
            }
            Some(0)
        } else if msgtyp > 0 {
            if msg_except {
                self.messages.iter().position(|m| m.mtype != msgtyp)
            } else {
                self.messages.iter().position(|m| m.mtype == msgtyp)
            }
        } else {
            let abs_type = -msgtyp;
            let mut best_idx: Option<usize> = None;
            let mut best_type: Option<isize> = None;
            for (idx, msg) in self.messages.iter().enumerate() {
                if msg.mtype <= abs_type {
                    match best_type {
                        None => {
                            best_type = Some(msg.mtype);
                            best_idx = Some(idx);
                        }
                        Some(t) if msg.mtype < t => {
                            best_type = Some(msg.mtype);
                            best_idx = Some(idx);
                        }
                        _ => {}
                    }
                }
            }
            best_idx
        };

        match pos {
            Some(idx) => {
                let msg_size = self.messages[idx].mtext.len();
                if msg_size > msgsz && !msg_noerror {
                    return Err(errno(E2BIG));
                }
                let msg = self.messages.remove(idx);
                self.total_bytes -= msg.mtext.len();
                self.rtime = now_sec;
                self.lrpid = pid as i32;
                Ok(msg)
            }
            None => {
                if msgflg & IPC_NOWAIT != 0 {
                    Err(errno(ENOMSG))
                } else {
                    // Should block, but we don't support blocking yet
                    Err(errno(ENOMSG))
                }
            }
        }
    }
}

/// Shared memory segment structure
pub struct ShmSegment {
    pub key: IpcKey,
    pub id: IpcId,
    pub size: usize,
    pub frames: Vec<Arc<FrameTracker>>,
    pub permissions: u32,
    pub attach_count: usize,
    pub marked_for_delete: bool,
}

impl ShmSegment {
    pub fn new(key: IpcKey, id: IpcId, size: usize, permissions: u32) -> Result<Self, isize> {
        let aligned_size = align_up(size, PAGE_SIZE);
        let page_count = aligned_size / PAGE_SIZE;
        let mut frames = Vec::with_capacity(page_count);
        for _ in 0..page_count {
            let frame = frame_alloc().ok_or(errno(ENOMEM))?;
            frames.push(Arc::new(frame));
        }
        Ok(Self {
            key,
            id,
            size: aligned_size,
            frames,
            permissions,
            attach_count: 0,
            marked_for_delete: false,
        })
    }
}

/// Global IPC resource manager
pub struct IpcManager {
    message_queues: BTreeMap<IpcId, Arc<Mutex<MessageQueue>>>,
    shm_segments: BTreeMap<IpcId, Arc<Mutex<ShmSegment>>>,
    shm_attachments: BTreeMap<(usize, usize), (IpcId, usize)>,
    next_msgid: IpcId,
    next_shmid: IpcId,
}

impl IpcManager {
    pub const fn new() -> Self {
        Self {
            message_queues: BTreeMap::new(),
            shm_segments: BTreeMap::new(),
            shm_attachments: BTreeMap::new(),
            next_msgid: 1,
            next_shmid: 1,
        }
    }

    pub fn create_msgq(
        &mut self,
        key: IpcKey,
        permissions: u32,
        uid: u32,
        gid: u32,
    ) -> Result<IpcId, isize> {
        if self.message_queues.len() >= msgmni_limit() {
            return Err(errno(ENOSPC));
        }
        let id = self.next_msgid;
        self.next_msgid += 1;
        let msgq = Arc::new(Mutex::new(MessageQueue::new(
            key,
            id,
            permissions,
            uid,
            gid,
            now_epoch_sec(),
        )));
        self.message_queues.insert(id, msgq);
        Ok(id)
    }

    pub fn get_msgq(&self, id: IpcId) -> Option<Arc<Mutex<MessageQueue>>> {
        self.message_queues.get(&id).cloned()
    }

    pub fn find_msgq_by_key(&self, key: IpcKey) -> Option<(IpcId, Arc<Mutex<MessageQueue>>)> {
        for (id, msgq) in self.message_queues.iter() {
            if msgq.lock().key == key {
                return Some((*id, msgq.clone()));
            }
        }
        None
    }

    pub fn remove_msgq(&mut self, id: IpcId) -> bool {
        self.message_queues.remove(&id).is_some()
    }

    pub fn create_shm(
        &mut self,
        key: IpcKey,
        size: usize,
        permissions: u32,
    ) -> Result<IpcId, isize> {
        if self.shm_segments.len() >= SHMMNI {
            return Err(errno(ENOMEM));
        }
        let id = self.next_shmid;
        self.next_shmid += 1;
        let shm = Arc::new(Mutex::new(ShmSegment::new(key, id, size, permissions)?));
        self.shm_segments.insert(id, shm);
        Ok(id)
    }

    pub fn get_shm(&self, id: IpcId) -> Option<Arc<Mutex<ShmSegment>>> {
        self.shm_segments.get(&id).cloned()
    }

    pub fn find_shm_by_key(&self, key: IpcKey) -> Option<(IpcId, Arc<Mutex<ShmSegment>>)> {
        for (id, shm) in self.shm_segments.iter() {
            let shm_locked = shm.lock();
            if shm_locked.key == key && !shm_locked.marked_for_delete {
                return Some((*id, shm.clone()));
            }
        }
        None
    }

    pub fn remove_shm(&mut self, id: IpcId) -> bool {
        let removed = self.shm_segments.remove(&id).is_some();
        if removed {
            self.shm_attachments.retain(|_, (shmid, _)| *shmid != id);
        }
        removed
    }

    pub fn attachment_count_of_pid(&self, pid: usize) -> usize {
        self.shm_attachments
            .keys()
            .filter(|(owner_pid, _)| *owner_pid == pid)
            .count()
    }

    pub fn add_attachment(
        &mut self,
        pid: usize,
        addr: usize,
        shmid: IpcId,
        size: usize,
    ) -> Result<(), isize> {
        if self.shm_attachments.contains_key(&(pid, addr)) {
            return Err(errno(EINVAL));
        }
        self.shm_attachments.insert((pid, addr), (shmid, size));
        Ok(())
    }

    pub fn remove_attachment(&mut self, pid: usize, addr: usize) -> Option<(IpcId, usize)> {
        self.shm_attachments.remove(&(pid, addr))
    }
}

#[inline]
fn align_up(value: usize, align: usize) -> usize {
    (value + align - 1) & !(align - 1)
}

/// Global IPC manager instance
static IPC_MANAGER: Mutex<IpcManager> = Mutex::new(IpcManager::new());

pub fn msgmni_limit() -> usize {
    MSGMNI_LIMIT.load(Ordering::Relaxed)
}

pub fn proc_kernel_msgmni() -> String {
    alloc::format!("{}\n", msgmni_limit())
}

pub fn set_msgmni_from_proc_write(buf: &[u8]) -> usize {
    let Ok(raw) = core::str::from_utf8(buf) else {
        return buf.len();
    };
    let trimmed = raw.trim();
    if let Ok(limit) = trimmed.parse::<usize>() {
        if limit > 0 {
            MSGMNI_LIMIT.store(limit, Ordering::Relaxed);
        }
    }
    buf.len()
}

/// Render `/proc/sysvipc/msg` in a Linux-compatible tabular layout.
pub fn proc_sysvipc_msg() -> String {
    let queues: Vec<Arc<Mutex<MessageQueue>>> = {
        let manager = IPC_MANAGER.lock();
        manager.message_queues.values().cloned().collect()
    };

    let mut out = String::from(
        "       key      msqid perms      cbytes       qnum lspid lrpid   uid   gid  cuid  cgid      stime      rtime      ctime\n",
    );
    for msgq in queues {
        let q = msgq.lock();
        let _ = core::fmt::Write::write_fmt(
            &mut out,
            format_args!(
                "{:>10x} {:>10} {:>5o} {:>11} {:>10} {:>5} {:>5} {:>5} {:>5} {:>5} {:>5} {:>10} {:>10} {:>10}\n",
                q.key as u32,
                q.id,
                q.permissions & 0o777,
                q.total_bytes,
                q.messages.len(),
                q.lspid,
                q.lrpid,
                q.uid,
                q.gid,
                q.cuid,
                q.cgid,
                q.stime,
                q.rtime,
                q.ctime
            ),
        );
    }
    out
}

// ============================================================================
// Message Queue System Calls
// ============================================================================

/// msgget - Get a message queue identifier
///
/// # Arguments
/// - key: IPC key
/// - msgflg: flags (IPC_CREAT, IPC_EXCL, permissions)
///
/// # Returns
/// - Success: message queue identifier
/// - Failure: -errno
pub fn sys_msgget(key: IpcKey, msgflg: i32) -> isize {
    let (euid, egid, _pid, is_super) = current_ipc_identity();
    let mut manager = IPC_MANAGER.lock();

    // Check if we're creating a private queue
    if key == IPC_PRIVATE {
        let permissions = (msgflg & 0o777) as u32;
        return match manager.create_msgq(key, permissions, euid, egid) {
            Ok(id) => id as isize,
            Err(e) => e,
        };
    }

    // Try to find existing queue with this key
    if let Some((id, msgq)) = manager.find_msgq_by_key(key) {
        // Queue exists
        if msgflg & IPC_CREAT != 0 && msgflg & IPC_EXCL != 0 {
            // Exclusive creation requested but queue exists
            return errno(EEXIST);
        }

        let req = requested_rw_bits(msgflg);
        if !is_super && req != 0 {
            let q = msgq.lock();
            if !q.has_access(euid, egid, req) {
                return errno(EACCES);
            }
        }
        return id as isize;
    }

    // Queue doesn't exist
    if msgflg & IPC_CREAT == 0 {
        // Not creating, so return error
        return errno(ENOENT);
    }

    // Create new queue
    let permissions = (msgflg & 0o777) as u32;
    match manager.create_msgq(key, permissions, euid, egid) {
        Ok(id) => id as isize,
        Err(e) => e,
    }
}

/// msgsnd - Send a message to a message queue
///
/// # Arguments
/// - msqid: message queue identifier
/// - msgp: pointer to message structure
/// - msgsz: size of message text
/// - msgflg: flags (IPC_NOWAIT)
///
/// # Returns
/// - Success: 0
/// - Failure: -errno
pub fn sys_msgsnd(msqid: i32, msgp: usize, msgsz: usize, _msgflg: i32) -> isize {
    if msgsz > MSGMAX {
        return errno(EINVAL);
    }
    if msgp == 0 {
        return errno(EFAULT);
    }
    let (euid, egid, pid, is_super) = current_ipc_identity();

    let manager = IPC_MANAGER.lock();
    let msgq = match manager.get_msgq(msqid) {
        Some(q) => q,
        None => return errno(EINVAL),
    };

    drop(manager);

    {
        let q = msgq.lock();
        if !is_super && !q.has_access(euid, egid, 0o2) {
            return errno(EACCES);
        }
    }

    // Read message from user space
    // Message structure: [mtype: isize][mtext: u8 array]
    let token = current_user_token();

    // Read mtype (first sizeof(isize) bytes)
    let mtype_size = core::mem::size_of::<isize>();
    let mtype = match user_mem::read_from_user::<isize>(
        token,
        msgp as *const isize,
        UserReadPolicy::DemandPaged,
    ) {
        Ok(v) => v,
        Err(_) => return errno(EFAULT),
    };

    if mtype <= 0 {
        return errno(EINVAL);
    }

    // Read mtext
    let mut mtext = Vec::new();
    mtext.resize(msgsz, 0);
    if user_mem::copy_from_user(
        token,
        (msgp + mtype_size) as *const u8,
        mtext.as_mut_slice(),
        UserReadPolicy::DemandPaged,
    )
    .is_err()
    {
        return errno(EFAULT);
    }

    let msg = Message { mtype, mtext };

    let mut msgq = msgq.lock();
    match msgq.send(msg, pid, now_epoch_sec()) {
        Ok(()) => 0,
        Err(e) => e,
    }
}

/// msgrcv - Receive a message from a message queue
///
/// # Arguments
/// - msqid: message queue identifier
/// - msgp: pointer to message buffer
/// - msgsz: size of message buffer
/// - msgtyp: message type to receive
/// - msgflg: flags (IPC_NOWAIT)
///
/// # Returns
/// - Success: number of bytes in message text
/// - Failure: -errno
pub fn sys_msgrcv(msqid: i32, msgp: usize, msgsz: usize, msgtyp: isize, msgflg: i32) -> isize {
    if msgp == 0 {
        return errno(EFAULT);
    }
    if msgsz > MSGMAX {
        return errno(EINVAL);
    }
    let (euid, egid, pid, is_super) = current_ipc_identity();

    let manager = IPC_MANAGER.lock();
    let msgq = match manager.get_msgq(msqid) {
        Some(q) => q,
        None => return errno(EINVAL),
    };

    drop(manager);

    {
        let q = msgq.lock();
        if !is_super && !q.has_access(euid, egid, 0o4) {
            return errno(EACCES);
        }
    }

    let mut msgq = msgq.lock();
    let msg = match msgq.receive(msgtyp, msgsz, msgflg, pid, now_epoch_sec()) {
        Ok(m) => m,
        Err(e) => return e,
    };

    let msg_size = msg.mtext.len();
    if msg_size > msgsz {
        if msgflg & MSG_NOERROR == 0 {
            // MSG_NOERROR not set
            return errno(E2BIG);
        }
        // Truncate message
    }

    let actual_size = msg_size.min(msgsz);

    // Write message to user space
    let token = current_user_token();

    // Write mtype
    let mtype_size = core::mem::size_of::<isize>();
    if user_mem::copy_to_user(
        token,
        msgp as *mut u8,
        &msg.mtype.to_ne_bytes()[..mtype_size],
        UserWritePolicy::DemandCowWithForkFallback,
    )
    .is_err()
    {
        return errno(EFAULT);
    }

    // Write mtext
    if user_mem::copy_to_user(
        token,
        (msgp + mtype_size) as *mut u8,
        &msg.mtext[..actual_size],
        UserWritePolicy::DemandCowWithForkFallback,
    )
    .is_err()
    {
        return errno(EFAULT);
    }

    actual_size as isize
}

/// msgctl - Message queue control operations
///
/// # Arguments
/// - msqid: message queue identifier
/// - cmd: command (IPC_STAT, IPC_SET, IPC_RMID)
/// - buf: pointer to msqid_ds structure
///
/// # Returns
/// - Success: 0
/// - Failure: -errno
pub fn sys_msgctl(msqid: i32, cmd: i32, _buf: usize) -> isize {
    let (euid, egid, _pid, is_super) = current_ipc_identity();
    let cmd = cmd & 0xff;
    let mut manager = IPC_MANAGER.lock();

    match cmd {
        IPC_RMID => {
            let Some(msgq) = manager.get_msgq(msqid) else {
                return errno(EINVAL);
            };
            let q = msgq.lock();
            if !is_super && !q.can_ctl(euid) {
                return errno(EPERM);
            }
            drop(q);
            if manager.remove_msgq(msqid) {
                0
            } else {
                errno(EINVAL)
            }
        }
        IPC_STAT => {
            if _buf == 0 {
                return errno(EFAULT);
            }
            let Some(msgq) = manager.get_msgq(msqid) else {
                return errno(EINVAL);
            };
            drop(manager);
            let ds = {
                let q = msgq.lock();
                if !is_super && !q.has_access(euid, egid, 0o4) {
                    return errno(EACCES);
                }
                msgq_to_user_ds(&q)
            };
            let token = current_user_token();
            let bytes = unsafe {
                core::slice::from_raw_parts(
                    (&ds as *const MsqidDsUser).cast::<u8>(),
                    core::mem::size_of::<MsqidDsUser>(),
                )
            };
            if user_mem::copy_to_user(
                token,
                _buf as *mut u8,
                bytes,
                UserWritePolicy::DemandCowWithForkFallback,
            )
            .is_err()
            {
                return errno(EFAULT);
            }
            0
        }
        IPC_SET => {
            if _buf == 0 {
                return errno(EFAULT);
            }
            let Some(msgq) = manager.get_msgq(msqid) else {
                return errno(EINVAL);
            };
            drop(manager);

            let token = current_user_token();
            let user_ds = match user_mem::read_from_user::<MsqidDsUser>(
                token,
                _buf as *const MsqidDsUser,
                UserReadPolicy::DemandPaged,
            ) {
                Ok(v) => v,
                Err(_) => return errno(EFAULT),
            };

            let mut q = msgq.lock();
            if !is_super && !q.can_ctl(euid) {
                return errno(EPERM);
            }
            let requested_qbytes = user_ds.msg_qbytes as usize;
            if requested_qbytes == 0 || requested_qbytes > MSGMNB {
                return errno(EINVAL);
            }
            q.permissions = (user_ds.msg_perm.mode as u32) & 0o777;
            q.max_bytes = requested_qbytes;
            q.ctime = now_epoch_sec();
            0
        }
        IPC_INFO | MSG_INFO => {
            if _buf == 0 {
                return errno(EFAULT);
            }
            let info = MsgInfoUser {
                msgpool: MSGMNB as i32,
                msgmap: msgmni_limit() as i32,
                msgmax: MSGMAX as i32,
                msgmnb: MSGMNB as i32,
                msgmni: msgmni_limit() as i32,
                msgssz: 0,
                msgtql: 0,
                msgseg: 0,
            };
            let token = current_user_token();
            let bytes = unsafe {
                core::slice::from_raw_parts(
                    (&info as *const MsgInfoUser).cast::<u8>(),
                    core::mem::size_of::<MsgInfoUser>(),
                )
            };
            if user_mem::copy_to_user(
                token,
                _buf as *mut u8,
                bytes,
                UserWritePolicy::DemandCowWithForkFallback,
            )
            .is_err()
            {
                return errno(EFAULT);
            }
            manager
                .message_queues
                .keys()
                .next_back()
                .copied()
                .unwrap_or(0) as isize
        }
        MSG_STAT => {
            if _buf == 0 {
                return errno(EFAULT);
            }
            let Some(msgq) = manager.get_msgq(msqid) else {
                return errno(EINVAL);
            };
            drop(manager);
            let ds = {
                let q = msgq.lock();
                if !is_super && !q.has_access(euid, egid, 0o4) {
                    return errno(EACCES);
                }
                msgq_to_user_ds(&q)
            };
            let token = current_user_token();
            let bytes = unsafe {
                core::slice::from_raw_parts(
                    (&ds as *const MsqidDsUser).cast::<u8>(),
                    core::mem::size_of::<MsqidDsUser>(),
                )
            };
            if user_mem::copy_to_user(
                token,
                _buf as *mut u8,
                bytes,
                UserWritePolicy::DemandCowWithForkFallback,
            )
            .is_err()
            {
                return errno(EFAULT);
            }
            msqid as isize
        }
        _ => errno(EINVAL),
    }
}

// ============================================================================
// Shared Memory System Calls
// ============================================================================

/// shmget - Get a shared memory identifier
///
/// # Arguments
/// - key: IPC key
/// - size: segment size in bytes
/// - shmflg: flags (IPC_CREAT, IPC_EXCL, permissions)
///
/// # Returns
/// - Success: shared memory identifier
/// - Failure: -errno
pub fn sys_shmget(key: IpcKey, size: usize, shmflg: i32) -> isize {
    if size < SHMMIN || size > SHMMAX {
        return errno(EINVAL);
    }

    let mut manager = IPC_MANAGER.lock();

    // Check if we're creating a private segment
    if key == IPC_PRIVATE {
        let permissions = (shmflg & 0o777) as u32;
        return match manager.create_shm(key, size, permissions) {
            Ok(id) => id as isize,
            Err(e) => e,
        };
    }

    // Try to find existing segment with this key
    if let Some((id, shm)) = manager.find_shm_by_key(key) {
        // Segment exists
        if shmflg & IPC_CREAT != 0 && shmflg & IPC_EXCL != 0 {
            // Exclusive creation requested but segment exists
            return errno(EEXIST);
        }
        // Check size matches
        let shm_size = shm.lock().size;
        if size > shm_size {
            return errno(EINVAL);
        }
        return id as isize;
    }

    // Segment doesn't exist
    if shmflg & IPC_CREAT == 0 {
        // Not creating, so return error
        return errno(ENOENT);
    }

    // Create new segment
    let permissions = (shmflg & 0o777) as u32;
    match manager.create_shm(key, size, permissions) {
        Ok(id) => id as isize,
        Err(e) => e,
    }
}

/// shmat - Attach shared memory segment
///
/// # Arguments
/// - shmid: shared memory identifier
/// - shmaddr: desired attach address (0 = let system choose)
/// - shmflg: flags (SHM_RDONLY, SHM_RND)
///
/// # Returns
/// - Success: attached address
/// - Failure: -errno
pub fn sys_shmat(shmid: i32, shmaddr: usize, _shmflg: i32) -> isize {
    const SHM_RDONLY: i32 = 0o10000;
    const SHM_RND: i32 = 0o20000;
    const SHM_EXEC: i32 = 0o100000;

    let process = current_process();
    let pid = process.pid.0;
    let mut manager = IPC_MANAGER.lock();
    let shm = match manager.get_shm(shmid) {
        Some(s) => s,
        None => {
            error!("[shm] shmat invalid shmid={} pid={}", shmid, pid);
            return errno(EINVAL);
        }
    };
    if manager.attachment_count_of_pid(pid) >= SHMSEG {
        error!(
            "[shm] shmat pid={} exceeds attachment limit {}",
            pid, SHMSEG
        );
        return errno(EMFILE);
    }

    let (size, frames) = {
        let shm_locked = shm.lock();
        if shm_locked.marked_for_delete {
            error!("[shm] shmat on deleted segment shmid={} pid={}", shmid, pid);
            return errno(EINVAL);
        }
        (shm_locked.size, shm_locked.frames.clone())
    };

    let mut map_perm = MapPermission::U | MapPermission::R;
    if _shmflg & SHM_RDONLY == 0 {
        map_perm |= MapPermission::W;
    }
    if _shmflg & SHM_EXEC != 0 {
        map_perm |= MapPermission::X;
    }

    let mut inner = process.inner_exclusive_access();
    let mmap_base_before = inner.mmap_base;
    let attach_addr = if shmaddr == 0 {
        let base = align_up(inner.mmap_base, PAGE_SIZE);
        inner.mmap_base = match base.checked_add(size) {
            Some(v) => v,
            None => {
                error!(
                    "[shm] shmat mmap_base overflow pid={} base={:#x} size={:#x}",
                    pid, base, size
                );
                return errno(ENOMEM);
            }
        };
        base
    } else {
        if (shmaddr & (PAGE_SIZE - 1)) != 0 {
            if _shmflg & SHM_RND != 0 {
                shmaddr & !(PAGE_SIZE - 1)
            } else {
                error!(
                    "[shm] shmat misaligned addr without SHM_RND pid={} shmid={} addr={:#x}",
                    pid, shmid, shmaddr
                );
                return errno(EINVAL);
            }
        } else {
            shmaddr
        }
    };
    let attach_end = match attach_addr.checked_add(size) {
        Some(v) => v,
        None => {
            error!(
                "[shm] shmat attach_end overflow pid={} shmid={} addr={:#x} size={:#x}",
                pid, shmid, attach_addr, size
            );
            return errno(ENOMEM);
        }
    };

    let overlap = inner
        .memory_set
        .overlap_count(VirtAddr(attach_addr), VirtAddr(attach_end));
    if overlap > 0 {
        error!(
            "[shm] shmat overlap pid={} shmid={} addr=[{:#x},{:#x}) overlap={} mmap_base_before={:#x} mmap_base_after={:#x}",
            pid,
            shmid,
            attach_addr,
            attach_end,
            overlap,
            mmap_base_before,
            inner.mmap_base
        );
        return errno(EINVAL);
    }
    if !inner.memory_set.insert_shm_area(
        VirtAddr(attach_addr),
        VirtAddr(attach_end),
        map_perm,
        frames,
    ) {
        error!(
            "[shm] shmat map failed pid={} shmid={} addr=[{:#x},{:#x})",
            pid, shmid, attach_addr, attach_end
        );
        return errno(EINVAL);
    }
    drop(inner);

    if manager
        .add_attachment(pid, attach_addr, shmid, size)
        .is_err()
    {
        let mut inner = process.inner_exclusive_access();
        inner
            .memory_set
            .remove_area_with_start_vpn(VirtAddr(attach_addr).floor());
        error!(
            "[shm] shmat attachment table conflict pid={} shmid={} addr={:#x}",
            pid, shmid, attach_addr
        );
        return errno(EINVAL);
    }
    let mut shm_locked = shm.lock();
    shm_locked.attach_count += 1;
    attach_addr as isize
}

/// shmdt - Detach shared memory segment
///
/// # Arguments
/// - shmaddr: address of attached segment
///
/// # Returns
/// - Success: 0
/// - Failure: -errno
pub fn sys_shmdt(shmaddr: usize) -> isize {
    if shmaddr == 0 {
        return errno(EINVAL);
    }

    let process = current_process();
    let pid = process.pid.0;
    let mut manager = IPC_MANAGER.lock();
    let (shmid, _size) = match manager.remove_attachment(pid, shmaddr) {
        Some(v) => v,
        None => return errno(EINVAL),
    };

    {
        let mut inner = process.inner_exclusive_access();
        inner
            .memory_set
            .remove_area_with_start_vpn(VirtAddr(shmaddr).floor());
    }

    let mut should_remove = false;
    if let Some(shm) = manager.get_shm(shmid) {
        let mut shm_locked = shm.lock();
        if shm_locked.attach_count > 0 {
            shm_locked.attach_count -= 1;
        }
        should_remove = shm_locked.marked_for_delete && shm_locked.attach_count == 0;
    }
    if should_remove {
        manager.remove_shm(shmid);
    }

    0
}

/// shmctl - Shared memory control operations
///
/// # Arguments
/// - shmid: shared memory identifier
/// - cmd: command (IPC_STAT, IPC_SET, IPC_RMID)
/// - buf: pointer to shmid_ds structure
///
/// # Returns
/// - Success: 0
/// - Failure: -errno
pub fn sys_shmctl(shmid: i32, cmd: i32, _buf: usize) -> isize {
    let mut manager = IPC_MANAGER.lock();

    match cmd {
        IPC_RMID => {
            if let Some(shm) = manager.get_shm(shmid) {
                let mut shm_locked = shm.lock();
                shm_locked.marked_for_delete = true;
                let should_remove = shm_locked.attach_count == 0;
                drop(shm_locked);
                if should_remove {
                    manager.remove_shm(shmid);
                }
                0
            } else {
                errno(EINVAL)
            }
        }
        IPC_STAT => {
            // Get/Set segment info
            // For simplicity, just check if segment exists
            if manager.get_shm(shmid).is_some() {
                0
            } else {
                errno(EINVAL)
            }
        }
        IPC_SET => {
            // Get/Set segment info
            // For simplicity, just check if segment exists
            if manager.get_shm(shmid).is_some() {
                0
            } else {
                errno(EINVAL)
            }
        }
        _ => errno(EINVAL),
    }
}

/// Cleanup all SHM attachments that belong to a process when it exits.
/// This prevents stale `(pid, addr)` attachment records from causing
/// false conflicts after pid reuse.
pub fn cleanup_shm_attachments_for_pid(pid: usize) {
    let mut manager = IPC_MANAGER.lock();
    let addrs: Vec<usize> = manager
        .shm_attachments
        .keys()
        .filter_map(|(owner_pid, addr)| if *owner_pid == pid { Some(*addr) } else { None })
        .collect();
    for addr in addrs {
        let Some((shmid, _size)) = manager.remove_attachment(pid, addr) else {
            continue;
        };
        let mut should_remove = false;
        if let Some(shm) = manager.get_shm(shmid) {
            let mut shm_locked = shm.lock();
            if shm_locked.attach_count > 0 {
                shm_locked.attach_count -= 1;
            }
            should_remove = shm_locked.marked_for_delete && shm_locked.attach_count == 0;
        }
        if should_remove {
            manager.remove_shm(shmid);
        }
    }
}
