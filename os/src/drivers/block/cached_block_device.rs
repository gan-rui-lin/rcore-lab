//! Cached Block Device Wrapper
//!
//! This module provides a caching layer for any BlockDevice implementation.
//! It significantly improves file system performance by reducing direct I/O operations.

use super::BlockDevice;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::sync::Arc;
use core::sync::atomic::{AtomicU64, Ordering};
use spin::Mutex;

/// Logical block size used by the block device API.
const BLOCK_SIZE: usize = 512;
/// Cache entry granularity (one cached page contains multiple logical blocks).
const CACHE_PAGE_SIZE: usize = 16 * 1024;
const BLOCKS_PER_PAGE: usize = CACHE_PAGE_SIZE / BLOCK_SIZE;

const fn parse_cache_size(value: &str) -> Option<usize> {
    let bytes = value.as_bytes();
    let mut out: usize = 0;
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if b < b'0' || b > b'9' {
            return None;
        }
        out = out.saturating_mul(10).saturating_add((b - b'0') as usize);
        i += 1;
    }
    Some(out)
}

const fn cache_size_blocks_from_env() -> usize {
    match option_env!("BLOCK_CACHE_SIZE") {
        Some(v) => match parse_cache_size(v) {
            Some(0) | None => 49_152,
            Some(n) => n,
        },
        None => 49_152,
    }
}

const fn cache_page_capacity_from_blocks(blocks: usize) -> usize {
    let pages = (blocks + BLOCKS_PER_PAGE - 1) / BLOCKS_PER_PAGE;
    if pages == 0 { 1 } else { pages }
}

/// Cache capacity in logical 512-byte blocks (compatible with old meaning).
const CACHE_SIZE_BLOCKS: usize = cache_size_blocks_from_env();
/// Actual cache entry capacity in 4KiB pages.
const CACHE_PAGE_CAPACITY: usize = cache_page_capacity_from_blocks(CACHE_SIZE_BLOCKS);

const TRACE_BLOCK_CACHE_STATS: bool = option_env!("TRACE_BLOCK_CACHE_STATS").is_some();
const CACHE_STATS_LOG_EVERY_GETS: u64 = 20_000;
const CACHE_PRESSURE_WARN_BURST: u64 = 1;
const CACHE_PRESSURE_WARN_EVERY: u64 = 4096;

static CACHE_GET_CALLS: AtomicU64 = AtomicU64::new(0);
static CACHE_HIT_CALLS: AtomicU64 = AtomicU64::new(0);
static CACHE_MISS_CALLS: AtomicU64 = AtomicU64::new(0);
static CACHE_EVICT_CALLS: AtomicU64 = AtomicU64::new(0);
static CACHE_DIRTY_EVICT_CALLS: AtomicU64 = AtomicU64::new(0);
static CACHE_BACKEND_READS: AtomicU64 = AtomicU64::new(0);
static CACHE_BACKEND_WRITES: AtomicU64 = AtomicU64::new(0);
static CACHE_LAST_LOG_STEP: AtomicU64 = AtomicU64::new(0);
static CACHE_FORCED_EVICT_COUNT: AtomicU64 = AtomicU64::new(0);

#[inline]
fn maybe_log_cache_stats() {
    if !TRACE_BLOCK_CACHE_STATS {
        return;
    }
    let gets = CACHE_GET_CALLS.load(Ordering::Relaxed);
    if gets == 0 {
        return;
    }
    let step = gets / CACHE_STATS_LOG_EVERY_GETS;
    if step == 0 {
        return;
    }
    let mut prev = CACHE_LAST_LOG_STEP.load(Ordering::Relaxed);
    while step > prev {
        match CACHE_LAST_LOG_STEP.compare_exchange_weak(
            prev,
            step,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => {
                let hits = CACHE_HIT_CALLS.load(Ordering::Relaxed);
                let misses = CACHE_MISS_CALLS.load(Ordering::Relaxed);
                let hit_pct = if gets == 0 {
                    0
                } else {
                    hits.saturating_mul(100) / gets
                };
                info!(
                    "[blk-cache] t_ms={} gets={} hit={} miss={} hit_pct={} evict={} dirty_evict={} backend_read={} backend_write={} page_cap={} blk_cap={}",
                    crate::timer::get_time_ms(),
                    gets,
                    hits,
                    misses,
                    hit_pct,
                    CACHE_EVICT_CALLS.load(Ordering::Relaxed),
                    CACHE_DIRTY_EVICT_CALLS.load(Ordering::Relaxed),
                    CACHE_BACKEND_READS.load(Ordering::Relaxed),
                    CACHE_BACKEND_WRITES.load(Ordering::Relaxed),
                    CACHE_PAGE_CAPACITY,
                    CACHE_SIZE_BLOCKS,
                );
                break;
            }
            Err(actual) => prev = actual,
        }
    }
}

