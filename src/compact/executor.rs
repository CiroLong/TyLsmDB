use std::collections::BTreeSet;

use crate::key::{InternalKey, SequenceNumber};
use crate::memtable::ValueRecord;

pub fn compact_entries(
    mut entries: Vec<(InternalKey, ValueRecord)>,
    read_seq: SequenceNumber,
) -> Vec<(InternalKey, ValueRecord)> {
    entries.sort_by(|(left, _), (right, _)| left.cmp(right));

    let mut seen = BTreeSet::new();
    let mut output = Vec::new();
    for (key, value) in entries {
        if key.sequence() > read_seq {
            continue;
        }
        if !seen.insert(key.user_key().to_vec()) {
            continue;
        }
        if let ValueRecord::Put(_) = value {
            output.push((key, value));
        }
    }
    output
}
