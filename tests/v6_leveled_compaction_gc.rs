use std::fs;
use std::ops::Bound::{Included, Unbounded};
use std::path::{Path, PathBuf};

use tylsmdb::compact::picker::CompactionPicker;
use tylsmdb::key::{InternalKey, ValueType};
use tylsmdb::version::{FileMeta, Version};
use tylsmdb::{DB, Options};

fn fresh_dir(name: &str) -> PathBuf {
    let path = PathBuf::from("target/tylsmdb-tests").join(name);
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("create fresh test dir");
    path
}

#[test]
fn picker_scores_l0_by_file_count_and_level_by_bytes() {
    let mut version = Version::new(4);
    version
        .add_file(0, file_meta(1, b"a", b"c", 10))
        .expect("add l0 1");
    version
        .add_file(0, file_meta(2, b"d", b"f", 10))
        .expect("add l0 2");
    version
        .add_file(1, file_meta(3, b"g", b"z", 300))
        .expect("add l1");

    let options = Options {
        level0_file_num_compaction_trigger: 4,
        max_bytes_for_level_base: 200,
        ..Options::default()
    };
    let picker = CompactionPicker::new(options);

    let task = picker.pick(&version).expect("pick level compaction");

    assert_eq!(task.input_level, 1);
    assert_eq!(task.output_level, 2);
    assert_eq!(task.input_files.len(), 1);
}

#[test]
fn l0_compaction_preserves_newest_values() {
    let path = fresh_dir("l0_compaction_preserves_newest_values");
    let db = DB::open(&path, Options::default()).expect("open db");

    db.put(b"k", b"old").expect("put old");
    db.flush().expect("flush old");
    db.put(b"k", b"new").expect("put new");
    db.flush().expect("flush new");

    db.compact_range(Unbounded, Unbounded).expect("compact all");

    assert_eq!(db.get(b"k").expect("get compacted"), Some(b"new".to_vec()));
    assert_eq!(db.level_file_counts()[0], 0);
    assert_eq!(db.level_file_counts()[1], 1);
}

#[test]
fn leveled_compaction_outputs_non_overlapping_ranges() {
    let path = fresh_dir("leveled_compaction_outputs_non_overlapping_ranges");
    let db = DB::open(&path, Options::default()).expect("open db");

    for (key, value) in [
        (b"a".as_slice(), b"1".as_slice()),
        (b"m", b"2"),
        (b"z", b"3"),
    ] {
        db.put(key, value).expect("put");
        db.flush().expect("flush");
    }

    db.compact_range(Unbounded, Unbounded).expect("compact l0");

    let counts = db.level_file_counts();
    assert_eq!(counts[0], 0);
    assert_eq!(counts[1], 1);
    assert_eq!(
        db.scan(Unbounded, Unbounded).expect("scan compacted"),
        vec![
            (b"a".to_vec(), b"1".to_vec()),
            (b"m".to_vec(), b"2".to_vec()),
            (b"z".to_vec(), b"3".to_vec())
        ]
    );
}

#[test]
fn tombstone_drops_when_no_lower_level_overlap() {
    let path = fresh_dir("tombstone_drops_when_no_lower_level_overlap");
    let db = DB::open(&path, Options::default()).expect("open db");

    db.put(b"gone", b"value").expect("put");
    db.flush().expect("flush value");
    db.delete(b"gone").expect("delete");
    db.flush().expect("flush tombstone");

    db.compact_range(Unbounded, Unbounded)
        .expect("compact tombstone");

    assert_eq!(db.get(b"gone").expect("get gone"), None);
    assert_eq!(
        db.scan(Unbounded, Unbounded).expect("scan"),
        Vec::<(Vec<u8>, Vec<u8>)>::new()
    );
}

#[test]
fn manual_compact_range_reduces_overlapping_files() {
    let path = fresh_dir("manual_compact_range_reduces_overlapping_files");
    let db = DB::open(&path, Options::default()).expect("open db");

    for value in [b"1".as_slice(), b"2", b"3"] {
        db.put(b"k", value).expect("put overlapping");
        db.flush().expect("flush overlapping");
    }
    assert!(db.level_file_counts()[0] >= 3);

    db.compact_range(Included(b"k".as_slice()), Included(b"k".as_slice()))
        .expect("manual compact k");

    assert_eq!(db.get(b"k").expect("get k"), Some(b"3".to_vec()));
    assert_eq!(db.level_file_counts()[0], 0);
}

#[test]
fn obsolete_files_are_deleted_after_publish() {
    let path = fresh_dir("obsolete_files_are_deleted_after_publish");
    let db = DB::open(&path, Options::default()).expect("open db");

    for value in [b"old".as_slice(), b"new"] {
        db.put(b"k", value).expect("put");
        db.flush().expect("flush");
    }
    let before = sst_files(&path).len();

    db.compact_range(Unbounded, Unbounded).expect("compact");

    let after = sst_files(&path).len();
    assert!(before > after, "obsolete input files should be deleted");
    assert_eq!(after, 1);
}

#[test]
fn write_stall_blocks_when_l0_stop_threshold_is_reached() {
    let path = fresh_dir("write_stall_blocks_when_l0_stop_threshold_is_reached");
    let db = DB::open(
        &path,
        Options {
            level0_stop_writes_trigger: 2,
            level0_slowdown_writes_trigger: 1,
            ..Options::default()
        },
    )
    .expect("open db");

    for value in [b"1".as_slice(), b"2"] {
        db.put(b"k", value).expect("put");
        db.flush().expect("flush");
    }
    assert!(db.level_file_counts()[0] >= 2);

    db.put(b"after-stall", b"ok").expect("write after stall");

    assert_eq!(
        db.get(b"after-stall").expect("get after stall"),
        Some(b"ok".to_vec())
    );
    assert!(
        db.level_file_counts()[0] < 2,
        "write pressure should compact L0 before accepting more writes"
    );
}

fn file_meta(number: u64, smallest: &[u8], largest: &[u8], file_size: u64) -> FileMeta {
    FileMeta {
        number,
        file_size,
        smallest: InternalKey::new(smallest.to_vec(), 1, ValueType::Put),
        largest: InternalKey::new(largest.to_vec(), 1, ValueType::Put),
        smallest_seq: 1,
        largest_seq: 1,
    }
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
