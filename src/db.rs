use std::collections::{BTreeSet, VecDeque};
use std::mem;
use std::ops::Bound;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::thread;
use std::time::Duration;

use crate::batch::{BatchRecord, WriteBatch};
use crate::bytes::Bytes;
use crate::cache::{BlockCache, CacheStats, TableCache};
use crate::compact::executor::compact_entries;
use crate::compact::picker::CompactionPicker;
use crate::compact::task::CompactionTask;
use crate::env::file::{table_file_name, wal_file_name};
use crate::error::{Error, Result};
use crate::iterator::{DBIterator, EntryIterator, MergeIterator, StorageIterator};
use crate::key::{InternalKey, SequenceNumber};
use crate::memtable::{MemTable, MemTableKind, ValueRecord};
use crate::metrics::{Metrics, MetricsSnapshot};
use crate::mvcc::conflict::ReadRange;
use crate::mvcc::watermark::Watermark;
use crate::options::{Options, ReadOptions, WalSyncMode, WriteOptions};
use crate::snapshot::Snapshot;
use crate::table::{SSTableBuilder, SSTableReader};
use crate::transaction::{Transaction, TransactionOptions};
use crate::util::rate_limiter::RateLimiter;
use crate::version::{FileMeta, Version, VersionEdit, VersionSet};
use crate::wal::{WalReader, WalWriter};

#[derive(Debug, Clone)]
pub struct DB {
    inner: Arc<DBInner>,
}

#[derive(Debug)]
struct DBInner {
    path: PathBuf,
    options: Options,
    state: RwLock<DBState>,
    versions: Mutex<VersionSet>,
    block_cache: BlockCache,
    table_cache: TableCache,
    watermark: Arc<Watermark>,
    metrics: Metrics,
    write_rate_limiter: Option<RateLimiter>,
    write_group: Mutex<VecDeque<Arc<PendingWrite>>>,
    wal: Mutex<WalWriter>,
}

#[derive(Debug)]
struct DBState {
    mutable: MemTable,
    immutables: Vec<MemTable>,
    l0_tables: Vec<TableRef>,
    level_tables: Vec<Vec<TableRef>>,
    last_sequence: SequenceNumber,
    closed: bool,
}

type TableRef = (FileMeta, Arc<SSTableReader>);

#[derive(Debug, Clone, Copy)]
enum FlushSource {
    Mutable,
    ImmutableOldest,
}

#[derive(Debug)]
struct PendingWrite {
    batch: Mutex<Option<WriteBatch>>,
    opts: WriteOptions,
    result: Mutex<Option<Result<()>>>,
    done: Condvar,
}

impl PendingWrite {
    fn new(batch: WriteBatch, opts: WriteOptions) -> Self {
        Self {
            batch: Mutex::new(Some(batch)),
            opts,
            result: Mutex::new(None),
            done: Condvar::new(),
        }
    }

    fn take_batch(&self) -> WriteBatch {
        self.batch
            .lock()
            .expect("pending write batch lock poisoned")
            .take()
            .expect("pending write batch is present")
    }

    fn complete(&self, result: Result<()>) {
        *self
            .result
            .lock()
            .expect("pending write result lock poisoned") = Some(result);
        self.done.notify_one();
    }

    fn wait(&self) -> Result<()> {
        let mut result = self
            .result
            .lock()
            .expect("pending write result lock poisoned");
        loop {
            if let Some(result) = result.take() {
                return result;
            }
            result = self
                .done
                .wait(result)
                .expect("pending write condvar wait poisoned");
        }
    }
}

struct GroupWriteItem {
    batch: WriteBatch,
    opts: WriteOptions,
    user_write_bytes: u64,
    start_sequence: SequenceNumber,
}

impl DB {
    pub fn open(path: impl AsRef<Path>, options: Options) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let env = Arc::clone(&options.env);
        if env.exists(&path) && options.error_if_exists {
            return Err(Error::InvalidArgument(format!(
                "database already exists: {}",
                path.display()
            )));
        }
        if !env.exists(&path) && !options.create_if_missing {
            return Err(Error::InvalidArgument(format!(
                "database does not exist: {}",
                path.display()
            )));
        }
        env.create_dir_all(&path)?;

        let options_for_versions = options.clone();
        let versions = if env.exists(&path.join("CURRENT")) {
            VersionSet::recover(&path, options_for_versions)?
        } else {
            VersionSet::create(&path, options_for_versions)?
        };
        let block_cache = BlockCache::new(options.block_cache_capacity);
        let table_cache = TableCache::new_with_env(512, Arc::clone(&env));
        let watermark = Arc::new(Watermark::new());
        let (l0_tables, level_tables) =
            open_version_tables(&path, versions.current().as_ref(), &table_cache)?;
        let mut state = DBState {
            mutable: MemTable::new(options.memtable_kind),
            immutables: Vec::new(),
            l0_tables,
            level_tables,
            last_sequence: versions.last_sequence(),
            closed: false,
        };
        let wal_path = path.join(wal_file_name(versions.log_number()));
        if env.exists(&wal_path) {
            recover_wal(env.as_ref(), &wal_path, &mut state)?;
        }
        let wal = WalWriter::create_with_env(env.as_ref(), &wal_path)?;
        let write_rate_limiter = options.write_rate_limit_bytes_per_sec.map(RateLimiter::new);

