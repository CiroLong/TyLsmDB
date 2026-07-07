use std::path::Path;

use crate::env::{Env, FsEnv, ReadableFile};
use crate::error::{Error, Result};
use crate::util::crc::crc32c;
use crate::wal::format::{WAL_RECORD_HEADER_SIZE, WalRecordType};

#[derive(Debug)]
pub struct WalReader {
    file: Box<dyn ReadableFile>,
}

impl WalReader {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let env = FsEnv;
        Self::open_with_env(&env, path)
    }

    pub fn open_with_env(env: &dyn Env, path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            file: env.open_readable(path.as_ref())?,
        })
    }

    pub fn read_record(&mut self) -> Result<Option<Vec<u8>>> {
        let mut header = [0_u8; WAL_RECORD_HEADER_SIZE];
        if !read_exact_or_eof(self.file.as_mut(), &mut header)? {
            return Ok(None);
        }

        let expected_checksum = u32::from_le_bytes(header[0..4].try_into().expect("u32 header"));
        let payload_len = u32::from_le_bytes(header[4..8].try_into().expect("len header")) as usize;
        let record_type = WalRecordType::from_u8(header[8])?;

        let mut payload = vec![0_u8; payload_len];
        if !read_exact_or_eof(self.file.as_mut(), &mut payload)? {
            return Ok(None);
        }

        let mut crc_input = Vec::with_capacity(1 + payload.len());
        crc_input.push(record_type as u8);
        crc_input.extend_from_slice(&payload);
        let actual_checksum = crc32c(&crc_input);
        if expected_checksum != actual_checksum {
            return Err(Error::Corruption("WAL checksum mismatch".to_string()));
        }

        Ok(Some(payload))
    }
}

fn read_exact_or_eof(file: &mut dyn ReadableFile, mut dst: &mut [u8]) -> Result<bool> {
    let mut read_any = false;
    while !dst.is_empty() {
        match file.read(dst) {
            Ok(0) => return Ok(false),
            Ok(n) => {
                read_any = true;
                let rest = dst.split_at_mut(n).1;
                dst = rest;
            }
            Err(err) => return Err(err),
        }
    }
    Ok(read_any)
}
