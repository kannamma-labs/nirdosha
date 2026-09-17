//! A Redis-backed, fleet-wide sibling to `ratelimit.rs`'s per-process
//! limiter — for a multi-instance deployment where per-process state
//! isn't enough to bound abuse across the whole fleet. Real, separate
//! follow-up work `ratelimit.rs`'s own doc comment already named, not
//! silently assumed away.
//!
//! Same fixed-window shape as `ratelimit.rs`, keyed by peer IP and the
//! current window's own start (`now / window_secs`) so every instance
//! sharing one Redis agrees on when a new window starts without
//! needing its own coordinated clock beyond what they already share via
//! wall time. `INCR` + a conditional `EXPIRE` run as one atomic Lua
//! script (`redis::Script`), not two separate round trips — a crash
//! between an `INCR` and a *separate* `EXPIRE` would leave a key with
//! no TTL, silently locking an IP out forever once its count first
//! exceeds the limit; the script closes that window entirely.

use std::net::IpAddr;
use std::time::Duration;

const SCRIPT: &str = r#"
local count = redis.call('INCR', KEYS[1])
if count == 1 then
    redis.call('EXPIRE', KEYS[1], ARGV[1])
end
return count
"#;

pub struct FleetRateLimiter {
    client: redis::Client,
    script: redis::Script,
}

impl FleetRateLimiter {
    pub fn new(redis_url: &str) -> redis::RedisResult<Self> {
        Ok(FleetRateLimiter { client: redis::Client::open(redis_url)?, script: redis::Script::new(SCRIPT) })
    }

    /// `Ok(true)` if this call is allowed under `max_per_window` calls
    /// per `window`, shared across every process pointed at the same
    /// Redis. `Err` on a Redis connectivity failure — this module has
    /// no opinion on whether a caller should then fail open or closed;
    /// `web::Router::check_fleet_rate_limit` makes that call.
    pub fn check(&self, key: IpAddr, max_per_window: u32, window: Duration) -> redis::RedisResult<bool> {
        let mut conn = self.client.get_connection()?;
        let window_secs = window.as_secs().max(1);
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
        let window_start = now / window_secs;
        let redis_key = format!("nirdosha:ratelimit:{key}:{window_start}");
        let count: u64 = self.script.key(&redis_key).arg(window_secs).invoke(&mut conn)?;
        Ok(count <= max_per_window as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Requires a real Redis reachable at `NIRDOSHA_TEST_REDIS_URL`
    /// (default `redis://127.0.0.1:6379/`) — `#[ignore]`d so the normal
    /// suite (no Redis in a typical dev/CI environment) stays green;
    /// run explicitly (`cargo test -- --ignored`) with one available.
    /// Verified against a real, disposable `redis:alpine` container
    /// during this feature's own development.
    fn test_client() -> FleetRateLimiter {
        let url = std::env::var("NIRDOSHA_TEST_REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379/".to_string());
        FleetRateLimiter::new(&url).expect("a real Redis must be reachable for this test")
    }

    fn unique_ip() -> IpAddr {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(1);
        std::net::Ipv4Addr::from(COUNTER.fetch_add(1, Ordering::Relaxed)).into()
    }

    #[test]
    #[ignore]
    fn calls_within_the_limit_are_allowed_then_denied_past_it() {
        let limiter = test_client();
        let ip = unique_ip();
        for _ in 0..5 {
            assert!(limiter.check(ip, 5, Duration::from_secs(60)).unwrap());
        }
        assert!(!limiter.check(ip, 5, Duration::from_secs(60)).unwrap());
    }

    #[test]
    #[ignore]
    fn distinct_ips_have_independent_windows() {
        let limiter = test_client();
        let a = unique_ip();
        let b = unique_ip();
        for _ in 0..5 {
            assert!(limiter.check(a, 5, Duration::from_secs(60)).unwrap());
        }
        assert!(!limiter.check(a, 5, Duration::from_secs(60)).unwrap());
        assert!(limiter.check(b, 5, Duration::from_secs(60)).unwrap());
    }

    #[test]
    #[ignore]
    fn a_new_window_resets_the_count() {
        let limiter = test_client();
        let ip = unique_ip();
        for _ in 0..5 {
            assert!(limiter.check(ip, 5, Duration::from_secs(1)).unwrap());
        }
        assert!(!limiter.check(ip, 5, Duration::from_secs(1)).unwrap());
        std::thread::sleep(Duration::from_millis(1100));
        assert!(limiter.check(ip, 5, Duration::from_secs(1)).unwrap());
    }

    /// The atomicity this module exists for: two independent
    /// `FleetRateLimiter`s (standing in for two separate server
    /// instances) pointed at the *same* Redis see the *same* count for
    /// the same key — the real proof this is fleet-wide, not
    /// per-connection state dressed up as shared state.
    #[test]
    #[ignore]
    fn two_independent_limiter_instances_share_one_count() {
        let a = test_client();
        let b = test_client();
        let ip = unique_ip();
        for _ in 0..3 {
            assert!(a.check(ip, 5, Duration::from_secs(60)).unwrap());
        }
        for _ in 0..2 {
            assert!(b.check(ip, 5, Duration::from_secs(60)).unwrap());
        }
        assert!(!a.check(ip, 5, Duration::from_secs(60)).unwrap());
        assert!(!b.check(ip, 5, Duration::from_secs(60)).unwrap());
    }
}
