use std::sync::Arc;

use super::watermark::Watermark;

#[derive(Debug)]
pub struct Snapshot {
    read_seq: u64,
    watermark: Arc<Watermark>,
}

impl Snapshot {
    pub(crate) fn new(read_seq: u64, watermark: Arc<Watermark>) -> Self {
        watermark.add(read_seq);
        Self {
            read_seq,
            watermark,
        }
    }

    pub fn read_seq(&self) -> u64 {
        self.read_seq
    }
}

impl Clone for Snapshot {
    fn clone(&self) -> Self {
        Self::new(self.read_seq, Arc::clone(&self.watermark))
    }
}

impl Drop for Snapshot {
    fn drop(&mut self) {
        self.watermark.remove(self.read_seq);
    }
}
