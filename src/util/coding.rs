use crate::error::{Error, Result};

pub fn require_non_empty(input: &[u8], context: &'static str) -> Result<()> {
    if input.is_empty() {
        return Err(Error::InvalidArgument(format!(
            "{context} must not be empty"
        )));
    }
    Ok(())
}

pub fn put_var_u32(dst: &mut Vec<u8>, value: u32) {
    put_var_u64(dst, u64::from(value));
}

pub fn put_var_u64(dst: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        dst.push((value as u8) | 0x80);
        value >>= 7;
    }
    dst.push(value as u8);
}

pub fn get_var_u32(src: &mut &[u8]) -> Result<u32> {
    let value = get_var_u64(src)?;
    u32::try_from(value).map_err(|_| Error::Corruption("varint u32 overflow".to_string()))
}

pub fn get_var_u64(src: &mut &[u8]) -> Result<u64> {
    let mut result = 0_u64;
    for shift in (0..64).step_by(7) {
        let Some((&byte, rest)) = src.split_first() else {
            return Err(Error::Corruption("truncated varint".to_string()));
        };
        *src = rest;

        result |= u64::from(byte & 0x7f) << shift;
        if byte < 0x80 {
            return Ok(result);
        }
    }

    Err(Error::Corruption("varint u64 overflow".to_string()))
}

pub fn take_bytes<'a>(src: &mut &'a [u8], len: usize) -> Result<&'a [u8]> {
    if src.len() < len {
        return Err(Error::Corruption("truncated byte slice".to_string()));
    }
    let (head, tail) = src.split_at(len);
    *src = tail;
    Ok(head)
}

#[cfg(test)]
mod tests {
    use super::{get_var_u32, get_var_u64, put_var_u32, put_var_u64};

    #[test]
    fn varint_u64_roundtrips_common_boundaries() {
        for value in [0, 1, 127, 128, 16_384, u64::MAX] {
            let mut encoded = Vec::new();
            put_var_u64(&mut encoded, value);
            let mut input = encoded.as_slice();
            assert_eq!(get_var_u64(&mut input).expect("decode varint"), value);
            assert!(input.is_empty());
        }
    }

    #[test]
    fn varint_u32_roundtrips_common_boundaries() {
        for value in [0, 1, 127, 128, 16_384, u32::MAX] {
            let mut encoded = Vec::new();
            put_var_u32(&mut encoded, value);
            let mut input = encoded.as_slice();
            assert_eq!(get_var_u32(&mut input).expect("decode varint"), value);
            assert!(input.is_empty());
        }
    }
}
