//! A fixed-window, per-process, per-IP rate limiter — `rfcs/0010`'s own
//! plan text is explicit that this is the right scope for now: a
//! fleet-wide limiter is real, separate follow-up work, the same
//! category as the durability log's own local-SQLite-only limit
//! (`docs/adr/0009`), not silently assumed away.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Hard cap on distinct tracked IPs (red team finding A10,
/// `scratch/red-team-report-main-d7fae42.md`) — without one, a botnet
/// scan or a misconfigured `trusted_proxies`/`X-Forwarded-For`
/// combination feeding many distinct fake IPs grows `windows` without
/// bound, per-process, forever. 10,000 is generous headroom for a rate
/// limiter meant to catch abuse on a handful of sensitive paths
/// (`rate_limited_paths`, this module's own doc comment), not to track
/// every legitimate visitor precisely — far more real distinct abusive
/// IPs than one process needs to remember at once.
const MAX_TRACKED_IPS: usize = 10_000;

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
        // Only a genuinely new key can grow `windows` past its current
        // size -- an existing key's entry gets reused/reset in place
        // below. Evict the single oldest entry (by last-seen `Instant`)
        // to make room, one linear scan over the whole map: O(n) at the
        // cap, an accepted tradeoff at this size rather than maintaining
        // a full LRU structure for what's meant to be an abuse backstop,
        // not a precision cache.
        if !windows.contains_key(&key) && windows.len() >= MAX_TRACKED_IPS {
            if let Some(oldest_key) = windows.iter().min_by_key(|(_, (t, _))| *t).map(|(k, _)| *k) {
                windows.remove(&oldest_key);
            }
        }
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

    /// A10: the tracked-IP map must never grow past `MAX_TRACKED_IPS`,
    /// and the entries it keeps after overflowing the cap must be the
    /// most-recently-seen ones -- the single oldest entry is evicted to
    /// make room for each new key past the cap, not an arbitrary one.
    #[test]
    fn distinct_ips_past_the_cap_evict_the_oldest_not_an_arbitrary_one() {
        let limiter = RateLimiter::new();
        let window = Duration::from_secs(60);

        // Fill to exactly the cap, in order -- ip 0 is the oldest.
        for i in 0..MAX_TRACKED_IPS {
            let ip: IpAddr = std::net::Ipv4Addr::from(i as u32).into();
            assert!(limiter.check(ip, 5, window));
        }
        {
            let windows = limiter.windows.lock().unwrap();
            assert_eq!(windows.len(), MAX_TRACKED_IPS, "must be exactly at the cap after filling it, not past it");
        }

        // One more, genuinely new, IP past the cap.
        let newcomer: IpAddr = std::net::Ipv4Addr::from(MAX_TRACKED_IPS as u32).into();
        assert!(limiter.check(newcomer, 5, window));

        let windows = limiter.windows.lock().unwrap();
        assert_eq!(windows.len(), MAX_TRACKED_IPS, "the map must never grow past the cap");
        let oldest: IpAddr = std::net::Ipv4Addr::from(0u32).into();
        assert!(!windows.contains_key(&oldest), "the single oldest entry must have been evicted to make room");
        assert!(windows.contains_key(&newcomer), "the newcomer that triggered eviction must be present");
        let second_oldest: IpAddr = std::net::Ipv4Addr::from(1u32).into();
        assert!(windows.contains_key(&second_oldest), "only the single oldest entry is evicted, not a batch");
    }
}
