use std::fs;
use std::ops::Bound::Unbounded;
use std::path::{Path, PathBuf};

use tylsmdb::env::{read_current, set_current};
use tylsmdb::key::{InternalKey, ValueType};
use tylsmdb::version::{FileMeta, ManifestReader, ManifestWriter, VersionEdit, VersionSet};
use tylsmdb::{DB, Options};

fn fresh_dir(name: &str) -> PathBuf {
    let path = PathBuf::from("target/tylsmdb-tests").join(name);
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("create fresh test dir");
    path
}

#[test]
fn manifest_edit_roundtrip() {
    let meta = sample_file_meta(7);
    let edits = vec![
        VersionEdit::NextFileNumber(12),
        VersionEdit::LastSequence(99),
        VersionEdit::LogNumber(3),
        VersionEdit::AddFile {
            level: 0,
            meta: meta.clone(),
        },
        VersionEdit::DeleteFile {
            level: 0,
            number: 7,
        },
    ];

    for edit in edits {
        let encoded = edit.encode();
        assert_eq!(VersionEdit::decode(&encoded).expect("decode edit"), edit);
    }
}

#[test]
fn current_points_to_manifest() {
    let path = fresh_dir("current_points_to_manifest");

    set_current(&path, "MANIFEST-000001").expect("set current");

    assert_eq!(
        fs::read_to_string(path.join("CURRENT")).expect("read current file"),
        "MANIFEST-000001\n"
    );
    assert_eq!(
        read_current(&path).expect("read current helper"),
        "MANIFEST-000001"
    );
}

#[test]
fn manifest_writer_reader_replays_edits() {
    let path = fresh_dir("manifest_writer_reader_replays_edits").join("MANIFEST-000001");
    let edits = vec![
        VersionEdit::NextFileNumber(10),
        VersionEdit::LogNumber(2),
        VersionEdit::AddFile {
            level: 0,
            meta: sample_file_meta(9),
        },
    ];

    {
        let mut writer = ManifestWriter::create(&path).expect("create manifest");
        for edit in &edits {
            writer.append(edit).expect("append edit");
        }
    }

    let mut reader = ManifestReader::open(&path).expect("open manifest");
    assert_eq!(reader.read_all().expect("read edits"), edits);
}

#[test]
fn manifest_rejects_corrupt_complete_record() {
    let path = fresh_dir("manifest_rejects_corrupt_complete_record").join("MANIFEST-000001");
    {
        let mut writer = ManifestWriter::create(&path).expect("create manifest");
        writer
            .append(&VersionEdit::NextFileNumber(4))
            .expect("append edit");
    }

    let mut bytes = fs::read(&path).expect("read manifest");
    bytes[0] ^= 0xff;
    fs::write(&path, bytes).expect("write corrupt manifest");

    let mut reader = ManifestReader::open(&path).expect("open manifest");
    assert!(reader.read_all().is_err());
}

#[test]
fn version_set_replays_manifest_state() {
    let path = fresh_dir("version_set_replays_manifest_state");
    let mut versions = VersionSet::create(&path, Options::default()).expect("create versions");
    let file_number = versions.allocate_file_number();
    let meta = sample_file_meta(file_number);
    versions
        .log_and_apply(VersionEdit::AddFile {
            level: 0,
            meta: meta.clone(),
        })
        .expect("add file");
    versions
        .log_and_apply(VersionEdit::LastSequence(42))
        .expect("set last sequence");

    let recovered = VersionSet::recover(&path, Options::default()).expect("recover versions");

    assert_eq!(recovered.current().l0_files, vec![meta]);
    assert!(recovered.next_file_number() > file_number);
    assert_eq!(recovered.last_sequence(), 42);
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

fn sample_file_meta(number: u64) -> FileMeta {
    FileMeta {
        number,
        file_size: 128,
        smallest: InternalKey::new(b"a".to_vec(), 3, ValueType::Put),
        largest: InternalKey::new(b"z".to_vec(), 1, ValueType::Put),
        smallest_seq: 1,
        largest_seq: 3,
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
