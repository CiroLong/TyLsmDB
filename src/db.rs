use std::mem;
use std::ops::Bound;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use crate::batch::{BatchRecord, WriteBatch};
use crate::bytes::Bytes;
use crate::cache::{BlockCache, CacheStats, TableCache};
use crate::env::file::{table_file_name, wal_file_name};
use crate::error::{Error, Result};
use crate::iterator::{DBIterator, EntryIterator, MergeIterator, StorageIterator};
use crate::key::{InternalKey, SequenceNumber};
use crate::memtable::{MemTable, ValueRecord};
use crate::options::{Options, ReadOptions, WalSyncMode, WriteOptions};
use crate::snapshot::Snapshot;
use crate::table::{SSTableBuilder, SSTableReader};
use crate::transaction::{Transaction, TransactionOptions};
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

impl DB {
    pub fn open(path: impl AsRef<Path>, options: Options) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if path.exists() && options.error_if_exists {
            return Err(Error::InvalidArgument(format!(
                "database already exists: {}",
                path.display()
            )));
        }
        if !path.exists() && !options.create_if_missing {
            return Err(Error::InvalidArgument(format!(
                "database does not exist: {}",
                path.display()
            )));
        }
        std::fs::create_dir_all(&path)?;

        let options_for_versions = options.clone();
        let versions = if path.join("CURRENT").exists() {
            VersionSet::recover(&path, options_for_versions)?
        } else {
            VersionSet::create(&path, options_for_versions)?
        };
        let block_cache = BlockCache::new(options.block_cache_capacity);
        let table_cache = TableCache::new(512);
        let (l0_tables, level_tables) =
            open_version_tables(&path, versions.current().as_ref(), &table_cache)?;
        let mut state = DBState {
            mutable: MemTable::new(),
            immutables: Vec::new(),
            l0_tables,
            level_tables,
            last_sequence: versions.last_sequence(),
            closed: false,
        };
        let wal_path = path.join(wal_file_name(versions.log_number()));
        if wal_path.exists() {
            recover_wal(&wal_path, &mut state)?;
        }
        let wal = WalWriter::create(&wal_path)?;