        Ok(Self {
            inner: Arc::new(DBInner {
                path,
                options,
                state: RwLock::new(state),
                versions: Mutex::new(versions),
                block_cache,
                table_cache,
                watermark,
                metrics: Metrics::new(),
                write_rate_limiter,
                write_group: Mutex::new(VecDeque::new()),
                wal: Mutex::new(wal),
            }),
        })
    }

    pub fn close(&self) -> Result<()> {
        let mut state = self.write_state()?;
        state.closed = true;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.inner.path
    }

    pub fn options(&self) -> &Options {
        &self.inner.options
    }

    pub fn put(&self, key: &[u8], value: &[u8]) -> Result<()> {
        let mut batch = WriteBatch::new();
        batch.put(key.to_vec(), value.to_vec());
        self.write(batch, WriteOptions::default())
    }

    pub fn delete(&self, key: &[u8]) -> Result<()> {
        let mut batch = WriteBatch::new();
        batch.delete(key.to_vec());
        self.write(batch, WriteOptions::default())
    }

    pub fn write(&self, batch: WriteBatch, opts: WriteOptions) -> Result<()> {
        if batch.is_empty() {
            return Ok(());
        }

        let should_wait_for_group =
            opts.sync || self.inner.options.wal_sync == WalSyncMode::PerWrite;
        let request = Arc::new(PendingWrite::new(batch, opts));
        let is_leader = {
            let mut group = self
                .inner
                .write_group
                .lock()
                .expect("write group lock poisoned");
            let is_leader = group.is_empty();
            group.push_back(Arc::clone(&request));
            is_leader
        };

        if !is_leader {
            return request.wait();
        }

        if should_wait_for_group {
            thread::sleep(self.inner.options.write_group_max_delay);
        }
        let requests = {
            let mut group = self
                .inner
                .write_group
                .lock()
                .expect("write group lock poisoned");
            group.drain(..).collect::<Vec<_>>()
        };
        let results = self.apply_write_group(&requests);
        for (request, result) in requests.iter().zip(results) {
            request.complete(result);
        }
        request.wait()
    }

    pub fn get(&self, key: &[u8]) -> Result<Option<Bytes>> {
        self.get_opt(key, ReadOptions::default())
    }

    pub fn get_opt(&self, key: &[u8], opts: ReadOptions) -> Result<Option<Bytes>> {
        let (read_seq, fill_cache, l0_tables, level_tables) = {
            let state = self.read_state()?;
            if state.closed {
                return Err(Error::Closed);
            }
            let read_seq = opts
                .snapshot
                .as_ref()
                .map(|snapshot| snapshot.read_seq())
                .unwrap_or(state.last_sequence);

            if let Some(record) = state.mutable.get(key, read_seq) {
                return Ok(record.into_visible_value());
            }
            for memtable in state.immutables.iter().rev() {
                if let Some(record) = memtable.get(key, read_seq) {
                    return Ok(record.into_visible_value());
                }
            }

            (
                read_seq,
                opts.fill_cache,
                state.l0_tables.clone(),
                state.level_tables.clone(),
            )
        };

        for (meta, table) in &l0_tables {
            if !file_overlaps_user_key(meta, key) {
                continue;
            }
            if !table.might_contain(key) {
                self.inner.metrics.record_bloom_useful();
                continue;
            }
            if let Some(record) = table.get_with_cache(
                key,
                read_seq,
                meta.number,
                Some(&self.inner.block_cache),
                fill_cache,
            )? {
                return Ok(record.into_visible_value());
            } else {
                self.inner.metrics.record_bloom_false_positive();
            }
        }

        for level in level_tables.iter().skip(1) {
            let Some((meta, table)) = level
                .iter()
                .find(|(meta, _)| file_overlaps_user_key(meta, key))
            else {
                continue;
            };
            if !table.might_contain(key) {
                self.inner.metrics.record_bloom_useful();
                continue;
            }
            if let Some(record) = table.get_with_cache(
                key,
                read_seq,
                meta.number,
                Some(&self.inner.block_cache),
                fill_cache,
            )? {
                return Ok(record.into_visible_value());
            } else {
                self.inner.metrics.record_bloom_false_positive();
            }
        }
        Ok(None)
    }

    pub fn scan(&self, lower: Bound<&[u8]>, upper: Bound<&[u8]>) -> Result<Vec<(Bytes, Bytes)>> {
        self.scan_opt(lower, upper, ReadOptions::default())
    }

    pub fn scan_opt(
        &self,
        lower: Bound<&[u8]>,
        upper: Bound<&[u8]>,
        opts: ReadOptions,
    ) -> Result<Vec<(Bytes, Bytes)>> {
        let (read_seq, mem_entries, l0_tables, level_tables) = {
            let state = self.read_state()?;
            if state.closed {
                return Err(Error::Closed);
            }
            let read_seq = opts
                .snapshot
                .as_ref()
                .map(|snapshot| snapshot.read_seq())
                .unwrap_or(state.last_sequence);
            let mut entries = Vec::new();
            extend_memtable_entries(&mut entries, &state.mutable);
            for memtable in state.immutables.iter().rev() {
                extend_memtable_entries(&mut entries, memtable);
            }
            (
                read_seq,
                entries,
                state.l0_tables.clone(),
                state.level_tables.clone(),
            )
        };

        let mut children: Vec<Box<dyn StorageIterator>> = Vec::new();
        if !mem_entries.is_empty() {
            children.push(Box::new(EntryIterator::new(mem_entries)));
        }
        for (meta, table) in &l0_tables {
            children.push(Box::new(EntryIterator::new(table.entries_with_cache(
                meta.number,
                Some(&self.inner.block_cache),
                opts.fill_cache,
            )?)));
        }
        for level in &level_tables {
            for (meta, table) in level {
                children.push(Box::new(EntryIterator::new(table.entries_with_cache(
                    meta.number,
                    Some(&self.inner.block_cache),
                    opts.fill_cache,
                )?)));
            }
        }

        let mut iter = DBIterator::new(
            Box::new(MergeIterator::new(children)),
            bound_to_owned(lower),
            bound_to_owned(upper),
            read_seq,
        );
        iter.collect()
    }

    pub fn snapshot(&self) -> Snapshot {
        self.read_state()
            .map(|state| Snapshot::new(state.last_sequence, Arc::clone(&self.inner.watermark)))
            .unwrap_or_else(|_| Snapshot::new(0, Arc::clone(&self.inner.watermark)))
    }

    pub fn transaction(&self, opts: TransactionOptions) -> Result<Transaction> {
        let snapshot = {
            let state = self.read_state()?;
            if state.closed {
                return Err(Error::Closed);
            }
            Snapshot::new(state.last_sequence, Arc::clone(&self.inner.watermark))
        };
        Ok(Transaction::new(self.clone(), snapshot, opts))
    }

    pub(crate) fn commit_transaction(
        &self,
        read_seq: SequenceNumber,
        batch: WriteBatch,
        read_keys: &BTreeSet<Bytes>,
        read_ranges: &[ReadRange],
    ) -> Result<()> {
        let write_keys = batch_write_keys(&batch);
        let user_write_bytes = batch_user_write_bytes(&batch);
        self.inner.metrics.record_user_write(user_write_bytes);
        self.apply_write_rate_limit(user_write_bytes);
        self.apply_write_pressure()?;

        let should_flush = {
            let mut state = self.write_state()?;
            if state.closed {
                return Err(Error::Closed);
            }

            self.ensure_no_transaction_conflicts(
                &state,
                read_seq,
                read_keys,
                read_ranges,
                &write_keys,
            )?;
            if batch.is_empty() {
                return Ok(());
            }

            let start_sequence = state.last_sequence + 1;
            if self.inner.options.wal_enabled {
                let payload = batch.encode_with_sequence(start_sequence);
                self.inner
                    .metrics
                    .record_wal_write(wal_record_bytes(&payload));
                let mut wal = self.write_wal()?;
                wal.append(&payload)?;
                if self.inner.options.wal_sync == WalSyncMode::PerWrite {
                    wal.sync()?;
                    self.inner.metrics.record_wal_sync();
                }
            }

            apply_batch(&mut state, start_sequence, &batch);
            freeze_mutable_if_needed(
                &mut state,
                self.inner.options.memtable_size,
                self.inner.options.memtable_kind,
            )
        };

        if should_flush {
            self.flush()?;
        }

        Ok(())
    }

    pub fn flush(&self) -> Result<()> {
        let mut state = self.write_state()?;
        if state.closed {
            return Err(Error::Closed);
        }

        let Some((memtable, source)) =
            take_oldest_flushable_memtable(&mut state, self.inner.options.memtable_kind)
        else {
            return Ok(());
        };

        let file_number = self.allocate_file_number()?;
        match self.write_l0_table(&memtable, file_number) {
            Ok((reader, meta)) => {
                if let Err(err) = self.publish_l0_table(meta.clone(), state.last_sequence) {
                    restore_flush_memtable(&mut state, memtable, source);
                    return Err(err);
                }
                state.l0_tables.insert(0, (meta, reader));
                Ok(())
            }
            Err(err) => {
                restore_flush_memtable(&mut state, memtable, source);
                Err(err)
            }
        }
    }

    pub fn compact_range(&self, lower: Bound<&[u8]>, upper: Bound<&[u8]>) -> Result<()> {
        let task = {
            let versions = self.write_versions()?;
            let picker = CompactionPicker::new(self.inner.options.clone());
            picker.pick_manual(
                versions.current().as_ref(),
                clone_bound_ref(&lower),
                clone_bound_ref(&upper),
            )
        };

        if let Some(task) = task {
            self.execute_compaction(task)?;
        }
        Ok(())
    }

    pub fn sync_wal(&self) -> Result<()> {
        self.write_wal()?.sync()?;
        self.inner.metrics.record_wal_sync();
        Ok(())
    }

    pub fn block_cache_stats(&self) -> CacheStats {
        self.inner.block_cache.stats()
    }

    pub fn metrics_snapshot(&self) -> MetricsSnapshot {
        self.inner.metrics.snapshot(self.inner.block_cache.stats())
    }

    pub fn level_file_counts(&self) -> Vec<usize> {
        self.read_state()
            .map(|state| {
                let mut counts = Vec::with_capacity(state.level_tables.len().max(1));
                counts.push(state.l0_tables.len());
                for level in state.level_tables.iter().skip(1) {
                    counts.push(level.len());
                }
                counts
            })
            .unwrap_or_default()
    }

    fn read_state(&self) -> Result<std::sync::RwLockReadGuard<'_, DBState>> {
        self.inner
            .state
            .read()
            .map_err(|_| Error::Corruption("db state read lock poisoned".to_string()))
    }

    fn write_state(&self) -> Result<std::sync::RwLockWriteGuard<'_, DBState>> {
        self.inner
            .state
            .write()
            .map_err(|_| Error::Corruption("db state write lock poisoned".to_string()))
    }

    fn write_wal(&self) -> Result<std::sync::MutexGuard<'_, WalWriter>> {
        self.inner
            .wal
            .lock()
            .map_err(|_| Error::Corruption("wal lock poisoned".to_string()))
    }

    fn write_versions(&self) -> Result<std::sync::MutexGuard<'_, VersionSet>> {
        self.inner
            .versions
            .lock()
            .map_err(|_| Error::Corruption("version set lock poisoned".to_string()))
    }

    fn ensure_no_transaction_conflicts(
        &self,
        state: &DBState,
        read_seq: SequenceNumber,
        read_keys: &BTreeSet<Bytes>,
        read_ranges: &[ReadRange],
        write_keys: &BTreeSet<Bytes>,
    ) -> Result<()> {
        for key in read_keys {
            if self.key_changed_after(state, key, read_seq)? {
                return Err(Error::TransactionConflict(format!(
                    "read key changed after snapshot: {}",
                    String::from_utf8_lossy(key)
                )));
            }
        }

        for key in write_keys {
            if self.key_changed_after(state, key, read_seq)? {
                return Err(Error::TransactionConflict(format!(
                    "write key changed after snapshot: {}",
                    String::from_utf8_lossy(key)
                )));
            }
        }

        for (lower, upper) in read_ranges {
            if self.range_changed_after(state, lower, upper, read_seq)? {
                return Err(Error::TransactionConflict(
                    "scanned range changed after snapshot".to_string(),
                ));
            }
        }

        Ok(())
    }

    fn key_changed_after(
        &self,
        state: &DBState,
        user_key: &[u8],
        read_seq: SequenceNumber,
    ) -> Result<bool> {
        self.any_entry_after(state, read_seq, |key| key.user_key() == user_key)
    }

    fn range_changed_after(
        &self,
        state: &DBState,
        lower: &Bound<Bytes>,
        upper: &Bound<Bytes>,
        read_seq: SequenceNumber,
    ) -> Result<bool> {
        self.any_entry_after(state, read_seq, |key| {
            within_owned_bounds(key.user_key(), lower, upper)
        })
    }

    fn any_entry_after(
        &self,
        state: &DBState,
        read_seq: SequenceNumber,
        mut predicate: impl FnMut(&InternalKey) -> bool,
    ) -> Result<bool> {
        for (key, _) in state.mutable.entries() {
            if key.sequence() > read_seq && predicate(&key) {
                return Ok(true);
            }
        }
        for memtable in &state.immutables {
            for (key, _) in memtable.entries() {
                if key.sequence() > read_seq && predicate(&key) {
                    return Ok(true);
                }
            }
        }

        for (meta, table) in &state.l0_tables {
            if meta.largest_seq <= read_seq {
                continue;
            }
            for (key, _) in
                table.entries_with_cache(meta.number, Some(&self.inner.block_cache), true)?
            {
                if key.sequence() > read_seq && predicate(&key) {
                    return Ok(true);
                }
            }
        }
        for level in &state.level_tables {
            for (meta, table) in level {
                if meta.largest_seq <= read_seq {
                    continue;
                }
                for (key, _) in
                    table.entries_with_cache(meta.number, Some(&self.inner.block_cache), true)?
                {
                    if key.sequence() > read_seq && predicate(&key) {
                        return Ok(true);
                    }
                }
            }
        }

        Ok(false)
    }

    fn allocate_file_number(&self) -> Result<u64> {
        Ok(self.write_versions()?.allocate_file_number())
    }

    fn write_l0_table(
        &self,
        memtable: &MemTable,
        file_number: u64,
    ) -> Result<(Arc<SSTableReader>, FileMeta)> {
        let table_name = table_file_name(file_number);
        let tmp_path = self.inner.path.join(format!("{table_name}.tmp"));
        let final_path = self.inner.path.join(table_name);
        if self.inner.options.env.exists(&final_path) {
            return Err(Error::Corruption(format!(
                "table file already exists: {}",
                final_path.display()
            )));
        }
        let mut builder = SSTableBuilder::create_with_env(
            self.inner.options.env.as_ref(),
            &tmp_path,
            self.inner.options.block_size,
            self.inner.options.table_compression,
        )?;
        for (key, value) in memtable.entries() {
            builder.add(key, &value)?;
        }
        builder.finish()?;
        self.inner.options.env.rename(&tmp_path, &final_path)?;
        let file_size = self.inner.options.env.metadata_len(&final_path)?;
        self.inner.metrics.record_sst_write(file_size);
        let reader = self
            .inner
            .table_cache
            .get_or_open(file_number, &final_path)?;
        let meta = file_meta_from_table(file_number, file_size, &reader)?;
        Ok((reader, meta))
    }

    fn publish_l0_table(&self, meta: FileMeta, last_sequence: SequenceNumber) -> Result<()> {
        let new_wal = {
            let mut versions = self.write_versions()?;
            versions.log_and_apply(VersionEdit::AddFile { level: 0, meta })?;
            versions.log_and_apply(VersionEdit::LastSequence(last_sequence))?;

            let new_log_number = versions.allocate_file_number();
            let wal_path = self.inner.path.join(wal_file_name(new_log_number));
            let new_wal = WalWriter::create_with_env(self.inner.options.env.as_ref(), &wal_path)?;
            versions.log_and_apply(VersionEdit::LogNumber(new_log_number))?;
            new_wal
        };

        *self.write_wal()? = new_wal;
        Ok(())
    }

    fn apply_write_pressure(&self) -> Result<()> {
        let l0_count = self
            .read_state()
            .map(|state| state.l0_tables.len())
            .unwrap_or_default();

        if l0_count >= self.inner.options.level0_stop_writes_trigger {
            self.compact_range(Bound::Unbounded, Bound::Unbounded)?;
        } else if l0_count >= self.inner.options.level0_slowdown_writes_trigger {
            thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    }

    fn apply_write_rate_limit(&self, bytes: u64) {
        let Some(limiter) = &self.inner.write_rate_limiter else {
            return;
        };
        let wait = limiter.reserve(bytes);
        if !wait.is_zero() {
            thread::sleep(wait);
        }
    }

    fn apply_write_group(&self, requests: &[Arc<PendingWrite>]) -> Vec<Result<()>> {
        let mut items = requests
            .iter()
            .map(|request| {
                let batch = request.take_batch();
                let user_write_bytes = batch_user_write_bytes(&batch);
                GroupWriteItem {
                    batch,
                    opts: request.opts,
                    user_write_bytes,
                    start_sequence: 0,
                }
            })
            .collect::<Vec<_>>();

        let total_user_write_bytes = items.iter().map(|item| item.user_write_bytes).sum();
        self.inner.metrics.record_user_write(total_user_write_bytes);
        self.apply_write_rate_limit(total_user_write_bytes);
        if let Err(err) = self.apply_write_pressure() {
            return repeat_group_error(err, requests.len());
        }

        let should_flush = match self.apply_write_group_to_memtable(&mut items) {
            Ok(should_flush) => should_flush,
            Err(err) => return repeat_group_error(err, requests.len()),
        };

        if should_flush && let Err(err) = self.flush() {
            return repeat_group_error(err, requests.len());
        }

        (0..requests.len()).map(|_| Ok(())).collect()
    }

    fn apply_write_group_to_memtable(&self, items: &mut [GroupWriteItem]) -> Result<bool> {
        let mut state = self.write_state()?;
        if state.closed {
            return Err(Error::Closed);
        }

        let mut next_sequence = state.last_sequence + 1;
        for item in items.iter_mut() {
            item.start_sequence = next_sequence;
            next_sequence += item.batch.records().len() as u64;
        }

        let mut should_sync = false;
        if self.inner.options.wal_enabled {
            let mut wal = self.write_wal()?;
            for item in items.iter() {
                if item.opts.disable_wal {
                    continue;
                }
                let payload = item.batch.encode_with_sequence(item.start_sequence);
                self.inner
                    .metrics
                    .record_wal_write(wal_record_bytes(&payload));
                wal.append(&payload)?;
                should_sync |=
                    item.opts.sync || self.inner.options.wal_sync == WalSyncMode::PerWrite;
            }
            if should_sync {
                wal.sync()?;
                self.inner.metrics.record_wal_sync();
            }
        }

        let mut should_flush = false;
        for item in items {
            apply_batch(&mut state, item.start_sequence, &item.batch);
            should_flush |= freeze_mutable_if_needed(
                &mut state,
                self.inner.options.memtable_size,
                self.inner.options.memtable_kind,
            );
        }
        Ok(should_flush)
    }

    fn execute_compaction(&self, task: CompactionTask) -> Result<bool> {
        if task.input_files.is_empty() {
            return Ok(false);
        }

        let gc_watermark = self
            .inner
            .watermark
            .oldest()
            .unwrap_or(self.read_state()?.last_sequence);
        let drop_tombstones = self.can_drop_tombstones(&task)?;

        if task.is_trivial_move()
            && !self.trivial_move_needs_rewrite(&task, gc_watermark, drop_tombstones)?
        {
            let meta = task.input_files[0].clone();
            {
                let mut versions = self.write_versions()?;
                versions.log_and_apply(VersionEdit::DeleteFile {
                    level: task.input_level,
                    number: meta.number,
                })?;
                versions.log_and_apply(VersionEdit::AddFile {
                    level: task.output_level,
                    meta,
                })?;
            }
            self.refresh_tables_from_version()?;
            return Ok(true);
        }

        let mut entries = Vec::new();
        for file in task.all_input_files() {
            self.inner.metrics.record_compaction_read(file.file_size);
            let table = self.inner.table_cache.get_or_open(
                file.number,
                &self.inner.path.join(table_file_name(file.number)),
            )?;
            entries.extend(table.entries_with_cache(
                file.number,
                Some(&self.inner.block_cache),
                true,
            )?);
        }

        let output_entries = compact_entries(entries, gc_watermark, drop_tombstones);
        let outputs = self.write_compaction_outputs(output_entries)?;

        {
            let mut versions = self.write_versions()?;
            for file in task.all_input_files() {
                let level = if task
                    .input_files
                    .iter()
                    .any(|input| input.number == file.number)
                {
                    task.input_level
                } else {
                    task.output_level
                };
                versions.log_and_apply(VersionEdit::DeleteFile {
                    level,
                    number: file.number,
                })?;
            }
            for (_, meta) in &outputs {
                versions.log_and_apply(VersionEdit::AddFile {
                    level: task.output_level,
                    meta: meta.clone(),
                })?;
            }
        }

        self.refresh_tables_from_version()?;
        self.delete_obsolete_files(task.all_input_files().map(|file| file.number))?;
        Ok(true)
    }

    fn trivial_move_needs_rewrite(
        &self,
        task: &CompactionTask,
        gc_watermark: SequenceNumber,
        drop_tombstones: bool,
    ) -> Result<bool> {
        let file = &task.input_files[0];
        let table = self.inner.table_cache.get_or_open(
            file.number,
            &self.inner.path.join(table_file_name(file.number)),
        )?;
        let entries = table.entries_with_cache(file.number, Some(&self.inner.block_cache), true)?;
        let compacted_entries = compact_entries(entries.clone(), gc_watermark, drop_tombstones);
        Ok(compacted_entries.len() != entries.len())
    }

    fn can_drop_tombstones(&self, task: &CompactionTask) -> Result<bool> {
        let versions = self.write_versions()?;
        let current = versions.current();
        Ok(!current
            .levels
            .iter()
            .skip(task.output_level + 1)
            .flatten()
            .any(|file| {
                file.user_key_overlaps_range(
                    task.smallest_user_key.as_slice(),
                    task.largest_user_key.as_slice(),
                )
            }))
    }

    fn write_compaction_outputs(
        &self,
        entries: Vec<(InternalKey, ValueRecord)>,
    ) -> Result<Vec<(Arc<SSTableReader>, FileMeta)>> {
        if entries.is_empty() {
            return Ok(Vec::new());
        }

        let chunks = split_compaction_entries(
            entries,
            self.inner.options.target_file_size_base,
            self.inner.options.max_subcompactions,
        );
        self.inner
            .metrics
            .record_subcompaction_tasks(chunks.len() as u64);

        if chunks.len() == 1 {
            let number = self.allocate_file_number()?;
            return Ok(vec![self.write_compaction_output(
                number,
                chunks.into_iter().next().expect("one chunk"),
            )?]);
        }

        thread::scope(|scope| {
            let mut handles = Vec::with_capacity(chunks.len());
            for chunk in chunks {
                handles.push(scope.spawn(move || {
                    let number = self.allocate_file_number()?;
                    self.write_compaction_output(number, chunk)
                }));
            }

            let mut outputs = Vec::with_capacity(handles.len());
            for handle in handles {
                let output = handle.join().map_err(|_| {
                    Error::Corruption("subcompaction worker panicked".to_string())
                })??;
                outputs.push(output);
            }
            Ok(outputs)
        })
    }

    fn write_compaction_output(
        &self,
        file_number: u64,
        entries: Vec<(InternalKey, ValueRecord)>,
    ) -> Result<(Arc<SSTableReader>, FileMeta)> {
        let table_name = table_file_name(file_number);
        let tmp_path = self.inner.path.join(format!("{table_name}.tmp"));
        let final_path = self.inner.path.join(table_name);
        let mut builder = SSTableBuilder::create_with_env(
            self.inner.options.env.as_ref(),
            &tmp_path,
            self.inner.options.block_size,
            self.inner.options.table_compression,
        )?;
        for (key, value) in entries {
            builder.add(key, &value)?;
        }
        builder.finish()?;
        self.inner.options.env.rename(&tmp_path, &final_path)?;
        let file_size = self.inner.options.env.metadata_len(&final_path)?;
        self.inner.metrics.record_sst_write(file_size);
        self.inner.metrics.record_compaction_write(file_size);
        let reader = self
            .inner
            .table_cache
            .get_or_open(file_number, &final_path)?;
        let meta = file_meta_from_table(file_number, file_size, &reader)?;
        Ok((reader, meta))
    }

    fn refresh_tables_from_version(&self) -> Result<()> {
        let current = self.write_versions()?.current();
        let (l0_tables, level_tables) =
            open_version_tables(&self.inner.path, current.as_ref(), &self.inner.table_cache)?;
        let mut state = self.write_state()?;
        state.l0_tables = l0_tables;
        state.level_tables = level_tables;
        Ok(())
    }

    fn delete_obsolete_files(&self, numbers: impl IntoIterator<Item = u64>) -> Result<()> {
        for number in numbers {
            let path = self.inner.path.join(table_file_name(number));
            match self.inner.options.env.remove_file(&path) {
                Ok(()) => {}
                Err(Error::Io(err)) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(err),
            }
        }
        Ok(())
    }
}