#[inline]
fn maybe_warn_cache_pressure() {
    let forced = CACHE_FORCED_EVICT_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
    if forced <= CACHE_PRESSURE_WARN_BURST || forced % CACHE_PRESSURE_WARN_EVERY == 0 {
        warn!(
            "Cache full with all pages recently used/in use, forcing eviction (count={})",
            forced
        );
    }
}

#[inline]
const fn page_id_of(block_id: usize) -> usize {
    block_id / BLOCKS_PER_PAGE
}

#[inline]
const fn block_off_in_page(block_id: usize) -> usize {
    block_id % BLOCKS_PER_PAGE
}

#[inline]
const fn block_mask(block_off: usize) -> u64 {
    1u64 << block_off
}

#[inline]
const fn page_block_range(block_off: usize) -> (usize, usize) {
    let start = block_off * BLOCK_SIZE;
    (start, start + BLOCK_SIZE)
}

/// A cached 4KiB page with per-512B block dirty/load tracking.
pub struct CachedPage {
    data: [u8; CACHE_PAGE_SIZE],
    page_id: usize,
    device: Arc<dyn BlockDevice>,
    loaded_mask: u64,
    dirty_mask: u64,
    accessed: bool,
}

impl CachedPage {
    pub fn new_empty(page_id: usize, device: Arc<dyn BlockDevice>) -> Self {
        Self {
            data: [0u8; CACHE_PAGE_SIZE],
            page_id,
            device,
            loaded_mask: 0,
            dirty_mask: 0,
            accessed: true,
        }
    }

    fn ensure_block_loaded(&mut self, block_off: usize) {
        let mask = block_mask(block_off);
        if self.loaded_mask & mask != 0 {
            return;
        }
        let lba = self.page_id * BLOCKS_PER_PAGE + block_off;
        let (start, end) = page_block_range(block_off);
        self.device.read_block(lba, &mut self.data[start..end]);
        CACHE_BACKEND_READS.fetch_add(1, Ordering::Relaxed);
        self.loaded_mask |= mask;
    }

    pub fn read_block(&mut self, block_off: usize, buf: &mut [u8]) {
        self.ensure_block_loaded(block_off);
        let (start, end) = page_block_range(block_off);
        let len = buf.len().min(BLOCK_SIZE);
        buf[..len].copy_from_slice(&self.data[start..end][..len]);
    }

    pub fn write_block(&mut self, block_off: usize, buf: &[u8], full_overwrite: bool) {
        if !full_overwrite {
            self.ensure_block_loaded(block_off);
        }
        let (start, end) = page_block_range(block_off);
        let len = buf.len().min(BLOCK_SIZE);
        self.data[start..start + len].copy_from_slice(&buf[..len]);
        if len < BLOCK_SIZE {
            // Partial overwrite must preserve trailing bytes.
            self.ensure_block_loaded(block_off);
        }
        let mask = block_mask(block_off);
        self.loaded_mask |= mask;
        self.dirty_mask |= mask;
        let _ = end;
    }

    pub fn has_dirty(&self) -> bool {
        self.dirty_mask != 0
    }

    #[inline]
    pub fn mark_accessed(&mut self) {
        self.accessed = true;
    }

    #[inline]
    pub fn take_accessed(&mut self) -> bool {
        let was = self.accessed;
        self.accessed = false;
        was
    }

    pub fn dirty_blocks_count(&self) -> usize {
        self.dirty_mask.count_ones() as usize
    }

    pub fn sync(&mut self) {
        if self.dirty_mask == 0 {
            return;
        }
        let base_lba = self.page_id * BLOCKS_PER_PAGE;
        for block_off in 0..BLOCKS_PER_PAGE {
            let mask = block_mask(block_off);
            if self.dirty_mask & mask == 0 {
                continue;
            }
            let (start, end) = page_block_range(block_off);
            self.device
                .write_block(base_lba + block_off, &self.data[start..end]);
            CACHE_BACKEND_WRITES.fetch_add(1, Ordering::Relaxed);
            self.dirty_mask &= !mask;
        }
    }
}

impl Drop for CachedPage {
    fn drop(&mut self) {
        self.sync();
    }
}

/// Cache manager implementing LRU eviction policy.
pub struct CacheManager {
    /// Queue of page ids for second-chance eviction.
    /// Front = newest insertion, Back = oldest candidate.
    queue: VecDeque<usize>,
    /// Cached pages indexed by page id.
    pages: BTreeMap<usize, Arc<Mutex<CachedPage>>>,
    /// Reference to the underlying device.
    device: Arc<dyn BlockDevice>,
}

impl CacheManager {
    pub fn new(device: Arc<dyn BlockDevice>) -> Self {
        Self {
            queue: VecDeque::new(),
            pages: BTreeMap::new(),
            device,
        }
    }

