use std::sync::Arc;

use super::Arena;

#[test]
fn allocated_bytes_are_shared_slices_owned_by_arena() {
    let mut arena = Arena::new();

    let bytes = arena.allocate(b"abc");
    let cloned = bytes.clone();

    assert!(Arc::ptr_eq(&bytes, &cloned));
    assert_eq!(&*bytes, b"abc");
    assert_eq!(arena.bytes(), 3);
}
