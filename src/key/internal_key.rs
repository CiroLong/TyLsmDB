use std::cmp::Ordering;

pub type SequenceNumber = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueType {
    Put = 1,
    Delete = 2,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InternalKey {
    user_key: Vec<u8>,
    sequence: SequenceNumber,
    value_type: ValueType,
}

impl InternalKey {
    pub fn new(user_key: Vec<u8>, sequence: SequenceNumber, value_type: ValueType) -> Self {
        Self {
            user_key,
            sequence,
            value_type,
        }
    }

    pub fn user_key(&self) -> &[u8] {
        &self.user_key
    }

    pub fn sequence(&self) -> SequenceNumber {
        self.sequence
    }

    pub fn value_type(&self) -> ValueType {
        self.value_type
    }
}

impl Ord for InternalKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.user_key
            .cmp(&other.user_key)
            .then_with(|| other.sequence.cmp(&self.sequence))
            .then_with(|| (self.value_type as u8).cmp(&(other.value_type as u8)))
    }
}

impl PartialOrd for InternalKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
#[path = "internal_key_tests.rs"]
mod tests;
