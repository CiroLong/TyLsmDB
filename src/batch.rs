use crate::bytes::Bytes;
use crate::error::{Error, Result};
use crate::util::coding::{get_var_u32, get_var_u64, put_var_u32, put_var_u64, take_bytes};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchRecord {
    Put { key: Bytes, value: Bytes },
    Delete { key: Bytes },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WriteBatch {
    records: Vec<BatchRecord>,
}

impl WriteBatch {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn put(&mut self, key: impl Into<Bytes>, value: impl Into<Bytes>) {
        self.records.push(BatchRecord::Put {
            key: key.into(),
            value: value.into(),
        });
    }

    pub fn delete(&mut self, key: impl Into<Bytes>) {
        self.records.push(BatchRecord::Delete { key: key.into() });
    }

    pub fn records(&self) -> &[BatchRecord] {
        &self.records
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn encode_with_sequence(&self, start_sequence: u64) -> Vec<u8> {
        let mut dst = Vec::new();
        put_var_u32(&mut dst, self.records.len() as u32);
        put_var_u64(&mut dst, start_sequence);

        for record in &self.records {
            match record {
                BatchRecord::Put { key, value } => {
                    dst.push(1);
                    put_var_u32(&mut dst, key.len() as u32);
                    dst.extend_from_slice(key);
                    put_var_u32(&mut dst, value.len() as u32);
                    dst.extend_from_slice(value);
                }
                BatchRecord::Delete { key } => {
                    dst.push(2);
                    put_var_u32(&mut dst, key.len() as u32);
                    dst.extend_from_slice(key);
                    put_var_u32(&mut dst, 0);
                }
            }
        }

        dst
    }

    pub fn decode_payload(payload: &[u8]) -> Result<(u64, WriteBatch)> {
        let mut src = payload;
        let count = get_var_u32(&mut src)?;
        let start_sequence = get_var_u64(&mut src)?;
        let mut batch = WriteBatch::new();

        for _ in 0..count {
            let value_type = *take_bytes(&mut src, 1)?
                .first()
                .ok_or_else(|| Error::Corruption("missing value type".to_string()))?;
            let key_len = get_var_u32(&mut src)? as usize;
            let key = take_bytes(&mut src, key_len)?.to_vec();
            let value_len = get_var_u32(&mut src)? as usize;
            let value = take_bytes(&mut src, value_len)?.to_vec();

            match value_type {
                1 => batch.put(key, value),
                2 => {
                    if !value.is_empty() {
                        return Err(Error::Corruption(
                            "delete record must not carry a value".to_string(),
                        ));
                    }
                    batch.delete(key);
                }
                _ => return Err(Error::Corruption("unknown batch record type".to_string())),
            }
        }

        if !src.is_empty() {
            return Err(Error::Corruption(
                "trailing batch payload bytes".to_string(),
            ));
        }

        Ok((start_sequence, batch))
    }
}
