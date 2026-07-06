use std::ops::Bound;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use crate::batch::{BatchRecord, WriteBatch};
use crate::bytes::Bytes;
use crate::error::{Error, Result};
use crate::key::SequenceNumber;
use crate::memtable::{MemTable, ValueRecord};
use crate::options::{Options, ReadOptions, WriteOptions};
use crate::snapshot::Snapshot;
use crate::transaction::{Transaction, TransactionOptions};

#[derive(Debug, Clone)]
pub struct DB {
    inner: Arc<DBInner>,
}

#[derive(Debug)]
struct DBInner {
    path: PathBuf,
    options: Options,
    state: RwLock<DBState>,
}

#[derive(Debug)]
struct DBState {
    mutable: MemTable,
    last_sequence: SequenceNumber,
    closed: bool,
}

impl DB {
    pub fn open(path: impl AsRef<Path>, options: Options) -> Result<Self> {
        Ok(Self {
            inner: Arc::new(DBInner {
                path: path.as_ref().to_path_buf(),
                options,
                state: RwLock::new(DBState {
                    mutable: MemTable::new(),
                    last_sequence: 0,
                    closed: false,
                }),
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

    pub fn write(&self, batch: WriteBatch, _opts: WriteOptions) -> Result<()> {
        if batch.is_empty() {
            return Ok(());
        }

        let mut state = self.write_state()?;
        if state.closed {
            return Err(Error::Closed);
        }

        let mut sequence = state.last_sequence + 1;
        for record in batch.records() {
            match record {
                BatchRecord::Put { key, value } => {
                    state.mutable.put(sequence, key.clone(), value.clone());
                }
                BatchRecord::Delete { key } => {
                    state.mutable.delete(sequence, key.clone());
                }
            }
            sequence += 1;
        }
        state.last_sequence = sequence - 1;
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

        Ok(match state.mutable.get(key, state.last_sequence) {
            Some(ValueRecord::Put(value)) => Some(value),
            Some(ValueRecord::Delete) | None => None,
        })
    }

    pub fn scan(&self, lower: Bound<&[u8]>, upper: Bound<&[u8]>) -> Result<Vec<(Bytes, Bytes)>> {
        let state = self.read_state()?;
        if state.closed {
            return Err(Error::Closed);
        }
        Ok(state.mutable.scan(lower, upper, state.last_sequence))
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
        Err(Error::Unsupported("flush arrives in V3"))
    }

    pub fn compact_range(&self, _lower: Bound<&[u8]>, _upper: Bound<&[u8]>) -> Result<()> {
        Err(Error::Unsupported("compaction arrives in V6"))
    }

    pub fn sync_wal(&self) -> Result<()> {
        Err(Error::Unsupported("WAL arrives in V2"))
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
}
