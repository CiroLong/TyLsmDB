use crate::error::{Error, Result};
use crate::key::InternalKey;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMeta {
    pub number: u64,
    pub file_size: u64,
    pub smallest: InternalKey,
    pub largest: InternalKey,
    pub smallest_seq: u64,
    pub largest_seq: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub l0_files: Vec<FileMeta>,
    pub levels: Vec<Vec<FileMeta>>,
}

impl Version {
    pub fn new(max_levels: usize) -> Self {
        Self {
            l0_files: Vec::new(),
            levels: vec![Vec::new(); max_levels],
        }
    }

    pub fn add_file(&mut self, level: usize, meta: FileMeta) -> Result<()> {
        if level == 0 {
            self.l0_files.insert(0, meta);
            return Ok(());
        }

        let files = self
            .levels
            .get_mut(level)
            .ok_or_else(|| Error::InvalidArgument(format!("invalid level: {level}")))?;
        files.push(meta);
        files.sort_by(|left, right| left.smallest.cmp(&right.smallest));
        Ok(())
    }

    pub fn delete_file(&mut self, level: usize, number: u64) -> Result<()> {
        if level == 0 {
            self.l0_files.retain(|file| file.number != number);
            return Ok(());
        }

        let files = self
            .levels
            .get_mut(level)
            .ok_or_else(|| Error::InvalidArgument(format!("invalid level: {level}")))?;
        files.retain(|file| file.number != number);
        Ok(())
    }
}
