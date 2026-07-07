use std::fs;
use std::ops::Bound::{Included, Unbounded};
use std::path::PathBuf;

use tylsmdb::{DB, Options, ReadOptions, Result};

fn fresh_dir(name: &str) -> PathBuf {
    let path = PathBuf::from("target/tylsmdb-tests").join(name);
    if path.exists() {
        fs::remove_dir_all(&path).expect("remove old test db");
    }
    fs::create_dir_all(path.parent().expect("test root")).expect("create test root");
    path
}

#[test]
fn scan_iter_returns_visible_rows_incrementally() -> Result<()> {
    let db = DB::open(
        fresh_dir("v10_scan_iter_returns_visible_rows_incrementally"),
        Options::default(),
    )?;

    db.put(b"a", b"old")?;
    db.put(b"a", b"new")?;
    db.put(b"b", b"keep")?;
    db.delete(b"c")?;
    db.put(b"d", b"tail")?;

    let mut iter = db.scan_iter(Included(b"a".as_slice()), Included(b"d".as_slice()))?;

    assert!(iter.is_valid());
    assert_eq!(iter.key(), Some(b"a".as_slice()));
    assert_eq!(iter.value(), Some(b"new".as_slice()));

    iter.next()?;
    assert!(iter.is_valid());
    assert_eq!(iter.key(), Some(b"b".as_slice()));
    assert_eq!(iter.value(), Some(b"keep".as_slice()));

    iter.next()?;
    assert!(iter.is_valid());
    assert_eq!(iter.key(), Some(b"d".as_slice()));
    assert_eq!(iter.value(), Some(b"tail".as_slice()));

    iter.next()?;
    assert!(!iter.is_valid());
    Ok(())
}

#[test]
fn scan_iter_snapshot_preserves_old_versions() -> Result<()> {
    let db = DB::open(
        fresh_dir("v10_scan_iter_snapshot_preserves_old_versions"),
        Options::default(),
    )?;

    db.put(b"k", b"old")?;
    let snapshot = db.snapshot();
    db.put(b"k", b"new")?;
    db.put(b"z", b"after")?;

    let mut iter = db.scan_iter_opt(
        Unbounded,
        Unbounded,
        ReadOptions {
            snapshot: Some(snapshot),
            ..ReadOptions::default()
        },
    )?;
    assert_eq!(iter.collect()?, vec![(b"k".to_vec(), b"old".to_vec())]);
    Ok(())
}

#[test]
fn scan_iter_lower_bound_does_not_load_all_prefix_blocks() -> Result<()> {
    let db = DB::open(
        fresh_dir("v10_scan_iter_lower_bound_does_not_load_all_prefix_blocks"),
        Options {
            block_size: 96,
            ..Options::default()
        },
    )?;

    for i in 0..120 {
        let key = format!("k{i:03}");
        let value = vec![b'x'; 64];
        db.put(key.as_bytes(), &value)?;
    }
    db.flush()?;

    let before = db.block_cache_stats();
    let iter = db.scan_iter(Included(b"k110".as_slice()), Unbounded)?;
    assert!(iter.is_valid());
    assert_eq!(iter.key(), Some(b"k110".as_slice()));

    let after = db.block_cache_stats();
    let misses = after.misses - before.misses;
    assert!(
        misses <= 2,
        "streaming lower-bound scan should read at most the first and target blocks, got {misses} cache misses"
    );
    assert!(misses > 0);
    Ok(())
}

#[test]
fn scan_iter_fill_cache_false_does_not_touch_block_cache() -> Result<()> {
    let db = DB::open(
        fresh_dir("v10_scan_iter_fill_cache_false_does_not_touch_block_cache"),
        Options {
            block_size: 96,
            ..Options::default()
        },
    )?;

    for i in 0..16 {
        let key = format!("k{i:03}");
        let value = vec![b'y'; 64];
        db.put(key.as_bytes(), &value)?;
    }
    db.flush()?;

    let before = db.block_cache_stats();
    let mut iter = db.scan_iter_opt(
        Unbounded,
        Unbounded,
        ReadOptions {
            fill_cache: false,
            ..ReadOptions::default()
        },
    )?;
    assert!(iter.is_valid());
    assert_eq!(iter.key(), Some(b"k000".as_slice()));
    iter.next()?;

    let after = db.block_cache_stats();
    assert_eq!(after, before);
    Ok(())
}
