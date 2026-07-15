use super::BlockHandle;

#[test]
fn block_handle_fixed_roundtrip() {
    let handle = BlockHandle::new(7, 11);
    let mut encoded = Vec::new();
    handle.encode_fixed(&mut encoded);

    assert_eq!(BlockHandle::decode_fixed(&encoded).expect("decode"), handle);
}
