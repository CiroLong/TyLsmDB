use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::table::block::Block;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
}

#[derive(Debug, Clone)]
pub struct BlockCache {
    inner: Arc<Mutex<BlockCacheInner>>,
}

#[derive(Debug)]
struct BlockCacheInner {
    blocks: HashMap<(u64, u64), Arc<Block>>,
    max_entries: usize,
    stats: CacheStats,
}

impl BlockCache {
    pub fn new(capacity_bytes: usize) -> Self {
        let max_entries = (capacity_bytes / 4096).max(1);
        Self {
            inner: Arc::new(Mutex::new(BlockCacheInner {
                blocks: HashMap::new(),
                max_entries,
                stats: CacheStats::default(),
            })),
        }
    }

    pub fn get(&self, table_number: u64, block_offset: u64) -> Option<Arc<Block>> {
        let mut inner = self.inner.lock().expect("block cache lock poisoned");
        let block = inner.blocks.get(&(table_number, block_offset)).cloned();
        if block.is_some() {
            inner.stats.hits += 1;
        } else {
            inner.stats.misses += 1;
        }
        block
    }

    pub fn insert(&self, table_number: u64, block_offset: u64, block: Arc<Block>) {
        let mut inner = self.inner.lock().expect("block cache lock poisoned");
        if inner.blocks.len() >= inner.max_entries
            && let Some(key) = inner.blocks.keys().next().copied()
        {
            inner.blocks.remove(&key);
        }
        inner.blocks.insert((table_number, block_offset), block);
    }

    pub fn stats(&self) -> CacheStats {
        self.inner.lock().expect("block cache lock poisoned").stats
    }
}
