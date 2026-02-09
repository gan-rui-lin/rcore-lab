use alloc::sync::Arc;
use easy_fs::BlockDevice;
use fatfs::{IoBase, IoError, Read, Seek, SeekFrom, Write};

const BLOCK_SIZE: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fat32IoError {
    UnexpectedEof,
    WriteZero,
    OutOfBounds,
}

impl IoError for Fat32IoError {
    fn is_interrupted(&self) -> bool {
        false
    }

    fn new_unexpected_eof_error() -> Self {
        Fat32IoError::UnexpectedEof
    }

    fn new_write_zero_error() -> Self {
        Fat32IoError::WriteZero
    }
}

pub(super) struct Fat32Disk {
    block_id: usize,
    offset: usize,
    device: Arc<dyn BlockDevice>,
    total_bytes: Option<u64>,
    base_lba: usize,
}

impl Fat32Disk {
    pub fn new(device: Arc<dyn BlockDevice>, total_bytes: Option<u64>, base_lba: usize) -> Self {
        Self {
            block_id: 0,
            offset: 0,
            device,
            total_bytes,
            base_lba,
        }
    }

    fn position(&self) -> u64 {
        (self.block_id * BLOCK_SIZE + self.offset) as u64
    }

    fn set_position(&mut self, pos: u64) {
        let pos = pos as usize;
        self.block_id = pos / BLOCK_SIZE;
        self.offset = pos % BLOCK_SIZE;
    }

    fn remaining_bytes(&self) -> Option<u64> {
        self.total_bytes
            .map(|total| total.saturating_sub(self.position()))
    }

    fn read_one(&mut self, buf: &mut [u8]) -> Result<usize, Fat32IoError> {
        let read_size = if self.offset == 0 && buf.len() >= BLOCK_SIZE {
            self.device
                .read_block(self.base_lba + self.block_id, &mut buf[..BLOCK_SIZE]);
            self.block_id += 1;
            BLOCK_SIZE
        } else {
            let mut data = [0u8; BLOCK_SIZE];
            let start = self.offset;
            let count = buf.len().min(BLOCK_SIZE - self.offset);
            self.device
                .read_block(self.base_lba + self.block_id, &mut data);
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

    fn write_one(&mut self, buf: &[u8]) -> Result<usize, Fat32IoError> {
        let write_size = if self.offset == 0 && buf.len() >= BLOCK_SIZE {
            self.device
                .write_block(self.base_lba + self.block_id, &buf[..BLOCK_SIZE]);
            self.block_id += 1;
            BLOCK_SIZE
        } else {
            let mut data = [0u8; BLOCK_SIZE];
            let start = self.offset;
            let count = buf.len().min(BLOCK_SIZE - self.offset);
            self.device
                .read_block(self.base_lba + self.block_id, &mut data);
            data[start..start + count].copy_from_slice(&buf[..count]);
            self.device.write_block(self.base_lba + self.block_id, &data);
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

impl IoBase for Fat32Disk {
    type Error = Fat32IoError;
}

impl Read for Fat32Disk {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut buf = buf;
        if let Some(remaining) = self.remaining_bytes() {
            if remaining == 0 {
                return Ok(0);
            }
            let max = remaining.min(buf.len() as u64) as usize;
            buf = &mut buf[..max];
        }
        let mut read_len = 0;
        while !buf.is_empty() {
            let n = self.read_one(buf)?;
            if n == 0 {
                break;
            }
            read_len += n;
            let tmp = buf;
            buf = &mut tmp[n..];
        }
        Ok(read_len)
    }
}

impl Write for Fat32Disk {
    fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut buf = buf;
        if let Some(remaining) = self.remaining_bytes() {
            if remaining == 0 {
                return Err(Fat32IoError::OutOfBounds);
            }
            let max = remaining.min(buf.len() as u64) as usize;
            buf = &buf[..max];
        }
        let mut write_len = 0;
        while !buf.is_empty() {
            let n = self.write_one(buf)?;
            if n == 0 {
                break;
            }
            write_len += n;
            buf = &buf[n..];
        }
        Ok(write_len)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl Seek for Fat32Disk {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64, Self::Error> {
        let cur = self.position() as i64;
        let new_pos = match pos {
            SeekFrom::Start(off) => Some(off as i64),
            SeekFrom::End(off) => self.total_bytes.map(|t| t as i64 + off),
            SeekFrom::Current(off) => cur.checked_add(off),
        }
        .ok_or(Fat32IoError::OutOfBounds)?;
        if new_pos < 0 {
            return Err(Fat32IoError::OutOfBounds);
        }
        if let Some(total) = self.total_bytes {
            if new_pos as u64 > total {
                return Err(Fat32IoError::OutOfBounds);
            }
        }
        self.set_position(new_pos as u64);
        Ok(self.position())
    }
}
