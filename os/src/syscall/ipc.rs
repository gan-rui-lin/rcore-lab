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
use crate::task::current_user_token;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use alloc::sync::Arc;
use spin::Mutex;

/// IPC key type (used to identify IPC objects)
pub type IpcKey = i32;

/// IPC identifier type (returned by xxxget() functions)
pub type IpcId = i32;

/// Special IPC key value for private IPC objects
pub const IPC_PRIVATE: IpcKey = 0;

/// IPC flags
pub const IPC_CREAT: i32 = 0o1000;  // Create if key doesn't exist
pub const IPC_EXCL: i32 = 0o2000;   // Fail if key exists
pub const IPC_NOWAIT: i32 = 0o4000; // Return immediately if would block

/// IPC control commands
pub const IPC_RMID: i32 = 0;  // Remove identifier
pub const IPC_SET: i32 = 1;   // Set options
pub const IPC_STAT: i32 = 2;  // Get options
pub const IPC_INFO: i32 = 3;  // Get info

/// Message queue limits
const MSGMAX: usize = 8192;      // Max message size
const MSGMNB: usize = 16384;     // Max queue size in bytes
const MSGMNI: usize = 128;       // Max number of message queues

/// Shared memory limits
const SHMMAX: usize = 32 * 1024 * 1024;  // Max segment size (32MB)
const SHMMIN: usize = 1;                  // Min segment size
const SHMMNI: usize = 128;                // Max number of segments
const SHMSEG: usize = 128;                // Max segments per process

/// Message structure for message queues
#[derive(Clone)]
pub struct Message {
    pub mtype: isize,      // Message type (must be > 0)
    pub mtext: Vec<u8>,    // Message data
}

/// Message queue structure
pub struct MessageQueue {
    pub key: IpcKey,
    pub id: IpcId,
    pub messages: Vec<Message>,
    pub total_bytes: usize,
    pub max_bytes: usize,
    pub permissions: u32,
}

impl MessageQueue {
    pub fn new(key: IpcKey, id: IpcId, permissions: u32) -> Self {
        Self {
            key,
            id,
            messages: Vec::new(),
            total_bytes: 0,
            max_bytes: MSGMNB,
            permissions,
        }
    }

    pub fn send(&mut self, msg: Message) -> Result<(), isize> {
        let msg_size = msg.mtext.len();

        if msg_size > MSGMAX {
            return Err(errno(EINVAL));
        }

        if self.total_bytes + msg_size > self.max_bytes {
            return Err(errno(EAGAIN));
        }

        self.messages.push(msg);
        self.total_bytes += msg_size;
        Ok(())
    }

