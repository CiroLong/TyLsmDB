use crate::error::Result;
use crate::key::InternalKey;
use crate::memtable::ValueRecord;
use crate::table::block::Block;

#[derive(Debug)]
pub struct BlockIterator {
    entries: Vec<(InternalKey, ValueRecord)>,
    index: usize,
}

impl BlockIterator {
    pub fn new(bytes: Vec<u8>) -> Result<Self> {
        let block = Block::decode(&bytes)?;
        Ok(Self {
            entries: block.entries().to_vec(),
            index: 0,
        })
    }

    pub fn seek_to_first(&mut self) {
        self.index = 0;
    }

    pub fn seek(&mut self, target: &InternalKey) -> Result<()> {
        self.index = self
            .entries
            .partition_point(|(key, _)| key < target)
            .min(self.entries.len());
        Ok(())
    }

    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<()> {
        if self.index < self.entries.len() {
            self.index += 1;
        }
        Ok(())
    }

    pub fn is_valid(&self) -> bool {
        self.index < self.entries.len()
    }

    pub fn key(&self) -> Option<&InternalKey> {
        self.entries.get(self.index).map(|(key, _)| key)
    }

    pub fn value(&self) -> Option<&ValueRecord> {
        self.entries.get(self.index).map(|(_, value)| value)
    }
}