trait IntoVisibleValue {
    fn into_visible_value(self) -> Option<Bytes>;
}

impl IntoVisibleValue for ValueRecord {
    fn into_visible_value(self) -> Option<Bytes> {
        match self {
            ValueRecord::Put(value) => Some(value),
            ValueRecord::Delete => None,
        }
    }
}

fn freeze_mutable_if_needed(
    state: &mut DBState,
    memtable_size: usize,
    memtable_kind: MemTableKind,
) -> bool {
    if state.mutable.is_empty() || state.mutable.approximate_size() < memtable_size {
        return false;
    }

    let frozen = mem::replace(&mut state.mutable, MemTable::new(memtable_kind));
    state.immutables.push(frozen);
    true
}

fn take_oldest_flushable_memtable(
    state: &mut DBState,
    memtable_kind: MemTableKind,
) -> Option<(MemTable, FlushSource)> {
    if !state.immutables.is_empty() {
        return Some((state.immutables.remove(0), FlushSource::ImmutableOldest));
    }
    if state.mutable.is_empty() {
        return None;
    }
    Some((
        mem::replace(&mut state.mutable, MemTable::new(memtable_kind)),
        FlushSource::Mutable,
    ))
}

fn restore_flush_memtable(state: &mut DBState, memtable: MemTable, source: FlushSource) {
    match source {
        FlushSource::Mutable => {
            if state.mutable.is_empty() {
                state.mutable = memtable;
            } else {
                state.immutables.push(memtable);
            }
        }
        FlushSource::ImmutableOldest => {
            state.immutables.insert(0, memtable);
        }
    }
}

