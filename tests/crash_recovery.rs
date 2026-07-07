use std::fs::{self, OpenOptions};
use std::io::Write;
use std::ops::Bound::Unbounded;
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

use tylsmdb::env::{Env, FsEnv, ReadableFile, WritableFile, WritableFileOptions};
use tylsmdb::memtable::MemTableKind;
use tylsmdb::table::format::CompressionType;
use tylsmdb::{DB, Options, WriteOptions};

fn fresh_dir(name: &str) -> PathBuf {
    let path = PathBuf::from("target/tylsmdb-tests").join(name);
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("create fresh test dir");
    path
}

#[test]
fn reopen_ignores_half_wal_record_and_leftover_tmp_sstable() {
    let path = fresh_dir("reopen_ignores_half_wal_record_and_leftover_tmp_sstable");
    let db = DB::open(&path, test_options()).expect("open db");

    db.put(b"synced-a", b"1").expect("put synced a");
    db.write(
        {
            let mut batch = tylsmdb::WriteBatch::new();
            batch.put(b"synced-b".to_vec(), b"2".to_vec());
            batch
        },
        WriteOptions {
            sync: true,
            disable_wal: false,
        },
    )
    .expect("sync write");
    db.flush().expect("flush");
    db.put(b"after-flush", b"3").expect("put after flush");
    db.sync_wal().expect("sync wal");
    drop(db);

    let wal_path = latest_wal_file(&path);
    let mut wal = OpenOptions::new()
        .append(true)
        .open(&wal_path)
        .expect("open current wal");
    wal.write_all(&[0xaa, 0xbb, 0xcc])
        .expect("append partial wal");
    fs::write(path.join("000099.sst.tmp"), b"partial table").expect("write tmp sst");

    let reopened = DB::open(&path, test_options()).expect("reopen");

    assert_eq!(
        reopened.get(b"synced-a").expect("get synced a"),
        Some(b"1".to_vec())
    );
    assert_eq!(
        reopened.get(b"synced-b").expect("get synced b"),
        Some(b"2".to_vec())
    );
    assert_eq!(
        reopened.get(b"after-flush").expect("get after flush"),
        Some(b"3".to_vec())
    );
    assert_eq!(
        reopened.scan(Unbounded, Unbounded).expect("scan"),
        vec![
            (b"after-flush".to_vec(), b"3".to_vec()),
            (b"synced-a".to_vec(), b"1".to_vec()),
            (b"synced-b".to_vec(), b"2".to_vec())
        ]
    );
}

fn test_options() -> Options {
    Options {
        memtable_kind: MemTableKind::SkipList,
        table_compression: CompressionType::Zstd,
        ..Options::default()
    }
}