        Ok(Self {
            inner: Arc::new(DBInner {
                path,
                options,
                state: RwLock::new(state),
                versions: Mutex::new(versions),
                block_cache,
                table_cache,
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

        let should_flush = {
            let mut state = self.write_state()?;
            if state.closed {
                return Err(Error::Closed);
            }

            let start_sequence = state.last_sequence + 1;
            if self.inner.options.wal_enabled && !opts.disable_wal {
                let payload = batch.encode_with_sequence(start_sequence);
                let mut wal = self.write_wal()?;
                wal.append(&payload)?;
                if opts.sync || self.inner.options.wal_sync == WalSyncMode::PerWrite {
                    wal.sync()?;
                }
            }

            apply_batch(&mut state, start_sequence, &batch);
            freeze_mutable_if_needed(&mut state, self.inner.options.memtable_size)
        };

        if should_flush {
            self.flush()?;
        }

        Ok(())
    }

    pub fn get(&self, key: &[u8]) -> Result<Option<Bytes>> {
        self.get_opt(key, ReadOptions::default())
    }

    pub fn get_opt(&self, key: &[u8], opts: ReadOptions) -> Result<Option<Bytes>> {
        let (read_seq, l0_tables, level_tables) = {
            let state = self.read_state()?;
            if state.closed {
                return Err(Error::Closed);
            }

            if let Some(record) = state.mutable.get(key, state.last_sequence) {
                return Ok(record.into_visible_value());
            }
            for memtable in state.immutables.iter().rev() {
                if let Some(record) = memtable.get(key, state.last_sequence) {
                    return Ok(record.into_visible_value());
                }
            }

            (
                state.last_sequence,
                state.l0_tables.clone(),
                state.level_tables.clone(),
            )
        };

        for (meta, table) in &l0_tables {
            if !file_overlaps_user_key(meta, key) {
                continue;
            }
            if let Some(record) = table.get_with_cache(
                key,
                read_seq,
                meta.number,
                Some(&self.inner.block_cache),
                opts.fill_cache,
            )? {
                return Ok(record.into_visible_value());
            }
        }

        for level in level_tables.iter().skip(1) {
            let Some((meta, table)) = level
                .iter()
                .find(|(meta, _)| file_overlaps_user_key(meta, key))
            else {
                continue;
            };
            if let Some(record) = table.get_with_cache(
                key,
                read_seq,
                meta.number,
                Some(&self.inner.block_cache),
                opts.fill_cache,
            )? {
                return Ok(record.into_visible_value());
            }
        }
        Ok(None)
    }

    pub fn scan(&self, lower: Bound<&[u8]>, upper: Bound<&[u8]>) -> Result<Vec<(Bytes, Bytes)>> {
        let (read_seq, mem_entries, l0_tables, level_tables) = {
            let state = self.read_state()?;
            if state.closed {
                return Err(Error::Closed);
            }
            let mut entries = Vec::new();
            extend_memtable_entries(&mut entries, &state.mutable);
            for memtable in state.immutables.iter().rev() {
                extend_memtable_entries(&mut entries, memtable);
            }
            (
                state.last_sequence,
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
            children.push(Box::new(EntryIterator::new(
                table.entries_with_cache(meta.number, Some(&self.inner.block_cache))?,
            )));
        }
        for level in &level_tables {
            for (meta, table) in level {
                children.push(Box::new(EntryIterator::new(
                    table.entries_with_cache(meta.number, Some(&self.inner.block_cache))?,
                )));
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
            .map(|state| Snapshot::new(state.last_sequence))
            .unwrap_or_else(|_| Snapshot::new(0))
    }

    pub fn transaction(&self, _opts: TransactionOptions) -> Result<Transaction> {
        Err(Error::Unsupported("transactions arrive in V7"))
    }

    pub fn flush(&self) -> Result<()> {
        let mut state = self.write_state()?;
        if state.closed {
            return Err(Error::Closed);
        }

        let Some((memtable, source)) = take_oldest_flushable_memtable(&mut state) else {
            return Ok(());
        };

        let file_number = self.allocate_file_number()?;
        match self.write_l0_table(&memtable, file_number) {
            Ok((reader, meta)) => {
                self.publish_l0_table(meta.clone(), state.last_sequence)?;
                state.l0_tables.insert(0, (meta, reader));
                Ok(())
            }
            Err(err) => {
                restore_flush_memtable(&mut state, memtable, source);
                Err(err)
            }
        }
    }

    pub fn compact_range(&self, _lower: Bound<&[u8]>, _upper: Bound<&[u8]>) -> Result<()> {
        Err(Error::Unsupported("compaction arrives in V6"))
    }

    pub fn sync_wal(&self) -> Result<()> {
        self.write_wal()?.sync()
    }

    pub fn block_cache_stats(&self) -> CacheStats {
        self.inner.block_cache.stats()
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
        if final_path.exists() {
            return Err(Error::Corruption(format!(
                "table file already exists: {}",
                final_path.display()
            )));
        }
        let mut builder = SSTableBuilder::create(&tmp_path, self.inner.options.block_size)?;
        for (key, value) in memtable.entries() {
            builder.add(key.clone(), value)?;
        }
        builder.finish()?;
        std::fs::rename(&tmp_path, &final_path)?;
        let file_size = std::fs::metadata(&final_path)?.len();
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
            let new_wal = WalWriter::create(&wal_path)?;
            versions.log_and_apply(VersionEdit::LogNumber(new_log_number))?;
            new_wal
        };

        *self.write_wal()? = new_wal;
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

fn freeze_mutable_if_needed(state: &mut DBState, memtable_size: usize) -> bool {
    if state.mutable.is_empty() || state.mutable.approximate_size() < memtable_size {
        return false;
    }

    state.immutables.push(mem::take(&mut state.mutable));
    true
}

fn take_oldest_flushable_memtable(state: &mut DBState) -> Option<(MemTable, FlushSource)> {
    if !state.immutables.is_empty() {
        return Some((state.immutables.remove(0), FlushSource::ImmutableOldest));
    }
    if state.mutable.is_empty() {
        return None;
    }
    Some((mem::take(&mut state.mutable), FlushSource::Mutable))
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
    entries.extend(
        memtable
            .entries()
            .map(|(key, value)| (key.clone(), value.clone())),
    );
}

fn recover_wal(path: &Path, state: &mut DBState) -> Result<()> {
    let mut reader = WalReader::open(path)?;
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

fn bound_to_owned(bound: Bound<&[u8]>) -> Bound<Bytes> {
    match bound {
        Bound::Included(value) => Bound::Included(value.to_vec()),
        Bound::Excluded(value) => Bound::Excluded(value.to_vec()),
        Bound::Unbounded => Bound::Unbounded,
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
