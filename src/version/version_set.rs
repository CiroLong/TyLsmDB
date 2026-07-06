use std::path::Path;
use std::sync::Arc;

use crate::env::file::{manifest_file_name, parse_manifest_number};
use crate::env::{read_current, set_current};
use crate::error::{Error, Result};
use crate::options::Options;
use crate::version::edit::VersionEdit;
use crate::version::manifest::{ManifestReader, ManifestWriter};
use crate::version::version::Version;

#[derive(Debug)]
pub struct VersionSet {
    current: Arc<Version>,
    next_file_number: u64,
    last_sequence: u64,
    log_number: u64,
    manifest_number: u64,
    manifest_writer: Option<ManifestWriter>,
}

impl VersionSet {
    pub fn new_empty(max_levels: usize) -> Self {
        Self {
            current: Arc::new(Version::new(max_levels)),
            next_file_number: 2,
            last_sequence: 0,
            log_number: 1,
            manifest_number: 1,
            manifest_writer: None,
        }
    }

    pub fn create(db_path: impl AsRef<Path>, options: Options) -> Result<Self> {
        let db_path = db_path.as_ref().to_path_buf();
        std::fs::create_dir_all(&db_path)?;
        let manifest_number = 1;
        let manifest_name = manifest_file_name(manifest_number);
        let manifest_path = db_path.join(&manifest_name);
        let mut manifest_writer = ManifestWriter::create(&manifest_path)?;
        manifest_writer.append(&VersionEdit::NextFileNumber(2))?;
        manifest_writer.append(&VersionEdit::LogNumber(1))?;
        manifest_writer.append(&VersionEdit::LastSequence(0))?;
        set_current(&db_path, &manifest_name)?;

        Ok(Self {
            current: Arc::new(Version::new(options.max_levels)),
            next_file_number: 2,
            last_sequence: 0,
            log_number: 1,
            manifest_number,
            manifest_writer: Some(manifest_writer),
        })
    }

    pub fn recover(db_path: impl AsRef<Path>, options: Options) -> Result<Self> {
        let db_path = db_path.as_ref().to_path_buf();
        let manifest_name = read_current(&db_path)?;
        let manifest_number = parse_manifest_number(&manifest_name)
            .ok_or_else(|| Error::Corruption("CURRENT points to invalid manifest".to_string()))?;
        let manifest_path = db_path.join(&manifest_name);

        let mut versions = Self {
            current: Arc::new(Version::new(options.max_levels)),
            next_file_number: 2,
            last_sequence: 0,
            log_number: 1,
            manifest_number,
            manifest_writer: None,
        };

        let mut reader = ManifestReader::open(&manifest_path)?;
        for edit in reader.read_all()? {
            versions.apply_edit(edit)?;
        }
        versions.manifest_writer = Some(ManifestWriter::append_to(&manifest_path)?);
        Ok(versions)
    }

    pub fn current(&self) -> Arc<Version> {
        Arc::clone(&self.current)
    }

    pub fn next_file_number(&self) -> u64 {
        self.next_file_number
    }

    pub fn last_sequence(&self) -> u64 {
        self.last_sequence
    }

    pub fn log_number(&self) -> u64 {
        self.log_number
    }

    pub fn manifest_number(&self) -> u64 {
        self.manifest_number
    }

    pub fn allocate_file_number(&mut self) -> u64 {
        let number = self.next_file_number;
        self.next_file_number += 1;
        number
    }

    pub fn log_and_apply(&mut self, edit: VersionEdit) -> Result<()> {
        if let Some(writer) = &mut self.manifest_writer {
            writer.append(&edit)?;
        }
        self.apply_edit(edit)
    }

    fn apply_edit(&mut self, edit: VersionEdit) -> Result<()> {
        match edit {
            VersionEdit::NextFileNumber(number) => {
                self.next_file_number = self.next_file_number.max(number);
            }
            VersionEdit::LastSequence(sequence) => {
                self.last_sequence = self.last_sequence.max(sequence);
            }
            VersionEdit::LogNumber(number) => {
                self.log_number = number;
                self.next_file_number = self.next_file_number.max(number + 1);
            }
            VersionEdit::AddFile { level, meta } => {
                self.next_file_number = self.next_file_number.max(meta.number + 1);
                let mut next = (*self.current).clone();
                next.add_file(level, meta)?;
                self.current = Arc::new(next);
            }
            VersionEdit::DeleteFile { level, number } => {
                let mut next = (*self.current).clone();
                next.delete_file(level, number)?;
                self.current = Arc::new(next);
            }
        }
        Ok(())
    }
}
