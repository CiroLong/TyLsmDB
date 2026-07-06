use crate::error::{Error, Result};
use crate::table::format::{decode_internal_key, encode_internal_key};
use crate::util::coding::{get_var_u32, get_var_u64, put_var_u32, put_var_u64};
use crate::version::version::FileMeta;

const TAG_NEXT_FILE_NUMBER: u32 = 1;
const TAG_LAST_SEQUENCE: u32 = 2;
const TAG_LOG_NUMBER: u32 = 3;
const TAG_ADD_FILE: u32 = 4;
const TAG_DELETE_FILE: u32 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionEdit {
    NextFileNumber(u64),
    LastSequence(u64),
    LogNumber(u64),
    AddFile { level: usize, meta: FileMeta },
    DeleteFile { level: usize, number: u64 },
}

impl VersionEdit {
    pub fn encode(&self) -> Vec<u8> {
        let mut dst = Vec::new();
        match self {
            Self::NextFileNumber(number) => {
                put_var_u32(&mut dst, TAG_NEXT_FILE_NUMBER);
                put_var_u64(&mut dst, *number);
            }
            Self::LastSequence(sequence) => {
                put_var_u32(&mut dst, TAG_LAST_SEQUENCE);
                put_var_u64(&mut dst, *sequence);
            }
            Self::LogNumber(number) => {
                put_var_u32(&mut dst, TAG_LOG_NUMBER);
                put_var_u64(&mut dst, *number);
            }
            Self::AddFile { level, meta } => {
                put_var_u32(&mut dst, TAG_ADD_FILE);
                put_var_u32(&mut dst, *level as u32);
                encode_file_meta(meta, &mut dst);
            }
            Self::DeleteFile { level, number } => {
                put_var_u32(&mut dst, TAG_DELETE_FILE);
                put_var_u32(&mut dst, *level as u32);
                put_var_u64(&mut dst, *number);
            }
        }
        dst
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut src = bytes;
        let tag = get_var_u32(&mut src)?;
        let edit = match tag {
            TAG_NEXT_FILE_NUMBER => Self::NextFileNumber(get_var_u64(&mut src)?),
            TAG_LAST_SEQUENCE => Self::LastSequence(get_var_u64(&mut src)?),
            TAG_LOG_NUMBER => Self::LogNumber(get_var_u64(&mut src)?),
            TAG_ADD_FILE => {
                let level = get_var_u32(&mut src)? as usize;
                let meta = decode_file_meta(&mut src)?;
                Self::AddFile { level, meta }
            }
            TAG_DELETE_FILE => {
                let level = get_var_u32(&mut src)? as usize;
                let number = get_var_u64(&mut src)?;
                Self::DeleteFile { level, number }
            }
            _ => return Err(Error::Corruption("unknown version edit tag".to_string())),
        };
        if !src.is_empty() {
            return Err(Error::Corruption("trailing version edit bytes".to_string()));
        }
        Ok(edit)
    }
}

fn encode_file_meta(meta: &FileMeta, dst: &mut Vec<u8>) {
    put_var_u64(dst, meta.number);
    put_var_u64(dst, meta.file_size);
    encode_internal_key(&meta.smallest, dst);
    encode_internal_key(&meta.largest, dst);
    put_var_u64(dst, meta.smallest_seq);
    put_var_u64(dst, meta.largest_seq);
}

fn decode_file_meta(src: &mut &[u8]) -> Result<FileMeta> {
    Ok(FileMeta {
        number: get_var_u64(src)?,
        file_size: get_var_u64(src)?,
        smallest: decode_internal_key(src)?,
        largest: decode_internal_key(src)?,
        smallest_seq: get_var_u64(src)?,
        largest_seq: get_var_u64(src)?,
    })
}
