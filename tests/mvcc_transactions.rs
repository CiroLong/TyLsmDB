use std::fs;
use std::ops::Bound::{Included, Unbounded};
use std::path::{Path, PathBuf};

use tylsmdb::table::SSTableReader;
use tylsmdb::{DB, Error, Options, ReadOptions, TransactionOptions};

fn fresh_dir(name: &str) -> PathBuf {
    let path = PathBuf::from("target/tylsmdb-tests").join(name);
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("create fresh test dir");
    path
}

#[test]
fn snapshot_keeps_old_value_after_update() {
    let path = fresh_dir("snapshot_keeps_old_value_after_update");
    let db = DB::open(&path, Options::default()).expect("open db");

    db.put(b"k", b"old").expect("put old");
    let snapshot = db.snapshot();
    db.put(b"k", b"new").expect("put new");

    assert_eq!(db.get(b"k").expect("get current"), Some(b"new".to_vec()));
    assert_eq!(
        db.get_opt(
            b"k",
            ReadOptions {
                snapshot: Some(snapshot.clone()),
                ..ReadOptions::default()
            },
        )
        .expect("get snapshot"),
        Some(b"old".to_vec())
    );
    assert_eq!(
        db.scan_opt(
            Unbounded,
            Unbounded,
            ReadOptions {
                snapshot: Some(snapshot),
                ..ReadOptions::default()
            },
        )
        .expect("scan snapshot"),
        vec![(b"k".to_vec(), b"old".to_vec())]
    );
}

#[test]
fn snapshot_drop_allows_old_version_gc() {
    let path = fresh_dir("snapshot_drop_allows_old_version_gc");
    let db = DB::open(&path, Options::default()).expect("open db");

    db.put(b"k", b"old").expect("put old");
    db.flush().expect("flush old");
    let snapshot = db.snapshot();
    db.put(b"k", b"new").expect("put new");
    db.flush().expect("flush new");

    db.compact_range(Unbounded, Unbounded)
        .expect("compact while snapshot is active");

    assert_eq!(
        db.get_opt(
            b"k",
            ReadOptions {
                snapshot: Some(snapshot.clone()),
                ..ReadOptions::default()
            },
        )
        .expect("get active snapshot"),
        Some(b"old".to_vec())
    );
    assert_eq!(total_sst_entries(&path), 2);

    drop(snapshot);
    db.compact_range(Unbounded, Unbounded)
        .expect("compact after snapshot drop");

    assert_eq!(db.get(b"k").expect("get current"), Some(b"new".to_vec()));
    assert_eq!(total_sst_entries(&path), 1);
}

#[test]
fn transaction_commit_applies_batch_atomically() {
    let path = fresh_dir("transaction_commit_applies_batch_atomically");
    let db = DB::open(&path, Options::default()).expect("open db");
    let mut txn = db
        .transaction(TransactionOptions::default())
        .expect("create transaction");

    txn.put(b"a", b"1").expect("txn put a");
    txn.put(b"b", b"2").expect("txn put b");

    assert_eq!(
        txn.get(b"a").expect("txn read own write"),
        Some(b"1".to_vec())
    );
    assert_eq!(db.get(b"a").expect("db does not see uncommitted a"), None);
    assert_eq!(db.get(b"b").expect("db does not see uncommitted b"), None);

    txn.commit().expect("commit transaction");

    assert_eq!(db.get(b"a").expect("get committed a"), Some(b"1".to_vec()));
    assert_eq!(db.get(b"b").expect("get committed b"), Some(b"2".to_vec()));
}

#[test]
fn transaction_reads_own_delete() {
    let path = fresh_dir("transaction_reads_own_delete");
    let db = DB::open(&path, Options::default()).expect("open db");
    db.put(b"k", b"base").expect("put base");
    let mut txn = db
        .transaction(TransactionOptions::default())
        .expect("create transaction");

    txn.delete(b"k").expect("txn delete");

    assert_eq!(txn.get(b"k").expect("txn sees delete"), None);
    assert_eq!(
        db.get(b"k").expect("db still sees base"),
        Some(b"base".to_vec())
    );

    txn.commit().expect("commit delete");

    assert_eq!(db.get(b"k").expect("delete committed"), None);
}

