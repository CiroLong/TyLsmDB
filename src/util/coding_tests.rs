use super::{get_var_u32, get_var_u64, put_var_u32, put_var_u64};

#[test]
fn varint_u64_roundtrips_common_boundaries() {
    for value in [0, 1, 127, 128, 16_384, u64::MAX] {
        let mut encoded = Vec::new();
        put_var_u64(&mut encoded, value);
        let mut input = encoded.as_slice();
        assert_eq!(get_var_u64(&mut input).expect("decode varint"), value);
        assert!(input.is_empty());
    }
}

#[test]
fn varint_u32_roundtrips_common_boundaries() {
    for value in [0, 1, 127, 128, 16_384, u32::MAX] {
        let mut encoded = Vec::new();
        put_var_u32(&mut encoded, value);
        let mut input = encoded.as_slice();
        assert_eq!(get_var_u32(&mut input).expect("decode varint"), value);
        assert!(input.is_empty());
    }
}
