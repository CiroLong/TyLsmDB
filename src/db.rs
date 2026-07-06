use std::collections::BTreeSet;
use std::mem;
use std::ops::Bound;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use crate::batch::{BatchRecord, WriteBatch};
use crate::bytes::Bytes;
use crate::error::{Error, Result};
use crate::key::{InternalKey, SequenceNumber};
use crate::memtable::{MemTable, ValueRecord};
use crate::options::{Options, ReadOptions, WalSyncMode, WriteOptions};
use crate::snapshot::Snapshot;
use crate::table::{SSTableBuilder, SSTableReader};
use crate::transaction::{Transaction, TransactionOptions};
use crate::wal::{WalReader, WalWriter};

const ACTIVE_WAL_FILE: &str = "000001.wal";

#[derive(Debug, Clone)]
pub struct DB {
    inner: Arc<DBInner>,
}

#[derive(Debug)]
struct DBInner {
    path: PathBuf,
    options: Options,
    state: RwLock<DBState>,
    wal: Mutex<WalWriter>,
}

#[derive(Debug)]
struct DBState {
    mutable: MemTable,
    immutables: Vec<MemTable>,
    l0_tables: Vec<Arc<SSTableReader>>,
    next_file_number: u64,
    last_sequence: SequenceNumber,
    closed: bool,
}

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

        let wal_path = path.join(ACTIVE_WAL_FILE);
        let mut state = DBState {
            mutable: MemTable::new(),
            immutables: Vec::new(),
            l0_tables: Vec::new(),
            next_file_number: next_table_file_number(&path)?,
            last_sequence: 0,
            closed: false,
        };
        if wal_path.exists() {
            recover_wal(&wal_path, &mut state)?;
        }
        let wal = WalWriter::create(&wal_path)?;

        Ok(Self {
            inner: Arc::new(DBInner {
                path,
                options,
                state: RwLock::new(state),
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

    pub fn get_opt(&self, key: &[u8], _opts: ReadOptions) -> Result<Option<Bytes>> {
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
        for table in &state.l0_tables {
            if let Some(record) = table.get(key, state.last_sequence)? {
                return Ok(record.into_visible_value());
            }
        }
        Ok(None)
    }

    pub fn scan(&self, lower: Bound<&[u8]>, upper: Bound<&[u8]>) -> Result<Vec<(Bytes, Bytes)>> {
        let state = self.read_state()?;
        if state.closed {
            return Err(Error::Closed);
        }
        let mut entries = Vec::new();
        extend_memtable_entries(&mut entries, &state.mutable);
        for memtable in state.immutables.iter().rev() {
            extend_memtable_entries(&mut entries, memtable);
        }
        for table in &state.l0_tables {
            entries.extend(table.entries()?);
        }
        Ok(visible_rows(entries, lower, upper, state.last_sequence))
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
        let Some((memtable, source, file_number)) = self.take_flush_memtable()? else {
            return Ok(());
        };

        match self.write_l0_table(&memtable, file_number) {
            Ok(reader) => {
                let mut state = self.write_state()?;
                state.l0_tables.insert(0, Arc::new(reader));
                Ok(())
            }
            Err(err) => {
                self.restore_flush_memtable(memtable, source)?;
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

    fn take_flush_memtable(&self) -> Result<Option<(MemTable, FlushSource, u64)>> {
        let mut state = self.write_state()?;
        if state.closed {
            return Err(Error::Closed);
        }

        let Some((memtable, source)) = take_oldest_flushable_memtable(&mut state) else {
            return Ok(None);
        };
        let file_number = state.next_file_number;
        state.next_file_number += 1;
        Ok(Some((memtable, source, file_number)))
    }

    fn restore_flush_memtable(&self, memtable: MemTable, source: FlushSource) -> Result<()> {
        let mut state = self.write_state()?;
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
        Ok(())
    }

    fn write_l0_table(&self, memtable: &MemTable, file_number: u64) -> Result<SSTableReader> {
        let tmp_path = self.inner.path.join(format!("{file_number:06}.sst.tmp"));
        let final_path = self.inner.path.join(format!("{file_number:06}.sst"));
        let mut builder = SSTableBuilder::create(&tmp_path, self.inner.options.block_size)?;
        for (key, value) in memtable.entries() {
            builder.add(key.clone(), value)?;
        }
        builder.finish()?;
        std::fs::rename(&tmp_path, &final_path)?;
        SSTableReader::open(&final_path)
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

fn extend_memtable_entries(entries: &mut Vec<(InternalKey, ValueRecord)>, memtable: &MemTable) {
    entries.extend(
        memtable
            .entries()
            .map(|(key, value)| (key.clone(), value.clone())),
    );
}

fn visible_rows(
    mut entries: Vec<(InternalKey, ValueRecord)>,
    lower: Bound<&[u8]>,
    upper: Bound<&[u8]>,
    read_seq: SequenceNumber,
) -> Vec<(Bytes, Bytes)> {
    entries.sort_by(|(left_key, _), (right_key, _)| left_key.cmp(right_key));

    let mut seen = BTreeSet::new();
    let mut rows = Vec::new();
    for (internal_key, value) in entries {
        if internal_key.sequence() > read_seq {
            continue;
        }
        let user_key = internal_key.user_key();
        if !within_bounds(user_key, &lower, &upper) {
            continue;
        }

        let user_key_vec = user_key.to_vec();
        if !seen.insert(user_key_vec.clone()) {
            continue;
        }
        if let ValueRecord::Put(value) = value {
            rows.push((user_key_vec, value));
        }
    }

    rows
}

fn within_bounds(key: &[u8], lower: &Bound<&[u8]>, upper: &Bound<&[u8]>) -> bool {
    let lower_ok = match lower {
        Bound::Included(bound) => key >= *bound,
        Bound::Excluded(bound) => key > *bound,
        Bound::Unbounded => true,
    };
    let upper_ok = match upper {
        Bound::Included(bound) => key <= *bound,
        Bound::Excluded(bound) => key < *bound,
        Bound::Unbounded => true,
    };
    lower_ok && upper_ok
}

fn next_table_file_number(path: &Path) -> Result<u64> {
    if !path.exists() {
        return Ok(2);
    }

    let mut next = 2;
    for entry in std::fs::read_dir(path)? {
        let path = entry?.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("sst") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if let Ok(number) = stem.parse::<u64>() {
            next = next.max(number + 1);
        }
    }
    Ok(next)
}

fn recover_wal(path: &Path, state: &mut DBState) -> Result<()> {
    let mut reader = WalReader::open(path)?;
    while let Some(payload) = reader.read_record()? {
        let (start_sequence, batch) = WriteBatch::decode_payload(&payload)?;
        apply_batch(state, start_sequence, &batch);
    }
    Ok(())
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
