//! A fixed-window, per-process `jti` replay cache for DPoP proofs (RFC
//! 9449 §11.1's own recommendation) — ported from `compiled-serve`'s
//! own `dpop_replay.rs` (that crate's own doc comment has the full
//! reasoning: a proof that passes every stateless check in `dpop.rs`
//! still must not be replayable within its own freshness window, or an
//! attacker who captures one valid proof off the wire could resubmit it
//! for the life of that window).
//!
//! **Per-process, not fleet-wide** — the same disclosed scope
//! `ratelimit.rs`'s own limiter states, for the identical reason: a
//! fleet-wide replay cache is real, separate follow-up work (this
//! crate's own `fleet-rate-limit` feature's Redis dependency would be
//! an obvious home for it later), not silently assumed away.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const MAX_TRACKED_JTIS: usize = 10_000;

pub struct DpopReplayCache {
    seen: Mutex<HashMap<String, Instant>>,
}

impl DpopReplayCache {
    pub fn new() -> Self {
        DpopReplayCache { seen: Mutex::new(HashMap::new()) }
    }

    /// `true` the first time `jti` is seen within `window` of its own
    /// first sighting, `false` on every repeat within that window (a
    /// replay). Entries older than `window` are pruned lazily.
    pub fn check_and_record(&self, jti: &str, window: Duration) -> bool {
        let mut seen = self.seen.lock().unwrap();
        let now = Instant::now();
        seen.retain(|_, first_seen| now.duration_since(*first_seen) < window);
        if seen.contains_key(jti) {
            return false;
        }
        if seen.len() >= MAX_TRACKED_JTIS {
            if let Some(oldest) = seen.iter().min_by_key(|(_, t)| **t).map(|(k, _)| k.clone()) {
                seen.remove(&oldest);
            }
        }
        seen.insert(jti.to_string(), now);
        true
    }
}

impl Default for DpopReplayCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_jti_is_accepted_once_and_rejected_on_replay() {
        let cache = DpopReplayCache::new();
        assert!(cache.check_and_record("proof-1", Duration::from_secs(300)));
        assert!(!cache.check_and_record("proof-1", Duration::from_secs(300)), "the same jti within the window must be rejected as a replay");
    }

    #[test]
    fn distinct_jtis_are_independent() {
        let cache = DpopReplayCache::new();
        assert!(cache.check_and_record("a", Duration::from_secs(300)));
        assert!(cache.check_and_record("b", Duration::from_secs(300)));
    }

    #[test]
    fn a_jti_outside_the_window_is_pruned_and_may_be_seen_again() {
        let cache = DpopReplayCache::new();
        assert!(cache.check_and_record("proof-2", Duration::from_millis(20)));
        std::thread::sleep(Duration::from_millis(40));
        assert!(cache.check_and_record("proof-2", Duration::from_millis(20)), "a jti older than the window has aged out and is not a replay anymore");
    }
}
