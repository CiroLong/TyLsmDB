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
#[path = "watermark_tests.rs"]
mod tests;
