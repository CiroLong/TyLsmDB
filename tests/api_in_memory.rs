use std::ops::Bound::{Excluded, Included, Unbounded};

use tylsmdb::{BatchRecord, DB, Options, WriteBatch};

fn test_db(name: &str) -> DB {
    let path = format!("target/tylsmdb-tests/{name}");
    DB::open(path, Options::default()).expect("open test db")
}

#[test]
fn options_batch_and_open_api_are_usable() {
    let options = Options::default();
    assert!(options.create_if_missing);
    assert_eq!(options.memtable_size, 4 * 1024 * 1024);

    let mut batch = WriteBatch::new();
    assert!(batch.is_empty());
    batch.put(b"hello".to_vec(), b"world".to_vec());
    batch.delete(b"gone".to_vec());

    assert_eq!(batch.records().len(), 2);
    assert_eq!(
        batch.records()[0],
        BatchRecord::Put {
            key: b"hello".to_vec(),
            value: b"world".to_vec()
        }
    );
    assert_eq!(
        batch.records()[1],
        BatchRecord::Delete {
            key: b"gone".to_vec()
        }
    );

    let db = test_db("options_batch_and_open_api_are_usable");
    assert!(db.path().ends_with("options_batch_and_open_api_are_usable"));
    assert!(db.options().create_if_missing);
}

#[test]
fn put_get_and_delete_work_in_memory() {
    let db = test_db("put_get_and_delete_work_in_memory");

    assert_eq!(db.get(b"missing").expect("get missing"), None);

    db.put(b"k1", b"v1").expect("put k1");
    assert_eq!(db.get(b"k1").expect("get k1"), Some(b"v1".to_vec()));

    db.put(b"k1", b"v2").expect("overwrite k1");
    assert_eq!(db.get(b"k1").expect("get k1 again"), Some(b"v2".to_vec()));

    db.delete(b"k1").expect("delete k1");
    assert_eq!(db.get(b"k1").expect("get deleted k1"), None);
}

#[test]
fn write_batch_is_applied_in_sequence_order() {
    let db = test_db("write_batch_is_applied_in_sequence_order");
    let mut batch = WriteBatch::new();
    batch.put(b"a".to_vec(), b"old".to_vec());
    batch.put(b"b".to_vec(), b"live".to_vec());
    batch.put(b"a".to_vec(), b"new".to_vec());
    batch.delete(b"b".to_vec());

    db.write(batch, Default::default()).expect("write batch");

    assert_eq!(db.get(b"a").expect("get a"), Some(b"new".to_vec()));
    assert_eq!(db.get(b"b").expect("get b"), None);
}

#[test]
fn scan_respects_bounds_tombstones_and_latest_versions() {
    let db = test_db("scan_respects_bounds_tombstones_and_latest_versions");
    db.put(b"a", b"old").expect("put a old");
    db.put(b"a", b"new").expect("put a new");
    db.put(b"b", b"hidden").expect("put b");
    db.put(b"c", b"outside").expect("put c");
    db.delete(b"b").expect("delete b");

    let rows = db
        .scan(Included(b"a".as_slice()), Excluded(b"c".as_slice()))
        .expect("scan a..c");
    assert_eq!(rows, vec![(b"a".to_vec(), b"new".to_vec())]);

    let all_rows = db.scan(Unbounded, Unbounded).expect("scan all");
    assert_eq!(
        all_rows,
        vec![
            (b"a".to_vec(), b"new".to_vec()),
            (b"c".to_vec(), b"outside".to_vec())
        ]
    );
}
