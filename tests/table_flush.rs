use std::fs;
use std::ops::Bound::{Excluded, Included, Unbounded};
use std::path::{Path, PathBuf};

use tylsmdb::{DB, Options};

fn fresh_dir(name: &str) -> PathBuf {
    let path = PathBuf::from("target/tylsmdb-tests").join(name);
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("create fresh test dir");
    path
}

#[test]
fn flush_moves_memtable_to_l0_table_and_get_reads_from_flushed_table() {
    let path = fresh_dir("flush_moves_memtable_to_l0_table_and_get_reads_from_flushed_table");
    let db = DB::open(&path, Options::default()).expect("open db");

    db.put(b"a", b"1").expect("put a");
    db.put(b"b", b"2").expect("put b");
    db.flush().expect("flush");

    assert_eq!(db.get(b"a").expect("get flushed a"), Some(b"1".to_vec()));
    assert_eq!(db.get(b"b").expect("get flushed b"), Some(b"2".to_vec()));
    assert_eq!(sst_files(&path).len(), 1);
}

#[test]
fn size_triggered_flush_writes_l0_table() {
    let path = fresh_dir("size_triggered_flush_writes_l0_table");
    let db = DB::open(
        &path,
        Options {
            memtable_size: 1,
            ..Options::default()
        },
    )
    .expect("open db");

    db.put(b"auto", b"flush").expect("put auto flush");

    assert_eq!(
        db.get(b"auto").expect("get auto flushed"),
        Some(b"flush".to_vec())
    );
    assert_eq!(sst_files(&path).len(), 1);
}

#[test]
fn scan_merges_memtable_and_table_versions() {
    let path = fresh_dir("scan_merges_memtable_and_table_versions");
    let db = DB::open(&path, Options::default()).expect("open db");

    db.put(b"a", b"old").expect("put old a");
    db.put(b"b", b"table").expect("put b");
    db.flush().expect("flush table");
    db.put(b"a", b"new").expect("put new a");
    db.delete(b"b").expect("delete b");
    db.put(b"c", b"mem").expect("put c");

    let rows = db
        .scan(Included(b"a".as_slice()), Excluded(b"d".as_slice()))
        .expect("scan merged");
    assert_eq!(
        rows,
        vec![
            (b"a".to_vec(), b"new".to_vec()),
            (b"c".to_vec(), b"mem".to_vec())
        ]
    );

    assert_eq!(
        db.scan(Unbounded, Unbounded).expect("scan all"),
        vec![
            (b"a".to_vec(), b"new".to_vec()),
            (b"c".to_vec(), b"mem".to_vec())
        ]
    );
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
