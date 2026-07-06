use crate::error::{Error, Result};
use crate::key::{InternalKey, ValueType};
use crate::util::coding::{get_var_u32, get_var_u64, put_var_u32, put_var_u64, take_bytes};

pub const TABLE_MAGIC: u64 = 0x5459_4c53_4d44_4201;
pub const FOOTER_SIZE: usize = 40;
pub const BLOCK_TRAILER_SIZE: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionType {
    None = 0,
    Zstd = 1,
}

impl CompressionType {
    pub fn from_u8(value: u8) -> Result<Self> {
        match value {
            0 => Ok(Self::None),
            1 => Ok(Self::Zstd),
            _ => Err(Error::Corruption(
                "unknown block compression type".to_string(),
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockHandle {
    pub offset: u64,
    pub size: u64,
}

impl BlockHandle {
    pub fn new(offset: u64, size: u64) -> Self {
        Self { offset, size }
    }

    pub fn encode_fixed(self, dst: &mut Vec<u8>) {
        dst.extend_from_slice(&self.offset.to_le_bytes());
        dst.extend_from_slice(&self.size.to_le_bytes());
    }

    pub fn decode_fixed(src: &[u8]) -> Result<Self> {
        if src.len() != 16 {
            return Err(Error::Corruption("invalid block handle length".to_string()));
        }
        let offset = u64::from_le_bytes(src[0..8].try_into().expect("offset bytes"));
        let size = u64::from_le_bytes(src[8..16].try_into().expect("size bytes"));
        Ok(Self { offset, size })
    }
}

pub fn encode_internal_key(key: &InternalKey, dst: &mut Vec<u8>) {
    put_var_u32(dst, key.user_key().len() as u32);
    dst.extend_from_slice(key.user_key());
    put_var_u64(dst, key.sequence());
    dst.push(key.value_type() as u8);
}

pub fn decode_internal_key(src: &mut &[u8]) -> Result<InternalKey> {
    let key_len = get_var_u32(src)? as usize;
    let user_key = take_bytes(src, key_len)?.to_vec();
    let sequence = get_var_u64(src)?;
    let value_type = decode_value_type(*take_bytes(src, 1)?.first().expect("value type"))?;
    Ok(InternalKey::new(user_key, sequence, value_type))
}

pub fn decode_value_type(value: u8) -> Result<ValueType> {
    match value {
        1 => Ok(ValueType::Put),
        2 => Ok(ValueType::Delete),
        _ => Err(Error::Corruption("unknown value type".to_string())),
    }
}

pub fn encode_footer(properties: BlockHandle, index: BlockHandle) -> Vec<u8> {
    let mut footer = Vec::with_capacity(FOOTER_SIZE);
    properties.encode_fixed(&mut footer);
    index.encode_fixed(&mut footer);
    footer.extend_from_slice(&TABLE_MAGIC.to_le_bytes());
    footer
}

pub fn decode_footer(src: &[u8]) -> Result<(BlockHandle, BlockHandle)> {
    if src.len() != FOOTER_SIZE {
        return Err(Error::Corruption("invalid table footer length".to_string()));
    }
    let properties = BlockHandle::decode_fixed(&src[0..16])?;
    let index = BlockHandle::decode_fixed(&src[16..32])?;
    let magic = u64::from_le_bytes(src[32..40].try_into().expect("magic bytes"));
    if magic != TABLE_MAGIC {
        return Err(Error::Corruption("invalid table magic".to_string()));
    }
    Ok((properties, index))
}

#[cfg(test)]
mod tests {
    use super::BlockHandle;

    #[test]
    fn block_handle_fixed_roundtrip() {
        let handle = BlockHandle::new(7, 11);
        let mut encoded = Vec::new();
        handle.encode_fixed(&mut encoded);

        assert_eq!(BlockHandle::decode_fixed(&encoded).expect("decode"), handle);
    }
}
