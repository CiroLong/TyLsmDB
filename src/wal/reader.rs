use std::fs::File;
use std::io::{ErrorKind, Read};
use std::path::Path;

use crate::error::{Error, Result};
use crate::util::crc::crc32c;
use crate::wal::format::{WAL_RECORD_HEADER_SIZE, WalRecordType};

#[derive(Debug)]
pub struct WalReader {
    file: File,
}

impl WalReader {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            file: File::open(path)?,
        })
    }

    pub fn read_record(&mut self) -> Result<Option<Vec<u8>>> {
        let mut header = [0_u8; WAL_RECORD_HEADER_SIZE];
        if !read_exact_or_eof(&mut self.file, &mut header)? {
            return Ok(None);
        }

        let expected_checksum = u32::from_le_bytes(header[0..4].try_into().expect("u32 header"));
        let payload_len = u32::from_le_bytes(header[4..8].try_into().expect("len header")) as usize;
        let record_type = WalRecordType::from_u8(header[8])?;

        let mut payload = vec![0_u8; payload_len];
        if !read_exact_or_eof(&mut self.file, &mut payload)? {
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

fn read_exact_or_eof(file: &mut File, mut dst: &mut [u8]) -> Result<bool> {
    let mut read_any = false;
    while !dst.is_empty() {
        match file.read(dst) {
            Ok(0) => return Ok(false),
            Ok(n) => {
                read_any = true;
                let rest = dst.split_at_mut(n).1;
                dst = rest;
            }
            Err(err) if err.kind() == ErrorKind::UnexpectedEof => return Ok(false),
            Err(err) => return Err(err.into()),
        }
    }
    Ok(read_any)
}
