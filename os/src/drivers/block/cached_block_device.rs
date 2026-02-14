

//! Cached Block Device Wrapper
//!
//! This module provides a caching layer for any BlockDevice implementation.
//! It significantly improves file system performance by reducing direct I/O operations.

use super::BlockDevice;
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use spin::Mutex;

/// Block size in bytes (512 bytes = standard sector size)
const BLOCK_SIZE: usize = 512;

/// Number of blocks to cache (64 blocks = 32KB cache)
/// Can be adjusted based on available memory
const CACHE_SIZE: usize = 64;

/// A single cached block with dirty tracking
pub struct CachedBlock {
    /// Cached block data
    data: [u8; BLOCK_SIZE],
    /// Block ID on the underlying device
    block_id: usize,
    /// Reference to the underlying device
    device: Arc<dyn BlockDevice>,
    /// Dirty flag - true if block has been modified
    dirty: bool,
}

impl CachedBlock {
    /// Load a block from the device into cache
    pub fn new(block_id: usize, device: Arc<dyn BlockDevice>) -> Self {
        let mut data = [0u8; BLOCK_SIZE];
        device.read_block(block_id, &mut data);
        Self {
            data,
            block_id,
            device,
            dirty: false,
        }
    }

    /// Read from cached block
    pub fn read(&self, buf: &mut [u8]) {
        let len = buf.len().min(BLOCK_SIZE);
        buf[..len].copy_from_slice(&self.data[..len]);
    }

    /// Write to cached block and mark as dirty
    pub fn write(&mut self, buf: &[u8]) {
        let len = buf.len().min(BLOCK_SIZE);
        self.data[..len].copy_from_slice(&buf[..len]);
        self.dirty = true;
    }

    /// Sync dirty block back to device
    pub fn sync(&mut self) {
        if self.dirty {
            self.device.write_block(self.block_id, &self.data);
            self.dirty = false;
        }
    }
}

impl Drop for CachedBlock {
    fn drop(&mut self) {
        // Ensure dirty blocks are written back when evicted
        self.sync();
    }
}

/// Cache manager implementing LRU eviction policy
pub struct CacheManager {
    /// Queue of (block_id, cached_block) pairs
    /// Front = most recently used, Back = least recently used
    queue: VecDeque<(usize, Arc<Mutex<CachedBlock>>)>,
    /// Reference to the underlying device
    device: Arc<dyn BlockDevice>,
}

impl CacheManager {
    /// Create a new cache manager
    pub fn new(device: Arc<dyn BlockDevice>) -> Self {
        Self {
            queue: VecDeque::new(),
            device,
        }
    }

    /// Get a cached block, loading it if not present
    pub fn get(&mut self, block_id: usize) -> Arc<Mutex<CachedBlock>> {
        // Check if block is already in cache
        if let Some(pos) = self.queue.iter().position(|(id, _)| *id == block_id) {
            // Move to front (most recently used)
            let cached = self.queue.remove(pos).unwrap();
            self.queue.push_front(cached.clone());
            return cached.1;
        }

        // Not in cache - need to load
        if self.queue.len() >= CACHE_SIZE {
            // Cache is full - evict LRU block
            // Find a block with no external references (strong_count == 1)
            if let Some(pos) = self
                .queue
                .iter()
                .position(|(_, cached)| Arc::strong_count(cached) == 1)
            {
                // Evict this block (Drop will sync if dirty)
                self.queue.remove(pos);
            } else {
                // All blocks are in use - evict from back anyway
                // This is a fallback and shouldn't happen often
                warn!("Cache full with all blocks in use, evicting LRU");
                self.queue.pop_back();
            }
        }

        // Load block into cache
        let cached_block = Arc::new(Mutex::new(CachedBlock::new(
            block_id,
            Arc::clone(&self.device),
        )));
        self.queue
            .push_front((block_id, Arc::clone(&cached_block)));
        cached_block
    }

    /// Sync all dirty blocks to device
    pub fn sync_all(&mut self) {
        for (_, cached) in self.queue.iter() {
            cached.lock().sync();
        }
    }

