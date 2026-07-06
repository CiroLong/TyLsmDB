use crate::version::FileMeta;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactionTask {
    pub input_level: usize,
    pub output_level: usize,
    pub input_files: Vec<FileMeta>,
    pub overlap_files: Vec<FileMeta>,
    pub smallest_user_key: Vec<u8>,
    pub largest_user_key: Vec<u8>,
    pub is_manual: bool,
}

impl CompactionTask {
    pub fn all_input_files(&self) -> impl Iterator<Item = &FileMeta> {
        self.input_files.iter().chain(self.overlap_files.iter())
    }

    pub fn is_trivial_move(&self) -> bool {
        self.input_level > 0 && self.input_files.len() == 1 && self.overlap_files.is_empty()
    }
}
