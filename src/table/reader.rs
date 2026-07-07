use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::cache::BlockCache;
use crate::env::{Env, FsEnv, ReadableFile};
use crate::error::{Error, Result};
use crate::iterator::StorageIterator;
use crate::key::InternalKey;
use crate::memtable::ValueRecord;
use crate::table::block::Block;
use crate::table::builder::IndexEntry;
use crate::table::filter::TableFilter;
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
    env: Arc<dyn Env>,
    index: Vec<IndexEntry>,
    properties: TableProperties,
    filter: TableFilter,
}

impl SSTableReader {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_env(Arc::new(FsEnv), path)
    }

    pub fn open_with_env(env: Arc<dyn Env>, path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let mut file = env.open_readable(&path)?;
        let file_len = file.len()?;
        if file_len < FOOTER_SIZE as u64 {
            return Err(Error::Corruption("SSTable is too small".to_string()));
        }
        file.seek(SeekFrom::Start(file_len - FOOTER_SIZE as u64))?;
        let mut footer = vec![0_u8; FOOTER_SIZE];
        file.read_exact(&mut footer)?;
        let (properties_handle, index_handle) = decode_footer(&footer)?;

        let properties_block = read_block(file.as_mut(), properties_handle)?;
        let properties = TableProperties::decode(&properties_block)?;
        let index_block = read_block(file.as_mut(), index_handle)?;
        let index = decode_index_entries(&index_block)?;

        Ok(Self {
            path,
            env,
            index,
            filter: TableFilter::decode(&properties.filter),
            properties,
        })
    }

    pub fn get(&self, user_key: &[u8], read_seq: u64) -> Result<Option<ValueRecord>> {
        self.get_with_cache(user_key, read_seq, 0, None, false)
    }

    pub fn get_with_cache(
        &self,
        user_key: &[u8],
        read_seq: u64,
        table_number: u64,
        block_cache: Option<&BlockCache>,
        fill_cache: bool,
    ) -> Result<Option<ValueRecord>> {
        if !self.might_contain(user_key) {
            return Ok(None);
        }

        let mut file = self.env.open_readable(&self.path)?;
        for index in &self.index {
            if !index_may_contain_user_key(index, user_key) {
                continue;
            }
            let block = self.read_decoded_block(
                file.as_mut(),
                index,
                table_number,
                block_cache,
                fill_cache,
            )?;
            for (key, value) in block.entries() {
                if key.user_key() == user_key && key.sequence() <= read_seq {
                    return Ok(Some(value.clone()));
                }
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

    pub fn might_contain(&self, user_key: &[u8]) -> bool {
        self.filter.may_contain(user_key)
    }

    pub(crate) fn entries(&self) -> Result<Vec<(InternalKey, ValueRecord)>> {
        self.entries_with_cache(0, None, true)
    }

    pub(crate) fn entries_with_cache(
        &self,
        table_number: u64,
        block_cache: Option<&BlockCache>,
        fill_cache: bool,
    ) -> Result<Vec<(InternalKey, ValueRecord)>> {
        let mut file = self.env.open_readable(&self.path)?;
        let mut entries = Vec::new();
        for index in &self.index {
            let block = self.read_decoded_block(
                file.as_mut(),
                index,
                table_number,
                block_cache,
                fill_cache,
            )?;
            entries.extend_from_slice(block.entries());
        }
        Ok(entries)
    }

    pub(crate) fn storage_iter(
        &self,
        table_number: u64,
        block_cache: Option<BlockCache>,
        fill_cache: bool,
    ) -> Result<SSTableStorageIterator> {
        SSTableStorageIterator::new(
            Arc::clone(&self.env),
            self.path.clone(),
            self.index.clone(),
            table_number,
            block_cache,
            fill_cache,
        )
    }

    fn read_decoded_block(
        &self,
        file: &mut dyn ReadableFile,
        index: &IndexEntry,
        table_number: u64,
        block_cache: Option<&BlockCache>,
        fill_cache: bool,
    ) -> Result<Arc<Block>> {
        if fill_cache
            && let Some(cache) = block_cache
            && let Some(block) = cache.get(table_number, index.handle.offset)
        {
            return Ok(block);
        }

        let block_bytes = read_block(file, index.handle)?;
        let block = Arc::new(Block::decode(&block_bytes)?);
        if fill_cache && let Some(cache) = block_cache {
            cache.insert(table_number, index.handle.offset, Arc::clone(&block));
        }
        Ok(block)
    }
}

#[derive(Debug)]
pub(crate) struct SSTableStorageIterator {
    file: Box<dyn ReadableFile>,
    index: Vec<IndexEntry>,
    table_number: u64,
    block_cache: Option<BlockCache>,
    fill_cache: bool,
    block_index: usize,
    current_block: Option<Arc<Block>>,
    entry_index: usize,
}

impl SSTableStorageIterator {
    fn new(
        env: Arc<dyn Env>,
        path: PathBuf,
        index: Vec<IndexEntry>,
        table_number: u64,
        block_cache: Option<BlockCache>,
        fill_cache: bool,
    ) -> Result<Self> {
        let file = env.open_readable(&path)?;
        let mut iter = Self {
            file,
            index,
            table_number,
            block_cache,
            fill_cache,
            block_index: 0,
            current_block: None,
            entry_index: 0,
        };
        iter.load_block_at(0)?;
        Ok(iter)
    }

    fn load_block_at(&mut self, index: usize) -> Result<()> {
        self.block_index = index;
        self.current_block = None;
        self.entry_index = 0;

        while self.block_index < self.index.len() {
            let block = read_decoded_block(
                self.file.as_mut(),
                &self.index[self.block_index],
                self.table_number,
                self.block_cache.as_ref(),
                self.fill_cache,
            )?;
            if !block.entries().is_empty() {
                self.current_block = Some(block);
                return Ok(());
            }
            self.block_index += 1;
        }
        Ok(())
    }

    fn advance_to_next_block(&mut self) -> Result<()> {
        self.load_block_at(self.block_index + 1)
    }
}

impl StorageIterator for SSTableStorageIterator {
    fn is_valid(&self) -> bool {
        self.current_block
            .as_ref()
            .is_some_and(|block| self.entry_index < block.entries().len())
    }

    fn key(&self) -> &InternalKey {
        &self
            .current_block
            .as_ref()
            .expect("valid table iterator")
            .entries()[self.entry_index]
            .0
    }

    fn value(&self) -> &ValueRecord {
        &self
            .current_block
            .as_ref()
            .expect("valid table iterator")
            .entries()[self.entry_index]
            .1
    }

    fn next(&mut self) -> Result<()> {
        if !self.is_valid() {
            return Ok(());
        }

        self.entry_index += 1;
        if !self.is_valid() {
            self.advance_to_next_block()?;
        }
        Ok(())
    }

    fn seek(&mut self, key: &InternalKey) -> Result<()> {
        let block_index = self
            .index
            .partition_point(|entry| entry.last_key < *key)
            .min(self.index.len());
        self.load_block_at(block_index)?;
        if let Some(block) = &self.current_block {
            self.entry_index = block
                .entries()
                .partition_point(|(entry_key, _)| entry_key < key);
            if self.entry_index >= block.entries().len() {
                self.advance_to_next_block()?;
            }
        }
        Ok(())
    }
}

fn read_decoded_block(
    file: &mut dyn ReadableFile,
    index: &IndexEntry,
    table_number: u64,
    block_cache: Option<&BlockCache>,
    fill_cache: bool,
) -> Result<Arc<Block>> {
    if fill_cache
        && let Some(cache) = block_cache
        && let Some(block) = cache.get(table_number, index.handle.offset)
    {
        return Ok(block);
    }

    let block_bytes = read_block(file, index.handle)?;
    let block = Arc::new(Block::decode(&block_bytes)?);
    if fill_cache && let Some(cache) = block_cache {
        cache.insert(table_number, index.handle.offset, Arc::clone(&block));
    }
    Ok(block)
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

pub(crate) fn read_block(file: &mut dyn ReadableFile, handle: BlockHandle) -> Result<Vec<u8>> {
    file.seek(SeekFrom::Start(handle.offset))?;
    let mut block = vec![0_u8; handle.size as usize];
    file.read_exact(&mut block)?;

    let mut trailer = [0_u8; BLOCK_TRAILER_SIZE];
    file.read_exact(&mut trailer)?;
    let compression = CompressionType::from_u8(trailer[0])?;
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

    match compression {
        CompressionType::None => Ok(block),
        CompressionType::Zstd => zstd::stream::decode_all(block.as_slice()).map_err(Into::into),
    }
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

fn index_may_contain_user_key(index: &IndexEntry, user_key: &[u8]) -> bool {
    user_key >= index.first_key.user_key() && user_key <= index.last_key.user_key()
}
