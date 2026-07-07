use std::fs;
use std::ops::Bound::{Excluded, Included, Unbounded};
use std::path::{Path, PathBuf};

use tylsmdb::env::file::table_file_name;
use tylsmdb::iterator::{DBIterator, EntryIterator};
use tylsmdb::key::{InternalKey, ValueType};
use tylsmdb::memtable::ValueRecord;
use tylsmdb::table::{SSTableBuilder, SSTableReader};
use tylsmdb::version::{FileMeta, VersionEdit, VersionSet};
use tylsmdb::{DB, Options, ReadOptions};

fn fresh_dir(name: &str) -> PathBuf {
    let path = PathBuf::from("target/tylsmdb-tests").join(name);
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("create fresh test dir");
    path
}

#[test]
fn db_iterator_filters_old_versions_and_tombstones() {
    let entries = vec![
        put(b"a", 5, b"new"),
        put(b"a", 2, b"old"),
        del(b"b", 4),
        put(b"b", 1, b"hidden"),
        put(b"c", 3, b"visible"),
    ];
    let mut iter = DBIterator::new(
        Box::new(EntryIterator::new(entries)),
        Included(b"a".to_vec()),
        Excluded(b"d".to_vec()),
        5,
    );

    assert_eq!(
        iter.collect().expect("collect visible rows"),
        vec![
            (b"a".to_vec(), b"new".to_vec()),
            (b"c".to_vec(), b"visible".to_vec())
        ]
    );
}

#[test]
fn l0_get_uses_newest_overlapping_file() {
    let path = fresh_dir("l0_get_uses_newest_overlapping_file");
    let db = DB::open(&path, Options::default()).expect("open db");

    db.put(b"k", b"old").expect("put old");
    db.flush().expect("flush old l0");
    db.put(b"k", b"new").expect("put new");
    db.flush().expect("flush new l0");

    assert_eq!(db.get(b"k").expect("get newest l0"), Some(b"new".to_vec()));
}

#[test]
fn lower_level_get_searches_one_file_per_level() {
    let path = fresh_dir("lower_level_get_searches_one_file_per_level");
    install_level_table(
        &path,
        1,
        vec![
            put(b"a", 3, b"left"),
            put(b"m", 2, b"middle"),
            put(b"z", 1, b"right"),
        ],
    );

    let db = DB::open(&path, Options::default()).expect("open db");

    assert_eq!(
        db.get(b"m").expect("get level table"),
        Some(b"middle".to_vec())
    );
    assert_eq!(db.get(b"missing").expect("get missing"), None);
}

#[test]
fn scan_merges_memtables_l0_and_levels() {
    let path = fresh_dir("scan_merges_memtables_l0_and_levels");
    install_level_table(
        &path,
        1,
        vec![
            put(b"a", 3, b"level-a"),
            put(b"b", 2, b"level-b"),
            put(b"d", 1, b"level-d"),
        ],
    );
    let db = DB::open(&path, Options::default()).expect("open db");

    db.put(b"b", b"l0-b").expect("put l0 b");
    db.flush().expect("flush l0");
    db.put(b"c", b"mem-c").expect("put mem c");
    db.delete(b"d").expect("delete d");

    assert_eq!(
        db.scan(Included(b"a".as_slice()), Excluded(b"e".as_slice()))
            .expect("scan merged"),
        vec![
            (b"a".to_vec(), b"level-a".to_vec()),
            (b"b".to_vec(), b"l0-b".to_vec()),
            (b"c".to_vec(), b"mem-c".to_vec())
        ]
    );
}

#[test]
fn bloom_filter_has_no_false_negatives() {
    let path = fresh_dir("bloom_filter_has_no_false_negatives").join("000007.sst");
    let keys = [b"alpha".as_slice(), b"beta".as_slice(), b"gamma".as_slice()];
    build_table(
        &path,
        keys.iter()
            .enumerate()
            .map(|(index, key)| put(key, (index + 1) as u64, b"value"))
            .collect(),
    );

    let reader = SSTableReader::open(&path).expect("open table");

    for key in keys {
        assert!(reader.might_contain(key), "filter must not hide {key:?}");
        assert_eq!(
            reader.get(key, 10).expect("get filtered key"),
            Some(ValueRecord::Put(b"value".to_vec()))
        );
    }
}

