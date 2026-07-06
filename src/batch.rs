use crate::bytes::Bytes;

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
}
