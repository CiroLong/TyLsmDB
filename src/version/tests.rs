use std::fs;
use std::path::PathBuf;

use crate::key::{InternalKey, ValueType};
use crate::options::Options;

use super::{FileMeta, ManifestReader, ManifestWriter, VersionEdit, VersionSet};

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
