use crate::util::coding::{get_var_u32, put_var_u32};

const DEFAULT_BITS_PER_KEY: usize = 10;

#[derive(Debug, Clone)]
pub struct BloomFilter {
    bits: Vec<u8>,
    bit_len: usize,
    probes: u32,
}

impl BloomFilter {
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let (&probes, data) = bytes.split_last()?;
        let mut src = data;
        let bit_len = get_var_u32(&mut src).ok()? as usize;
        Some(Self {
            bits: src.to_vec(),
            bit_len,
            probes: u32::from(probes).max(1),
        })
    }

    pub fn may_contain(&self, key: &[u8]) -> bool {
        if self.bit_len == 0 {
            return true;
        }

        let hash = bloom_hash(key);
        let delta = hash.rotate_right(17);
        for probe in 0..self.probes {
            let bit_pos = hash.wrapping_add(probe.wrapping_mul(delta)) as usize % self.bit_len;
            if self.bits[bit_pos / 8] & (1 << (bit_pos % 8)) == 0 {
                return false;
            }
        }
        true
    }
}

pub fn build_bloom_filter<'a>(keys: impl IntoIterator<Item = &'a [u8]>) -> Vec<u8> {
    let keys: Vec<&[u8]> = keys.into_iter().collect();
    if keys.is_empty() {
        return Vec::new();
    }

    let bit_len = (keys.len() * DEFAULT_BITS_PER_KEY).max(64);
    let byte_len = bit_len.div_ceil(8);
    let probes = ((DEFAULT_BITS_PER_KEY as f64) * 0.69).round() as u32;
    let probes = probes.clamp(1, 30);
    let mut bits = vec![0_u8; byte_len];

    for key in keys {
        let hash = bloom_hash(key);
        let delta = hash.rotate_right(17);
        for probe in 0..probes {
            let bit_pos = hash.wrapping_add(probe.wrapping_mul(delta)) as usize % bit_len;
            bits[bit_pos / 8] |= 1 << (bit_pos % 8);
        }
    }

    let mut encoded = Vec::new();
    put_var_u32(&mut encoded, bit_len as u32);
    encoded.extend_from_slice(&bits);
    encoded.push(probes as u8);
    encoded
}

fn bloom_hash(key: &[u8]) -> u32 {
    let mut hash = 2_166_136_261_u32;
    for byte in key {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(16_777_619);
    }
    hash
}
