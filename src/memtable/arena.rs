use std::sync::Arc;

pub type ArenaBytes = Arc<[u8]>;

#[derive(Debug, Default)]
pub struct Arena {
    chunks: Vec<ArenaBytes>,
    bytes: usize,
}

impl Arena {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn allocate(&mut self, bytes: &[u8]) -> ArenaBytes {
        let owned = ArenaBytes::from(bytes);
        self.bytes += owned.len();
        self.chunks.push(Arc::clone(&owned));
        owned
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }
}

#[cfg(test)]
#[path = "arena_tests.rs"]
mod tests;
