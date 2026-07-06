use crate::error::Result;
use crate::key::InternalKey;
use crate::table::format::{decode_internal_key, encode_internal_key};
use crate::util::coding::{get_var_u64, put_var_u64};

#[derive(Debug, Clone)]
pub struct TableProperties {
    pub num_entries: u64,
    pub smallest_key: Option<InternalKey>,
    pub largest_key: Option<InternalKey>,
}

impl TableProperties {
    pub fn encode(&self) -> Vec<u8> {
        let mut dst = Vec::new();
        put_var_u64(&mut dst, self.num_entries);
        encode_optional_key(self.smallest_key.as_ref(), &mut dst);
        encode_optional_key(self.largest_key.as_ref(), &mut dst);
        dst
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut src = bytes;
        let num_entries = get_var_u64(&mut src)?;
        let smallest_key = decode_optional_key(&mut src)?;
        let largest_key = decode_optional_key(&mut src)?;
        Ok(Self {
            num_entries,
            smallest_key,
            largest_key,
        })
    }
}

fn encode_optional_key(key: Option<&InternalKey>, dst: &mut Vec<u8>) {
    match key {
        Some(key) => {
            dst.push(1);
            encode_internal_key(key, dst);
        }
        None => dst.push(0),
    }
}

fn decode_optional_key(src: &mut &[u8]) -> Result<Option<InternalKey>> {
    let Some((&tag, rest)) = src.split_first() else {
        return Ok(None);
    };
    *src = rest;
    match tag {
        0 => Ok(None),
        _ => Ok(Some(decode_internal_key(src)?)),
    }
}
