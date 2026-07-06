use crate::util::bloom::{BloomFilter, build_bloom_filter};

#[derive(Debug, Clone)]
pub struct TableFilter {
    bloom: Option<BloomFilter>,
}

impl TableFilter {
    pub fn from_keys<'a>(keys: impl IntoIterator<Item = &'a [u8]>) -> Vec<u8> {
        build_bloom_filter(keys)
    }

    pub fn decode(bytes: &[u8]) -> Self {
        Self {
            bloom: BloomFilter::from_bytes(bytes),
        }
    }

    pub fn may_contain(&self, key: &[u8]) -> bool {
        self.bloom
            .as_ref()
            .is_none_or(|filter| filter.may_contain(key))
    }
}
