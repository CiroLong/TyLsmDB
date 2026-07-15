use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::ops::Bound::Unbounded;
use std::path::{Path, PathBuf};

use tylsmdb::options::{WalSyncMode, WriteOptions};
use tylsmdb::{DB, Error, Options, WriteBatch};

fn fresh_dir(name: &str) -> PathBuf {
    let path = PathBuf::from("target/tylsmdb-tests").join(name);
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("create fresh test dir");
    path
}

#[test]
fn reopen_replays_puts_and_deletes() {
    let path = fresh_dir("reopen_replays_puts_and_deletes");

    {
        let db = DB::open(&path, Options::default()).expect("open db");
        db.put(b"a", b"1").expect("put a");
        db.put(b"b", b"2").expect("put b");
        db.delete(b"b").expect("delete b");
        db.sync_wal().expect("sync wal");
    }

    let reopened = DB::open(&path, Options::default()).expect("reopen db");
    assert_eq!(reopened.get(b"a").expect("get a"), Some(b"1".to_vec()));
    assert_eq!(reopened.get(b"b").expect("get b"), None);
    assert_eq!(
        reopened.scan(Unbounded, Unbounded).expect("scan reopened"),
        vec![(b"a".to_vec(), b"1".to_vec())]
    );
}

#[test]
fn per_write_sync_mode_persists_after_reopen() {
    let path = fresh_dir("per_write_sync_mode_persists_after_reopen");
    let options = Options {
        wal_sync: WalSyncMode::PerWrite,
        ..Options::default()
    };

    {
        let db = DB::open(&path, options.clone()).expect("open db");
        let mut batch = WriteBatch::new();
        batch.put(b"synced".to_vec(), b"value".to_vec());
        db.write(batch, WriteOptions::default())
            .expect("write synced");
    }

    let reopened = DB::open(&path, options).expect("reopen db");
    assert_eq!(
        reopened.get(b"synced").expect("get synced"),
        Some(b"value".to_vec())
    );
}

#[test]
fn trailing_partial_wal_record_is_ignored_on_db_open() {
    let path = fresh_dir("trailing_partial_wal_record_is_ignored_on_db_open");

    {
        let db = DB::open(&path, Options::default()).expect("open db");
        db.put(b"ok", b"value").expect("put ok");
        db.sync_wal().expect("sync wal");
    }
    append_bytes(&path.join("000001.wal"), &[9, 8, 7]);

    let reopened = DB::open(&path, Options::default()).expect("reopen db");
    assert_eq!(
        reopened.get(b"ok").expect("get ok"),
        Some(b"value".to_vec())
    );
}

#[test]
fn corrupt_complete_wal_record_returns_error_on_db_open() {
    let path = fresh_dir("corrupt_complete_wal_record_returns_error_on_db_open");

    {
        let db = DB::open(&path, Options::default()).expect("open db");
        db.put(b"bad", b"value").expect("put bad");
        db.sync_wal().expect("sync wal");
    }
    corrupt_first_byte(&path.join("000001.wal"));

    assert!(matches!(
        DB::open(&path, Options::default()),
        Err(Error::Corruption(_))
    ));
}

fn append_bytes(path: &Path, bytes: &[u8]) {
    let mut file = OpenOptions::new()
        .append(true)
        .open(path)
        .expect("open file for append");
    file.write_all(bytes).expect("append bytes");
}

fn corrupt_first_byte(path: &Path) {
    let mut file = OpenOptions::new()
        .write(true)
        .open(path)
        .expect("open file for corruption");
    file.seek(SeekFrom::Start(0)).expect("seek to start");
    file.write_all(&[0xff]).expect("corrupt byte");
}
