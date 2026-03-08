#[cfg(target_arch = "riscv64")]
mod virtio_blk;
#[cfg(target_arch = "loongarch64")]
mod virtio_blk_pci;
mod cached_block_device;

#[cfg(target_arch = "riscv64")]
pub use virtio_blk::VirtIOBlock;
#[cfg(target_arch = "loongarch64")]
pub use virtio_blk_pci::VirtIOPCIBlock;
pub use cached_block_device::{CachedBlockDevice, CacheStats};

use crate::board::BlockDeviceImpl;
use alloc::sync::Arc;
use easy_fs::BlockDevice;
use lazy_static::*;

lazy_static! {
    /// Global block device instance with caching layer.
    /// This wraps the raw VirtIO device with a 64-block LRU cache
    /// to significantly improve file system performance.
    pub static ref BLOCK_DEVICE: Arc<dyn BlockDevice> = {
        let raw_device: Arc<dyn BlockDevice> = Arc::new(BlockDeviceImpl::new());
        CachedBlockDevice::new(raw_device)
    };
}

/// Get cache statistics from the global block device
pub fn get_cache_stats() -> Option<CacheStats> {
    // Try to downcast to CachedBlockDevice
    // Note: This requires the actual type, not the trait object
    // For now, we'll add a global cache stats function
    None
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
