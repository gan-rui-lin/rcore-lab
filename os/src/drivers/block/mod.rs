mod cached_block_device;
#[cfg(target_arch = "riscv64")]
mod virtio_blk;
#[cfg(target_arch = "loongarch64")]
mod virtio_blk_pci;

pub use cached_block_device::{CacheStats, CachedBlockDevice};
#[cfg(target_arch = "riscv64")]
pub use virtio_blk::VirtIOBlock;
#[cfg(target_arch = "loongarch64")]
pub use virtio_blk_pci::VirtIOPCIBlock;

use crate::board::BlockDeviceImpl;
use alloc::sync::Arc;
use easy_fs::BlockDevice;
use lazy_static::*;

lazy_static! {
    /// Concrete cached block device, kept so kernel code can flush dirty pages explicitly.
    pub static ref CACHED_BLOCK_DEVICE: Arc<CachedBlockDevice> = {
        let raw_device: Arc<dyn BlockDevice> = Arc::new(BlockDeviceImpl::new());
        CachedBlockDevice::new(raw_device)
    };
    /// Global block device instance with caching layer.
    /// This wraps the raw VirtIO device with a 64-block LRU cache
    /// to significantly improve file system performance.
    pub static ref BLOCK_DEVICE: Arc<dyn BlockDevice> = {
        let cached: Arc<CachedBlockDevice> = Arc::clone(&CACHED_BLOCK_DEVICE);
        cached
    };
}

/// Flush all dirty cached blocks to the underlying block device.
pub fn sync_block_cache() {
    CACHED_BLOCK_DEVICE.sync();
}

/// Get cache statistics from the global block device
pub fn get_cache_stats() -> Option<CacheStats> {
    Some(CACHED_BLOCK_DEVICE.stats())
}

/// Basic read/write test for the block device.
#[allow(unused)]
pub fn block_device_test() {
    let block_device = BLOCK_DEVICE.clone();
    let mut write_buffer = [0u8; 512];
    let mut read_buffer = [0u8; 512];
    for i in 0..512 {
        for byte in write_buffer.iter_mut() {
            *byte = i as u8;
        }
        block_device.write_block(i as usize, &write_buffer);
        block_device.read_block(i as usize, &mut read_buffer);
        assert_eq!(write_buffer, read_buffer);
    }
    println!("block device test passed!");
}
