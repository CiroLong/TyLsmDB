use crate::key::{InternalKey, ValueType};
use crate::options::Options;
use crate::version::{FileMeta, Version};

use super::CompactionPicker;

#[test]
fn picker_scores_l0_by_file_count_and_level_by_bytes() {
    let mut version = Version::new(4);
    version
        .add_file(0, file_meta(1, b"a", b"c", 10))
        .expect("add l0 1");
    version
        .add_file(0, file_meta(2, b"d", b"f", 10))
        .expect("add l0 2");
    version
        .add_file(1, file_meta(3, b"g", b"z", 300))
        .expect("add l1");

    let options = Options {
        level0_file_num_compaction_trigger: 4,
        max_bytes_for_level_base: 200,
        ..Options::default()
    };
    let picker = CompactionPicker::new(options);

    let task = picker.pick(&version).expect("pick level compaction");

    assert_eq!(task.input_level, 1);
    assert_eq!(task.output_level, 2);
    assert_eq!(task.input_files.len(), 1);
}

fn file_meta(number: u64, smallest: &[u8], largest: &[u8], file_size: u64) -> FileMeta {
    FileMeta {
        number,
        file_size,
        smallest: InternalKey::new(smallest.to_vec(), 1, ValueType::Put),
        largest: InternalKey::new(largest.to_vec(), 1, ValueType::Put),
        smallest_seq: 1,
        largest_seq: 1,
    }
}
