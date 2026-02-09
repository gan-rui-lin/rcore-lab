use easy_fs::BlockDevice;
use lwext4_rust::KernelDevOp;

use alloc::sync::Arc;

const BLOCK_SIZE: usize = 512;
const TRACE_DISK: bool = option_env!("TRACE_DISK").is_some();
const SEEK_SET: u32 = 0;
const SEEK_CUR: u32 = 1;
const SEEK_END: u32 = 2;

pub(super) struct Ext4Disk {
    block_id: usize,
    offset: usize,
    device: Arc<dyn BlockDevice>,
    total_bytes: i64,
}

impl Ext4Disk {
    pub fn new(device: Arc<dyn BlockDevice>, total_bytes: i64) -> Self {
        Self {
            block_id: 0,
            offset: 0,
            device,
            total_bytes,
        }
    }

    fn size(&self) -> i64 {
        self.total_bytes
    }

    fn position(&self) -> i64 {
        (self.block_id * BLOCK_SIZE + self.offset) as i64
    }

    fn set_position(&mut self, pos: i64) {
        let pos = core::cmp::max(0, pos) as usize;
        self.block_id = pos / BLOCK_SIZE;
        self.offset = pos % BLOCK_SIZE;
    }

    fn read_one(&mut self, buf: &mut [u8]) -> Result<usize, i32> {
        if TRACE_DISK {
            trace!(
                "ext4: disk read block={} off={} len={}",
                self.block_id,
                self.offset,
                buf.len()
            );
        }
        let read_size = if self.offset == 0 && buf.len() >= BLOCK_SIZE {
            self.device.read_block(self.block_id, &mut buf[..BLOCK_SIZE]);
            self.block_id += 1;
            BLOCK_SIZE
        } else {
            let mut data = [0u8; BLOCK_SIZE];
            let start = self.offset;
            let count = buf.len().min(BLOCK_SIZE - self.offset);
            self.device.read_block(self.block_id, &mut data);
            buf[..count].copy_from_slice(&data[start..start + count]);
            self.offset += count;
            if self.offset >= BLOCK_SIZE {
                self.block_id += 1;
                self.offset -= BLOCK_SIZE;
            }
            count
        };
        Ok(read_size)
    }

    fn write_one(&mut self, buf: &[u8]) -> Result<usize, i32> {
        if TRACE_DISK {
            trace!(
                "ext4: disk write block={} off={} len={}",
                self.block_id,
                self.offset,
                buf.len()
            );
        }
        let write_size = if self.offset == 0 && buf.len() >= BLOCK_SIZE {
            self.device.write_block(self.block_id, &buf[..BLOCK_SIZE]);
            self.block_id += 1;
            BLOCK_SIZE
        } else {
            let mut data = [0u8; BLOCK_SIZE];
            let start = self.offset;
            let count = buf.len().min(BLOCK_SIZE - self.offset);
            self.device.read_block(self.block_id, &mut data);
            data[start..start + count].copy_from_slice(&buf[..count]);
            self.device.write_block(self.block_id, &data);
            self.offset += count;
            if self.offset >= BLOCK_SIZE {
                self.block_id += 1;
                self.offset -= BLOCK_SIZE;
            }
            count
        };
        Ok(write_size)
    }
}

impl KernelDevOp for Ext4Disk {
    type DevType = Ext4Disk;

    fn write(dev: &mut Self::DevType, buf: &[u8]) -> Result<usize, i32> {
        let mut write_len = 0;
        let mut remaining = buf;
        while !remaining.is_empty() {
            match dev.write_one(remaining) {
                Ok(0) => break,
                Ok(n) => {
                    remaining = &remaining[n..];
                    write_len += n;
                }
                Err(_) => return Err(-1),
            }
        }
        Ok(write_len)
    }

    fn read(dev: &mut Self::DevType, buf: &mut [u8]) -> Result<usize, i32> {
        let mut read_len = 0;
        let mut remaining = buf;
        while !remaining.is_empty() {
            match dev.read_one(remaining) {
                Ok(0) => break,
                Ok(n) => {
                    let tmp = remaining;
                    remaining = &mut tmp[n..];
                    read_len += n;
                }
                Err(_) => return Err(-1),
            }
        }
        Ok(read_len)
    }

    fn seek(dev: &mut Self::DevType, off: i64, whence: i32) -> Result<i64, i32> {
        let new_pos = match whence as u32 {
            SEEK_SET => Some(off),
            SEEK_CUR => dev.position().checked_add(off),
            SEEK_END => dev.size().checked_add(off),
            _ => None,
        }
        .ok_or(-1)?;
        dev.set_position(new_pos);
        Ok(dev.position())
    }

    fn flush(_dev: &mut Self::DevType) -> Result<usize, i32> {
        Ok(0)
    }
}
