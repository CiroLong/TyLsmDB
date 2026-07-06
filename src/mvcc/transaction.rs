use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound;

use crate::batch::{BatchRecord, WriteBatch};
use crate::bytes::Bytes;
use crate::db::DB;
use crate::error::{Error, Result};
use crate::memtable::ValueRecord;
use crate::options::ReadOptions;

use super::conflict::ReadRange;
use super::snapshot::Snapshot;

#[derive(Debug, Clone, Default)]
pub struct TransactionOptions {
    pub read_only: bool,
}

#[derive(Debug)]
pub struct Transaction {
    db: DB,
    read_seq: u64,
    snapshot: Snapshot,
    opts: TransactionOptions,
    writes: WriteBatch,
    read_keys: BTreeSet<Bytes>,
    read_ranges: Vec<ReadRange>,
    closed: bool,
}

impl Transaction {
    pub(crate) fn new(db: DB, snapshot: Snapshot, opts: TransactionOptions) -> Self {
        let read_seq = snapshot.read_seq();
        Self {
            db,
            read_seq,
            snapshot,
            opts,
            writes: WriteBatch::new(),
            read_keys: BTreeSet::new(),
            read_ranges: Vec::new(),
            closed: false,
        }
    }

    pub fn read_seq(&self) -> u64 {
        self.read_seq
    }

    pub fn get(&mut self, key: &[u8]) -> Result<Option<Bytes>> {
        self.ensure_open()?;
        if let Some(value) = self.local_value(key) {
            return Ok(value.into_visible_value());
        }

        self.read_keys.insert(key.to_vec());
        self.db.get_opt(
            key,
            ReadOptions {
                snapshot: Some(self.snapshot.clone()),
                ..ReadOptions::default()
            },
        )
    }

    pub fn scan(
        &mut self,
        lower: Bound<&[u8]>,
        upper: Bound<&[u8]>,
    ) -> Result<Vec<(Bytes, Bytes)>> {
        self.ensure_open()?;
        let lower_owned = bound_to_owned(lower);
        let upper_owned = bound_to_owned(upper);
        self.read_ranges
            .push((clone_bound(&lower_owned), clone_bound(&upper_owned)));

        let mut rows: BTreeMap<Bytes, Bytes> = self
            .db
            .scan_opt(
                clone_bound_as_ref(&lower_owned),
                clone_bound_as_ref(&upper_owned),
                ReadOptions {
                    snapshot: Some(self.snapshot.clone()),
                    ..ReadOptions::default()
                },
            )?
            .into_iter()
            .collect();

        for record in self.writes.records() {
            match record {
                BatchRecord::Put { key, value } => {
                    if within_bounds(key, &lower_owned, &upper_owned) {
                        rows.insert(key.clone(), value.clone());
                    }
                }
                BatchRecord::Delete { key } => {
                    if within_bounds(key, &lower_owned, &upper_owned) {
                        rows.remove(key);
                    }
                }
            }
        }

        Ok(rows.into_iter().collect())
    }

    pub fn put(&mut self, key: &[u8], value: &[u8]) -> Result<()> {
        self.ensure_open()?;
        self.ensure_writable()?;
        self.writes.put(key.to_vec(), value.to_vec());
        Ok(())
    }

    pub fn delete(&mut self, key: &[u8]) -> Result<()> {
        self.ensure_open()?;
        self.ensure_writable()?;
        self.writes.delete(key.to_vec());
        Ok(())
    }

    pub fn commit(mut self) -> Result<()> {
        self.ensure_open()?;
        self.closed = true;
        self.db.commit_transaction(
            self.read_seq,
            self.writes.clone(),
            &self.read_keys,
            &self.read_ranges,
        )
    }

    pub fn rollback(mut self) -> Result<()> {
        self.ensure_open()?;
        self.closed = true;
        Ok(())
    }

    fn local_value(&self, key: &[u8]) -> Option<ValueRecord> {
        self.writes
            .records()
            .iter()
            .rev()
            .find_map(|record| match record {
                BatchRecord::Put {
                    key: record_key,
                    value,
                } if record_key.as_slice() == key => Some(ValueRecord::Put(value.clone())),
                BatchRecord::Delete { key: record_key } if record_key.as_slice() == key => {
                    Some(ValueRecord::Delete)
                }
                _ => None,
            })
    }

    fn ensure_open(&self) -> Result<()> {
        if self.closed {
            return Err(Error::Closed);
        }
        Ok(())
    }

    fn ensure_writable(&self) -> Result<()> {
        if self.opts.read_only {
            return Err(Error::InvalidArgument(
                "read-only transaction cannot write".to_string(),
            ));
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

fn bound_to_owned(bound: Bound<&[u8]>) -> Bound<Bytes> {
    match bound {
        Bound::Included(value) => Bound::Included(value.to_vec()),
        Bound::Excluded(value) => Bound::Excluded(value.to_vec()),
        Bound::Unbounded => Bound::Unbounded,
    }
}

fn clone_bound(bound: &Bound<Bytes>) -> Bound<Bytes> {
    match bound {
        Bound::Included(value) => Bound::Included(value.clone()),
        Bound::Excluded(value) => Bound::Excluded(value.clone()),
        Bound::Unbounded => Bound::Unbounded,
    }
}

fn clone_bound_as_ref(bound: &Bound<Bytes>) -> Bound<&[u8]> {
    match bound {
        Bound::Included(value) => Bound::Included(value.as_slice()),
        Bound::Excluded(value) => Bound::Excluded(value.as_slice()),
        Bound::Unbounded => Bound::Unbounded,
    }
}

fn within_bounds(key: &[u8], lower: &Bound<Bytes>, upper: &Bound<Bytes>) -> bool {
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
