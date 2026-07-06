use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::key::InternalKey;
use crate::memtable::ValueRecord;
use crate::table::block_builder::BlockBuilder;
use crate::table::filter::TableFilter;
use crate::table::format::{
    BLOCK_TRAILER_SIZE, BlockHandle, CompressionType, encode_footer, encode_internal_key,
};
use crate::table::properties::TableProperties;
use crate::util::coding::{put_var_u32, put_var_u64};
use crate::util::crc::crc32c;

#[derive(Debug)]
pub struct SSTableBuilder {
    path: PathBuf,
    file: File,
    offset: u64,
    block_size: usize,
    current_block: BlockBuilder,
    current_block_first_key: Option<InternalKey>,
    index_entries: Vec<IndexEntry>,
    smallest_key: Option<InternalKey>,
    largest_key: Option<InternalKey>,
    filter_keys: Vec<Vec<u8>>,
    num_entries: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct IndexEntry {
    pub first_key: InternalKey,
    pub last_key: InternalKey,
    pub handle: BlockHandle,
}

impl SSTableBuilder {
    pub fn create(path: impl AsRef<Path>, block_size: usize) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&path)?;
        Ok(Self {
            path,
            file,
            offset: 0,
            block_size: block_size.max(1),
            current_block: BlockBuilder::new(16),
            current_block_first_key: None,
            index_entries: Vec::new(),
            smallest_key: None,
            largest_key: None,
            filter_keys: Vec::new(),
            num_entries: 0,
        })
    }

    pub fn add(&mut self, key: InternalKey, value: &ValueRecord) -> Result<()> {
        if self.current_block_first_key.is_none() {
            self.current_block_first_key = Some(key.clone());
        }
        if self.smallest_key.is_none() {
            self.smallest_key = Some(key.clone());
        }
        self.largest_key = Some(key.clone());
        self.filter_keys.push(key.user_key().to_vec());
        self.current_block.add(key, value);
        self.num_entries += 1;

        if self.current_block.approximate_size() >= self.block_size {
            self.flush_current_block()?;
        }
        Ok(())
    }

    pub fn finish(mut self) -> Result<()> {
        self.flush_current_block()?;

        let properties = TableProperties {
            num_entries: self.num_entries,
            smallest_key: self.smallest_key.clone(),
            largest_key: self.largest_key.clone(),
            filter: TableFilter::from_keys(self.filter_keys.iter().map(Vec::as_slice)),
        };
        let properties_block = properties.encode();
        let properties_handle = self.write_block(&properties_block)?;

        let index_block = encode_index_entries(&self.index_entries);
        let index_handle = self.write_block(&index_block)?;

        let footer = encode_footer(properties_handle, index_handle);
        self.file.write_all(&footer)?;
        self.offset += footer.len() as u64;
        self.file.sync_all()?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn flush_current_block(&mut self) -> Result<()> {
        if self.current_block.is_empty() {
            return Ok(());
        }
        let first_key = self
            .current_block_first_key
            .take()
            .expect("non-empty block has first key");
        let last_key = self
            .largest_key
            .clone()
            .expect("non-empty table has largest");
        let block = std::mem::replace(&mut self.current_block, BlockBuilder::new(16)).finish();
        let handle = self.write_block(&block)?;
        self.index_entries.push(IndexEntry {
            first_key,
            last_key,
            handle,
        });
        Ok(())
    }

    fn write_block(&mut self, block: &[u8]) -> Result<BlockHandle> {
        let handle = BlockHandle::new(self.offset, block.len() as u64);
        self.file.write_all(block)?;
        let compression = CompressionType::None as u8;
        let mut checksum_input = Vec::with_capacity(1 + block.len());
        checksum_input.push(compression);
        checksum_input.extend_from_slice(block);
        let checksum = crc32c(&checksum_input);
        self.file.write_all(&[compression])?;
        self.file.write_all(&checksum.to_le_bytes())?;
        self.offset += block.len() as u64 + BLOCK_TRAILER_SIZE as u64;
        Ok(handle)
    }
}

pub(crate) fn encode_index_entries(entries: &[IndexEntry]) -> Vec<u8> {
    let mut dst = Vec::new();
    put_var_u32(&mut dst, entries.len() as u32);
    for entry in entries {
        encode_internal_key(&entry.first_key, &mut dst);
        encode_internal_key(&entry.last_key, &mut dst);
        put_var_u64(&mut dst, entry.handle.offset);
        put_var_u64(&mut dst, entry.handle.size);
    }
    dst
}