fn extend_memtable_entries(entries: &mut Vec<(InternalKey, ValueRecord)>, memtable: &MemTable) {
    entries.extend(memtable.entries());
}

fn recover_wal(env: &dyn crate::env::Env, path: &Path, state: &mut DBState) -> Result<()> {
    let mut reader = WalReader::open_with_env(env, path)?;
    while let Some(payload) = reader.read_record()? {
        let (start_sequence, batch) = WriteBatch::decode_payload(&payload)?;
        apply_batch(state, start_sequence, &batch);
    }
    Ok(())
}

fn open_version_tables(
    db_path: &Path,
    version: &Version,
    table_cache: &TableCache,
) -> Result<(Vec<TableRef>, Vec<Vec<TableRef>>)> {
    let mut l0_tables = Vec::with_capacity(version.l0_files.len());
    for file in &version.l0_files {
        l0_tables.push((
            file.clone(),
            table_cache.get_or_open(file.number, &db_path.join(table_file_name(file.number)))?,
        ));
    }

    let mut level_tables = Vec::with_capacity(version.levels.len());
    for level in &version.levels {
        let mut tables = Vec::with_capacity(level.len());
        for file in level {
            tables.push((
                file.clone(),
                table_cache
                    .get_or_open(file.number, &db_path.join(table_file_name(file.number)))?,
            ));
        }
        level_tables.push(tables);
    }

    Ok((l0_tables, level_tables))
}

