use std::ops::Bound::{Excluded, Included};

use crate::key::{InternalKey, ValueType};
use crate::memtable::ValueRecord;

use super::{DBIterator, EntryIterator};

#[test]
fn db_iterator_filters_old_versions_and_tombstones() {
    let entries = vec![
        put(b"a", 5, b"new"),
        put(b"a", 2, b"old"),
        del(b"b", 4),
        put(b"b", 1, b"hidden"),
        put(b"c", 3, b"visible"),
    ];
    let mut iter = DBIterator::new(
        Box::new(EntryIterator::new(entries)),
        Included(b"a".to_vec()),
        Excluded(b"d".to_vec()),
        5,
    );

    assert_eq!(
        iter.collect().expect("collect visible rows"),
        vec![
            (b"a".to_vec(), b"new".to_vec()),
            (b"c".to_vec(), b"visible".to_vec())
        ]
    );
}

fn put(key: &[u8], sequence: u64, value: &[u8]) -> (InternalKey, ValueRecord) {
    (
        InternalKey::new(key.to_vec(), sequence, ValueType::Put),
        ValueRecord::Put(value.to_vec()),
    )
}

fn del(key: &[u8], sequence: u64) -> (InternalKey, ValueRecord) {
    (
        InternalKey::new(key.to_vec(), sequence, ValueType::Delete),
        ValueRecord::Delete,
    )
}
