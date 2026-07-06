use std::sync::atomic::{AtomicU64, Ordering};

use crate::cache::CacheStats;

#[derive(Debug, Default)]
pub struct Metrics {
    user_write_bytes: AtomicU64,
    wal_write_bytes: AtomicU64,
    sst_write_bytes: AtomicU64,
    compaction_read_bytes: AtomicU64,
    compaction_write_bytes: AtomicU64,
    bloom_useful: AtomicU64,
    bloom_false_positive: AtomicU64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MetricsSnapshot {
    pub user_write_bytes: u64,
    pub wal_write_bytes: u64,
    pub sst_write_bytes: u64,
    pub compaction_read_bytes: u64,
    pub compaction_write_bytes: u64,
    pub block_cache_hits: u64,
    pub block_cache_misses: u64,
    pub bloom_useful: u64,
    pub bloom_false_positive: u64,
}

impl Metrics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_user_write(&self, bytes: u64) {
        self.user_write_bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn record_wal_write(&self, bytes: u64) {
        self.wal_write_bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn record_sst_write(&self, bytes: u64) {
        self.sst_write_bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn record_compaction_read(&self, bytes: u64) {
        self.compaction_read_bytes
            .fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn record_compaction_write(&self, bytes: u64) {
        self.compaction_write_bytes
            .fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn snapshot(&self, cache_stats: CacheStats) -> MetricsSnapshot {
        MetricsSnapshot {
            user_write_bytes: self.user_write_bytes.load(Ordering::Relaxed),
            wal_write_bytes: self.wal_write_bytes.load(Ordering::Relaxed),
            sst_write_bytes: self.sst_write_bytes.load(Ordering::Relaxed),
            compaction_read_bytes: self.compaction_read_bytes.load(Ordering::Relaxed),
            compaction_write_bytes: self.compaction_write_bytes.load(Ordering::Relaxed),
            block_cache_hits: cache_stats.hits,
            block_cache_misses: cache_stats.misses,
            bloom_useful: self.bloom_useful.load(Ordering::Relaxed),
            bloom_false_positive: self.bloom_false_positive.load(Ordering::Relaxed),
        }
    }
}
