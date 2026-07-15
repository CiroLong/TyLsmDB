use std::time::Duration;

use super::RateLimiter;

#[test]
fn rate_limiter_reports_wait_when_budget_is_exhausted() {
    let limiter = RateLimiter::new(10);

    assert_eq!(limiter.reserve(4), Duration::ZERO);
    assert!(limiter.reserve(20) > Duration::ZERO);

    let limiter = RateLimiter::new(10);
    let first_wait = limiter.reserve(30);
    let second_wait = limiter.reserve(10);
    assert!(second_wait > first_wait);
}
