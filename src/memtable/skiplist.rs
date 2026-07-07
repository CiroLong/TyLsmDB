use std::cmp::Ordering;
use std::ops::Bound;
use std::sync::Mutex;

use crossbeam_skiplist::SkipMap;

use crate::key::{InternalKey, SequenceNumber, ValueType};

use super::arena::{Arena, ArenaBytes};
use super::btree::ValueRecord;

#[derive(Debug)]
pub struct SkipListMemTable {
    map: SkipMap<ArenaInternalKey, ArenaValueRecord>,
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
            ArenaInternalKey::new(key, seq, ValueType::Put),
            ArenaValueRecord::Put(value),
        );
    }

    pub fn delete(&self, seq: SequenceNumber, key: Vec<u8>) {
        let key = self
            .arena
            .lock()
            .expect("arena lock poisoned")
            .allocate(&key);
        self.map.insert(
            ArenaInternalKey::new(key, seq, ValueType::Delete),
            ArenaValueRecord::Delete,
        );
    }

    pub fn get(&self, key: &[u8], read_seq: SequenceNumber) -> Option<ValueRecord> {
        let seek_key = ArenaInternalKey::new(ArenaBytes::from(key), read_seq, ValueType::Put);
        for entry in self.map.range(seek_key..) {
            match entry.key().user_key().cmp(key) {
                Ordering::Equal => return Some(entry.value().to_value_record()),
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
            .map(|entry| {
                (
                    entry.key().to_internal_key(),
                    entry.value().to_value_record(),
                )
            })
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct ArenaInternalKey {
    user_key: ArenaBytes,
    sequence: SequenceNumber,
    value_type: ValueType,
}

impl ArenaInternalKey {
    fn new(user_key: ArenaBytes, sequence: SequenceNumber, value_type: ValueType) -> Self {
        Self {
            user_key,
            sequence,
            value_type,
        }
    }

    fn user_key(&self) -> &[u8] {
        &self.user_key
    }

    fn to_internal_key(&self) -> InternalKey {
        InternalKey::new(self.user_key.to_vec(), self.sequence, self.value_type)
    }
}

impl Ord for ArenaInternalKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.user_key
            .cmp(&other.user_key)
            .then_with(|| other.sequence.cmp(&self.sequence))
            .then_with(|| (self.value_type as u8).cmp(&(other.value_type as u8)))
    }
}

impl PartialOrd for ArenaInternalKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ArenaValueRecord {
    Put(ArenaBytes),
    Delete,
}

impl ArenaValueRecord {
    fn to_value_record(&self) -> ValueRecord {
        match self {
            Self::Put(value) => ValueRecord::Put(value.to_vec()),
            Self::Delete => ValueRecord::Delete,
        }
    }
}
