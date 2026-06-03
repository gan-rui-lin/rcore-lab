//! signalfd: file descriptor for accepting signals.
use super::{File, PollEvents};
use crate::mm::UserBuffer;
use crate::sync::UPIntrFreeCell;
use crate::task::{current_task, suspend_current_and_run_next, SignalFlags};

pub const SIGNALFD_EAGAIN: usize = usize::MAX - 3;

const SFD_NONBLOCK: usize = 0x800;
#[allow(dead_code)]
const SFD_CLOEXEC: usize = 0o2000000;

#[repr(C)]
struct SignalfdSiginfo {
    ssi_signo: u32,
    ssi_errno: i32,
    ssi_code: i32,
    ssi_pid: u32,
    ssi_uid: u32,
    ssi_fd: i32,
    ssi_tid: u32,
    ssi_band: u32,
    ssi_overrun: u32,
    ssi_trapno: u32,
    ssi_status: i32,
    ssi_int: i32,
    ssi_ptr: u64,
    ssi_utime: u64,
    ssi_stime: u64,
    ssi_addr: u64,
    ssi_addr_lsb: u16,
    _pad: [u8; 46],
}

pub struct SignalFdFile {
    mask: UPIntrFreeCell<SignalFlags>,
    nonblock: bool,
}

impl SignalFdFile {
    pub fn new(mask: SignalFlags, flags: usize) -> Self {
        Self {
            mask: unsafe { UPIntrFreeCell::new(mask) },
            nonblock: flags & SFD_NONBLOCK != 0,
        }
    }

    #[allow(dead_code)]
    pub fn update_mask(&self, mask: SignalFlags) {
        let mut m = self.mask.exclusive_access();
        *m = mask;
    }
}

impl File for SignalFdFile {
    fn readable(&self) -> bool {
        true
    }
    fn writable(&self) -> bool {
        false
    }
    fn read(&self, mut buf: UserBuffer) -> usize {
        let info_size = core::mem::size_of::<SignalfdSiginfo>();
        let buf_len: usize = buf.buffers.iter_mut().map(|s| s.len()).sum();
        if buf_len < info_size {
            return SIGNALFD_EAGAIN;
        }

        loop {
            let task = current_task().unwrap();
            let mask = *self.mask.exclusive_access();
            let pending_sig = task.with_signals(|signals| signals.signal_pending & mask);

            if !pending_sig.is_empty() {
                // Find first pending signal that matches mask
                for signum in 1..=63u32 {
                    let flag = SignalFlags::from_bits_truncate(1u64 << signum);
                    if pending_sig.contains(flag) {
                        // Consume the signal
                        task.with_signals_mut(|signals| {
                            signals.signal_pending.remove(flag);
                        });
                        // Write signalfd_siginfo
                        let mut info = SignalfdSiginfo {
                            ssi_signo: signum,
                            ssi_errno: 0,
                            ssi_code: 0,
                            ssi_pid: 0,
                            ssi_uid: 0,
                            ssi_fd: -1,
                            ssi_tid: 0,
                            ssi_band: 0,
                            ssi_overrun: 0,
                            ssi_trapno: 0,
                            ssi_status: 0,
                            ssi_int: 0,
                            ssi_ptr: 0,
                            ssi_utime: 0,
                            ssi_stime: 0,
                            ssi_addr: 0,
                            ssi_addr_lsb: 0,
                            _pad: [0; 46],
                        };
                        let bytes = unsafe {
                            core::slice::from_raw_parts(
                                &info as *const _ as *const u8,
                                info_size,
                            )
                        };
                        let mut offset = 0;
                        for slice in buf.buffers.iter_mut() {
                            let len = slice.len().min(info_size - offset);
                            slice[..len].copy_from_slice(&bytes[offset..offset + len]);
                            offset += len;
                            if offset >= info_size {
                                break;
                            }
                        }
                        let _ = &mut info; // suppress unused mut
                        return info_size;
                    }
                }
            }

            if self.nonblock {
                return SIGNALFD_EAGAIN;
            }
            suspend_current_and_run_next();
        }
    }
    fn write(&self, _buf: UserBuffer) -> usize {
        0
    }
    fn poll(&self, _events: PollEvents) -> PollEvents {
        let task = current_task().unwrap();
        let mask = *self.mask.exclusive_access();
        let pending = task.with_signals(|signals| signals.signal_pending & mask);
        let mut events = PollEvents::empty();
        if !pending.is_empty() {
            events |= PollEvents::POLLIN;
        }
        events
    }
    fn status_flags(&self) -> u32 {
        if self.nonblock {
            0x800
        } else {
            0
        }
    }
}
