use crate::error::Result;
use crate::key::InternalKey;
use crate::memtable::ValueRecord;

use super::storage_iterator::StorageIterator;

pub struct MergeIterator {
    children: Vec<Box<dyn StorageIterator>>,
    current: Option<usize>,
}

impl MergeIterator {
    pub fn new(children: Vec<Box<dyn StorageIterator>>) -> Self {
        let mut iter = Self {
            children,
            current: None,
        };
        iter.pick_current();
        iter
    }

    fn pick_current(&mut self) {
        self.current = self
            .children
            .iter()
            .enumerate()
            .filter(|(_, child)| child.is_valid())
            .min_by(|(left_index, left), (right_index, right)| {
                left.key()
                    .cmp(right.key())
                    .then_with(|| left_index.cmp(right_index))
            })
            .map(|(index, _)| index);
    }
}

impl StorageIterator for MergeIterator {
    fn is_valid(&self) -> bool {
        self.current.is_some()
    }

    fn key(&self) -> &InternalKey {
        self.children[self.current.expect("valid merge iterator")].key()
    }

    fn value(&self) -> &ValueRecord {
        self.children[self.current.expect("valid merge iterator")].value()
    }

    fn next(&mut self) -> Result<()> {
        if let Some(current) = self.current {
            self.children[current].next()?;
        }
        self.pick_current();
        Ok(())
    }

    fn seek(&mut self, key: &InternalKey) -> Result<()> {
        for child in &mut self.children {
            child.seek(key)?;
        }
        self.pick_current();
        Ok(())
    }
}