fn file_meta_from_table(number: u64, file_size: u64, reader: &SSTableReader) -> Result<FileMeta> {
    let entries = reader.entries()?;
    let (smallest_seq, largest_seq) = entries
        .iter()
        .map(|(key, _)| key.sequence())
        .fold(None, |range: Option<(u64, u64)>, sequence| match range {
            Some((smallest, largest)) => Some((smallest.min(sequence), largest.max(sequence))),
            None => Some((sequence, sequence)),
        })
        .ok_or_else(|| Error::Corruption("empty SSTable has no sequence range".to_string()))?;

    Ok(FileMeta {
        number,
        file_size,
        smallest: reader
            .smallest_key()
            .cloned()
            .ok_or_else(|| Error::Corruption("empty SSTable has no smallest key".to_string()))?,
        largest: reader
            .largest_key()
            .cloned()
            .ok_or_else(|| Error::Corruption("empty SSTable has no largest key".to_string()))?,
        smallest_seq,
        largest_seq,
    })
}

fn file_overlaps_user_key(meta: &FileMeta, user_key: &[u8]) -> bool {
    user_key >= meta.smallest.user_key() && user_key <= meta.largest.user_key()
}

trait FileMetaUserKeyOverlap {
    fn user_key_overlaps_range(&self, smallest: &[u8], largest: &[u8]) -> bool;
}

