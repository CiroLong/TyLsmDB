use crate::key::{InternalKey, ValueType};
use crate::memtable::ValueRecord;
use crate::util::coding::{put_var_u32, put_var_u64};

#[derive(Debug)]
pub struct BlockBuilder {
    data: Vec<u8>,
    restarts: Vec<u32>,
    last_user_key: Vec<u8>,
    entry_count: usize,
    restart_interval: usize,
}

impl BlockBuilder {
    pub fn new(restart_interval: usize) -> Self {
        Self {
            data: Vec::new(),
            restarts: Vec::new(),
            last_user_key: Vec::new(),
            entry_count: 0,
            restart_interval: restart_interval.max(1),
        }
    }

    pub fn add(&mut self, key: InternalKey, value: &ValueRecord) {
        let user_key = key.user_key();
        let shared_len = if self.entry_count.is_multiple_of(self.restart_interval) {
            self.restarts.push(self.data.len() as u32);
            0
        } else {
            shared_prefix_len(&self.last_user_key, user_key)
        };
        let unshared = &user_key[shared_len..];
        let (value_type, value_bytes) = match value {
            ValueRecord::Put(value) => (ValueType::Put, value.as_slice()),
            ValueRecord::Delete => (ValueType::Delete, &[][..]),
        };

        put_var_u32(&mut self.data, shared_len as u32);
        put_var_u32(&mut self.data, unshared.len() as u32);
        put_var_u32(&mut self.data, value_bytes.len() as u32);
        self.data.push(value_type as u8);
        put_var_u64(&mut self.data, key.sequence());
        self.data.extend_from_slice(unshared);
        self.data.extend_from_slice(value_bytes);

        self.last_user_key = user_key.to_vec();
        self.entry_count += 1;
    }

    pub fn is_empty(&self) -> bool {
        self.entry_count == 0
    }

    pub fn approximate_size(&self) -> usize {
        self.data.len() + self.restarts.len() * std::mem::size_of::<u32>() + 4
    }

    pub fn finish(mut self) -> Vec<u8> {
        if self.restarts.is_empty() {
            self.restarts.push(0);
        }
        for offset in &self.restarts {
            self.data.extend_from_slice(&offset.to_le_bytes());
        }
        self.data
            .extend_from_slice(&(self.restarts.len() as u32).to_le_bytes());
        self.data
    }
}

fn shared_prefix_len(left: &[u8], right: &[u8]) -> usize {
    left.iter()
        .zip(right.iter())
        .take_while(|(left, right)| left == right)
        .count()
}
