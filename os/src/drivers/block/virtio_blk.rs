use super::BlockDevice;
use crate::board::VIRTIO_BLK;
use crate::DEV_NON_BLOCKING_ACCESS;
use crate::drivers::bus::virtio::VirtioHal;
use crate::sync::{Condvar, UPIntrFreeCell};
use crate::task::schedule;
use alloc::collections::BTreeMap;
use virtio_drivers::{BlkResp, Error as VirtIOError, RespStatus, VirtIOBlk, VirtIOHeader};

#[allow(unused)]
const VIRTIO0: usize = VIRTIO_BLK;

/// VirtIO block device wrapper with interrupt-driven I/O support.
pub struct VirtIOBlock {
    virtio_blk: UPIntrFreeCell<VirtIOBlk<'static, VirtioHal>>,
    condvars: BTreeMap<u16, Condvar>,
}

const VIRTIO_IO_RETRY_LIMIT: usize = 3;

impl BlockDevice for VirtIOBlock {
    fn read_block(&self, block_id: usize, buf: &mut [u8]) {
        let nb = *DEV_NON_BLOCKING_ACCESS.exclusive_access();
        if nb {
            #[cfg(feature = "legacy_qemu")]
            {
                let mut resp = BlkResp::default();
                let task_cx_ptr = self.virtio_blk.exclusive_session(|blk| {
                    let token = unsafe { blk.read_block_nb(block_id, buf, &mut resp).unwrap() };
                    self.condvars.get(&token).unwrap().wait_no_sched()
                });
                schedule(task_cx_ptr);
                assert_eq!(
                    resp.status(),
                    RespStatus::Ok,
                    "Error when reading VirtIOBlk"
                );
            }
            #[cfg(not(feature = "legacy_qemu"))]
            {
                let mut resp = BlkResp::default();
                let task_cx_ptr = self.virtio_blk.exclusive_session(|blk| {
                    match unsafe { blk.read_block_nb(block_id, buf, &mut resp) } {
                        Ok(token) => self.condvars.get(&token).map(|condvar| condvar.wait_no_sched()),
                        Err(err) => {
                            warn!(
                                "virtio-blk nb read failed, fallback to blocking: block_id={} err={:?}",
                                block_id, err
                            );
                            None
                        }
                    }
                });
                if let Some(task_cx_ptr) = task_cx_ptr {
                    schedule(task_cx_ptr);
                    if resp.status() != RespStatus::Ok {
                        warn!(
                            "virtio-blk nb read error, retrying blocking: block_id={}",
                            block_id
                        );
                        self.read_block_blocking_retry(block_id, buf);
                        return;
                    }
                    return;
                }
            }
        } else {
            self.read_block_blocking_retry(block_id, buf);
        }
    }
    fn write_block(&self, block_id: usize, buf: &[u8]) {
        let nb = *DEV_NON_BLOCKING_ACCESS.exclusive_access();
        if nb {
            #[cfg(feature = "legacy_qemu")]
            {
                let mut resp = BlkResp::default();
                let task_cx_ptr = self.virtio_blk.exclusive_session(|blk| {
                    let token = unsafe { blk.write_block_nb(block_id, buf, &mut resp).unwrap() };
                    self.condvars.get(&token).unwrap().wait_no_sched()
                });
                schedule(task_cx_ptr);
                assert_eq!(
                    resp.status(),
                    RespStatus::Ok,
                    "Error when writing VirtIOBlk"
                );
            }
            #[cfg(not(feature = "legacy_qemu"))]
            {
                let mut resp = BlkResp::default();
                let task_cx_ptr = self.virtio_blk.exclusive_session(|blk| {
                    match unsafe { blk.write_block_nb(block_id, buf, &mut resp) } {
                        Ok(token) => self.condvars.get(&token).map(|condvar| condvar.wait_no_sched()),
                        Err(err) => {
                            warn!(
                                "virtio-blk nb write failed, fallback to blocking: block_id={} err={:?}",
                                block_id, err
                            );
                            None
                        }
                    }
                });
                if let Some(task_cx_ptr) = task_cx_ptr {
                    schedule(task_cx_ptr);
                    if resp.status() != RespStatus::Ok {
                        warn!(
                            "virtio-blk nb write error, retrying blocking: block_id={}",
                            block_id
                        );
                        self.write_block_blocking_retry(block_id, buf);
                        return;
                    }
                    return;
                }
            }
        } else {
            self.write_block_blocking_retry(block_id, buf);
        }
    }
    fn handle_irq(&self) {
        self.virtio_blk.exclusive_session(|blk| {
            while let Ok(token) = blk.pop_used() {
                self.condvars.get(&token).unwrap().signal();
            }
        });
    }
}

impl VirtIOBlock {
    fn read_block_blocking_retry(&self, block_id: usize, buf: &mut [u8]) {
        let mut last_err = None;
        for attempt in 0..=VIRTIO_IO_RETRY_LIMIT {
            let mut blk = self.virtio_blk.exclusive_access();
            match blk.read_block(block_id, buf) {
                Ok(()) => return,
                Err(err) => {
                    last_err = Some(err);
                    if err == VirtIOError::IoError && attempt < VIRTIO_IO_RETRY_LIMIT {
                        warn!(
                            "virtio-blk read IoError, retrying: block_id={} attempt={}",
                            block_id,
                            attempt + 1
                        );
                        continue;
                    }
                    error!(
                        "virtio-blk read failed: block_id={} err={:?}",
                        block_id, err
                    );
                    break;
                }
            }
        }
        panic!(
            "Error when reading VirtIOBlk: block_id={} err={:?}",
            block_id, last_err
        );
    }

    fn write_block_blocking_retry(&self, block_id: usize, buf: &[u8]) {
        let mut last_err = None;
        for attempt in 0..=VIRTIO_IO_RETRY_LIMIT {
            let mut blk = self.virtio_blk.exclusive_access();
            match blk.write_block(block_id, buf) {
                Ok(()) => return,
                Err(err) => {
                    last_err = Some(err);
                    if err == VirtIOError::IoError && attempt < VIRTIO_IO_RETRY_LIMIT {
                        warn!(
                            "virtio-blk write IoError, retrying: block_id={} attempt={}",
                            block_id,
                            attempt + 1
                        );
                        continue;
                    }
                    error!(
                        "virtio-blk write failed: block_id={} err={:?}",
                        block_id, err
                    );
                    break;
                }
            }
        }
        panic!(
            "Error when writing VirtIOBlk: block_id={} err={:?}",
            block_id, last_err
        );
    }
}

impl VirtIOBlock {
    /// Create a new VirtIO block device wrapper.
    pub fn new() -> Self {
        let virtio_blk = unsafe {
            UPIntrFreeCell::new(
                VirtIOBlk::<VirtioHal>::new(&mut *(VIRTIO0 as *mut VirtIOHeader)).unwrap(),
            )
        };
        let mut condvars = BTreeMap::new();
        let channels = virtio_blk.exclusive_access().virt_queue_size();
        for i in 0..channels {
            let condvar = Condvar::new();
            condvars.insert(i, condvar);
        }
        Self {
            virtio_blk,
            condvars,
        }
    }
}
