use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::error::Result;
use crate::table::SSTableReader;

#[derive(Debug, Clone)]
pub struct TableCache {
    inner: Arc<Mutex<TableCacheInner>>,
}

#[derive(Debug)]
struct TableCacheInner {
    tables: HashMap<u64, Arc<SSTableReader>>,
    max_entries: usize,
}

impl TableCache {
    pub fn new(max_entries: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(TableCacheInner {
                tables: HashMap::new(),
                max_entries: max_entries.max(1),
            })),
        }
    }

    pub fn get_or_open(&self, number: u64, path: &Path) -> Result<Arc<SSTableReader>> {
        if let Some(table) = self
            .inner
            .lock()
            .expect("table cache lock poisoned")
            .tables
            .get(&number)
            .cloned()
        {
            return Ok(table);
        }

        let table = Arc::new(SSTableReader::open(path)?);
        let mut inner = self.inner.lock().expect("table cache lock poisoned");
        if inner.tables.len() >= inner.max_entries
            && let Some(key) = inner.tables.keys().next().copied()
        {
            inner.tables.remove(&key);
        }
        inner.tables.insert(number, Arc::clone(&table));
        Ok(table)
    }
}
