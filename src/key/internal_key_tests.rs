use super::{InternalKey, ValueType};

#[test]
fn internal_key_orders_user_key_asc_and_sequence_desc() {
    let mut keys = [
        InternalKey::new(b"a".to_vec(), 7, ValueType::Put),
        InternalKey::new(b"a".to_vec(), 9, ValueType::Put),
        InternalKey::new(b"b".to_vec(), 1, ValueType::Put),
    ];

    keys.sort();

    assert_eq!(keys[0].sequence(), 9);
    assert_eq!(keys[1].sequence(), 7);
    assert_eq!(keys[2].user_key(), b"b");
}
