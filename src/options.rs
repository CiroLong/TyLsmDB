use std::sync::Arc;
use std::time::Duration;

use crate::env::{Env, FsEnv};
use crate::memtable::MemTableKind;
use crate::snapshot::Snapshot;
use crate::table::format::CompressionType;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalSyncMode {
    Never,
    PerWrite,
}

#[derive(Debug, Clone)]
pub struct Options {
    pub create_if_missing: bool,
    pub error_if_exists: bool,
    pub memtable_size: usize,
    pub max_immutable_memtables: usize,
    pub block_size: usize,
    pub target_file_size_base: usize,
    pub max_levels: usize,
    pub level0_file_num_compaction_trigger: usize,
    pub level0_slowdown_writes_trigger: usize,
    pub level0_stop_writes_trigger: usize,
    pub max_bytes_for_level_base: usize,
    pub max_bytes_for_level_multiplier: f64,
    pub wal_enabled: bool,
    pub wal_sync: WalSyncMode,
    pub bloom_false_positive_rate: f64,
    pub block_cache_capacity: usize,
    pub max_background_flushes: usize,
    pub max_background_compactions: usize,
    pub max_subcompactions: usize,
    pub memtable_kind: MemTableKind,
    pub table_compression: CompressionType,
    pub write_rate_limit_bytes_per_sec: Option<u64>,
    pub write_group_max_delay: Duration,
    pub env: Arc<dyn Env>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            create_if_missing: true,
            error_if_exists: false,
            memtable_size: 4 * 1024 * 1024,
            max_immutable_memtables: 3,
            block_size: 4 * 1024,
            target_file_size_base: 64 * 1024 * 1024,
            max_levels: 7,
            level0_file_num_compaction_trigger: 4,
            level0_slowdown_writes_trigger: 12,
            level0_stop_writes_trigger: 20,
            max_bytes_for_level_base: 256 * 1024 * 1024,
            max_bytes_for_level_multiplier: 10.0,
            wal_enabled: true,
            wal_sync: WalSyncMode::Never,
            bloom_false_positive_rate: 0.01,
            block_cache_capacity: 64 * 1024 * 1024,
            max_background_flushes: 1,
            max_background_compactions: 1,
            max_subcompactions: 1,
            memtable_kind: MemTableKind::BTree,
            table_compression: CompressionType::None,
            write_rate_limit_bytes_per_sec: None,
            write_group_max_delay: Duration::from_micros(250),
            env: Arc::new(FsEnv),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct WriteOptions {
    pub sync: bool,
    pub disable_wal: bool,
}

#[derive(Debug, Clone)]
pub struct ReadOptions {
    pub verify_checksums: bool,
    pub fill_cache: bool,
    pub total_order_seek: bool,
    pub snapshot: Option<Snapshot>,
}

impl Default for ReadOptions {
    fn default() -> Self {
        Self {
            verify_checksums: true,
            fill_cache: true,
            total_order_seek: false,
            snapshot: None,
        }
    }
}
