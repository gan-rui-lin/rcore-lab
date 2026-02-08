//! Simple in-memory pipe implementation.
use super::File;
use crate::mm::UserBuffer;
use crate::sync::UPSafeCell;
use crate::task::suspend_current_and_run_next;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

const DEFAULT_PIPE_CAPACITY: usize = 4096;

struct Pipe {
    buf: Vec<u8>,
    head: usize,
    tail: usize,
    len: usize,
    read_open: bool,
    write_open: bool,
}

impl Pipe {
    fn new(capacity: usize) -> Self {
        Self {
            buf: vec![0; capacity.max(1)],
            head: 0,
            tail: 0,
            len: 0,
            read_open: true,
            write_open: true,
        }
    }

    fn capacity(&self) -> usize {
        self.buf.len()
    }

    fn read_into(&mut self, out: &mut [u8]) -> usize {
        let mut read = 0;
        while read < out.len() && self.len > 0 {
            let cap = self.capacity();
            let chunk = (cap - self.head)
                .min(self.len)
                .min(out.len() - read);
            out[read..read + chunk].copy_from_slice(&self.buf[self.head..self.head + chunk]);
            self.head = (self.head + chunk) % cap;
            self.len -= chunk;
            read += chunk;
        }
        read
    }

    fn write_from(&mut self, data: &[u8]) -> usize {
        let mut written = 0;
        while written < data.len() && self.len < self.capacity() {
            let cap = self.capacity();
            let space = cap - self.len;
            let chunk = (cap - self.tail)
                .min(space)
                .min(data.len() - written);
            self.buf[self.tail..self.tail + chunk]
                .copy_from_slice(&data[written..written + chunk]);
            self.tail = (self.tail + chunk) % cap;
            self.len += chunk;
            written += chunk;
        }
        written
    }
}

pub struct PipeEnd {
    readable: bool,
    writable: bool,
    pipe: Arc<UPSafeCell<Pipe>>,
}

impl PipeEnd {
    fn new(pipe: Arc<UPSafeCell<Pipe>>, readable: bool, writable: bool) -> Self {
        Self {
            readable,
            writable,
            pipe,
        }
    }
}

impl Drop for PipeEnd {
    fn drop(&mut self) {
        let mut pipe = self.pipe.exclusive_access();
        if self.readable {
            pipe.read_open = false;
        }
        if self.writable {
            pipe.write_open = false;
        }
    }
}

impl File for PipeEnd {
    fn readable(&self) -> bool {
        self.readable
    }

    fn writable(&self) -> bool {
        self.writable
    }

    fn read(&self, mut user_buf: UserBuffer) -> usize {
        let mut total = 0;
        for slice in user_buf.buffers.iter_mut() {
            loop {
                let mut pipe = self.pipe.exclusive_access();
                if pipe.len > 0 {
                    let n = pipe.read_into(*slice);
                    total += n;
                    break;
                }
                if !pipe.write_open {
                    return total;
                }
                if total > 0 {
                    return total;
                }
                drop(pipe);
                suspend_current_and_run_next();
            }
        }
        total
    }

    fn write(&self, user_buf: UserBuffer) -> usize {
        let mut total = 0;
        for slice in user_buf.buffers.iter() {
            let mut offset = 0;
            while offset < slice.len() {
                let mut pipe = self.pipe.exclusive_access();
                if !pipe.read_open {
                    return total;
                }
                if pipe.len < pipe.capacity() {
                    let n = pipe.write_from(&slice[offset..]);
                    total += n;
                    offset += n;
                    if n == 0 {
                        break;
                    }
                } else {
                    if total > 0 {
                        return total;
                    }
                    drop(pipe);
                    suspend_current_and_run_next();
                }
            }
        }
        total
    }
}

/// Create a pipe and return (read_end, write_end).
pub fn make_pipe(capacity: usize) -> (Arc<dyn File + Send + Sync>, Arc<dyn File + Send + Sync>) {
    let pipe = Arc::new(unsafe { UPSafeCell::new(Pipe::new(capacity.max(DEFAULT_PIPE_CAPACITY))) });
    let read_end = Arc::new(PipeEnd::new(pipe.clone(), true, false));
    let write_end = Arc::new(PipeEnd::new(pipe, false, true));
    (read_end, write_end)
}
