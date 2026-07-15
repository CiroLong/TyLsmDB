use super::Watermark;

#[test]
fn oldest_tracks_multiset_entries() {
    let watermark = Watermark::new();

    assert_eq!(watermark.oldest(), None);
    watermark.add(7);
    watermark.add(3);
    watermark.add(3);

    assert_eq!(watermark.oldest(), Some(3));
    watermark.remove(3);
    assert_eq!(watermark.oldest(), Some(3));
    watermark.remove(3);
    assert_eq!(watermark.oldest(), Some(7));
    watermark.remove(7);
    assert_eq!(watermark.oldest(), None);
}
