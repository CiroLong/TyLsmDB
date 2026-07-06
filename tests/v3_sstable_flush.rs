use std::fs;
use std::ops::Bound::{Excluded, Included, Unbounded};
use std::path::{Path, PathBuf};

use tylsmdb::key::{InternalKey, ValueType};
use tylsmdb::memtable::ValueRecord;
use tylsmdb::table::{BlockBuilder, BlockIterator, SSTableBuilder, SSTableReader};
use tylsmdb::{DB, Options};

fn fresh_dir(name: &str) -> PathBuf {
    let path = PathBuf::from("target/tylsmdb-tests").join(name);
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("create fresh test dir");
    path
}

#[test]
fn block_roundtrip_with_prefix_compression() {
    let mut builder = BlockBuilder::new(2);
    builder.add(
        InternalKey::new(b"account:001".to_vec(), 3, ValueType::Put),
        &ValueRecord::Put(b"alice".to_vec()),
    );
    builder.add(
        InternalKey::new(b"account:002".to_vec(), 2, ValueType::Put),
        &ValueRecord::Put(b"bob".to_vec()),
    );
    builder.add(
        InternalKey::new(b"account:002".to_vec(), 1, ValueType::Delete),
        &ValueRecord::Delete,
    );

    let block = builder.finish();
    assert!(
        block.len() < 80,
        "prefix compression should keep this compact"
    );

    let mut iter = BlockIterator::new(block).expect("decode block");
    iter.seek_to_first();

    assert_eq!(iter.key().expect("first key").user_key(), b"account:001");
    assert_eq!(
        iter.value().expect("first value"),
        &ValueRecord::Put(b"alice".to_vec())
    );

    iter.next().expect("next");
    assert_eq!(iter.key().expect("second key").user_key(), b"account:002");
    assert_eq!(iter.key().expect("second key").sequence(), 2);

    iter.seek(&InternalKey::new(
        b"account:002".to_vec(),
        u64::MAX,
        ValueType::Put,
    ))
    .expect("seek account:002");
    assert_eq!(iter.key().expect("seek key").user_key(), b"account:002");
}

#[test]
fn table_builder_reader_roundtrip() {
    let path = fresh_dir("table_builder_reader_roundtrip").join("000001.sst");
    build_sample_table(&path);

    let reader = SSTableReader::open(&path).expect("open table");
    assert_eq!(
        reader.get(b"a", 10).expect("get a"),
        Some(ValueRecord::Put(b"new".to_vec()))
    );
    assert_eq!(
        reader.get(b"b", 10).expect("get b"),
        Some(ValueRecord::Delete)
    );
    assert_eq!(reader.get(b"missing", 10).expect("get missing"), None);
    assert_eq!(reader.smallest_key().expect("smallest").user_key(), b"a");
    assert_eq!(reader.largest_key().expect("largest").user_key(), b"c");

    let rows: Vec<_> = reader
        .iter()
        .expect("iter")
        .map(|(key, value)| (key.user_key().to_vec(), key.sequence(), value))
        .collect();
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0].0, b"a");
}

#[test]
fn reader_rejects_corrupt_block_checksum() {
    let path = fresh_dir("reader_rejects_corrupt_block_checksum").join("000001.sst");
    build_sample_table(&path);

    let mut bytes = fs::read(&path).expect("read table");
    bytes[0] ^= 0xff;
    fs::write(&path, bytes).expect("write corrupt table");

    let reader = SSTableReader::open(&path).expect("open table metadata");
    assert!(
        reader.get(b"a", 10).is_err(),
        "corrupt data block checksum should be rejected"
    );
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

fn build_sample_table(path: &Path) {
    let mut builder = SSTableBuilder::create(path, 64).expect("create table builder");
    builder
        .add(
            InternalKey::new(b"a".to_vec(), 3, ValueType::Put),
            &ValueRecord::Put(b"new".to_vec()),
        )
        .expect("add a new");
    builder
        .add(
            InternalKey::new(b"a".to_vec(), 1, ValueType::Put),
            &ValueRecord::Put(b"old".to_vec()),
        )
        .expect("add a old");
    builder
        .add(
            InternalKey::new(b"b".to_vec(), 2, ValueType::Delete),
            &ValueRecord::Delete,
        )
        .expect("add b delete");
    builder
        .add(
            InternalKey::new(b"c".to_vec(), 1, ValueType::Put),
            &ValueRecord::Put(b"value".to_vec()),
        )
        .expect("add c");
    builder.finish().expect("finish table");
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
