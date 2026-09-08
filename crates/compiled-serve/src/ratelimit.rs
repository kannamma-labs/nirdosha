//! A fixed-window, per-process, per-IP rate limiter — `rfcs/0010`'s own
//! plan text is explicit that this is the right scope for now: a
//! fleet-wide limiter is real, separate follow-up work, the same
//! category as the durability log's own local-SQLite-only limit
//! (`docs/adr/0009`), not silently assumed away.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct RateLimiter {
    windows: Mutex<HashMap<IpAddr, (Instant, u32)>>,
}

impl RateLimiter {
    pub fn new() -> Self {
        RateLimiter { windows: Mutex::new(HashMap::new()) }
    }

    /// `true` if this call is allowed under `max_per_window` calls per
    /// `window` — a real fixed window (not sliding), reset the first
    /// time a call arrives after the previous window's own start plus
    /// `window` has elapsed. Simpler than a sliding window and
    /// sufficient for the stated goal (bound worst-case brute-force
    /// throughput on `/auth/login`-shaped routes), not a precision
    /// traffic-shaping tool.
    pub fn check(&self, key: IpAddr, max_per_window: u32, window: Duration) -> bool {
        let mut windows = self.windows.lock().unwrap();
        let now = Instant::now();
        let entry = windows.entry(key).or_insert((now, 0));
        if now.duration_since(entry.0) >= window {
            *entry = (now, 0);
        }
        if entry.1 >= max_per_window {
            return false;
        }
        entry.1 += 1;
        true
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip() -> IpAddr {
        "127.0.0.1".parse().unwrap()
    }

    #[test]
    fn calls_within_the_limit_are_allowed() {
        let limiter = RateLimiter::new();
        for _ in 0..5 {
            assert!(limiter.check(ip(), 5, Duration::from_secs(60)));
        }
    }

    #[test]
    fn the_call_past_the_limit_within_one_window_is_denied() {
        let limiter = RateLimiter::new();
        for _ in 0..5 {
            assert!(limiter.check(ip(), 5, Duration::from_secs(60)));
        }
        assert!(!limiter.check(ip(), 5, Duration::from_secs(60)));
    }

    #[test]
    fn a_new_window_resets_the_count() {
        let limiter = RateLimiter::new();
        for _ in 0..5 {
            assert!(limiter.check(ip(), 5, Duration::from_millis(20)));
        }
        assert!(!limiter.check(ip(), 5, Duration::from_millis(20)));
        std::thread::sleep(Duration::from_millis(30));
        assert!(limiter.check(ip(), 5, Duration::from_millis(20)));
    }

    #[test]
    fn different_ips_have_independent_windows() {
        let limiter = RateLimiter::new();
        let a: IpAddr = "10.0.0.1".parse().unwrap();
        let b: IpAddr = "10.0.0.2".parse().unwrap();
        for _ in 0..5 {
            assert!(limiter.check(a, 5, Duration::from_secs(60)));
        }
        assert!(!limiter.check(a, 5, Duration::from_secs(60)));
        assert!(limiter.check(b, 5, Duration::from_secs(60)));
    }
}
