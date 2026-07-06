use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound;

use crate::key::{InternalKey, SequenceNumber, ValueType};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueRecord {
    Put(Vec<u8>),
    Delete,
}

#[derive(Debug, Default)]
pub struct MemTable {
    map: BTreeMap<InternalKey, ValueRecord>,
    approximate_size: usize,
}

impl MemTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn put(&mut self, seq: SequenceNumber, key: Vec<u8>, value: Vec<u8>) {
        self.approximate_size += key.len() + value.len() + std::mem::size_of::<InternalKey>();
        self.map.insert(
            InternalKey::new(key, seq, ValueType::Put),
            ValueRecord::Put(value),
        );
    }

    pub fn delete(&mut self, seq: SequenceNumber, key: Vec<u8>) {
        self.approximate_size += key.len() + std::mem::size_of::<InternalKey>();
        self.map.insert(
            InternalKey::new(key, seq, ValueType::Delete),
            ValueRecord::Delete,
        );
    }

    pub fn get(&self, key: &[u8], read_seq: SequenceNumber) -> Option<ValueRecord> {
        self.map
            .iter()
            .find(|(internal_key, _)| {
                internal_key.user_key() == key && internal_key.sequence() <= read_seq
            })
            .map(|(_, value)| value.clone())
    }

    pub fn scan(
        &self,
        lower: Bound<&[u8]>,
        upper: Bound<&[u8]>,
        read_seq: SequenceNumber,
    ) -> Vec<(Vec<u8>, Vec<u8>)> {
        let mut seen = BTreeSet::new();
        let mut rows = Vec::new();

        for (internal_key, value) in &self.map {
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
                rows.push((user_key_vec, value.clone()));
            }
        }

        rows
    }

    pub fn approximate_size(&self) -> usize {
        self.approximate_size
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub(crate) fn entries(&self) -> impl Iterator<Item = (&InternalKey, &ValueRecord)> {
        self.map.iter()
    }
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
mod tests {
    use std::ops::Bound::{Excluded, Included, Unbounded};

    use super::{MemTable, ValueRecord};

    #[test]
    fn newest_visible_value_wins() {
        let mut table = MemTable::new();
        table.put(1, b"k".to_vec(), b"old".to_vec());
        table.put(2, b"k".to_vec(), b"new".to_vec());

        assert_eq!(table.get(b"k", 2), Some(ValueRecord::Put(b"new".to_vec())));
    }

    #[test]
    fn tombstone_hides_older_value() {
        let mut table = MemTable::new();
        table.put(1, b"k".to_vec(), b"old".to_vec());
        table.delete(2, b"k".to_vec());

        assert_eq!(table.get(b"k", 2), Some(ValueRecord::Delete));
    }

    #[test]
    fn lower_read_sequence_sees_previous_version() {
        let mut table = MemTable::new();
        table.put(1, b"k".to_vec(), b"old".to_vec());
        table.put(3, b"k".to_vec(), b"new".to_vec());

        assert_eq!(table.get(b"k", 2), Some(ValueRecord::Put(b"old".to_vec())));
    }

    #[test]
    fn scan_returns_sorted_unique_user_keys() {
        let mut table = MemTable::new();
        table.put(1, b"a".to_vec(), b"old".to_vec());
        table.put(2, b"a".to_vec(), b"new".to_vec());
        table.put(3, b"b".to_vec(), b"hidden".to_vec());
        table.put(4, b"c".to_vec(), b"outside".to_vec());
        table.delete(5, b"b".to_vec());

        assert_eq!(
            table.scan(Included(b"a"), Excluded(b"c"), 5),
            vec![(b"a".to_vec(), b"new".to_vec())]
        );
        assert_eq!(
            table.scan(Unbounded, Unbounded, 5),
            vec![
                (b"a".to_vec(), b"new".to_vec()),
                (b"c".to_vec(), b"outside".to_vec()),
            ]
        );
    }
}
