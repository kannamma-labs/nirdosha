//! A fixed-window, per-process `jti` replay cache for DPoP proofs (RFC
//! 9449 §11.1's own recommendation) — the exact same shape as
//! `ratelimit.rs`'s per-IP fixed window, applied to a different key (a
//! proof's own `jti`, not a caller's IP). `nir_dpop_verify`
//! (`runtime-kernels`) is deliberately a pure function of its inputs
//! (its own doc comment) and so cannot hold this state itself; a proof
//! that passes every stateless check (signature, `htm`/`htu`/`iat`,
//! `cnf.jkt` binding) still must not be replayable within its own
//! freshness window, or an attacker who captures one valid proof off
//! the wire could resubmit it for the life of that window.
//!
//! **Per-process, not fleet-wide** — the same disclosed scope
//! `ratelimit.rs`'s own doc comment states for its limiter, for the
//! identical reason: a fleet-wide replay cache is real, separate
//! follow-up work (would need a shared store, e.g. `redis` -- already a
//! dependency of `runtime-kernels` for `mq`, so a fleet-wide version has
//! an obvious home later), not silently assumed away.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Hard cap on distinct tracked `jti`s, same reasoning as
/// `ratelimit::MAX_TRACKED_IPS`: without one, a flood of distinct
/// (possibly bogus) proofs grows this map without bound.
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
    /// replay) — `window` should be the same freshness window
    /// `nir_dpop_verify`'s own `max_age_secs` already bounds `iat` to,
    /// so a `jti` this cache still remembers is, by construction, one
    /// `nir_dpop_verify` would still consider fresh enough to matter.
    /// Entries older than `window` are pruned lazily (checked, not
    /// swept on a timer) the moment this or any other key is next
    /// looked up, so memory use stays bounded by live traffic, not by a
    /// separate background task this crate would need to manage.
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
