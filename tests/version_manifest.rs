use std::fs;
use std::ops::Bound::Unbounded;
use std::path::{Path, PathBuf};

use tylsmdb::{DB, Options};

fn fresh_dir(name: &str) -> PathBuf {
    let path = PathBuf::from("target/tylsmdb-tests").join(name);
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("create fresh test dir");
    path
}

#[test]
fn reopen_recovers_flushed_sstables_and_active_wal() {
    let path = fresh_dir("reopen_recovers_flushed_sstables_and_active_wal");
    {
        let db = DB::open(&path, Options::default()).expect("open db");
        db.put(b"flushed", b"table").expect("put table value");
        db.flush().expect("flush table");
        db.put(b"wal", b"active").expect("put wal value");
        db.sync_wal().expect("sync active wal");
    }
    fs::remove_file(path.join("000001.wal")).expect("remove flushed wal");

    assert!(path.join("CURRENT").exists());

    let reopened = DB::open(&path, Options::default()).expect("reopen db");

    assert_eq!(
        reopened.get(b"flushed").expect("get flushed"),
        Some(b"table".to_vec())
    );
    assert_eq!(
        reopened.get(b"wal").expect("get wal"),
        Some(b"active".to_vec())
    );
    assert_eq!(
        reopened.scan(Unbounded, Unbounded).expect("scan reopened"),
        vec![
            (b"flushed".to_vec(), b"table".to_vec()),
            (b"wal".to_vec(), b"active".to_vec())
        ]
    );
}

#[test]
fn reopen_preserves_next_file_number() {
    let path = fresh_dir("reopen_preserves_next_file_number");
    {
        let db = DB::open(&path, Options::default()).expect("open db");
        db.put(b"a", b"1").expect("put a");
        db.flush().expect("flush first table");
    }
    fs::remove_file(path.join("000001.wal")).expect("remove flushed wal");
    {
        let db = DB::open(&path, Options::default()).expect("reopen db");
        assert_eq!(db.get(b"a").expect("get recovered a"), Some(b"1".to_vec()));
        db.put(b"b", b"2").expect("put b");
        db.flush().expect("flush second table");
    }

    let files = sst_files(&path);
    assert_eq!(files.len(), 2);
    assert_ne!(files[0].file_name(), files[1].file_name());
    assert!(path.join("CURRENT").exists());
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
