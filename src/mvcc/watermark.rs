use std::collections::BTreeMap;
use std::sync::Mutex;

#[derive(Debug, Default)]
pub struct Watermark {
    active: Mutex<BTreeMap<u64, usize>>,
}

impl Watermark {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&self, sequence: u64) {
        let mut active = self.active.lock().expect("watermark lock poisoned");
        *active.entry(sequence).or_insert(0) += 1;
    }

    pub fn remove(&self, sequence: u64) {
        let mut active = self.active.lock().expect("watermark lock poisoned");
        let Some(count) = active.get_mut(&sequence) else {
            return;
        };
        *count -= 1;
        if *count == 0 {
            active.remove(&sequence);
        }
    }

    pub fn oldest(&self) -> Option<u64> {
        self.active
            .lock()
            .expect("watermark lock poisoned")
            .first_key_value()
            .map(|(sequence, _)| *sequence)
    }
}

#[cfg(test)]
mod tests {
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
}
