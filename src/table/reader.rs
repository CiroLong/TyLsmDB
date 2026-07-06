use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::key::InternalKey;
use crate::memtable::ValueRecord;
use crate::table::block::Block;
use crate::table::builder::IndexEntry;
use crate::table::format::{
    BLOCK_TRAILER_SIZE, BlockHandle, CompressionType, FOOTER_SIZE, decode_footer,
    decode_internal_key,
};
use crate::table::properties::TableProperties;
use crate::util::coding::{get_var_u32, get_var_u64};
use crate::util::crc::crc32c;

#[derive(Debug)]
pub struct SSTableReader {
    path: PathBuf,
    index: Vec<IndexEntry>,
    properties: TableProperties,
}

impl SSTableReader {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let mut file = File::open(&path)?;
        let file_len = file.metadata()?.len();
        if file_len < FOOTER_SIZE as u64 {
            return Err(Error::Corruption("SSTable is too small".to_string()));
        }
        file.seek(SeekFrom::Start(file_len - FOOTER_SIZE as u64))?;
        let mut footer = vec![0_u8; FOOTER_SIZE];
        file.read_exact(&mut footer)?;
        let (properties_handle, index_handle) = decode_footer(&footer)?;

        let properties_block = read_block(&mut file, properties_handle)?;
        let properties = TableProperties::decode(&properties_block)?;
        let index_block = read_block(&mut file, index_handle)?;
        let index = decode_index_entries(&index_block)?;

        Ok(Self {
            path,
            index,
            properties,
        })
    }

    pub fn get(&self, user_key: &[u8], read_seq: u64) -> Result<Option<ValueRecord>> {
        for (key, value) in self.entries()? {
            if key.user_key() == user_key && key.sequence() <= read_seq {
                return Ok(Some(value));
            }
        }
        Ok(None)
    }

    pub fn iter(&self) -> Result<TableIterator> {
        Ok(TableIterator {
            entries: self.entries()?,
            index: 0,
        })
    }

    pub fn smallest_key(&self) -> Option<&InternalKey> {
        self.properties.smallest_key.as_ref()
    }

    pub fn largest_key(&self) -> Option<&InternalKey> {
        self.properties.largest_key.as_ref()
    }

    pub(crate) fn entries(&self) -> Result<Vec<(InternalKey, ValueRecord)>> {
        let mut file = File::open(&self.path)?;
        let mut entries = Vec::new();
        for index in &self.index {
            let block = read_block(&mut file, index.handle)?;
            entries.extend_from_slice(Block::decode(&block)?.entries());
        }
        Ok(entries)
    }
}

#[derive(Debug)]
pub struct TableIterator {
    entries: Vec<(InternalKey, ValueRecord)>,
    index: usize,
}

impl Iterator for TableIterator {
    type Item = (InternalKey, ValueRecord);

    fn next(&mut self) -> Option<Self::Item> {
        let item = self.entries.get(self.index).cloned();
        if item.is_some() {
            self.index += 1;
        }
        item
    }
}

pub(crate) fn read_block(file: &mut File, handle: BlockHandle) -> Result<Vec<u8>> {
    file.seek(SeekFrom::Start(handle.offset))?;
    let mut block = vec![0_u8; handle.size as usize];
    file.read_exact(&mut block)?;

    let mut trailer = [0_u8; BLOCK_TRAILER_SIZE];
    file.read_exact(&mut trailer)?;
    let compression = CompressionType::from_u8(trailer[0])?;
    if compression != CompressionType::None {
        return Err(Error::Corruption(
            "compressed block is unsupported".to_string(),
        ));
    }
    let expected = u32::from_le_bytes(trailer[1..5].try_into().expect("checksum bytes"));
    let mut checksum_input = Vec::with_capacity(1 + block.len());
    checksum_input.push(trailer[0]);
    checksum_input.extend_from_slice(&block);
    let actual = crc32c(&checksum_input);
    if expected != actual {
        return Err(Error::Corruption(
            "SSTable block checksum mismatch".to_string(),
        ));
    }

    Ok(block)
}

fn decode_index_entries(bytes: &[u8]) -> Result<Vec<IndexEntry>> {
    let mut src = bytes;
    let count = get_var_u32(&mut src)?;
    let mut entries = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let first_key = decode_internal_key(&mut src)?;
        let last_key = decode_internal_key(&mut src)?;
        let offset = get_var_u64(&mut src)?;
        let size = get_var_u64(&mut src)?;
        entries.push(IndexEntry {
            first_key,
            last_key,
            handle: BlockHandle::new(offset, size),
        });
    }
    if !src.is_empty() {
        return Err(Error::Corruption("trailing index bytes".to_string()));
    }
    Ok(entries)
}
