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
    task::{
        block_current_and_run_next, current_process, current_task, current_user_token, wakeup_task,
        TaskControlBlock,
    },
    timer::get_time_us,
};
use alloc::collections::{BTreeMap, VecDeque};
use alloc::string::String;
use alloc::sync::{Arc, Weak};
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
pub const SHM_LOCK: i32 = 11;
pub const SHM_UNLOCK: i32 = 12;
pub const SHM_STAT: i32 = 13;
pub const SHM_INFO: i32 = 14;
pub const SHM_STAT_ANY: i32 = 15;
pub const SHM_DEST: u32 = 0o1000;
pub const SHM_LOCKED: u32 = 0o2000;
pub const GETPID: i32 = 11;
pub const GETVAL: i32 = 12;
pub const GETALL: i32 = 13;
pub const GETNCNT: i32 = 14;
pub const GETZCNT: i32 = 15;
pub const SETVAL: i32 = 16;
pub const SETALL: i32 = 17;
pub const SEM_STAT: i32 = 18;
pub const SEM_INFO: i32 = 19;
pub const SEM_STAT_ANY: i32 = 20;
pub const SEM_UNDO: i16 = 0x1000;

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
const SHMALL: usize = (SHMMAX / PAGE_SIZE) * SHMMNI;

const SEMMNI: usize = 128;
const SEMMSL: usize = 32000;
const SEMOPM: usize = 500;
const SEMVMX: i32 = 32767;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SembufUser {
    sem_num: u16,
    sem_op: i16,
    sem_flg: i16,
}

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

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ShmidDsUser {
    shm_perm: IpcPermUser,
    shm_segsz: usize,
    shm_atime: i64,
    shm_dtime: i64,
    shm_ctime: i64,
    shm_cpid: i32,
    shm_lpid: i32,
    shm_nattch: u64,
    unused4: u64,
    unused5: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ShminfoUser {
    shmmax: u64,
    shmmin: u64,
    shmmni: u64,
    shmseg: u64,
    shmall: u64,
    reserved1: u64,
    reserved2: u64,
    reserved3: u64,
    reserved4: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ShmInfoUser {
    used_ids: i32,
    shm_tot: u64,
    shm_rss: u64,
    shm_swp: u64,
    swap_attempts: u64,
    swap_successes: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SemidDsUser {
    sem_perm: IpcPermUser,
    sem_otime: i64,
    sem_ctime: i64,
    sem_nsems: usize,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SemInfoUser {
    semmap: i32,
    semmni: i32,
    semmns: i32,
    semmnu: i32,
    semmsl: i32,
    semopm: i32,
    semume: i32,
    semusz: i32,
    semvmx: i32,
    semaem: i32,
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

fn shm_to_user_ds(shm: &ShmSegment) -> ShmidDsUser {
    ShmidDsUser {
        shm_perm: make_shm_perm_user(shm),
        shm_segsz: shm.size,
        shm_atime: shm.atime,
        shm_dtime: shm.dtime,
        shm_ctime: shm.ctime,
        shm_cpid: shm.cpid,
        shm_lpid: shm.lpid,
        shm_nattch: shm.attach_count as u64,
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

#[cfg(target_arch = "loongarch64")]
fn make_shm_perm_user(shm: &ShmSegment) -> IpcPermUser {
    IpcPermUser {
        key: shm.key,
        uid: shm.uid,
        gid: shm.gid,
        cuid: shm.cuid,
        cgid: shm.cgid,
        mode: shm.mode_bits(),
        seq: 0,
        pad2: 0,
        pad3: 0,
        unused1: 0,
        unused2: 0,
    }
}

#[cfg(target_arch = "loongarch64")]
fn make_sem_perm_user(sem: &SemaphoreSet) -> IpcPermUser {
    IpcPermUser {
        key: sem.key,
        uid: sem.uid,
        gid: sem.gid,
        cuid: sem.cuid,
        cgid: sem.cgid,
        mode: (sem.permissions & 0o777) as u32,
        seq: 0,
        pad2: 0,
        pad3: 0,
        unused1: 0,
        unused2: 0,
    }
}

#[cfg(not(target_arch = "loongarch64"))]
fn make_shm_perm_user(shm: &ShmSegment) -> IpcPermUser {
    IpcPermUser {
        key: shm.key,
        uid: shm.uid,
        gid: shm.gid,
        cuid: shm.cuid,
        cgid: shm.cgid,
        mode: shm.mode_bits() as u16,
        pad1: 0,
        seq: 0,
        pad2: 0,
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

#[cfg(not(target_arch = "loongarch64"))]
fn make_sem_perm_user(sem: &SemaphoreSet) -> IpcPermUser {
    IpcPermUser {
        key: sem.key,
        uid: sem.uid,
        gid: sem.gid,
        cuid: sem.cuid,
        cgid: sem.cgid,
        mode: (sem.permissions & 0o777) as u16,
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
    pub mapped_size: usize,
    pub frames: Vec<Arc<FrameTracker>>,
    pub permissions: u32,
    pub uid: u32,
    pub gid: u32,
    pub cuid: u32,
    pub cgid: u32,
    pub ctime: i64,
    pub atime: i64,
    pub dtime: i64,
    pub cpid: i32,
    pub lpid: i32,
    pub locked: bool,
    pub attach_count: usize,
    pub marked_for_delete: bool,
}

impl ShmSegment {
    pub fn new(
        key: IpcKey,
        id: IpcId,
        size: usize,
        permissions: u32,
        uid: u32,
        gid: u32,
        pid: usize,
        now_sec: i64,
    ) -> Result<Self, isize> {
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
            size,
            mapped_size: aligned_size,
            frames,
            permissions,
            uid,
            gid,
            cuid: uid,
            cgid: gid,
            ctime: now_sec,
            atime: 0,
            dtime: 0,
            cpid: pid as i32,
            lpid: 0,
            locked: false,
            attach_count: 0,
            marked_for_delete: false,
        })
    }

    pub fn mode_bits(&self) -> u32 {
        let mut mode = self.permissions & 0o777;
        if self.marked_for_delete {
            mode |= SHM_DEST;
        }
        if self.locked {
            mode |= SHM_LOCKED;
        }
        mode
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
}

struct SemWaiter {
    task: Weak<TaskControlBlock>,
    sem_num: usize,
    wait_zero: bool,
}

/// System V semaphore set.
pub struct SemaphoreSet {
    pub key: IpcKey,
    pub id: IpcId,
    pub permissions: u32,
    pub uid: u32,
    pub gid: u32,
    pub cuid: u32,
    pub cgid: u32,
    pub ctime: i64,
    pub otime: i64,
    pub values: Vec<i32>,
    pub last_pid: Vec<i32>,
    waiters: VecDeque<SemWaiter>,
    removed: bool,
}

impl SemaphoreSet {
    pub fn new(
        key: IpcKey,
        id: IpcId,
        nsems: usize,
        permissions: u32,
        uid: u32,
        gid: u32,
        now_sec: i64,
    ) -> Self {
        Self {
            key,
            id,
            permissions,
            uid,
            gid,
            cuid: uid,
            cgid: gid,
            ctime: now_sec,
            otime: 0,
            values: alloc::vec![0; nsems],
            last_pid: alloc::vec![0; nsems],
            waiters: VecDeque::new(),
            removed: false,
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

    fn to_user_ds(&self) -> SemidDsUser {
        SemidDsUser {
            sem_perm: make_sem_perm_user(self),
            sem_otime: self.otime,
            sem_ctime: self.ctime,
            sem_nsems: self.values.len(),
        }
    }

    fn waiter_count(&self, sem_num: usize, wait_zero: bool) -> isize {
        self.waiters
            .iter()
            .filter(|w| w.sem_num == sem_num && w.wait_zero == wait_zero)
            .count() as isize
    }

    fn wake_waiters(&mut self) {
        let mut to_wake = Vec::new();
        while let Some(waiter) = self.waiters.pop_front() {
            if let Some(task) = waiter.task.upgrade() {
                to_wake.push(task);
            }
        }
        for task in to_wake {
            wakeup_task(task);
        }
    }
}

/// Global IPC resource manager
pub struct IpcManager {
    message_queues: BTreeMap<IpcId, Arc<Mutex<MessageQueue>>>,
    shm_segments: BTreeMap<IpcId, Arc<Mutex<ShmSegment>>>,
    shm_attachments: BTreeMap<(usize, usize), (IpcId, usize)>,
    sem_sets: BTreeMap<IpcId, Arc<Mutex<SemaphoreSet>>>,
    next_msgid: IpcId,
    next_shmid: IpcId,
    next_semid: IpcId,
}

impl IpcManager {
    pub const fn new() -> Self {
        Self {
            message_queues: BTreeMap::new(),
            shm_segments: BTreeMap::new(),
            shm_attachments: BTreeMap::new(),
            sem_sets: BTreeMap::new(),
            next_msgid: 1,
            next_shmid: 1,
            next_semid: 1,
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
        uid: u32,
        gid: u32,
        pid: usize,
    ) -> Result<IpcId, isize> {
        if self.shm_segments.len() >= SHMMNI {
            return Err(errno(ENOMEM));
        }
        let id = self.next_shmid;
        self.next_shmid += 1;
        let shm = Arc::new(Mutex::new(ShmSegment::new(
            key,
            id,
            size,
            permissions,
            uid,
            gid,
            pid,
            now_epoch_sec(),
        )?));
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

    pub fn inherit_attachments_for_fork(&mut self, parent_pid: usize, child_pid: usize) {
        let inherited: Vec<(usize, IpcId, usize)> = self
            .shm_attachments
            .iter()
            .filter_map(|((pid, addr), (shmid, size))| {
                if *pid == parent_pid {
                    Some((*addr, *shmid, *size))
                } else {
                    None
                }
            })
            .collect();
        for (addr, shmid, size) in inherited {
            self.shm_attachments
                .insert((child_pid, addr), (shmid, size));
            if let Some(shm) = self.get_shm(shmid) {
                shm.lock().attach_count += 1;
            }
        }
    }

    pub fn create_sem(
        &mut self,
        key: IpcKey,
        nsems: usize,
        permissions: u32,
        uid: u32,
        gid: u32,
    ) -> Result<IpcId, isize> {
        if self.sem_sets.len() >= SEMMNI {
            return Err(errno(ENOSPC));
        }
        let id = self.next_semid;
        self.next_semid += 1;
        let sem = Arc::new(Mutex::new(SemaphoreSet::new(
            key,
            id,
            nsems,
            permissions,
            uid,
            gid,
            now_epoch_sec(),
        )));
        self.sem_sets.insert(id, sem);
        Ok(id)
    }

    pub fn get_sem(&self, id: IpcId) -> Option<Arc<Mutex<SemaphoreSet>>> {
        self.sem_sets.get(&id).cloned()
    }

    pub fn find_sem_by_key(&self, key: IpcKey) -> Option<(IpcId, Arc<Mutex<SemaphoreSet>>)> {
        for (id, sem) in self.sem_sets.iter() {
            if sem.lock().key == key {
                return Some((*id, sem.clone()));
            }
        }
        None
    }

    pub fn remove_sem(&mut self, id: IpcId) -> bool {
        if let Some(sem) = self.sem_sets.remove(&id) {
            let mut sem = sem.lock();
            sem.removed = true;
            sem.wake_waiters();
            true
        } else {
            false
        }
    }

    pub fn sem_by_index(&self, index: usize) -> Option<(IpcId, Arc<Mutex<SemaphoreSet>>)> {
        self.sem_sets
            .iter()
            .nth(index)
            .map(|(id, sem)| (*id, sem.clone()))
    }

    pub fn highest_sem_index(&self) -> isize {
        if self.sem_sets.is_empty() {
            0
        } else {
            self.sem_sets.len() as isize - 1
        }
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

pub fn proc_kernel_shmmax() -> String {
    alloc::format!("{}\n", SHMMAX)
}

pub fn proc_kernel_shmmni() -> String {
    alloc::format!("{}\n", SHMMNI)
}

pub fn proc_kernel_shmall() -> String {
    alloc::format!("{}\n", SHMALL)
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

/// Render `/proc/sysvipc/shm` in a Linux-compatible tabular layout.
pub fn proc_sysvipc_shm() -> String {
    let segments: Vec<Arc<Mutex<ShmSegment>>> = {
        let manager = IPC_MANAGER.lock();
        manager.shm_segments.values().cloned().collect()
    };

    let mut out = String::from(
        "       key      shmid perms                  size  cpid  lpid nattch   uid   gid  cuid  cgid      atime      dtime      ctime        rss       swap\n",
    );
    for shm in segments {
        let s = shm.lock();
        let _ = core::fmt::Write::write_fmt(
            &mut out,
            format_args!(
                "{:>10} {:>10} {:>5o} {:>21} {:>5} {:>5} {:>6} {:>5} {:>5} {:>5} {:>5} {:>10} {:>10} {:>10} {:>10} {:>10}\n",
                s.key,
                s.id,
                s.mode_bits(),
                s.size,
                s.cpid,
                s.lpid,
                s.attach_count,
                s.uid,
                s.gid,
                s.cuid,
                s.cgid,
                s.atime,
                s.dtime,
                s.ctime,
                s.mapped_size,
                0
            ),
        );
    }
    out
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

/// Render `/proc/sysvipc/sem` in a Linux-compatible tabular layout.
pub fn proc_sysvipc_sem() -> String {
    let sets: Vec<Arc<Mutex<SemaphoreSet>>> = {
        let manager = IPC_MANAGER.lock();
        manager.sem_sets.values().cloned().collect()
    };

    let mut out = String::from(
        "       key      semid perms      nsems   uid   gid  cuid  cgid      otime      ctime\n",
    );
    for sem in sets {
        let s = sem.lock();
        let _ = core::fmt::Write::write_fmt(
            &mut out,
            format_args!(
                "{:>10x} {:>10} {:>5o} {:>10} {:>5} {:>5} {:>5} {:>5} {:>10} {:>10}\n",
                s.key as u32,
                s.id,
                s.permissions & 0o777,
                s.values.len(),
                s.uid,
                s.gid,
                s.cuid,
                s.cgid,
                s.otime,
                s.ctime
            ),
        );
    }
    out
}

fn sem_info_user(sets: usize, total_sems: usize) -> SemInfoUser {
    SemInfoUser {
        semmap: SEMMNI as i32,
        semmni: SEMMNI as i32,
        semmns: total_sems.max(SEMMNI * SEMMSL) as i32,
        semmnu: SEMMNI as i32,
        semmsl: SEMMSL as i32,
        semopm: SEMOPM as i32,
        semume: SEMOPM as i32,
        semusz: sets as i32,
        semvmx: SEMVMX,
        semaem: SEMVMX,
    }
}

fn copy_struct_to_user<T>(dst: usize, value: &T) -> isize {
    if dst == 0 {
        return errno(EFAULT);
    }
    let bytes = unsafe {
        core::slice::from_raw_parts((value as *const T).cast::<u8>(), core::mem::size_of::<T>())
    };
    if user_mem::copy_to_user(
        current_user_token(),
        dst as *mut u8,
        bytes,
        UserWritePolicy::DemandCowWithForkFallback,
    )
    .is_err()
    {
        errno(EFAULT)
    } else {
        0
    }
}

fn copy_sem_values_to_user(dst: usize, values: &[i32]) -> isize {
    if dst == 0 {
        return errno(EFAULT);
    }
    let mut out = Vec::with_capacity(values.len() * 2);
    for value in values {
        let v = (*value as u16).to_ne_bytes();
        out.extend_from_slice(&v);
    }
    if user_mem::copy_to_user(
        current_user_token(),
        dst as *mut u8,
        out.as_slice(),
        UserWritePolicy::DemandCowWithForkFallback,
    )
    .is_err()
    {
        errno(EFAULT)
    } else {
        0
    }
}

fn read_sem_values_from_user(src: usize, nsems: usize) -> Result<Vec<i32>, isize> {
    if src == 0 {
        return Err(errno(EFAULT));
    }
    let mut raw = alloc::vec![0u8; nsems * 2];
    if user_mem::copy_from_user(
        current_user_token(),
        src as *const u8,
        raw.as_mut_slice(),
        UserReadPolicy::StrictChecked,
    )
    .is_err()
    {
        return Err(errno(EFAULT));
    }
    let mut values = Vec::with_capacity(nsems);
    for chunk in raw.chunks_exact(2) {
        let value = u16::from_ne_bytes([chunk[0], chunk[1]]) as i32;
        if value > SEMVMX {
            return Err(errno(ERANGE));
        }
        values.push(value);
    }
    Ok(values)
}

fn read_sem_ops(sops: usize, nsops: usize) -> Result<Vec<SembufUser>, isize> {
    if sops == 0 {
        return Err(errno(EFAULT));
    }
    let mut ops = Vec::with_capacity(nsops);
    let token = current_user_token();
    for i in 0..nsops {
        let ptr = (sops + i * core::mem::size_of::<SembufUser>()) as *const SembufUser;
        let op = user_mem::read_from_user::<SembufUser>(token, ptr, UserReadPolicy::StrictChecked)
            .map_err(|_| errno(EFAULT))?;
        ops.push(op);
    }
    Ok(ops)
}

fn sem_ops_can_apply(sem: &SemaphoreSet, ops: &[SembufUser]) -> Result<bool, isize> {
    let mut temp = sem.values.clone();
    for op in ops {
        let idx = op.sem_num as usize;
        if idx >= temp.len() {
            return Err(errno(EFBIG));
        }
        let cur = temp[idx];
        if op.sem_op > 0 {
            let next = cur + op.sem_op as i32;
            if next > SEMVMX {
                return Err(errno(ERANGE));
            }
            temp[idx] = next;
        } else if op.sem_op < 0 {
            let need = -(op.sem_op as i32);
            if cur < need {
                return Ok(false);
            }
            temp[idx] = cur - need;
        } else if cur != 0 {
            return Ok(false);
        }
    }
    Ok(true)
}

fn sem_ops_apply(sem: &mut SemaphoreSet, ops: &[SembufUser], pid: usize) {
    for op in ops {
        let idx = op.sem_num as usize;
        sem.values[idx] += op.sem_op as i32;
        sem.last_pid[idx] = pid as i32;
    }
    sem.otime = now_epoch_sec();
    sem.wake_waiters();
}

// ============================================================================
// Semaphore System Calls
// ============================================================================

pub fn sys_semget(key: IpcKey, nsems: usize, semflg: i32) -> isize {
    if nsems > SEMMSL {
        return errno(EINVAL);
    }
    let (euid, egid, _pid, is_super) = current_ipc_identity();
    let mut manager = IPC_MANAGER.lock();

    if key == IPC_PRIVATE {
        if nsems == 0 {
            return errno(EINVAL);
        }
        return match manager.create_sem(key, nsems, (semflg & 0o777) as u32, euid, egid) {
            Ok(id) => id as isize,
            Err(e) => e,
        };
    }

    if let Some((id, sem)) = manager.find_sem_by_key(key) {
        if semflg & IPC_CREAT != 0 && semflg & IPC_EXCL != 0 {
            return errno(EEXIST);
        }
        let sem = sem.lock();
        if nsems > 0 && nsems > sem.values.len() {
            return errno(EINVAL);
        }
        let req = requested_rw_bits(semflg);
        if !is_super && req != 0 && !sem.has_access(euid, egid, req) {
            return errno(EACCES);
        }
        return id as isize;
    }

    if semflg & IPC_CREAT == 0 {
        return errno(ENOENT);
    }
    if nsems == 0 {
        return errno(EINVAL);
    }
    match manager.create_sem(key, nsems, (semflg & 0o777) as u32, euid, egid) {
        Ok(id) => id as isize,
        Err(e) => e,
    }
}

pub fn sys_semctl(semid: i32, semnum: usize, cmd: i32, arg: usize) -> isize {
    let (euid, egid, _pid, is_super) = current_ipc_identity();
    let cmd = cmd & 0xff;
    let mut manager = IPC_MANAGER.lock();

    if cmd == IPC_INFO || cmd == SEM_INFO {
        let total_sems = manager
            .sem_sets
            .values()
            .map(|s| s.lock().values.len())
            .sum::<usize>();
        let info = sem_info_user(manager.sem_sets.len(), total_sems);
        let ret = manager.highest_sem_index();
        drop(manager);
        let copied = copy_struct_to_user(arg, &info);
        return if copied == 0 { ret } else { copied };
    }

    let (real_semid, sem) = if cmd == SEM_STAT || cmd == SEM_STAT_ANY {
        let Some((id, sem)) = manager.sem_by_index(semid as usize) else {
            return errno(EINVAL);
        };
        (id, sem)
    } else {
        let Some(sem) = manager.get_sem(semid) else {
            return errno(EINVAL);
        };
        (semid, sem)
    };

    match cmd {
        IPC_RMID => {
            {
                let sem = sem.lock();
                if !is_super && !sem.can_ctl(euid) {
                    return errno(EPERM);
                }
            }
            if manager.remove_sem(real_semid) {
                0
            } else {
                errno(EINVAL)
            }
        }
        IPC_STAT | SEM_STAT | SEM_STAT_ANY => {
            drop(manager);
            let ds = {
                let sem = sem.lock();
                if cmd != SEM_STAT_ANY && !is_super && !sem.has_access(euid, egid, 0o4) {
                    return errno(EACCES);
                }
                sem.to_user_ds()
            };
            let copied = copy_struct_to_user(arg, &ds);
            if copied == 0 && (cmd == SEM_STAT || cmd == SEM_STAT_ANY) {
                real_semid as isize
            } else {
                copied
            }
        }
        IPC_SET => {
            drop(manager);
            if arg == 0 {
                return errno(EFAULT);
            }
            let token = current_user_token();
            let user_ds = user_mem::read_from_user::<SemidDsUser>(
                token,
                arg as *const SemidDsUser,
                UserReadPolicy::StrictChecked,
            )
            .map_err(|_| errno(EFAULT));
            let Ok(user_ds) = user_ds else {
                return errno(EFAULT);
            };
            let mut sem = sem.lock();
            if !is_super && !sem.can_ctl(euid) {
                return errno(EPERM);
            }
            #[cfg(target_arch = "loongarch64")]
            {
                sem.permissions = user_ds.sem_perm.mode & 0o777;
            }
            #[cfg(not(target_arch = "loongarch64"))]
            {
                sem.permissions = (user_ds.sem_perm.mode as u32) & 0o777;
            }
            sem.uid = user_ds.sem_perm.uid;
            sem.gid = user_ds.sem_perm.gid;
            sem.ctime = now_epoch_sec();
            0
        }
        GETVAL => {
            drop(manager);
            let sem = sem.lock();
            if semnum >= sem.values.len() {
                return errno(EINVAL);
            }
            sem.values[semnum] as isize
        }
        GETPID => {
            drop(manager);
            let sem = sem.lock();
            if semnum >= sem.values.len() {
                return errno(EINVAL);
            }
            sem.last_pid[semnum] as isize
        }
        GETNCNT | GETZCNT => {
            drop(manager);
            let sem = sem.lock();
            if semnum >= sem.values.len() {
                return errno(EINVAL);
            }
            sem.waiter_count(semnum, cmd == GETZCNT)
        }
        GETALL => {
            drop(manager);
            let sem = sem.lock();
            copy_sem_values_to_user(arg, sem.values.as_slice())
        }
        SETVAL => {
            drop(manager);
            let value = arg as i32;
            if value < 0 || value > SEMVMX {
                return errno(ERANGE);
            }
            let mut sem = sem.lock();
            if !is_super && !sem.has_access(euid, egid, 0o2) {
                return errno(EACCES);
            }
            if semnum >= sem.values.len() {
                return errno(EINVAL);
            }
            sem.values[semnum] = value;
            sem.last_pid[semnum] = current_process().pid.0 as i32;
            sem.ctime = now_epoch_sec();
            sem.wake_waiters();
            0
        }
        SETALL => {
            drop(manager);
            let values = match read_sem_values_from_user(arg, sem.lock().values.len()) {
                Ok(v) => v,
                Err(e) => return e,
            };
            let mut sem = sem.lock();
            if !is_super && !sem.has_access(euid, egid, 0o2) {
                return errno(EACCES);
            }
            sem.values.copy_from_slice(values.as_slice());
            let pid = current_process().pid.0 as i32;
            for last in sem.last_pid.iter_mut() {
                *last = pid;
            }
            sem.ctime = now_epoch_sec();
            sem.wake_waiters();
            0
        }
        _ => errno(EINVAL),
    }
}

pub fn sys_semtimedop(semid: i32, sops: usize, nsops: usize, timeout: usize) -> isize {
    let mut short_timeout = false;
    if timeout != 0 {
        let mut raw = [0u8; core::mem::size_of::<usize>() * 2];
        if user_mem::copy_from_user(
            current_user_token(),
            timeout as *const u8,
            &mut raw,
            UserReadPolicy::StrictChecked,
        )
        .is_err()
        {
            return errno(EFAULT);
        }
        let word = core::mem::size_of::<usize>();
        let mut sec_bytes = [0u8; core::mem::size_of::<usize>()];
        let mut nsec_bytes = [0u8; core::mem::size_of::<usize>()];
        sec_bytes.copy_from_slice(&raw[..word]);
        nsec_bytes.copy_from_slice(&raw[word..]);
        let sec = usize::from_ne_bytes(sec_bytes);
        let nsec = usize::from_ne_bytes(nsec_bytes);
        if nsec >= 1_000_000_000 {
            return errno(EINVAL);
        }
        short_timeout = sec == 0 && nsec <= 1_000_000;
    }
    sys_semop_inner(semid, sops, nsops, short_timeout)
}

pub fn sys_semop(semid: i32, sops: usize, nsops: usize) -> isize {
    sys_semop_inner(semid, sops, nsops, false)
}

fn sys_semop_inner(semid: i32, sops: usize, nsops: usize, short_timeout: bool) -> isize {
    if nsops == 0 {
        return errno(EINVAL);
    }
    if nsops > SEMOPM {
        return errno(E2BIG);
    }
    let ops = match read_sem_ops(sops, nsops) {
        Ok(ops) => ops,
        Err(e) => return e,
    };
    let (euid, egid, pid, is_super) = current_ipc_identity();
    let sem = {
        let manager = IPC_MANAGER.lock();
        let Some(sem) = manager.get_sem(semid) else {
            return errno(EINVAL);
        };
        sem
    };

    loop {
        {
            let mut sem = sem.lock();
            if sem.removed {
                return errno(EIDRM);
            }
            let needs_write = ops.iter().any(|op| op.sem_op != 0);
            let req = if needs_write { 0o2 } else { 0o4 };
            if !is_super && !sem.has_access(euid, egid, req) {
                return errno(EACCES);
            }
            match sem_ops_can_apply(&sem, ops.as_slice()) {
                Ok(true) => {
                    sem_ops_apply(&mut sem, ops.as_slice(), pid);
                    return 0;
                }
                Ok(false) => {
                    let blocking = ops
                        .iter()
                        .find(|op| {
                            let cur = sem.values[op.sem_num as usize];
                            (op.sem_op == 0 && cur != 0)
                                || (op.sem_op < 0 && cur < -(op.sem_op as i32))
                        })
                        .copied()
                        .unwrap_or_default();
                    if (blocking.sem_flg as i32 & IPC_NOWAIT) != 0 {
                        return errno(EAGAIN);
                    }
                    if short_timeout {
                        return errno(EAGAIN);
                    }
                    if let Some(task) = current_task() {
                        sem.waiters.push_back(SemWaiter {
                            task: Arc::downgrade(&task),
                            sem_num: blocking.sem_num as usize,
                            wait_zero: blocking.sem_op == 0,
                        });
                    }
                }
                Err(e) => return e,
            }
        }

        block_current_and_run_next();
        if let Some(task) = current_task() {
            if task.take_interrupted() {
                let mut sem = sem.lock();
                if let Some(pos) = sem.waiters.iter().position(|w| {
                    w.task
                        .upgrade()
                        .map(|t| Arc::ptr_eq(&t, &task))
                        .unwrap_or(false)
                }) {
                    sem.waiters.remove(pos);
                }
                return errno(EINTR);
            }
        }
        if IPC_MANAGER.lock().get_sem(semid).is_none() {
            return errno(EIDRM);
        }
    }
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

    let (euid, egid, pid, is_super) = current_ipc_identity();
    let mut manager = IPC_MANAGER.lock();

    // Check if we're creating a private segment
    if key == IPC_PRIVATE {
        let permissions = (shmflg & 0o777) as u32;
        return match manager.create_shm(key, size, permissions, euid, egid, pid) {
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
        {
            let shm_locked = shm.lock();
            if size > shm_locked.size {
                return errno(EINVAL);
            }
            let req = requested_rw_bits(shmflg);
            if !is_super && req != 0 && !shm_locked.has_access(euid, egid, req) {
                return errno(EACCES);
            }
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
    match manager.create_shm(key, size, permissions, euid, egid, pid) {
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
    let (euid, egid, _, is_super) = current_ipc_identity();
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
        let req = if _shmflg & SHM_RDONLY != 0 { 0o4 } else { 0o6 };
        if !is_super && !shm_locked.has_access(euid, egid, req) {
            return errno(EACCES);
        }
        (shm_locked.mapped_size, shm_locked.frames.clone())
    };

    let mut map_perm = MapPermission::U | MapPermission::R;
    if _shmflg & SHM_RDONLY == 0 {
        map_perm |= MapPermission::W;
    }
    if _shmflg & SHM_EXEC != 0 {
        map_perm |= MapPermission::X;
    }

    let mmap_base_before = process.mmap_base();
    let attach_addr = if shmaddr == 0 {
        match process.alloc_mmap_base(size) {
            Ok(base) => base,
            Err(err) => {
                error!(
                    "[shm] shmat mmap_base overflow pid={} base={:#x} size={:#x}",
                    pid,
                    align_up(mmap_base_before, PAGE_SIZE),
                    size
                );
                return err;
            }
        }
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

    let overlap = process.with_memory_set(|memory_set| {
        memory_set.overlap_count(VirtAddr(attach_addr), VirtAddr(attach_end))
    });
    if overlap > 0 {
        error!(
            "[shm] shmat overlap pid={} shmid={} addr=[{:#x},{:#x}) overlap={} mmap_base_before={:#x} mmap_base_after={:#x}",
            pid,
            shmid,
            attach_addr,
            attach_end,
            overlap,
            mmap_base_before,
            process.mmap_base()
        );
        return errno(EINVAL);
    }
    if !process.with_memory_set_mut(|memory_set| {
        memory_set.insert_shm_area(
            VirtAddr(attach_addr),
            VirtAddr(attach_end),
            map_perm,
            frames,
        )
    }) {
        error!(
            "[shm] shmat map failed pid={} shmid={} addr=[{:#x},{:#x})",
            pid, shmid, attach_addr, attach_end
        );
        return errno(EINVAL);
    }

    if manager
        .add_attachment(pid, attach_addr, shmid, size)
        .is_err()
    {
        process.with_memory_set_mut(|memory_set| {
            memory_set.remove_area_with_start_vpn(VirtAddr(attach_addr).floor());
        });
        error!(
            "[shm] shmat attachment table conflict pid={} shmid={} addr={:#x}",
            pid, shmid, attach_addr
        );
        return errno(EINVAL);
    }
    let mut shm_locked = shm.lock();
    shm_locked.attach_count += 1;
    shm_locked.atime = now_epoch_sec();
    shm_locked.lpid = pid as i32;
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
        process.with_memory_set_mut(|memory_set| {
            memory_set.remove_area_with_start_vpn(VirtAddr(shmaddr).floor());
        });
    }

    let mut should_remove = false;
    if let Some(shm) = manager.get_shm(shmid) {
        let mut shm_locked = shm.lock();
        if shm_locked.attach_count > 0 {
            shm_locked.attach_count -= 1;
        }
        shm_locked.dtime = now_epoch_sec();
        shm_locked.lpid = pid as i32;
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
    fn copy_to_user_struct<T>(ptr: usize, value: &T) -> Result<(), isize> {
        let bytes = unsafe {
            core::slice::from_raw_parts((value as *const T).cast::<u8>(), core::mem::size_of::<T>())
        };
        user_mem::copy_to_user(
            current_user_token(),
            ptr as *mut u8,
            bytes,
            UserWritePolicy::DemandCowWithForkFallback,
        )
    }

    let (euid, egid, _pid, is_super) = current_ipc_identity();
    let cmd = cmd & 0xff;
    let mut manager = IPC_MANAGER.lock();

    match cmd {
        IPC_RMID => {
            let Some(shm) = manager.get_shm(shmid) else {
                return errno(EINVAL);
            };
            let mut shm_locked = shm.lock();
            if !is_super && !shm_locked.can_ctl(euid) {
                return errno(EPERM);
            }
            shm_locked.marked_for_delete = true;
            shm_locked.ctime = now_epoch_sec();
            let should_remove = shm_locked.attach_count == 0;
            drop(shm_locked);
            if should_remove {
                manager.remove_shm(shmid);
            }
            0
        }
        IPC_STAT => {
            if _buf == 0 {
                return errno(EFAULT);
            }
            let Some(shm) = manager.get_shm(shmid) else {
                return errno(EINVAL);
            };
            drop(manager);
            let ds = {
                let shm_locked = shm.lock();
                if !is_super && !shm_locked.has_access(euid, egid, 0o4) {
                    return errno(EACCES);
                }
                shm_to_user_ds(&shm_locked)
            };
            match copy_to_user_struct(_buf, &ds) {
                Ok(()) => 0,
                Err(_) => errno(EFAULT),
            }
        }
        IPC_SET => {
            if _buf == 0 {
                return errno(EFAULT);
            }
            let Some(shm) = manager.get_shm(shmid) else {
                return errno(EINVAL);
            };
            drop(manager);
            let token = current_user_token();
            let user_ds = match user_mem::read_from_user::<ShmidDsUser>(
                token,
                _buf as *const ShmidDsUser,
                UserReadPolicy::DemandPaged,
            ) {
                Ok(v) => v,
                Err(_) => return errno(EFAULT),
            };

            let mut shm_locked = shm.lock();
            if !is_super && !shm_locked.can_ctl(euid) {
                return errno(EPERM);
            }
            shm_locked.permissions = (user_ds.shm_perm.mode as u32) & 0o777;
            shm_locked.uid = user_ds.shm_perm.uid;
            shm_locked.gid = user_ds.shm_perm.gid;
            shm_locked.ctime = now_epoch_sec();
            0
        }
        IPC_INFO => {
            if _buf == 0 {
                return errno(EFAULT);
            }
            let info = ShminfoUser {
                shmmax: SHMMAX as u64,
                shmmin: SHMMIN as u64,
                shmmni: SHMMNI as u64,
                shmseg: SHMSEG as u64,
                shmall: SHMALL as u64,
                reserved1: 0,
                reserved2: 0,
                reserved3: 0,
                reserved4: 0,
            };
            let max_id = manager
                .shm_segments
                .keys()
                .next_back()
                .copied()
                .unwrap_or(0);
            drop(manager);
            match copy_to_user_struct(_buf, &info) {
                Ok(()) => max_id as isize,
                Err(_) => errno(EFAULT),
            }
        }
        SHM_INFO => {
            if _buf == 0 {
                return errno(EFAULT);
            }
            let mut info = ShmInfoUser::default();
            for shm in manager.shm_segments.values() {
                let shm_locked = shm.lock();
                info.used_ids += 1;
                info.shm_tot += shm_locked.frames.len() as u64;
                info.shm_rss += shm_locked.frames.len() as u64;
            }
            let max_id = manager
                .shm_segments
                .keys()
                .next_back()
                .copied()
                .unwrap_or(0);
            drop(manager);
            match copy_to_user_struct(_buf, &info) {
                Ok(()) => max_id as isize,
                Err(_) => errno(EFAULT),
            }
        }
        SHM_STAT | SHM_STAT_ANY => {
            if _buf == 0 {
                return errno(EFAULT);
            }
            let Some(shm) = manager.get_shm(shmid) else {
                return errno(EINVAL);
            };
            drop(manager);
            let ds = {
                let shm_locked = shm.lock();
                if cmd == SHM_STAT && !is_super && !shm_locked.has_access(euid, egid, 0o4) {
                    return errno(EACCES);
                }
                shm_to_user_ds(&shm_locked)
            };
            match copy_to_user_struct(_buf, &ds) {
                Ok(()) => shmid as isize,
                Err(_) => errno(EFAULT),
            }
        }
        SHM_LOCK | SHM_UNLOCK => {
            let Some(shm) = manager.get_shm(shmid) else {
                return errno(EINVAL);
            };
            let mut shm_locked = shm.lock();
            if !is_super && !shm_locked.can_ctl(euid) {
                return errno(EPERM);
            }
            shm_locked.locked = cmd == SHM_LOCK;
            shm_locked.ctime = now_epoch_sec();
            0
        }
        _ => errno(EINVAL),
    }
}

pub fn inherit_shm_attachments_for_fork(parent_pid: usize, child_pid: usize) {
    IPC_MANAGER
        .lock()
        .inherit_attachments_for_fork(parent_pid, child_pid);
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
            shm_locked.dtime = now_epoch_sec();
            shm_locked.lpid = pid as i32;
            should_remove = shm_locked.marked_for_delete && shm_locked.attach_count == 0;
        }
        if should_remove {
            manager.remove_shm(shmid);
        }
    }
}
