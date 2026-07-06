use std::fs::{self, OpenOptions};
use std::io::Write;
use std::ops::Bound::Unbounded;
use std::path::PathBuf;

use tylsmdb::memtable::MemTableKind;
use tylsmdb::table::format::CompressionType;
use tylsmdb::{DB, Options, WriteOptions};

fn fresh_dir(name: &str) -> PathBuf {
    let path = PathBuf::from("target/tylsmdb-tests").join(name);
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("create fresh test dir");
    path
}

#[test]
fn reopen_ignores_half_wal_record_and_leftover_tmp_sstable() {
    let path = fresh_dir("reopen_ignores_half_wal_record_and_leftover_tmp_sstable");
    let db = DB::open(&path, test_options()).expect("open db");

    db.put(b"synced-a", b"1").expect("put synced a");
    db.write(
        {
            let mut batch = tylsmdb::WriteBatch::new();
            batch.put(b"synced-b".to_vec(), b"2".to_vec());
            batch
        },
        WriteOptions {
            sync: true,
            disable_wal: false,
        },
    )
    .expect("sync write");
    db.flush().expect("flush");
    db.put(b"after-flush", b"3").expect("put after flush");
    db.sync_wal().expect("sync wal");
    drop(db);

    let wal_path = latest_wal_file(&path);
    let mut wal = OpenOptions::new()
        .append(true)
        .open(&wal_path)
        .expect("open current wal");
    wal.write_all(&[0xaa, 0xbb, 0xcc])
        .expect("append partial wal");
    fs::write(path.join("000099.sst.tmp"), b"partial table").expect("write tmp sst");

    let reopened = DB::open(&path, test_options()).expect("reopen");

    assert_eq!(
        reopened.get(b"synced-a").expect("get synced a"),
        Some(b"1".to_vec())
    );
    assert_eq!(
        reopened.get(b"synced-b").expect("get synced b"),
        Some(b"2".to_vec())
    );
    assert_eq!(
        reopened.get(b"after-flush").expect("get after flush"),
        Some(b"3".to_vec())
    );
    assert_eq!(
        reopened.scan(Unbounded, Unbounded).expect("scan"),
        vec![
            (b"after-flush".to_vec(), b"3".to_vec()),
            (b"synced-a".to_vec(), b"1".to_vec()),
            (b"synced-b".to_vec(), b"2".to_vec())
        ]
    );
}

fn test_options() -> Options {
    Options {
        memtable_kind: MemTableKind::SkipList,
        table_compression: CompressionType::Zstd,
        ..Options::default()
    }
}

fn latest_wal_file(path: &std::path::Path) -> PathBuf {
    fs::read_dir(path)
        .expect("read db dir")
        .map(|entry| entry.expect("dir entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "wal"))
        .max()
        .expect("current wal file")
}
