use std::ops::Bound;

use crate::compact::leveled::max_bytes_for_level;
use crate::compact::task::CompactionTask;
use crate::options::Options;
use crate::version::{FileMeta, Version};

#[derive(Debug, Clone)]
pub struct CompactionPicker {
    options: Options,
}

impl CompactionPicker {
    pub fn new(options: Options) -> Self {
        Self { options }
    }

    pub fn pick(&self, version: &Version) -> Option<CompactionTask> {
        let (level, score) = self.best_score(version)?;
        if score < 1.0 {
            return None;
        }
        self.pick_level(version, level, false)
    }

    pub fn pick_manual(
        &self,
        version: &Version,
        lower: Bound<&[u8]>,
        upper: Bound<&[u8]>,
    ) -> Option<CompactionTask> {
        if let Some(task) = self.pick_l0_manual(version, &lower, &upper) {
            return Some(task);
        }

        for level in 1..version.levels.len().saturating_sub(1) {
            let Some(file) = version.levels[level]
                .iter()
                .find(|file| file_overlaps_range(file, &lower, &upper))
                .cloned()
            else {
                continue;
            };
            return Some(self.task_for_files(version, level, vec![file], true));
        }

        None
    }

    fn best_score(&self, version: &Version) -> Option<(usize, f64)> {
        let l0_trigger = self.options.level0_file_num_compaction_trigger.max(1) as f64;
        let mut best = (0, version.l0_files.len() as f64 / l0_trigger);

        for level in 1..version.levels.len().saturating_sub(1) {
            let bytes: u64 = version.levels[level]
                .iter()
                .map(|file| file.file_size)
                .sum();
            let score = bytes as f64 / max_bytes_for_level(&self.options, level).max(1) as f64;
            if score > best.1 {
                best = (level, score);
            }
        }

        Some(best)
    }

    fn pick_level(
        &self,
        version: &Version,
        level: usize,
        is_manual: bool,
    ) -> Option<CompactionTask> {
        if level == 0 {
            let files = version.l0_files.clone();
            if files.is_empty() {
                return None;
            }
            return Some(self.task_for_files(version, 0, files, is_manual));
        }

        let file = version.levels.get(level)?.first()?.clone();
        Some(self.task_for_files(version, level, vec![file], is_manual))
    }

    fn pick_l0_manual(
        &self,
        version: &Version,
        lower: &Bound<&[u8]>,
        upper: &Bound<&[u8]>,
    ) -> Option<CompactionTask> {
        let files: Vec<_> = version
            .l0_files
            .iter()
            .filter(|file| file_overlaps_range(file, lower, upper))
            .cloned()
            .collect();
        if files.is_empty() {
            return None;
        }
        Some(self.task_for_files(version, 0, files, true))
    }

    fn task_for_files(
        &self,
        version: &Version,
        input_level: usize,
        input_files: Vec<FileMeta>,
        is_manual: bool,
    ) -> CompactionTask {
        let output_level = (input_level + 1).min(version.levels.len().saturating_sub(1));
        let (smallest_user_key, largest_user_key) = user_key_range(&input_files);
        let overlap_files = version
            .levels
            .get(output_level)
            .map(|level| {
                level
                    .iter()
                    .filter(|file| {
                        file.user_key_overlaps(
                            smallest_user_key.as_slice(),
                            largest_user_key.as_slice(),
                        )
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();

        CompactionTask {
            input_level,
            output_level,
            input_files,
            overlap_files,
            smallest_user_key,
            largest_user_key,
            is_manual,
        }
    }
}

trait UserKeyOverlap {
    fn user_key_overlaps(&self, smallest: &[u8], largest: &[u8]) -> bool;
}

impl UserKeyOverlap for FileMeta {
    fn user_key_overlaps(&self, smallest: &[u8], largest: &[u8]) -> bool {
        self.smallest.user_key() <= largest && self.largest.user_key() >= smallest
    }
}

fn user_key_range(files: &[FileMeta]) -> (Vec<u8>, Vec<u8>) {
    let smallest = files
        .iter()
        .map(|file| file.smallest.user_key())
        .min()
        .expect("compaction files are non-empty")
        .to_vec();
    let largest = files
        .iter()
        .map(|file| file.largest.user_key())
        .max()
        .expect("compaction files are non-empty")
        .to_vec();
    (smallest, largest)
}

fn file_overlaps_range(file: &FileMeta, lower: &Bound<&[u8]>, upper: &Bound<&[u8]>) -> bool {
    let lower_ok = match lower {
        Bound::Included(bound) => file.largest.user_key() >= *bound,
        Bound::Excluded(bound) => file.largest.user_key() > *bound,
        Bound::Unbounded => true,
    };
    let upper_ok = match upper {
        Bound::Included(bound) => file.smallest.user_key() <= *bound,
        Bound::Excluded(bound) => file.smallest.user_key() < *bound,
        Bound::Unbounded => true,
    };
    lower_ok && upper_ok
}
