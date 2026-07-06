use std::collections::BTreeMap;
use std::fs;
use std::ops::Bound::Unbounded;
use std::path::PathBuf;

use proptest::prelude::*;
use tylsmdb::memtable::MemTableKind;
use tylsmdb::table::format::CompressionType;
use tylsmdb::{DB, Options};

fn fresh_dir(name: &str) -> PathBuf {
    let path = PathBuf::from("target/tylsmdb-tests").join(name);
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("create fresh test dir");
    path
}

#[test]
fn deterministic_operations_match_btreemap_oracle() {
    let path = fresh_dir("deterministic_operations_match_btreemap_oracle");
    let mut db = DB::open(&path, test_options()).expect("open db");
    let mut oracle = BTreeMap::<Vec<u8>, Vec<u8>>::new();

    for step in 0..150 {
        let key = format!("k-{:02}", (step * 17) % 31).into_bytes();
        let value = format!("value-{step:03}").into_bytes();
        match step % 9 {
            0..=3 => {
                db.put(&key, &value).expect("put");
                oracle.insert(key, value);
            }
            4 => {
                db.delete(&key).expect("delete");
                oracle.remove(&key);
            }
            5 => {
                assert_eq!(db.get(&key).expect("get"), oracle.get(&key).cloned());
            }
            6 => assert_full_scan_matches(&db, &oracle),
            7 => db.flush().expect("flush"),
            _ => {
                db.compact_range(Unbounded, Unbounded).expect("compact");
                drop(db);
                db = DB::open(&path, test_options()).expect("reopen");
            }
        }
        assert_full_scan_matches(&db, &oracle);
    }
}

#[test]
fn put_delete_survives_reopen_flush_and_compact() {
    let path = fresh_dir("put_delete_survives_reopen_flush_and_compact");
    let mut db = DB::open(&path, test_options()).expect("open db");
    let mut oracle = BTreeMap::<Vec<u8>, Vec<u8>>::new();

    for (op, key_id, value_id) in [
        (0, 0, 0),
        (0, 0, 0),
        (0, 0, 31),
        (1, 9, 35),
        (2, 7, 1),
        (1, 8, 41),
        (2, 8, 34),
        (6, 6, 46),
        (6, 0, 36),
        (5, 1, 59),
        (6, 3, 60),
    ] {
        let key = format!("rk-{key_id:02}").into_bytes();
        let value = format!("rv-{value_id:02}").into_bytes();
        match op {
            0 | 1 => {
                db.put(&key, &value).expect("put");
                oracle.insert(key, value);
            }
            2 => {
                db.delete(&key).expect("delete");
                oracle.remove(&key);
            }
            5 => db.flush().expect("flush"),
            6 => {
                db.compact_range(Unbounded, Unbounded).expect("compact");
                drop(db);
                db = DB::open(&path, test_options()).expect("reopen");
            }
            _ => unreachable!("test sequence only uses write/flush/compact"),
        }
        assert_full_scan_matches(&db, &oracle);
    }
}

fn assert_full_scan_matches(db: &DB, oracle: &BTreeMap<Vec<u8>, Vec<u8>>) {
    assert_eq!(
        db.scan(Unbounded, Unbounded).expect("scan"),
        oracle
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<Vec<_>>()
    );
}

fn test_options() -> Options {
    Options {
        memtable_kind: MemTableKind::SkipList,
        table_compression: CompressionType::Zstd,
        memtable_size: 256,
        block_size: 256,
        target_file_size_base: 512,
        max_subcompactions: 3,
        ..Options::default()
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(16))]

    #[test]
    fn random_operations_match_btreemap_oracle(ops in proptest::collection::vec((0_u8..7, 0_u8..16, 0_u8..64), 1..48)) {
        let path = fresh_dir("random_operations_match_btreemap_oracle");
        let mut db = DB::open(&path, test_options()).expect("open db");
        let mut oracle = BTreeMap::<Vec<u8>, Vec<u8>>::new();

        for (op, key_id, value_id) in ops {
            let key = format!("rk-{key_id:02}").into_bytes();
            let value = format!("rv-{value_id:02}").into_bytes();
            match op {
                0 | 1 => {
                    db.put(&key, &value).expect("put");
                    oracle.insert(key, value);
                }
                2 => {
                    db.delete(&key).expect("delete");
                    oracle.remove(&key);
                }
                3 => {
                    prop_assert_eq!(db.get(&key).expect("get"), oracle.get(&key).cloned());
                }
                4 => assert_full_scan_matches(&db, &oracle),
                5 => db.flush().expect("flush"),
                _ => {
                    db.compact_range(Unbounded, Unbounded).expect("compact");
                    drop(db);
                    db = DB::open(&path, test_options()).expect("reopen");
                }
            }
            assert_full_scan_matches(&db, &oracle);
        }
    }
}
