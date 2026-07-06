use crate::error::{Error, Result};
use crate::key::{InternalKey, ValueType};
use crate::memtable::ValueRecord;
use crate::table::format::decode_value_type;
use crate::util::coding::{get_var_u32, get_var_u64, take_bytes};

#[derive(Debug, Clone)]
pub struct Block {
    entries: Vec<(InternalKey, ValueRecord)>,
}

impl Block {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 4 {
            return Err(Error::Corruption("block missing restart count".to_string()));
        }

        let restart_count_pos = bytes.len() - 4;
        let restart_count = u32::from_le_bytes(
            bytes[restart_count_pos..]
                .try_into()
                .expect("restart count"),
        ) as usize;
        let restart_bytes = restart_count
            .checked_mul(4)
            .ok_or_else(|| Error::Corruption("restart count overflow".to_string()))?;
        if restart_count_pos < restart_bytes {
            return Err(Error::Corruption("invalid restart section".to_string()));
        }
        let data_end = restart_count_pos - restart_bytes;

        let mut src = &bytes[..data_end];
        let mut last_user_key = Vec::new();
        let mut entries = Vec::new();

        while !src.is_empty() {
            let shared_len = get_var_u32(&mut src)? as usize;
            let unshared_len = get_var_u32(&mut src)? as usize;
            let value_len = get_var_u32(&mut src)? as usize;
            let value_type = decode_value_type(*take_bytes(&mut src, 1)?.first().expect("type"))?;
            let sequence = get_var_u64(&mut src)?;
            if shared_len > last_user_key.len() {
                return Err(Error::Corruption(
                    "block entry shared prefix is too long".to_string(),
                ));
            }
            let unshared = take_bytes(&mut src, unshared_len)?;
            let value = take_bytes(&mut src, value_len)?;

            let mut user_key = last_user_key[..shared_len].to_vec();
            user_key.extend_from_slice(unshared);
            last_user_key = user_key.clone();

            let record = match value_type {
                ValueType::Put => ValueRecord::Put(value.to_vec()),
                ValueType::Delete => {
                    if !value.is_empty() {
                        return Err(Error::Corruption(
                            "delete block entry carries value bytes".to_string(),
                        ));
                    }
                    ValueRecord::Delete
                }
            };
            entries.push((InternalKey::new(user_key, sequence, value_type), record));
        }

        Ok(Self { entries })
    }

    pub fn entries(&self) -> &[(InternalKey, ValueRecord)] {
        &self.entries
    }
}