impl FileMetaUserKeyOverlap for FileMeta {
    fn user_key_overlaps_range(&self, smallest: &[u8], largest: &[u8]) -> bool {
        self.smallest.user_key() <= largest && self.largest.user_key() >= smallest
    }
}

fn bound_to_owned(bound: Bound<&[u8]>) -> Bound<Bytes> {
    match bound {
        Bound::Included(value) => Bound::Included(value.to_vec()),
        Bound::Excluded(value) => Bound::Excluded(value.to_vec()),
        Bound::Unbounded => Bound::Unbounded,
    }
}

fn clone_bound_ref<'a>(bound: &Bound<&'a [u8]>) -> Bound<&'a [u8]> {
    match bound {
        Bound::Included(value) => Bound::Included(*value),
        Bound::Excluded(value) => Bound::Excluded(*value),
        Bound::Unbounded => Bound::Unbounded,
    }
}

fn within_owned_bounds(key: &[u8], lower: &Bound<Bytes>, upper: &Bound<Bytes>) -> bool {
    let lower_ok = match lower {
        Bound::Included(bound) => key >= bound.as_slice(),
        Bound::Excluded(bound) => key > bound.as_slice(),
        Bound::Unbounded => true,
    };
    let upper_ok = match upper {
        Bound::Included(bound) => key <= bound.as_slice(),
        Bound::Excluded(bound) => key < bound.as_slice(),
        Bound::Unbounded => true,
    };
    lower_ok && upper_ok
}

