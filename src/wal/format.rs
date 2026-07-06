use crate::error::{Error, Result};

pub const WAL_RECORD_HEADER_SIZE: usize = 9;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalRecordType {
    Full = 1,
}

impl WalRecordType {
    pub fn from_u8(value: u8) -> Result<Self> {
        match value {
            1 => Ok(Self::Full),
            _ => Err(Error::Corruption("unknown WAL record type".to_string())),
        }
    }
}
