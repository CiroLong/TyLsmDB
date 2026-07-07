use std::path::Path;

use crate::env::{Env, FsEnv, WritableFile, WritableFileOptions};
use crate::error::Result;
use crate::util::crc::crc32c;
use crate::wal::format::WalRecordType;

#[derive(Debug)]
pub struct WalWriter {
    file: Box<dyn WritableFile>,
}

impl WalWriter {
    pub fn create(path: impl AsRef<Path>) -> Result<Self> {
        let env = FsEnv;
        Self::create_with_env(&env, path)
    }

    pub fn create_with_env(env: &dyn Env, path: impl AsRef<Path>) -> Result<Self> {
        let file = env.open_writable(path.as_ref(), WritableFileOptions::append())?;
        Ok(Self { file })
    }

    pub fn append(&mut self, payload: &[u8]) -> Result<()> {
        let record_type = WalRecordType::Full as u8;
        let mut crc_input = Vec::with_capacity(1 + payload.len());
        crc_input.push(record_type);
        crc_input.extend_from_slice(payload);
        let checksum = crc32c(&crc_input);

        self.file.write_all(&checksum.to_le_bytes())?;
        self.file.write_all(&(payload.len() as u32).to_le_bytes())?;
        self.file.write_all(&[record_type])?;
        self.file.write_all(payload)?;
        Ok(())
    }

    pub fn sync(&mut self) -> Result<()> {
        self.file.sync_all()?;
        Ok(())
    }
}
