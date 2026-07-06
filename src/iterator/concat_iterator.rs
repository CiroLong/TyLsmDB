use crate::error::Result;
use crate::key::InternalKey;
use crate::memtable::ValueRecord;

use super::storage_iterator::StorageIterator;

pub struct ConcatIterator {
    children: Vec<Box<dyn StorageIterator>>,
    current: usize,
}

impl ConcatIterator {
    pub fn new(children: Vec<Box<dyn StorageIterator>>) -> Self {
        let mut iter = Self {
            children,
            current: 0,
        };
        iter.skip_empty();
        iter
    }

    fn skip_empty(&mut self) {
        while self.current < self.children.len() && !self.children[self.current].is_valid() {
            self.current += 1;
        }
    }
}

impl StorageIterator for ConcatIterator {
    fn is_valid(&self) -> bool {
        self.current < self.children.len() && self.children[self.current].is_valid()
    }

    fn key(&self) -> &InternalKey {
        self.children[self.current].key()
    }

    fn value(&self) -> &ValueRecord {
        self.children[self.current].value()
    }

    fn next(&mut self) -> Result<()> {
        if self.current < self.children.len() {
            self.children[self.current].next()?;
            self.skip_empty();
        }
        Ok(())
    }

    fn seek(&mut self, key: &InternalKey) -> Result<()> {
        for (index, child) in self.children.iter_mut().enumerate() {
            child.seek(key)?;
            if child.is_valid() {
                self.current = index;
                return Ok(());
            }
        }
        self.current = self.children.len();
        Ok(())
    }
}
