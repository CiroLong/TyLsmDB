use std::fs;
use std::ops::Bound::{Included, Unbounded};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

use tylsmdb::memtable::MemTableKind;
use tylsmdb::table::format::CompressionType;
use tylsmdb::{DB, Options, WalSyncMode};

fn fresh_dir(name: &str) -> PathBuf {
    let path = PathBuf::from("target/tylsmdb-tests").join(name);
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("create fresh test dir");
    path
}

#[test]
fn skiplist_memtable_matches_btree_visible_behavior() {
    let path = fresh_dir("skiplist_memtable_matches_btree_visible_behavior");
    let db = DB::open(
        &path,
        Options {
            memtable_kind: MemTableKind::SkipList,
            memtable_size: 128,
            ..Options::default()
        },
    )
    .expect("open db");

    db.put(b"a", b"1").expect("put a1");
    db.put(b"b", b"2").expect("put b");
    db.put(b"a", b"3").expect("put a3");
    db.delete(b"b").expect("delete b");

    assert_eq!(db.get(b"a").expect("get a"), Some(b"3".to_vec()));
    assert_eq!(db.get(b"b").expect("get b"), None);
    assert_eq!(
        db.scan(Unbounded, Unbounded).expect("scan"),
        vec![(b"a".to_vec(), b"3".to_vec())]
    );

    db.flush().expect("flush skiplist");
    drop(db);

    let reopened = DB::open(&path, Options::default()).expect("reopen");
    assert_eq!(
        reopened.get(b"a").expect("get reopened a"),
        Some(b"3".to_vec())
    );
    assert_eq!(reopened.get(b"b").expect("get reopened b"), None);
}

#[test]
fn zstd_compressed_tables_roundtrip_and_shrink_repeated_blocks() {
    let plain_path = fresh_dir("zstd_plain_baseline");
    let zstd_path = fresh_dir("zstd_compressed_tables_roundtrip");
    let plain = DB::open(
        &plain_path,
        Options {
            block_size: 256,
            table_compression: CompressionType::None,
            ..Options::default()
        },
    )
    .expect("open plain");
    let compressed = DB::open(
        &zstd_path,
        Options {
            block_size: 256,
            table_compression: CompressionType::Zstd,
            ..Options::default()
        },
    )
    .expect("open compressed");

    for index in 0..80 {
        let key = format!("key-{index:03}");
        let value = vec![b'x'; 1024];
        plain.put(key.as_bytes(), &value).expect("plain put");
        compressed
            .put(key.as_bytes(), &value)
            .expect("compressed put");
    }
    plain.flush().expect("plain flush");
    compressed.flush().expect("compressed flush");

    assert_eq!(
        compressed.get(b"key-042").expect("get compressed"),
        Some(vec![b'x'; 1024])
    );
    assert!(
        total_sst_bytes(&zstd_path) < total_sst_bytes(&plain_path),
        "zstd table should be smaller for repeated values"
    );
}

#[test]
fn metrics_track_write_wal_sst_and_cache_activity() {
    let path = fresh_dir("metrics_track_write_wal_sst_and_cache_activity");
    let db = DB::open(
        &path,
        Options {
            block_size: 128,
            ..Options::default()
        },
    )
    .expect("open db");

    db.put(b"k", b"value").expect("put");
    db.flush().expect("flush");
    assert_eq!(db.get(b"k").expect("first get"), Some(b"value".to_vec()));
    assert_eq!(db.get(b"k").expect("second get"), Some(b"value".to_vec()));

    let metrics = db.metrics_snapshot();

    assert!(metrics.user_write_bytes >= b"k".len() as u64 + b"value".len() as u64);
    assert!(metrics.wal_write_bytes > 0);
    assert!(metrics.sst_write_bytes > 0);
    assert!(metrics.block_cache_misses > 0);
    assert!(metrics.block_cache_hits > 0);
}

#[test]
fn bloom_metrics_record_useful_negative_filter_results() {
    let path = fresh_dir("bloom_metrics_record_useful_negative_filter_results");
    let db = DB::open(
        &path,
        Options {
            block_size: 128,
            ..Options::default()
        },
    )
    .expect("open db");

    db.put(b"a", b"1").expect("put a");
    db.put(b"z", b"2").expect("put z");
    db.flush().expect("flush");

    assert_eq!(db.get(b"m").expect("missing get"), None);

    let metrics = db.metrics_snapshot();
    assert!(metrics.bloom_useful > 0);
}

#[test]
fn group_commit_batches_concurrent_sync_writes() {
    let path = fresh_dir("group_commit_batches_concurrent_sync_writes");
    let db = Arc::new(
        DB::open(
            &path,
            Options {
                wal_sync: WalSyncMode::PerWrite,
                write_group_max_delay: Duration::from_millis(5),
                ..Options::default()
            },
        )
        .expect("open db"),
    );
    let writers = 8;
    let barrier = Arc::new(Barrier::new(writers));
    let mut handles = Vec::new();

    for index in 0..writers {
        let db = Arc::clone(&db);
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            barrier.wait();
            let key = format!("group-{index:02}");
            db.put(key.as_bytes(), b"value").expect("put");
        }));
    }

    for handle in handles {
        handle.join().expect("writer thread joins");
    }

    for index in 0..writers {
        let key = format!("group-{index:02}");
        assert_eq!(
            db.get(key.as_bytes()).expect("get"),
            Some(b"value".to_vec())
        );
    }

    let metrics = db.metrics_snapshot();
    assert!(metrics.wal_sync_count > 0);
    assert!(
        metrics.wal_sync_count < writers as u64,
        "group commit should sync fewer times than the number of concurrent writers"
    );
}

#[test]
fn rate_limiter_reports_wait_when_budget_is_exhausted() {
    let limiter = tylsmdb::util::rate_limiter::RateLimiter::new(10);

    assert_eq!(limiter.reserve(4), std::time::Duration::ZERO);
    assert!(limiter.reserve(20) > std::time::Duration::ZERO);

    let limiter = tylsmdb::util::rate_limiter::RateLimiter::new(10);
    let first_wait = limiter.reserve(30);
    let second_wait = limiter.reserve(10);
    assert!(second_wait > first_wait);
}

#[test]
fn parallel_subcompaction_records_multiple_tasks_and_keeps_sorted_results() {
    let path = fresh_dir("manual_compaction_with_small_subcompaction_budget");
    let db = DB::open(
        &path,
        Options {
            max_subcompactions: 4,
            target_file_size_base: 256,
            ..Options::default()
        },
    )
    .expect("open db");

    for round in 0..4 {
        for index in 0..20 {
            let key = format!("k-{index:03}");
            let value = format!("v-{round}-{index}");
            db.put(key.as_bytes(), value.as_bytes()).expect("put");
        }
        db.flush().expect("flush round");
    }

    db.compact_range(Included(b"k-000"), Included(b"k-999"))
        .expect("compact");

    let rows = db.scan(Unbounded, Unbounded).expect("scan compacted");
    assert_eq!(rows.len(), 20);
    assert_eq!(rows.first().expect("first").0, b"k-000".to_vec());
    assert_eq!(rows.last().expect("last").0, b"k-019".to_vec());

    let metrics = db.metrics_snapshot();
    assert!(metrics.subcompaction_tasks > 1);
    assert!(metrics.max_subcompaction_parallelism > 1);
}

fn total_sst_bytes(path: &Path) -> u64 {
    fs::read_dir(path)
        .expect("read db dir")
        .map(|entry| entry.expect("dir entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "sst"))
        .map(|path| fs::metadata(path).expect("sst metadata").len())
        .sum()
}