    fn evict_if_needed(&mut self) {
        if self.pages.len() < CACHE_PAGE_CAPACITY {
            return;
        }

        // Second-chance scan: recently accessed pages get one extra turn.
        let scan_budget = self.queue.len().saturating_mul(2).max(1);
        for _ in 0..scan_budget {
            let Some(candidate_id) = self.queue.pop_back() else {
                break;
            };
            let Some(candidate) = self.pages.get(&candidate_id).cloned() else {
                continue;
            };

            if Arc::strong_count(&candidate) > 1 {
                self.queue.push_front(candidate_id);
                continue;
            }

            let mut guard = candidate.lock();
            if guard.take_accessed() {
                drop(guard);
                self.queue.push_front(candidate_id);
                continue;
            }
            let was_dirty = guard.has_dirty();
            drop(guard);
            self.pages.remove(&candidate_id);
            CACHE_EVICT_CALLS.fetch_add(1, Ordering::Relaxed);
            if was_dirty {
                CACHE_DIRTY_EVICT_CALLS.fetch_add(1, Ordering::Relaxed);
            }
            return;
        }

        maybe_warn_cache_pressure();
        if let Some(candidate_id) = self.queue.pop_back() {
            if let Some(candidate) = self.pages.remove(&candidate_id) {
                CACHE_EVICT_CALLS.fetch_add(1, Ordering::Relaxed);
                if candidate.lock().has_dirty() {
                    CACHE_DIRTY_EVICT_CALLS.fetch_add(1, Ordering::Relaxed);
                }
            }
        }

    }

    fn get_cached_page(&mut self, page_id: usize) -> Option<Arc<Mutex<CachedPage>>> {
        if let Some(cached) = self.pages.get(&page_id).cloned() {
            CACHE_HIT_CALLS.fetch_add(1, Ordering::Relaxed);
            cached.lock().mark_accessed();
            maybe_log_cache_stats();
            return Some(cached);
        }
        None
    }

    pub fn get_page(&mut self, page_id: usize) -> Arc<Mutex<CachedPage>> {
        CACHE_GET_CALLS.fetch_add(1, Ordering::Relaxed);
        if let Some(cached) = self.get_cached_page(page_id) {
            return cached;
        }
        CACHE_MISS_CALLS.fetch_add(1, Ordering::Relaxed);
        self.evict_if_needed();

        let cached_page = Arc::new(Mutex::new(CachedPage::new_empty(
            page_id,
            Arc::clone(&self.device),
        )));
        self.queue.push_front(page_id);
        self.pages.insert(page_id, Arc::clone(&cached_page));
        maybe_log_cache_stats();
        cached_page
    }

    pub fn sync_all(&mut self) {
        for cached in self.pages.values() {
            cached.lock().sync();
        }
    }

    pub fn stats(&self) -> CacheStats {
        let mut dirty_count = 0;
        for cached in self.pages.values() {
            dirty_count += cached.lock().dirty_blocks_count();
        }
        CacheStats {
            total_blocks: self.pages.len().saturating_mul(BLOCKS_PER_PAGE),
            dirty_blocks: dirty_count,
            capacity: CACHE_PAGE_CAPACITY.saturating_mul(BLOCKS_PER_PAGE),
        }
    }
}

/// Cache statistics for monitoring.
#[derive(Debug, Clone, Copy)]
pub struct CacheStats {
    /// Number of logical blocks currently represented in cache pages.
    pub total_blocks: usize,
    /// Number of dirty logical blocks.
    pub dirty_blocks: usize,
    /// Maximum logical block capacity.
    pub capacity: usize,
}

/// Cached block device wrapper.
pub struct CachedBlockDevice {
    cache: Mutex<CacheManager>,
}

impl CachedBlockDevice {
    /// Wrap a block device with caching.
    pub fn new(device: Arc<dyn BlockDevice>) -> Arc<Self> {
        Arc::new(Self {
            cache: Mutex::new(CacheManager::new(device)),
        })
    }

    /// Sync all cached dirty data to the underlying device.
    pub fn sync(&self) {
        self.cache.lock().sync_all();
    }

    /// Get current cache statistics.
    pub fn stats(&self) -> CacheStats {
        self.cache.lock().stats()
    }
}

impl BlockDevice for CachedBlockDevice {
    fn read_block(&self, block_id: usize, buf: &mut [u8]) {
        let page_id = page_id_of(block_id);
        let block_off = block_off_in_page(block_id);
        let cached = self.cache.lock().get_page(page_id);
        cached.lock().read_block(block_off, buf);
    }

    fn write_block(&self, block_id: usize, buf: &[u8]) {
        let page_id = page_id_of(block_id);
        let block_off = block_off_in_page(block_id);
        let full_overwrite = buf.len() >= BLOCK_SIZE;
        let cached = self.cache.lock().get_page(page_id);
        cached.lock().write_block(block_off, buf, full_overwrite);
    }

    fn handle_irq(&self) {
        // Caching layer does not need special IRQ handling.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
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
