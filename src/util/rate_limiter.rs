use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct RateLimiter {
    bytes_per_sec: u64,
    state: Mutex<RateLimiterState>,
}

#[derive(Debug)]
struct RateLimiterState {
    available: u64,
    last_refill: Instant,
}

impl RateLimiter {
    pub fn new(bytes_per_sec: u64) -> Self {
        Self {
            bytes_per_sec: bytes_per_sec.max(1),
            state: Mutex::new(RateLimiterState {
                available: bytes_per_sec.max(1),
                last_refill: Instant::now(),
            }),
        }
    }

    pub fn reserve(&self, bytes: u64) -> Duration {
        let mut state = self.state.lock().expect("rate limiter lock poisoned");
        let now = Instant::now();
        if now >= state.last_refill {
            let elapsed = now.saturating_duration_since(state.last_refill);
            let refill = (elapsed.as_secs_f64() * self.bytes_per_sec as f64) as u64;
            if refill > 0 {
                state.available = (state.available + refill).min(self.bytes_per_sec);
            }
            state.last_refill = now;
        }

        if bytes <= state.available {
            state.available -= bytes;
            return Duration::ZERO;
        }

        let missing = bytes - state.available;
        state.available = 0;
        let wait = Duration::from_secs_f64(missing as f64 / self.bytes_per_sec as f64);
        state.last_refill += wait;
        state.last_refill.saturating_duration_since(now)
    }
}

#[cfg(test)]
#[path = "rate_limiter_tests.rs"]
mod tests;