fn batch_write_keys(batch: &WriteBatch) -> BTreeSet<Bytes> {
    batch
        .records()
        .iter()
        .map(|record| match record {
            BatchRecord::Put { key, .. } | BatchRecord::Delete { key } => key.clone(),
        })
        .collect()
}

fn batch_user_write_bytes(batch: &WriteBatch) -> u64 {
    batch
        .records()
        .iter()
        .map(|record| match record {
            BatchRecord::Put { key, value } => key.len() + value.len(),
            BatchRecord::Delete { key } => key.len(),
        } as u64)
        .sum()
}

fn wal_record_bytes(payload: &[u8]) -> u64 {
    payload.len() as u64 + 9
}

fn repeat_group_error(err: Error, count: usize) -> Vec<Result<()>> {
    match err {
        Error::Closed => (0..count).map(|_| Err(Error::Closed)).collect(),
        Error::InvalidArgument(message) => (0..count)
            .map(|_| Err(Error::InvalidArgument(message.clone())))
            .collect(),
        Error::Corruption(message) => (0..count)
            .map(|_| Err(Error::Corruption(message.clone())))
            .collect(),
        Error::Unsupported(feature) => (0..count)
            .map(|_| Err(Error::Unsupported(feature)))
            .collect(),
        Error::TransactionConflict(message) => (0..count)
            .map(|_| Err(Error::TransactionConflict(message.clone())))
            .collect(),
        Error::Io(err) => {
            let message = err.to_string();
            (0..count)
                .map(|_| Err(Error::Corruption(message.clone())))
                .collect()
        }
    }
}