#[test]
fn write_write_conflict_is_rejected() {
    let path = fresh_dir("write_write_conflict_is_rejected");
    let db = DB::open(&path, Options::default()).expect("open db");
    db.put(b"k", b"base").expect("put base");
    let mut txn1 = db
        .transaction(TransactionOptions::default())
        .expect("create txn1");
    let mut txn2 = db
        .transaction(TransactionOptions::default())
        .expect("create txn2");

    txn1.put(b"k", b"txn1").expect("txn1 put");
    txn2.put(b"k", b"txn2").expect("txn2 put");

    txn1.commit().expect("txn1 commit");
    let err = txn2.commit().expect_err("txn2 should conflict");

    assert!(matches!(err, Error::TransactionConflict(_)));
    assert_eq!(db.get(b"k").expect("get committed"), Some(b"txn1".to_vec()));
}

#[test]
fn read_write_conflict_is_rejected() {
    let path = fresh_dir("read_write_conflict_is_rejected");
    let db = DB::open(&path, Options::default()).expect("open db");
    db.put(b"k", b"base").expect("put base");
    let mut txn = db
        .transaction(TransactionOptions::default())
        .expect("create transaction");

    assert_eq!(txn.get(b"k").expect("txn read"), Some(b"base".to_vec()));
    db.put(b"k", b"outside").expect("outside write");
    txn.put(b"other", b"value").expect("txn put other");

    let err = txn.commit().expect_err("txn should conflict");

    assert!(matches!(err, Error::TransactionConflict(_)));
    assert_eq!(db.get(b"other").expect("other not committed"), None);
}

#[test]
fn range_phantom_conflict_is_rejected() {
    let path = fresh_dir("range_phantom_conflict_is_rejected");
    let db = DB::open(&path, Options::default()).expect("open db");
    db.put(b"a", b"1").expect("put a");
    db.put(b"z", b"3").expect("put z");
    let mut txn = db
        .transaction(TransactionOptions::default())
        .expect("create transaction");

    assert_eq!(
        txn.scan(Included(b"a".as_slice()), Included(b"z".as_slice()))
            .expect("txn scan"),
        vec![
            (b"a".to_vec(), b"1".to_vec()),
            (b"z".to_vec(), b"3".to_vec())
        ]
    );
    db.put(b"m", b"2").expect("outside insert in scanned range");
    txn.put(b"y", b"txn").expect("txn put");

    let err = txn.commit().expect_err("txn should conflict");

    assert!(matches!(err, Error::TransactionConflict(_)));
    assert_eq!(db.get(b"y").expect("phantom txn write not committed"), None);
}

#[test]
fn rollback_discards_writes() {
    let path = fresh_dir("rollback_discards_writes");
    let db = DB::open(&path, Options::default()).expect("open db");
    let mut txn = db
        .transaction(TransactionOptions::default())
        .expect("create transaction");

    txn.put(b"k", b"value").expect("txn put");
    txn.rollback().expect("rollback");

    assert_eq!(db.get(b"k").expect("rollback discarded write"), None);
}

fn total_sst_entries(path: &Path) -> usize {
    sst_files(path)
        .into_iter()
        .map(|path| {
            SSTableReader::open(path)
                .expect("open sstable")
                .iter()
                .expect("iterate sstable")
                .count()
        })
        .sum()
}

fn sst_files(path: &Path) -> Vec<PathBuf> {
    let mut files: Vec<_> = fs::read_dir(path)
        .expect("read db dir")
        .map(|entry| entry.expect("dir entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "sst"))
        .collect();
    files.sort();
    files
}
