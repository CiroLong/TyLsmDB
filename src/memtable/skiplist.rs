use std::cmp::Ordering;
use std::ops::Bound;
use std::sync::Mutex;

use crossbeam_skiplist::SkipMap;

use crate::key::{InternalKey, SequenceNumber, ValueType};

use super::arena::Arena;
use super::btree::ValueRecord;

#[derive(Debug)]
pub struct SkipListMemTable {
    map: SkipMap<InternalKey, ValueRecord>,
    arena: Mutex<Arena>,
}

impl SkipListMemTable {
    pub fn new() -> Self {
        Self {
            map: SkipMap::new(),
            arena: Mutex::new(Arena::new()),
        }
    }

    pub fn put(&self, seq: SequenceNumber, key: Vec<u8>, value: Vec<u8>) {
        let (key, value) = {
            let mut arena = self.arena.lock().expect("arena lock poisoned");
            (arena.allocate(&key), arena.allocate(&value))
        };
        self.map.insert(
            InternalKey::new(key, seq, ValueType::Put),
            ValueRecord::Put(value),
        );
    }

    pub fn delete(&self, seq: SequenceNumber, key: Vec<u8>) {
        let key = self
            .arena
            .lock()
            .expect("arena lock poisoned")
            .allocate(&key);
        self.map.insert(
            InternalKey::new(key, seq, ValueType::Delete),
            ValueRecord::Delete,
        );
    }

    pub fn get(&self, key: &[u8], read_seq: SequenceNumber) -> Option<ValueRecord> {
        let seek_key = InternalKey::new(key.to_vec(), read_seq, ValueType::Put);
        for entry in self.map.range(seek_key..) {
            match entry.key().user_key().cmp(key) {
                Ordering::Equal => return Some(entry.value().clone()),
                Ordering::Greater => break,
                Ordering::Less => {}
            }
        }
        None
    }

    pub fn scan(
        &self,
        lower: Bound<&[u8]>,
        upper: Bound<&[u8]>,
        read_seq: SequenceNumber,
    ) -> Vec<(Vec<u8>, Vec<u8>)> {
        super::visible_scan(self.entries(), lower, upper, read_seq)
    }

    pub fn entries(&self) -> Vec<(InternalKey, ValueRecord)> {
        self.map
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect()
    }

    pub fn approximate_size(&self) -> usize {
        self.arena.lock().expect("arena lock poisoned").bytes()
            + self.map.len() * std::mem::size_of::<InternalKey>()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

impl Default for SkipListMemTable {
    fn default() -> Self {
        Self::new()
    }
}
