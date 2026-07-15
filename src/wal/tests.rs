use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::error::Error;

use super::{WalReader, WalWriter};

fn fresh_dir(name: &str) -> PathBuf {
    let path = PathBuf::from("target/tylsmdb-tests").join(name);
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("create fresh test dir");
    path
}

#[test]
fn wal_writer_reader_roundtrips_payloads() {
    let dir = fresh_dir("wal_writer_reader_roundtrips_payloads");
    let path = dir.join("000001.wal");

    let mut writer = WalWriter::create(&path).expect("create wal");
    writer.append(b"first").expect("append first");
    writer.append(b"second").expect("append second");
    writer.sync().expect("sync wal");

    let mut reader = WalReader::open(&path).expect("open reader");
    assert_eq!(
        reader.read_record().expect("read first"),
        Some(b"first".to_vec())
    );
    assert_eq!(
        reader.read_record().expect("read second"),
        Some(b"second".to_vec())
    );
    assert_eq!(reader.read_record().expect("read eof"), None);
}

#[test]
fn wal_reader_ignores_trailing_partial_record() {
    let dir = fresh_dir("wal_reader_ignores_trailing_partial_record");
    let path = dir.join("000001.wal");

    let mut writer = WalWriter::create(&path).expect("create wal");
    writer.append(b"complete").expect("append complete");
    writer.sync().expect("sync wal");
    append_bytes(&path, &[1, 2, 3, 4]);

    let mut reader = WalReader::open(&path).expect("open reader");
    assert_eq!(
        reader.read_record().expect("read complete"),
        Some(b"complete".to_vec())
    );
    assert_eq!(reader.read_record().expect("partial is eof"), None);
}

#[test]
fn wal_reader_rejects_corrupt_complete_record() {
    let dir = fresh_dir("wal_reader_rejects_corrupt_complete_record");
    let path = dir.join("000001.wal");

    let mut writer = WalWriter::create(&path).expect("create wal");
    writer.append(b"payload").expect("append payload");
    writer.sync().expect("sync wal");
    corrupt_first_byte(&path);

    let mut reader = WalReader::open(&path).expect("open reader");
    assert!(matches!(reader.read_record(), Err(Error::Corruption(_))));
}

fn append_bytes(path: &Path, bytes: &[u8]) {
    let mut file = OpenOptions::new()
        .append(true)
        .open(path)
        .expect("open file for append");
    file.write_all(bytes).expect("append bytes");
}

fn corrupt_first_byte(path: &Path) {
    let mut file = OpenOptions::new()
        .write(true)
        .open(path)
        .expect("open file for corruption");
    file.seek(SeekFrom::Start(0)).expect("seek to start");
    file.write_all(&[0xff]).expect("corrupt byte");
}
