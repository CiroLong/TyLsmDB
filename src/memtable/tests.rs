use std::ops::Bound::{Excluded, Unbounded};

use super::{MemTable, MemTableKind, ValueRecord};

#[test]
fn both_memtable_kinds_have_same_visible_behavior() {
    for kind in [MemTableKind::BTree, MemTableKind::SkipList] {
        let mut table = MemTable::new(kind);
        table.put(1, b"a".to_vec(), b"old".to_vec());
        table.put(2, b"a".to_vec(), b"new".to_vec());
        table.put(3, b"b".to_vec(), b"value".to_vec());
        table.delete(4, b"b".to_vec());

        assert_eq!(table.kind(), kind);
        assert_eq!(table.get(b"a", 4), Some(ValueRecord::Put(b"new".to_vec())));
        assert_eq!(table.get(b"b", 4), Some(ValueRecord::Delete));
        assert_eq!(
            table.scan(Unbounded, Excluded(b"z"), 4),
            vec![(b"a".to_vec(), b"new".to_vec())]
        );
    }
}
