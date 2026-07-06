use crate::error::Result;
use crate::key::InternalKey;
use crate::table::format::{decode_internal_key, encode_internal_key};
use crate::util::coding::{get_var_u32, get_var_u64, put_var_u32, put_var_u64, take_bytes};

#[derive(Debug, Clone)]
pub struct TableProperties {
    pub num_entries: u64,
    pub smallest_key: Option<InternalKey>,
    pub largest_key: Option<InternalKey>,
    pub filter: Vec<u8>,
}

impl TableProperties {
    pub fn encode(&self) -> Vec<u8> {
        let mut dst = Vec::new();
        put_var_u64(&mut dst, self.num_entries);
        encode_optional_key(self.smallest_key.as_ref(), &mut dst);
        encode_optional_key(self.largest_key.as_ref(), &mut dst);
        put_var_u32(&mut dst, self.filter.len() as u32);
        dst.extend_from_slice(&self.filter);
        dst
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut src = bytes;
        let num_entries = get_var_u64(&mut src)?;
        let smallest_key = decode_optional_key(&mut src)?;
        let largest_key = decode_optional_key(&mut src)?;
        let filter = if src.is_empty() {
            Vec::new()
        } else {
            let filter_len = get_var_u32(&mut src)? as usize;
            take_bytes(&mut src, filter_len)?.to_vec()
        };
        Ok(Self {
            num_entries,
            smallest_key,
            largest_key,
            filter,
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
