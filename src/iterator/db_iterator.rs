use std::collections::BTreeSet;
use std::ops::Bound;

use crate::bytes::Bytes;
use crate::error::Result;
use crate::key::{InternalKey, SequenceNumber, ValueType};
use crate::memtable::ValueRecord;

use super::storage_iterator::StorageIterator;

pub struct DBIterator {
    inner: Box<dyn StorageIterator>,
    lower: Bound<Bytes>,
    upper: Bound<Bytes>,
    read_seq: SequenceNumber,
    seen: BTreeSet<Bytes>,
    current: Option<(Bytes, Bytes)>,
}

impl DBIterator {
    pub fn new(
        inner: Box<dyn StorageIterator>,
        lower: Bound<Bytes>,
        upper: Bound<Bytes>,
        read_seq: SequenceNumber,
    ) -> Self {
        Self::try_new(inner, lower, upper, read_seq)
            .expect("iterator initialization should not fail")
    }

    pub fn try_new(
        inner: Box<dyn StorageIterator>,
        lower: Bound<Bytes>,
        upper: Bound<Bytes>,
        read_seq: SequenceNumber,
    ) -> Result<Self> {
        let mut iter = Self {
            inner,
            lower,
            upper,
            read_seq,
            seen: BTreeSet::new(),
            current: None,
        };
        iter.seek_to_lower_bound()?;
        iter.advance_to_next_visible()?;
        Ok(iter)
    }

    pub fn is_valid(&self) -> bool {
        self.current.is_some()
    }

    pub fn key(&self) -> Option<&[u8]> {
        self.current.as_ref().map(|(key, _)| key.as_slice())
    }

    pub fn value(&self) -> Option<&[u8]> {
        self.current.as_ref().map(|(_, value)| value.as_slice())
    }

    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<()> {
        if self.current.is_some() {
            self.advance_to_next_visible()?;
        }
        Ok(())
    }

    pub fn collect(&mut self) -> Result<Vec<(Bytes, Bytes)>> {
        let mut rows = Vec::new();
        while self.is_valid() {
            rows.push((
                self.key().expect("valid key").to_vec(),
                self.value().expect("valid value").to_vec(),
            ));
            self.next()?;
        }
        Ok(rows)
    }

    fn advance_to_next_visible(&mut self) -> Result<()> {
        self.current = None;
        while self.inner.is_valid() {
            let internal_key = self.inner.key().clone();
            let value = self.inner.value().clone();
            self.inner.next()?;

            if internal_key.sequence() > self.read_seq {
                continue;
            }
            let user_key = internal_key.user_key().to_vec();
            if !within_bounds(&user_key, &self.lower, &self.upper) {
                continue;
            }
            if !self.seen.insert(user_key.clone()) {
                continue;
            }
            if let ValueRecord::Put(value) = value {
                self.current = Some((user_key, value));
                return Ok(());
            }
        }
        Ok(())
    }

    fn seek_to_lower_bound(&mut self) -> Result<()> {
        let Some(target) = lower_bound_seek_key(&self.lower) else {
            return Ok(());
        };
        self.inner.seek(&target)
    }
}

fn lower_bound_seek_key(lower: &Bound<Bytes>) -> Option<InternalKey> {
    match lower {
        Bound::Included(key) => Some(InternalKey::new(
            key.clone(),
            SequenceNumber::MAX,
            ValueType::Put,
        )),
        Bound::Excluded(key) => Some(InternalKey::new(key.clone(), 0, ValueType::Delete)),
        Bound::Unbounded => None,
    }
}

fn within_bounds(key: &[u8], lower: &Bound<Bytes>, upper: &Bound<Bytes>) -> bool {
    let lower_ok = match lower {
        Bound::Included(bound) => key >= bound.as_slice(),
        Bound::Excluded(bound) => key > bound.as_slice(),
        Bound::Unbounded => true,
    };
    let upper_ok = match upper {
        Bound::Included(bound) => key <= bound.as_slice(),
        Bound::Excluded(bound) => key < bound.as_slice(),
        Bound::Unbounded => true,
    };
    lower_ok && upper_ok
}
