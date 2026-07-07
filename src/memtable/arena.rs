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
mod tests {
    use std::sync::Arc;

    use super::Arena;

    #[test]
    fn allocated_bytes_are_shared_slices_owned_by_arena() {
        let mut arena = Arena::new();

        let bytes = arena.allocate(b"abc");
        let cloned = bytes.clone();

        assert!(Arc::ptr_eq(&bytes, &cloned));
        assert_eq!(&*bytes, b"abc");
        assert_eq!(arena.bytes(), 3);
    }
}
