use super::{BatchRecord, WriteBatch};

#[test]
fn write_batch_payload_roundtrips_with_start_sequence() {
    let mut batch = WriteBatch::new();
    batch.put(b"a".to_vec(), b"1".to_vec());
    batch.delete(b"b".to_vec());

    let payload = batch.encode_with_sequence(42);
    let (start_sequence, decoded) = WriteBatch::decode_payload(&payload).expect("decode payload");

    assert_eq!(start_sequence, 42);
    assert_eq!(decoded.records(), batch.records());
    assert_eq!(
        decoded.records(),
        &[
            BatchRecord::Put {
                key: b"a".to_vec(),
                value: b"1".to_vec()
            },
            BatchRecord::Delete { key: b"b".to_vec() }
        ]
    );
}
