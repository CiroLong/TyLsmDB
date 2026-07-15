use std::fs;
use std::path::{Path, PathBuf};

use crate::key::{InternalKey, ValueType};
use crate::memtable::ValueRecord;

use super::{BlockBuilder, BlockIterator, SSTableBuilder, SSTableReader};

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

fn build_table(path: &Path, mut entries: Vec<(InternalKey, ValueRecord)>) {
    entries.sort_by(|(left, _), (right, _)| left.cmp(right));
    let mut builder = SSTableBuilder::create(path, 64).expect("create builder");
    for (key, value) in entries {
        builder.add(key, &value).expect("add entry");
    }
    builder.finish().expect("finish table");
}

fn put(key: &[u8], sequence: u64, value: &[u8]) -> (InternalKey, ValueRecord) {
    (
        InternalKey::new(key.to_vec(), sequence, ValueType::Put),
        ValueRecord::Put(value.to_vec()),
    )
}
