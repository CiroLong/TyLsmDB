use std::path::Path;

use crate::env::{Env, FsEnv, WritableFile, WritableFileOptions};
use crate::error::Result;
use crate::util::crc::crc32c;
use crate::version::edit::VersionEdit;
use crate::wal::WalReader;
use crate::wal::format::WalRecordType;

#[derive(Debug)]
pub struct ManifestWriter {
    file: Box<dyn WritableFile>,
}

impl ManifestWriter {
    pub fn create(path: impl AsRef<Path>) -> Result<Self> {
        let env = FsEnv;
        Self::create_with_env(&env, path)
    }

    pub fn create_with_env(env: &dyn Env, path: impl AsRef<Path>) -> Result<Self> {
        let file = env.open_writable(path.as_ref(), WritableFileOptions::create())?;
        Ok(Self { file })
    }

    pub fn append_to(path: impl AsRef<Path>) -> Result<Self> {
        let env = FsEnv;
        Self::append_to_with_env(&env, path)
    }

    pub fn append_to_with_env(env: &dyn Env, path: impl AsRef<Path>) -> Result<Self> {
        let file = env.open_writable(path.as_ref(), WritableFileOptions::append())?;
        Ok(Self { file })
    }

    pub fn append(&mut self, edit: &VersionEdit) -> Result<()> {
        let payload = edit.encode();
        let record_type = WalRecordType::Full as u8;
        let mut crc_input = Vec::with_capacity(1 + payload.len());
        crc_input.push(record_type);
        crc_input.extend_from_slice(&payload);
        let checksum = crc32c(&crc_input);

        self.file.write_all(&checksum.to_le_bytes())?;
        self.file.write_all(&(payload.len() as u32).to_le_bytes())?;
        self.file.write_all(&[record_type])?;
        self.file.write_all(&payload)?;
        self.file.sync_all()?;
        Ok(())
    }
}

#[derive(Debug)]
pub struct ManifestReader {
    reader: WalReader,
}

impl ManifestReader {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            reader: WalReader::open(path)?,
        })
    }

    pub fn read_all(&mut self) -> Result<Vec<VersionEdit>> {
        let mut edits = Vec::new();
        while let Some(payload) = self.reader.read_record()? {
            edits.push(VersionEdit::decode(&payload)?);
        }
        Ok(edits)
    }
}
