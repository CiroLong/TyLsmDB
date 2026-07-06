#[derive(Debug, Default)]
pub struct Arena {
    chunks: Vec<Vec<u8>>,
    bytes: usize,
}

impl Arena {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn allocate(&mut self, bytes: &[u8]) -> Vec<u8> {
        let owned = bytes.to_vec();
        self.bytes += owned.len();
        self.chunks.push(owned.clone());
        owned
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }
}
