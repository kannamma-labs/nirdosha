//! A short-lived, exactly-once challenge store for WebAuthn registration
//! and login ceremonies — the same shape of problem `dpop_replay.rs`
//! already solved once in this crate (a value that must be seen at most
//! once, within a bounded freshness window), applied to a different key
//! (a minted challenge, not an inbound proof's `jti`) and a different
//! consumption rule (a challenge is removed the moment it's redeemed,
//! not merely marked seen — a DPoP `jti` never needs to be looked up
//! again after its first sighting, but nothing else in that cache
//! actually deletes the entry early; a WebAuthn challenge does, since a
//! second `/finish` call with the same challenge must find nothing to
//! redeem).
//!
//! **Per-process, not fleet-wide** — the same disclosed scope
//! `dpop_replay.rs`/`ratelimit.rs` already state for their own state: a
//! fleet-wide challenge store is real, separate follow-up work, not
//! silently assumed away.

use nirdosha_rt::prelude::SharedTable;
use std::time::{Duration, Instant};

/// Hard cap on distinct outstanding challenges, same reasoning as
/// `dpop_replay::MAX_TRACKED_JTIS` — without one, a flood of
/// `/register/start`/`/login/start` calls that never finish grows this
/// map without bound.
const MAX_TRACKED_CHALLENGES: usize = 10_000;

/// What a challenge remembers about the ceremony it was minted for --
/// looked up and consumed together at `/finish` time, never read back
/// piecemeal.
#[derive(Debug, Clone)]
pub struct ChallengeContext {
    pub subject: String,
    pub minted_at: Instant,
}

/// Backed by `nirdosha_rt::prelude::SharedTable`, the dialect's own
/// managed primitive, not a raw `std::sync::Mutex<HashMap<..>>` — see
/// `webauthn_store.rs`'s own doc comment for why (a raw `Mutex` is a
/// real, named `cargo nirdosha verify` violation). `SharedTable` has no
/// `retain`-style bulk eviction, so pruning here is `snapshot()` (a
/// sorted clone of every entry) followed by individual `remove()` calls
/// — less efficient than an in-place `retain`, fine at this store's own
/// scale (bounded by `MAX_TRACKED_CHALLENGES`), and still race-free
/// per-entry the same way the table's every other method is.
#[derive(Clone)]
pub struct ChallengeStore {
    outstanding: SharedTable<String, ChallengeContext>,
}

impl ChallengeStore {
    pub fn new() -> Self {
        ChallengeStore { outstanding: SharedTable::new() }
    }

    /// Mints and records a fresh, random challenge for `subject`,
    /// pruning anything older than `window` first (lazy, on this same
    /// call, the same posture `dpop_replay.rs` already uses — no
    /// separate background sweep to manage). 32 bytes of OS entropy,
    /// base64url-encoded: the same width and source
    /// `identity::AuthConfig::demo()` already trusts for its own
    /// ephemeral signing secret.
    pub fn mint(&self, subject: &str, window: Duration) -> Result<String, String> {
        use base64::Engine as _;
        let mut buf = [0u8; 32];
        std::fs::File::open("/dev/urandom")
            .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut buf))
            .map_err(|e| format!("reading OS entropy for a webauthn challenge: {e}"))?;
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf);

        let now = Instant::now();
        let stale: Vec<(String, ChallengeContext)> = self.outstanding.snapshot().into_iter().filter(|(_, ctx)| now.duration_since(ctx.minted_at) >= window).collect();
        for (stale_key, _) in stale {
            self.outstanding.remove(&stale_key);
        }
        if self.outstanding.len() >= MAX_TRACKED_CHALLENGES {
            if let Some((oldest_key, _)) = self.outstanding.snapshot().into_iter().min_by_key(|(_, ctx)| ctx.minted_at) {
                self.outstanding.remove(&oldest_key);
            }
        }
        self.outstanding.insert(challenge.clone(), ChallengeContext { subject: subject.to_string(), minted_at: now });
        Ok(challenge)
    }

    /// Redeems `challenge` exactly once: present and fresh (within
    /// `window` of its own minting) returns its context and removes it
    /// from the store in the same call, so a second redemption attempt
    /// -- a genuine replay of a `/finish` request -- finds nothing.
    /// Stale-but-still-present (never redeemed, aged out) is removed too
    /// rather than left to the next `mint`'s own lazy prune, since a
    /// caller checking "did this fail because it doesn't exist or
    /// because it expired" only ever gets one honest answer either way:
    /// `None`.
    pub fn redeem(&self, challenge: &str, window: Duration) -> Result<Option<ChallengeContext>, String> {
        let Some(ctx) = self.outstanding.remove(&challenge.to_string()) else { return Ok(None) };
        if Instant::now().duration_since(ctx.minted_at) >= window {
            return Ok(None);
        }
        Ok(Some(ctx))
    }
}

impl Default for ChallengeStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_minted_challenge_redeems_exactly_once() {
        let store = ChallengeStore::new();
        let challenge = store.mint("alice", Duration::from_secs(300)).unwrap();
        let ctx = store.redeem(&challenge, Duration::from_secs(300)).unwrap();
        assert_eq!(ctx.unwrap().subject, "alice");
        let second = store.redeem(&challenge, Duration::from_secs(300)).unwrap();
        assert!(second.is_none(), "redeeming the same challenge twice must fail the second time");
    }

    #[test]
    fn an_unknown_challenge_redeems_to_none() {
        let store = ChallengeStore::new();
        assert!(store.redeem("never-minted", Duration::from_secs(300)).unwrap().is_none());
    }

    #[test]
    fn a_challenge_outside_its_window_is_not_redeemable() {
        let store = ChallengeStore::new();
        let challenge = store.mint("bob", Duration::from_millis(20)).unwrap();
        std::thread::sleep(Duration::from_millis(40));
        assert!(store.redeem(&challenge, Duration::from_millis(20)).unwrap().is_none(), "an expired challenge must not redeem, even though it was real");
    }

    #[test]
    fn distinct_subjects_get_distinct_challenges() {
        let store = ChallengeStore::new();
        let a = store.mint("alice", Duration::from_secs(300)).unwrap();
        let b = store.mint("bob", Duration::from_secs(300)).unwrap();
        assert_ne!(a, b);
    }
}
