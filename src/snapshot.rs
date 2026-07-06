#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snapshot {
    read_seq: u64,
}

impl Snapshot {
    pub(crate) fn new(read_seq: u64) -> Self {
        Self { read_seq }
    }

    pub fn read_seq(&self) -> u64 {
        self.read_seq
    }
}