    pub fn receive(&mut self, msgtyp: isize, msgflg: i32) -> Result<Message, isize> {
        // Find message matching the type criterion
        let pos = if msgtyp == 0 {
            // Get first message
            if self.messages.is_empty() {
                return Err(errno(ENOMSG));
            }
            Some(0)
        } else if msgtyp > 0 {
            // Get first message with mtype == msgtyp
            self.messages.iter().position(|m| m.mtype == msgtyp)
        } else {
            // Get first message with mtype <= abs(msgtyp)
            let abs_type = -msgtyp;
            self.messages.iter().position(|m| m.mtype <= abs_type)
        };

        match pos {
            Some(idx) => {
                let msg = self.messages.remove(idx);
                self.total_bytes -= msg.mtext.len();
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
    pub data: Vec<u8>,
    pub permissions: u32,
    pub attach_count: usize,
}

impl ShmSegment {
    pub fn new(key: IpcKey, id: IpcId, size: usize, permissions: u32) -> Self {
        Self {
            key,
            id,
            size,
            data: alloc::vec![0u8; size],
            permissions,
            attach_count: 0,
        }
    }
}

/// Global IPC resource manager
pub struct IpcManager {
    message_queues: BTreeMap<IpcId, Arc<Mutex<MessageQueue>>>,
    shm_segments: BTreeMap<IpcId, Arc<Mutex<ShmSegment>>>,
    next_msgid: IpcId,
    next_shmid: IpcId,
}

impl IpcManager {
    pub const fn new() -> Self {
        Self {
            message_queues: BTreeMap::new(),
            shm_segments: BTreeMap::new(),
            next_msgid: 1,
            next_shmid: 1,
        }
    }

    pub fn create_msgq(&mut self, key: IpcKey, permissions: u32) -> IpcId {
        let id = self.next_msgid;
        self.next_msgid += 1;
        let msgq = Arc::new(Mutex::new(MessageQueue::new(key, id, permissions)));
        self.message_queues.insert(id, msgq);
        id
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

    pub fn create_shm(&mut self, key: IpcKey, size: usize, permissions: u32) -> IpcId {
        let id = self.next_shmid;
        self.next_shmid += 1;
        let shm = Arc::new(Mutex::new(ShmSegment::new(key, id, size, permissions)));
        self.shm_segments.insert(id, shm);
        id
    }

    pub fn get_shm(&self, id: IpcId) -> Option<Arc<Mutex<ShmSegment>>> {
        self.shm_segments.get(&id).cloned()
    }

    pub fn find_shm_by_key(&self, key: IpcKey) -> Option<(IpcId, Arc<Mutex<ShmSegment>>)> {
        for (id, shm) in self.shm_segments.iter() {
            if shm.lock().key == key {
                return Some((*id, shm.clone()));
            }
        }
        None
    }

    pub fn remove_shm(&mut self, id: IpcId) -> bool {
        self.shm_segments.remove(&id).is_some()
    }
}

/// Global IPC manager instance
static IPC_MANAGER: Mutex<IpcManager> = Mutex::new(IpcManager::new());

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
    let mut manager = IPC_MANAGER.lock();

    // Check if we're creating a private queue
    if key == IPC_PRIVATE {
        let permissions = (msgflg & 0o777) as u32;
        let id = manager.create_msgq(key, permissions);
        return id as isize;
    }

    // Try to find existing queue with this key
    if let Some((id, _)) = manager.find_msgq_by_key(key) {
        // Queue exists
        if msgflg & IPC_CREAT != 0 && msgflg & IPC_EXCL != 0 {
            // Exclusive creation requested but queue exists
            return errno(EEXIST);
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
    let id = manager.create_msgq(key, permissions);
    id as isize
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
    use crate::mm::translated_byte_buffer;

    if msgsz > MSGMAX {
        return errno(EINVAL);
    }

    let manager = IPC_MANAGER.lock();
    let msgq = match manager.get_msgq(msqid) {
        Some(q) => q,
        None => return errno(EINVAL),
    };

    drop(manager);

    // Read message from user space
    // Message structure: [mtype: isize][mtext: u8 array]
    let token = current_user_token();

    // Read mtype (first sizeof(isize) bytes)
    let mtype_size = core::mem::size_of::<isize>();
    let mtype_buffers = translated_byte_buffer(token, msgp as *const u8, mtype_size);
    let mut mtype_bytes = [0u8; 8];
    let mut offset = 0;
    for buf in mtype_buffers {
        let len = buf.len().min(mtype_size - offset);
        mtype_bytes[offset..offset + len].copy_from_slice(&buf[..len]);
        offset += len;
    }
    let mtype = isize::from_ne_bytes(mtype_bytes);

    if mtype <= 0 {
        return errno(EINVAL);
    }

    // Read mtext
    let mtext_buffers = translated_byte_buffer(token, (msgp + mtype_size) as *const u8, msgsz);
    let mut mtext = Vec::with_capacity(msgsz);
    for buf in mtext_buffers {
        mtext.extend_from_slice(buf);
    }

    let msg = Message { mtype, mtext };

    let mut msgq = msgq.lock();
    match msgq.send(msg) {
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
    use crate::mm::translated_byte_buffer;

    let manager = IPC_MANAGER.lock();
    let msgq = match manager.get_msgq(msqid) {
        Some(q) => q,
        None => return errno(EINVAL),
    };

    drop(manager);

    let mut msgq = msgq.lock();
    let msg = match msgq.receive(msgtyp, msgflg) {
        Ok(m) => m,
        Err(e) => return e,
    };

    let msg_size = msg.mtext.len();
    if msg_size > msgsz {
        if msgflg & 0o10000 == 0 {  // MSG_NOERROR not set
            return errno(E2BIG);
        }
        // Truncate message
    }

    let actual_size = msg_size.min(msgsz);

    // Write message to user space
    let token = current_user_token();

    // Write mtype
    let mtype_size = core::mem::size_of::<isize>();
    let mtype_bytes = msg.mtype.to_ne_bytes();
    let mtype_buffers = translated_byte_buffer(token, msgp as *const u8, mtype_size);
    let mut offset = 0;
    for buf in mtype_buffers {
        let len = buf.len().min(mtype_size - offset);
        unsafe {
            core::ptr::copy_nonoverlapping(
                mtype_bytes[offset..].as_ptr(),
                buf.as_ptr() as *mut u8,
                len,
            );
        }
        offset += len;
    }

    // Write mtext
    let mtext_buffers = translated_byte_buffer(token, (msgp + mtype_size) as *const u8, actual_size);
    offset = 0;
    for buf in mtext_buffers {
        let len = buf.len().min(actual_size - offset);
        unsafe {
            core::ptr::copy_nonoverlapping(
                msg.mtext[offset..].as_ptr(),
                buf.as_ptr() as *mut u8,
                len,
            );
        }
        offset += len;
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
    let mut manager = IPC_MANAGER.lock();

    match cmd {
        IPC_RMID => {
            // Remove message queue
            if manager.remove_msgq(msqid) {
                0
            } else {
                errno(EINVAL)
            }
        }
        IPC_STAT | IPC_SET => {
            // Get/Set queue info
            // For simplicity, just check if queue exists
            if manager.get_msgq(msqid).is_some() {
                0
            } else {
                errno(EINVAL)
            }
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
        let id = manager.create_shm(key, size, permissions);
        return id as isize;
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
    let id = manager.create_shm(key, size, permissions);
    id as isize
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
    let manager = IPC_MANAGER.lock();
    let shm = match manager.get_shm(shmid) {
        Some(s) => s,
        None => return errno(EINVAL),
    };

    let mut shm_locked = shm.lock();
    shm_locked.attach_count += 1;
    let _size = shm_locked.size;

    drop(shm_locked);
    drop(manager);

    // For simplicity, we'll use mmap to attach the shared memory
    // In a full implementation, we would map the shared memory data
    // into the process's address space

    // If shmaddr is 0, let the system choose the address
    if shmaddr == 0 {
        // Use mmap-like allocation
        // For now, return a fixed address in high memory
        let addr = 0x2000_0000 + (shmid as usize * SHMMAX);
        addr as isize
    } else {
        // User specified address (for simplicity, just accept it)
        shmaddr as isize
    }
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
    // For simplicity, we'll just return success
    // In a full implementation, we would:
    // 1. Find which segment this address belongs to
    // 2. Decrement attach_count
    // 3. Unmap the memory from the process

    if shmaddr == 0 {
        return errno(EINVAL);
    }

    // Estimate which shmid this might be
    let estimated_id = ((shmaddr - 0x2000_0000) / SHMMAX) as i32;

    let manager = IPC_MANAGER.lock();
    if let Some(shm) = manager.get_shm(estimated_id) {
        let mut shm_locked = shm.lock();
        if shm_locked.attach_count > 0 {
            shm_locked.attach_count -= 1;
        }
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
            // Remove shared memory segment
            if manager.remove_shm(shmid) {
                0
            } else {
                errno(EINVAL)
            }
        }
        IPC_STAT | IPC_SET => {
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
