use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;

use crate::error::Result;
use crate::util::crc::crc32c;
use crate::wal::format::WalRecordType;

#[derive(Debug)]
pub struct WalWriter {
    file: File,
}

impl WalWriter {
    pub fn create(path: impl AsRef<Path>) -> Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(path)?;
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

    pub fn sync(&self) -> Result<()> {
        self.file.sync_all()?;
        Ok(())
    }
}
