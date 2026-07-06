use std::collections::BTreeSet;

use crate::key::{InternalKey, SequenceNumber};
use crate::memtable::ValueRecord;

pub fn compact_entries(
    mut entries: Vec<(InternalKey, ValueRecord)>,
    gc_watermark: SequenceNumber,
) -> Vec<(InternalKey, ValueRecord)> {
    entries.sort_by(|(left, _), (right, _)| left.cmp(right));

    let mut seen_below_watermark = BTreeSet::new();
    let mut output = Vec::new();
    for (key, value) in entries {
        if key.sequence() > gc_watermark {
            output.push((key, value));
            continue;
        }

        if !seen_below_watermark.insert(key.user_key().to_vec()) {
            continue;
        }
        if let ValueRecord::Put(_) = value {
            output.push((key, value));
        }
    }
    output
}
