//! Short-lived, single-use decision capabilities and the issuer that mints
//! them under one governed [`PolicyBundle`].

use std::collections::HashSet;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use nirdosha_rt::Auth;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::bundle::PolicyBundle;

/// A short-lived, single-use authorization to exercise one `resource` +
/// `effect` under one governed `bundle_hash`. See the crate's module doc
/// comment for what this does and does not defend against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionCapability {
    pub subject: String,
    pub resource: String,
    pub effect: String,
    pub bundle_hash: String,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
    pub nonce: String,
}

impl DecisionCapability {
    pub fn is_expired(&self, now_ms: u64) -> bool {
        now_ms >= self.expires_at_ms
    }
}

/// Mints [`DecisionCapability`]s under one governed [`PolicyBundle`]. One
/// issuer per bundle per process; the internal counter only needs to make
/// nonces unique within that process, since the capability is never
/// serialized to a caller outside it (RFC 0029 §7.1: the catalog, adapter,
/// and this issuer are the only things that ever construct one for real
/// use in the generated app).
///
/// `issued` is the provenance registry: every nonce this issuer has ever
/// minted, independent of whether a gateway has consumed it yet. A gateway
/// built from this issuer (via [`crate::gateway::GatewayCore::new`]) shares
/// this same `Arc`, so it can tell a genuinely minted capability apart from
/// a hand-constructed one with a fabricated nonce -- the latter is rejected
/// before any effect runs.
pub struct CapabilityIssuer {
    bundle: PolicyBundle,
    ttl_ms: u64,
    counter: AtomicU64,
    issued: Arc<Mutex<HashSet<String>>>,
}

impl CapabilityIssuer {
    pub fn new(bundle: PolicyBundle, ttl_ms: u64) -> Self {
        Self { bundle, ttl_ms, counter: AtomicU64::new(0), issued: Arc::new(Mutex::new(HashSet::new())) }
    }

    pub fn bundle(&self) -> &PolicyBundle {
        &self.bundle
    }

    /// The shared provenance registry a gateway binds to at construction.
    /// Exposed so a gateway can be built from `&self` without this crate
    /// needing a combined issuer+gateway type.
    pub fn issued_registry(&self) -> Arc<Mutex<HashSet<String>>> {
        self.issued.clone()
    }

    /// Mint a capability for `auth` to exercise `resource`/`effect`, or
    /// fail closed if the governing bundle's own validity window doesn't
    /// cover `now_ms` -- an expired *bundle* can't mint a fresh capability
    /// just because the request itself is new.
    pub fn mint(
        &self,
        auth: &Auth,
        resource: &str,
        effect: &str,
        now_ms: u64,
    ) -> Result<DecisionCapability, GatewayError> {
        if !self.bundle.is_active(now_ms) {
            return Err(GatewayError::BundleNotActive);
        }
        let seq = self.counter.fetch_add(1, Ordering::Relaxed);
        let nonce = format!(
            "sha256:{:x}",
            Sha256::digest(
                format!("{}:{}:{resource}:{effect}:{now_ms}:{seq}", self.bundle.bundle_hash, auth.user())
                    .as_bytes()
            )
        );
        self.issued.lock().expect("issued-registry lock poisoned").insert(nonce.clone());
        Ok(DecisionCapability {
            subject: auth.user().to_string(),
            resource: resource.to_string(),
            effect: effect.to_string(),
            bundle_hash: self.bundle.bundle_hash.clone(),
            issued_at_ms: now_ms,
            expires_at_ms: now_ms + self.ttl_ms,
            nonce,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GatewayError {
    BundleNotActive,
    CapabilityMismatch { expected_resource: &'static str, expected_effect: &'static str },
    /// `capability.bundle_hash` does not match the bundle the gateway was
    /// bound to at construction -- covers both a hand-forged hash and a
    /// genuinely-minted capability from a bundle that has since rotated.
    BundleMismatch,
    /// `capability.nonce` was never minted by this gateway's bound issuer --
    /// the forgery case: nothing about resource/effect/expiry distinguishes
    /// a hand-built `DecisionCapability` from a real one, but this does.
    NotIssued,
    CapabilityExpired,
    CapabilityReplayed,
    /// The replay store itself failed (e.g. a durable store's I/O error) --
    /// fail closed: a capability is never treated as non-replayed just
    /// because the store couldn't be consulted.
    ReplayStoreUnavailable(String),
    Execution(String),
}

impl fmt::Display for GatewayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GatewayError::BundleNotActive => {
                write!(f, "the governing policy bundle is not within its effective_from/expires_at window")
            }
            GatewayError::CapabilityMismatch { expected_resource, expected_effect } => write!(
                f,
                "capability does not authorize this gateway (expected resource `{expected_resource}`, effect `{expected_effect}`)"
            ),
            GatewayError::BundleMismatch => {
                write!(f, "capability was not issued under the bundle this gateway currently trusts")
            }
            GatewayError::NotIssued => {
                write!(f, "capability was not minted by this gateway's bound issuer")
            }
            GatewayError::CapabilityExpired => write!(f, "capability has expired"),
            GatewayError::CapabilityReplayed => write!(f, "capability has already been consumed"),
            GatewayError::ReplayStoreUnavailable(e) => write!(f, "replay store unavailable: {e}"),
            GatewayError::Execution(e) => write!(f, "gateway-executed effect failed: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::EffectGateway;
    use crate::TransferRequestGatewayV1;

    const VALID_BUNDLE: &str = r#"
[bundle]
authority_id = "banking-domain-authority"
policy_owner = "payments-platform-team"
jurisdiction = "IN"
approved_by = "banking-domain-authority"
signed_by = "banking-domain-authority"
effective_from = "2026-01-01T00:00:00Z"
expires_at = "2027-01-01T00:00:00Z"
"#;

    fn ms_2026_06_01() -> u64 {
        crate::bundle::parse_rfc3339_utc_seconds("2026-06-01T00:00:00Z").unwrap() * 1000
    }

    fn issuer() -> CapabilityIssuer {
        let bundle = PolicyBundle::from_toml_str(VALID_BUNDLE).unwrap();
        CapabilityIssuer::new(bundle, 5 * 60 * 1000)
    }

    #[test]
    fn mint_fails_when_bundle_window_does_not_cover_now() {
        let issuer = issuer();
        let auth = Auth::login("alice", &["Customer"]);
        let before_window = crate::bundle::parse_rfc3339_utc_seconds("2025-06-01T00:00:00Z").unwrap() * 1000;
        let err = issuer
            .mint(&auth, TransferRequestGatewayV1::RESOURCE, TransferRequestGatewayV1::EFFECT, before_window)
            .unwrap_err();
        assert_eq!(err, GatewayError::BundleNotActive);
    }

    #[test]
    fn mint_succeeds_inside_bundle_window() {
        let issuer = issuer();
        let auth = Auth::login("alice", &["Customer"]);
        let now = ms_2026_06_01();
        let cap = issuer
            .mint(&auth, TransferRequestGatewayV1::RESOURCE, TransferRequestGatewayV1::EFFECT, now)
            .expect("mint should succeed inside the bundle window");
        assert_eq!(cap.subject, "alice");
        assert!(!cap.is_expired(now));
    }
}