fn split_compaction_entries(
    entries: Vec<(InternalKey, ValueRecord)>,
    target_file_size: usize,
    max_outputs: usize,
) -> Vec<Vec<(InternalKey, ValueRecord)>> {
    let max_outputs = max_outputs.max(1);
    let target_file_size = target_file_size.max(1);
    let mut chunks = Vec::new();
    let mut current = Vec::new();
    let mut current_size = 0;
    let mut previous_user_key = Vec::new();

    for entry in entries {
        let entry_size = estimate_entry_size(&entry);
        let user_key_changed =
            previous_user_key.is_empty() || previous_user_key.as_slice() != entry.0.user_key();
        if !current.is_empty()
            && user_key_changed
            && current_size >= target_file_size
            && chunks.len() + 1 < max_outputs
        {
            chunks.push(current);
            current = Vec::new();
            current_size = 0;
        }
        previous_user_key = entry.0.user_key().to_vec();
        current_size += entry_size;
        current.push(entry);
    }

    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn estimate_entry_size((key, value): &(InternalKey, ValueRecord)) -> usize {
    key.user_key().len()
        + std::mem::size_of::<InternalKey>()
        + match value {
            ValueRecord::Put(value) => value.len(),
            ValueRecord::Delete => 0,
        }
}

fn apply_batch(state: &mut DBState, start_sequence: SequenceNumber, batch: &WriteBatch) {
    for (sequence, record) in (start_sequence..).zip(batch.records()) {
        match record {
            BatchRecord::Put { key, value } => {
                state.mutable.put(sequence, key.clone(), value.clone());
            }
            BatchRecord::Delete { key } => {
                state.mutable.delete(sequence, key.clone());
            }
        }
        state.last_sequence = state.last_sequence.max(sequence);
    }
}
