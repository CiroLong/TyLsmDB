use std::cmp::Ordering;

pub fn compare_user_key(left: &[u8], right: &[u8]) -> Ordering {
    left.cmp(right)
}