    /// Get cache statistics
    pub fn stats(&self) -> CacheStats {
        let mut dirty_count = 0;
        for (_, cached) in self.queue.iter() {
            if cached.lock().dirty {
                dirty_count += 1;
            }
        }
        CacheStats {
            total_blocks: self.queue.len(),
            dirty_blocks: dirty_count,
            capacity: CACHE_SIZE,
        }
    }
}

/// Cache statistics for monitoring
#[derive(Debug, Clone, Copy)]
pub struct CacheStats {
    /// Number of blocks currently in cache
    pub total_blocks: usize,
    /// Number of dirty (modified) blocks
    pub dirty_blocks: usize,
    /// Maximum cache capacity
    pub capacity: usize,
}

/// Cached block device wrapper
pub struct CachedBlockDevice {
    /// Cache manager
    cache: Mutex<CacheManager>,
}

impl CachedBlockDevice {
    /// Wrap a block device with caching
    pub fn new(device: Arc<dyn BlockDevice>) -> Arc<Self> {
        Arc::new(Self {
            cache: Mutex::new(CacheManager::new(device)),
        })
    }

    /// Sync all cached data to underlying device
    pub fn sync(&self) {
        self.cache.lock().sync_all();
    }

    /// Get cache statistics
    pub fn stats(&self) -> CacheStats {
        self.cache.lock().stats()
    }
}

impl BlockDevice for CachedBlockDevice {
    fn read_block(&self, block_id: usize, buf: &mut [u8]) {
        let cached = self.cache.lock().get(block_id);
        cached.lock().read(buf);
    }

    fn write_block(&self, block_id: usize, buf: &[u8]) {
        let cached = self.cache.lock().get(block_id);
        cached.lock().write(buf);
    }

    fn handle_irq(&self) {
        // Pass through to underlying device if needed
        // For now, caching layer doesn't need special IRQ handling
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    struct MockBlockDevice {
        blocks: Mutex<Vec<[u8; BLOCK_SIZE]>>,
        read_count: Mutex<usize>,
        write_count: Mutex<usize>,
    }

    impl MockBlockDevice {
        fn new(num_blocks: usize) -> Arc<Self> {
            Arc::new(Self {
                blocks: Mutex::new(vec![[0u8; BLOCK_SIZE]; num_blocks]),
                read_count: Mutex::new(0),
                write_count: Mutex::new(0),
            })
        }

        fn get_stats(&self) -> (usize, usize) {
            (*self.read_count.lock(), *self.write_count.lock())
        }
    }

    impl BlockDevice for MockBlockDevice {
        fn read_block(&self, block_id: usize, buf: &mut [u8]) {
            *self.read_count.lock() += 1;
            let blocks = self.blocks.lock();
            buf.copy_from_slice(&blocks[block_id]);
        }

        fn write_block(&self, block_id: usize, buf: &[u8]) {
            *self.write_count.lock() += 1;
            let mut blocks = self.blocks.lock();
            blocks[block_id].copy_from_slice(buf);
        }

        fn handle_irq(&self) {}
    }

    #[test]
    fn test_cache_reduces_io() {
        let mock_dev = MockBlockDevice::new(100);
        let cached_dev = CachedBlockDevice::new(Arc::clone(&mock_dev) as Arc<dyn BlockDevice>);

        let mut buf = [0u8; BLOCK_SIZE];

        // Read same block 10 times - should only hit device once
        for _ in 0..10 {
            cached_dev.read_block(0, &mut buf);
        }

        let (reads, _) = mock_dev.get_stats();
        assert_eq!(reads, 1, "Cache should reduce reads to 1");

        // Write same block multiple times - should not write to device yet
        for _ in 0..5 {
            cached_dev.write_block(0, &buf);
        }

        let (_, writes) = mock_dev.get_stats();
        assert_eq!(writes, 0, "Writes should be buffered");

        // Sync should write once
        cached_dev.sync();
        let (_, writes) = mock_dev.get_stats();
        assert_eq!(writes, 1, "Sync should write once");
    }
}