#[test]
fn block_cache_records_hits_after_repeated_get() {
    let path = fresh_dir("block_cache_records_hits_after_repeated_get");
    let db = DB::open(&path, Options::default()).expect("open db");

    db.put(b"cache-key", b"cache-value").expect("put cache key");
    db.flush().expect("flush table");

    assert_eq!(
        db.get(b"cache-key").expect("first get"),
        Some(b"cache-value".to_vec())
    );
    let after_first = db.block_cache_stats();
    assert_eq!(
        db.get(b"cache-key").expect("second get"),
        Some(b"cache-value".to_vec())
    );
    let after_second = db.block_cache_stats();

    assert!(after_second.hits > after_first.hits);
}

#[test]
fn scan_with_fill_cache_false_does_not_populate_block_cache() {
    let path = fresh_dir("scan_with_fill_cache_false_does_not_populate_block_cache");
    let db = DB::open(&path, Options::default()).expect("open db");

    db.put(b"scan-cache-key", b"scan-cache-value").expect("put");
    db.flush().expect("flush table");

    let before_scan = db.block_cache_stats();
    assert_eq!(
        db.scan_opt(
            Unbounded,
            Unbounded,
            ReadOptions {
                fill_cache: false,
                ..ReadOptions::default()
            },
        )
        .expect("scan without filling cache"),
        vec![(b"scan-cache-key".to_vec(), b"scan-cache-value".to_vec())]
    );
    let after_scan = db.block_cache_stats();
    assert_eq!(after_scan, before_scan);

    assert_eq!(
        db.get(b"scan-cache-key").expect("get after scan"),
        Some(b"scan-cache-value".to_vec())
    );
    let after_get = db.block_cache_stats();
    assert!(after_get.misses > after_scan.misses);
}

fn install_level_table(path: &Path, level: usize, entries: Vec<(InternalKey, ValueRecord)>) {
    let mut versions = VersionSet::create(path, Options::default()).expect("create versions");
    let number = versions.allocate_file_number();
    let table_path = path.join(table_file_name(number));
    build_table(&table_path, entries.clone());
    let meta = file_meta(number, &table_path, &entries);

    versions
        .log_and_apply(VersionEdit::AddFile { level, meta })
        .expect("add level file");
    versions
        .log_and_apply(VersionEdit::LastSequence(
            entries
                .iter()
                .map(|(key, _)| key.sequence())
                .max()
                .expect("non-empty entries"),
        ))
        .expect("set last sequence");
}

fn build_table(path: &Path, mut entries: Vec<(InternalKey, ValueRecord)>) {
    entries.sort_by(|(left, _), (right, _)| left.cmp(right));
    let mut builder = SSTableBuilder::create(path, 64).expect("create builder");
    for (key, value) in entries {
        builder.add(key, &value).expect("add entry");
    }
    builder.finish().expect("finish table");
}

fn file_meta(number: u64, path: &Path, entries: &[(InternalKey, ValueRecord)]) -> FileMeta {
    let smallest = entries
        .iter()
        .map(|(key, _)| key)
        .min()
        .expect("smallest key")
        .clone();
    let largest = entries
        .iter()
        .map(|(key, _)| key)
        .max()
        .expect("largest key")
        .clone();
    let smallest_seq = entries
        .iter()
        .map(|(key, _)| key.sequence())
        .min()
        .expect("smallest seq");
    let largest_seq = entries
        .iter()
        .map(|(key, _)| key.sequence())
        .max()
        .expect("largest seq");

    FileMeta {
        number,
        file_size: fs::metadata(path).expect("table metadata").len(),
        smallest,
        largest,
        smallest_seq,
        largest_seq,
    }
}

fn put(key: &[u8], sequence: u64, value: &[u8]) -> (InternalKey, ValueRecord) {
    (
        InternalKey::new(key.to_vec(), sequence, ValueType::Put),
        ValueRecord::Put(value.to_vec()),
    )
}

fn del(key: &[u8], sequence: u64) -> (InternalKey, ValueRecord) {
    (
        InternalKey::new(key.to_vec(), sequence, ValueType::Delete),
        ValueRecord::Delete,
    )
}
