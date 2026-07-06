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
mod tests {
    use super::{InternalKey, ValueType};

    #[test]
    fn internal_key_orders_user_key_asc_and_sequence_desc() {
        let mut keys = [
            InternalKey::new(b"a".to_vec(), 7, ValueType::Put),
            InternalKey::new(b"a".to_vec(), 9, ValueType::Put),
            InternalKey::new(b"b".to_vec(), 1, ValueType::Put),
        ];

        keys.sort();

        assert_eq!(keys[0].sequence(), 9);
        assert_eq!(keys[1].sequence(), 7);
        assert_eq!(keys[2].user_key(), b"b");
    }
}
