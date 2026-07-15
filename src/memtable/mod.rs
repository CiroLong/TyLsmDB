use std::collections::BTreeSet;
use std::ops::Bound;

use crate::key::{InternalKey, SequenceNumber};

pub mod arena;
pub mod btree;
pub mod skiplist;

pub use btree::ValueRecord;
use skiplist::SkipListMemTable;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MemTableKind {
    #[default]
    BTree,
    SkipList,
}

#[derive(Debug)]
pub enum MemTable {
    BTree(btree::MemTable),
    SkipList(Box<SkipListMemTable>),
}

impl MemTable {
    pub fn new(kind: MemTableKind) -> Self {
        match kind {
            MemTableKind::BTree => Self::BTree(btree::MemTable::new()),
            MemTableKind::SkipList => Self::SkipList(Box::default()),
        }
    }

    pub fn kind(&self) -> MemTableKind {
        match self {
            Self::BTree(_) => MemTableKind::BTree,
            Self::SkipList(_) => MemTableKind::SkipList,
        }
    }

    pub fn put(&mut self, seq: SequenceNumber, key: Vec<u8>, value: Vec<u8>) {
        match self {
            Self::BTree(table) => table.put(seq, key, value),
            Self::SkipList(table) => table.put(seq, key, value),
        }
    }

    pub fn delete(&mut self, seq: SequenceNumber, key: Vec<u8>) {
        match self {
            Self::BTree(table) => table.delete(seq, key),
            Self::SkipList(table) => table.delete(seq, key),
        }
    }

    pub fn get(&self, key: &[u8], read_seq: SequenceNumber) -> Option<ValueRecord> {
        match self {
            Self::BTree(table) => table.get(key, read_seq),
            Self::SkipList(table) => table.get(key, read_seq),
        }
    }

    pub fn scan(
        &self,
        lower: Bound<&[u8]>,
        upper: Bound<&[u8]>,
        read_seq: SequenceNumber,
    ) -> Vec<(Vec<u8>, Vec<u8>)> {
        match self {
            Self::BTree(table) => table.scan(lower, upper, read_seq),
            Self::SkipList(table) => table.scan(lower, upper, read_seq),
        }
    }

    pub fn approximate_size(&self) -> usize {
        match self {
            Self::BTree(table) => table.approximate_size(),
            Self::SkipList(table) => table.approximate_size(),
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Self::BTree(table) => table.is_empty(),
            Self::SkipList(table) => table.is_empty(),
        }
    }

    pub(crate) fn entries(&self) -> Vec<(InternalKey, ValueRecord)> {
        match self {
            Self::BTree(table) => table
                .entries()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
            Self::SkipList(table) => table.entries(),
        }
    }
}

impl Default for MemTable {
    fn default() -> Self {
        Self::new(MemTableKind::BTree)
    }
}

pub(crate) fn visible_scan(
    entries: Vec<(InternalKey, ValueRecord)>,
    lower: Bound<&[u8]>,
    upper: Bound<&[u8]>,
    read_seq: SequenceNumber,
) -> Vec<(Vec<u8>, Vec<u8>)> {
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

#[cfg(test)]
mod tests;
