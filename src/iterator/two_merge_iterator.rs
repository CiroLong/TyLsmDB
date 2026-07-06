use crate::error::Result;
use crate::key::InternalKey;
use crate::memtable::ValueRecord;

use super::merge_iterator::MergeIterator;
use super::storage_iterator::StorageIterator;

pub struct TwoMergeIterator {
    inner: MergeIterator,
}

impl TwoMergeIterator {
    pub fn new(left: Box<dyn StorageIterator>, right: Box<dyn StorageIterator>) -> Self {
        Self {
            inner: MergeIterator::new(vec![left, right]),
        }
    }
}

impl StorageIterator for TwoMergeIterator {
    fn is_valid(&self) -> bool {
        self.inner.is_valid()
    }

    fn key(&self) -> &InternalKey {
        self.inner.key()
    }

    fn value(&self) -> &ValueRecord {
        self.inner.value()
    }

    fn next(&mut self) -> Result<()> {
        self.inner.next()
    }

    fn seek(&mut self, key: &InternalKey) -> Result<()> {
        self.inner.seek(key)
    }
}
