use crate::error::Result;
use crate::key::InternalKey;
use crate::memtable::ValueRecord;

pub trait StorageIterator {
    fn is_valid(&self) -> bool;
    fn key(&self) -> &InternalKey;
    fn value(&self) -> &ValueRecord;
    fn next(&mut self) -> Result<()>;
    fn seek(&mut self, key: &InternalKey) -> Result<()>;
}

#[derive(Debug)]
pub struct EntryIterator {
    entries: Vec<(InternalKey, ValueRecord)>,
    index: usize,
}

impl EntryIterator {
    pub fn new(mut entries: Vec<(InternalKey, ValueRecord)>) -> Self {
        entries.sort_by(|(left, _), (right, _)| left.cmp(right));
        Self { entries, index: 0 }
    }
}

impl StorageIterator for EntryIterator {
    fn is_valid(&self) -> bool {
        self.index < self.entries.len()
    }

    fn key(&self) -> &InternalKey {
        &self.entries[self.index].0
    }

    fn value(&self) -> &ValueRecord {
        &self.entries[self.index].1
    }

    fn next(&mut self) -> Result<()> {
        if self.index < self.entries.len() {
            self.index += 1;
        }
        Ok(())
    }

    fn seek(&mut self, key: &InternalKey) -> Result<()> {
        self.index = self
            .entries
            .partition_point(|(entry_key, _)| entry_key < key);
        Ok(())
    }
}
