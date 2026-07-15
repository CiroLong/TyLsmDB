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