fn latest_wal_file(path: &std::path::Path) -> PathBuf {
    fs::read_dir(path)
        .expect("read db dir")
        .map(|entry| entry.expect("dir entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "wal"))
        .max()
        .expect("current wal file")
}

#[test]
fn injected_wal_append_half_record_is_not_applied_and_is_ignored_on_reopen() {
    let path = fresh_dir("injected_wal_append_half_record_is_not_applied_and_is_ignored_on_reopen");
    let env = Arc::new(FaultEnv::default());
    let db = DB::open(&path, test_options_with_env(env.clone())).expect("open db");

    db.write(sync_put(b"durable", b"1"), sync_write_options())
        .expect("write durable");
    env.arm(FaultPoint::WalAppendHalfRecord);
    assert!(
        db.write(sync_put(b"half-wal", b"lost"), sync_write_options())
            .is_err()
    );
    assert_eq!(db.get(b"half-wal").expect("get failed write"), None);
    drop(db);

    let reopened = DB::open(&path, test_options()).expect("reopen");
    assert_eq!(
        reopened.get(b"durable").expect("get durable"),
        Some(b"1".to_vec())
    );
    assert_eq!(reopened.get(b"half-wal").expect("get half wal"), None);
    assert_scan_sorted_unique(&reopened);
}

#[test]
fn injected_wal_sync_failures_keep_previous_synced_data_consistent() {
    for fault in [FaultPoint::WalSyncBefore, FaultPoint::WalSyncAfter] {
        let path = fresh_dir(&format!(
            "injected_wal_sync_failures_keep_previous_synced_data_consistent_{fault:?}"
        ));
        let env = Arc::new(FaultEnv::default());
        let db = DB::open(&path, test_options_with_env(env.clone())).expect("open db");

        db.write(sync_put(b"before-sync-failure", b"1"), sync_write_options())
            .expect("write durable");
        env.arm(fault);
        assert!(
            db.write(sync_put(b"sync-error", b"maybe"), sync_write_options())
                .is_err()
        );
        assert_eq!(db.get(b"sync-error").expect("failed write invisible"), None);
        drop(db);

        let reopened = DB::open(&path, test_options()).expect("reopen");
        assert_eq!(
            reopened.get(b"before-sync-failure").expect("get durable"),
            Some(b"1".to_vec())
        );
        assert_scan_sorted_unique(&reopened);
    }
}

#[test]
fn injected_sst_half_file_restores_memtable_and_reopen_recovers_from_wal() {
    let path = fresh_dir("injected_sst_half_file_restores_memtable_and_reopen_recovers_from_wal");
    let env = Arc::new(FaultEnv::default());
    let db = DB::open(&path, test_options_with_env(env.clone())).expect("open db");

    db.put(b"table-key", b"value").expect("put");
    env.arm(FaultPoint::SstWriteHalfFile);
    assert!(db.flush().is_err());
    assert_eq!(
        db.get(b"table-key").expect("memtable restored"),
        Some(b"value".to_vec())
    );
    db.sync_wal().expect("sync restored wal");
    drop(db);

    let reopened = DB::open(&path, test_options()).expect("reopen");
    assert_eq!(
        reopened.get(b"table-key").expect("recovered from wal"),
        Some(b"value".to_vec())
    );
    assert_scan_sorted_unique(&reopened);
}

#[test]
fn injected_sst_synced_before_manifest_failure_leaves_orphan_table_ignored() {
    let path = fresh_dir("injected_sst_synced_before_manifest_failure_leaves_orphan_table_ignored");
    let env = Arc::new(FaultEnv::default());
    let db = DB::open(&path, test_options_with_env(env.clone())).expect("open db");

    db.put(b"manifest-gap", b"value").expect("put");
    env.arm(FaultPoint::SstSyncedBeforeManifest);
    assert!(db.flush().is_err());
    assert_eq!(
        db.get(b"manifest-gap").expect("memtable restored"),
        Some(b"value".to_vec())
    );
    db.sync_wal().expect("sync wal");
    drop(db);

    let reopened = DB::open(&path, test_options()).expect("reopen");
    assert_eq!(
        reopened.get(b"manifest-gap").expect("recovered from wal"),
        Some(b"value".to_vec())
    );
    assert_scan_sorted_unique(&reopened);
}

#[test]
fn injected_manifest_half_record_is_ignored_on_reopen() {
    let path = fresh_dir("injected_manifest_half_record_is_ignored_on_reopen");
    let env = Arc::new(FaultEnv::default());
    let db = DB::open(&path, test_options_with_env(env.clone())).expect("open db");

    db.put(b"manifest-half", b"value").expect("put");
    env.arm(FaultPoint::ManifestAppendHalfRecord);
    assert!(db.flush().is_err());
    db.sync_wal().expect("sync wal");
    drop(db);

    let reopened = DB::open(&path, test_options()).expect("reopen");
    assert_eq!(
        reopened.get(b"manifest-half").expect("recovered from wal"),
        Some(b"value".to_vec())
    );
    assert_scan_sorted_unique(&reopened);
}

#[test]
fn injected_current_rename_failures_are_recoverable() {
    for fault in [
        FaultPoint::CurrentRenameBefore,
        FaultPoint::CurrentRenameAfter,
    ] {
        let path = fresh_dir(&format!(
            "injected_current_rename_failures_are_recoverable_{fault:?}"
        ));
        let env = Arc::new(FaultEnv::default());
        env.arm(fault);
        assert!(DB::open(&path, test_options_with_env(env)).is_err());

        let reopened = DB::open(&path, test_options()).expect("reopen after current fault");
        reopened.put(b"after-current", b"ok").expect("put");
        reopened.sync_wal().expect("sync wal");
        drop(reopened);

        let final_reopen = DB::open(&path, test_options()).expect("final reopen");
        assert_eq!(
            final_reopen.get(b"after-current").expect("get"),
            Some(b"ok".to_vec())
        );
        assert_scan_sorted_unique(&final_reopen);
    }
}

#[test]
fn injected_env_is_used_for_recovery_and_table_reads() {
    let path = fresh_dir("injected_env_is_used_for_recovery_and_table_reads");
    {
        let db = DB::open(&path, test_options()).expect("open db");
        db.put(b"env-read", b"value").expect("put");
        db.flush().expect("flush table");
        db.sync_wal().expect("sync wal");
    }

    let env = Arc::new(FaultEnv::default());
    let reopened = DB::open(&path, test_options_with_env(env.clone())).expect("reopen with env");
    assert_eq!(
        reopened.get(b"env-read").expect("get via injected env"),
        Some(b"value".to_vec())
    );
    assert!(
        env.read_open_count() > 0,
        "recovery and table reads should open readable files through Options.env"
    );
}

fn test_options_with_env(env: Arc<dyn Env>) -> Options {
    Options {
        env,
        ..test_options()
    }
}

fn sync_put(key: &[u8], value: &[u8]) -> tylsmdb::WriteBatch {
    let mut batch = tylsmdb::WriteBatch::new();
    batch.put(key.to_vec(), value.to_vec());
    batch
}

fn sync_write_options() -> WriteOptions {
    WriteOptions {
        sync: true,
        disable_wal: false,
    }
}

fn assert_scan_sorted_unique(db: &DB) {
    let rows = db.scan(Unbounded, Unbounded).expect("scan");
    for pair in rows.windows(2) {
        assert!(pair[0].0 < pair[1].0, "scan keys must be sorted and unique");
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FaultPoint {
    WalAppendHalfRecord,
    WalSyncBefore,
    WalSyncAfter,
    SstWriteHalfFile,
    SstSyncedBeforeManifest,
    ManifestAppendHalfRecord,
    CurrentRenameBefore,
    CurrentRenameAfter,
}

#[derive(Debug, Default)]
struct FaultEnv {
    fs: FsEnv,
    armed: Arc<Mutex<Option<FaultPoint>>>,
    read_opens: Arc<AtomicU64>,
}

impl FaultEnv {
    fn arm(&self, fault: FaultPoint) {
        *self.armed.lock().expect("fault lock poisoned") = Some(fault);
    }

    fn take_matching(&self, path: &std::path::Path, event: FaultEvent) -> Option<FaultPoint> {
        take_matching_fault(&self.armed, path, event)
    }

    fn read_open_count(&self) -> u64 {
        self.read_opens.load(Ordering::Relaxed)
    }
}

impl Env for FaultEnv {
    fn create_dir_all(&self, path: &std::path::Path) -> tylsmdb::Result<()> {
        self.fs.create_dir_all(path)
    }

    fn exists(&self, path: &std::path::Path) -> bool {
        self.fs.exists(path)
    }

    fn open_writable(
        &self,
        path: &std::path::Path,
        options: WritableFileOptions,
    ) -> tylsmdb::Result<Box<dyn WritableFile>> {
        Ok(Box::new(FaultFile {
            path: path.to_path_buf(),
            inner: self.fs.open_writable(path, options)?,
            armed: Arc::clone(&self.armed),
        }))
    }

    fn open_readable(&self, path: &std::path::Path) -> tylsmdb::Result<Box<dyn ReadableFile>> {
        self.read_opens.fetch_add(1, Ordering::Relaxed);
        self.fs.open_readable(path)
    }

    fn read_to_string(&self, path: &std::path::Path) -> tylsmdb::Result<String> {
        self.fs.read_to_string(path)
    }

    fn rename(&self, from: &std::path::Path, to: &std::path::Path) -> tylsmdb::Result<()> {
        if self.take_matching(to, FaultEvent::RenameBefore).is_some() {
            return Err(injected_error("rename before"));
        }
        self.fs.rename(from, to)?;
        if self.take_matching(to, FaultEvent::RenameAfter).is_some() {
            return Err(injected_error("rename after"));
        }
        Ok(())
    }

    fn metadata_len(&self, path: &std::path::Path) -> tylsmdb::Result<u64> {
        self.fs.metadata_len(path)
    }

    fn remove_file(&self, path: &std::path::Path) -> tylsmdb::Result<()> {
        self.fs.remove_file(path)
    }

    fn sync_directory(&self, path: &std::path::Path) -> tylsmdb::Result<()> {
        self.fs.sync_directory(path)
    }
}

#[derive(Debug)]
struct FaultFile {
    path: PathBuf,
    inner: Box<dyn WritableFile>,
    armed: Arc<Mutex<Option<FaultPoint>>>,
}

impl WritableFile for FaultFile {
    fn write_all(&mut self, bytes: &[u8]) -> tylsmdb::Result<()> {
        match take_matching_fault(&self.armed, &self.path, FaultEvent::Write) {
            Some(FaultPoint::WalAppendHalfRecord | FaultPoint::ManifestAppendHalfRecord) => {
                let partial_len = (bytes.len() / 2).max(1).min(bytes.len());
                self.inner.write_all(&bytes[..partial_len])?;
                Err(injected_error("partial write"))
            }
            Some(FaultPoint::SstWriteHalfFile) => {
                let partial_len = (bytes.len() / 2).max(1).min(bytes.len());
                self.inner.write_all(&bytes[..partial_len])?;
                Err(injected_error("partial sst write"))
            }
            Some(FaultPoint::SstSyncedBeforeManifest) => Err(injected_error("before manifest")),
            Some(other) => panic!("unexpected write fault: {other:?}"),
            None => self.inner.write_all(bytes),
        }
    }

    fn sync_all(&mut self) -> tylsmdb::Result<()> {
        match take_matching_fault(&self.armed, &self.path, FaultEvent::SyncBefore) {
            Some(FaultPoint::WalSyncBefore) => return Err(injected_error("wal sync before")),
            Some(other) => panic!("unexpected sync-before fault: {other:?}"),
            None => {}
        }
        self.inner.sync_all()?;
        match take_matching_fault(&self.armed, &self.path, FaultEvent::SyncAfter) {
            Some(FaultPoint::WalSyncAfter) => Err(injected_error("wal sync after")),
            Some(other) => panic!("unexpected sync-after fault: {other:?}"),
            None => Ok(()),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum FaultEvent {
    Write,
    SyncBefore,
    SyncAfter,
    RenameBefore,
    RenameAfter,
}

fn take_matching_fault(
    armed: &Mutex<Option<FaultPoint>>,
    path: &std::path::Path,
    event: FaultEvent,
) -> Option<FaultPoint> {
    let mut armed = armed.lock().expect("fault lock poisoned");
    if armed.is_some_and(|fault| fault_matches(fault, path, event)) {
        armed.take()
    } else {
        None
    }
}

fn fault_matches(fault: FaultPoint, path: &std::path::Path, event: FaultEvent) -> bool {
    match (fault, event) {
        (FaultPoint::WalAppendHalfRecord, FaultEvent::Write) => is_wal(path),
        (FaultPoint::WalSyncBefore, FaultEvent::SyncBefore) => is_wal(path),
        (FaultPoint::WalSyncAfter, FaultEvent::SyncAfter) => is_wal(path),
        (FaultPoint::SstWriteHalfFile, FaultEvent::Write) => is_sst_tmp(path),
        (FaultPoint::SstSyncedBeforeManifest, FaultEvent::Write) => is_manifest(path),
        (FaultPoint::ManifestAppendHalfRecord, FaultEvent::Write) => is_manifest(path),
        (FaultPoint::CurrentRenameBefore, FaultEvent::RenameBefore) => is_current(path),
        (FaultPoint::CurrentRenameAfter, FaultEvent::RenameAfter) => is_current(path),
        _ => false,
    }
}

fn is_wal(path: &std::path::Path) -> bool {
    path.extension().is_some_and(|extension| extension == "wal")
}

fn is_sst_tmp(path: &std::path::Path) -> bool {
    path.file_name()
        .is_some_and(|name| name.to_string_lossy().ends_with(".sst.tmp"))
}

fn is_manifest(path: &std::path::Path) -> bool {
    path.file_name()
        .is_some_and(|name| name.to_string_lossy().starts_with("MANIFEST-"))
}

fn is_current(path: &std::path::Path) -> bool {
    path.file_name().is_some_and(|name| name == "CURRENT")
}

fn injected_error(message: &'static str) -> tylsmdb::Error {
    tylsmdb::Error::Io(std::io::Error::other(format!("injected {message} failure")))
}
