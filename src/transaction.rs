#[derive(Debug, Clone, Default)]
pub struct TransactionOptions {
    pub read_only: bool,
}

#[derive(Debug)]
pub struct Transaction {
    read_seq: u64,
}

impl Transaction {
    #[allow(dead_code)]
    pub(crate) fn new(read_seq: u64) -> Self {
        Self { read_seq }
    }

    pub fn read_seq(&self) -> u64 {
        self.read_seq
    }
}
