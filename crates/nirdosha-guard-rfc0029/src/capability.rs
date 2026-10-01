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
    /// `None` unless the minting [`CapabilityIssuer`] was constructed with
    /// [`CapabilityIssuer::with_signing_key`] -- see `crate::authority`'s
    /// module doc. Strictly additive: every existing caller that never
    /// sets a signing key gets `None` here, exactly as before this field
    /// existed.
    #[serde(default)]
    pub signature_b64: Option<String>,
}

impl DecisionCapability {
    pub fn is_expired(&self, now_ms: u64) -> bool {
        now_ms >= self.expires_at_ms
    }

    /// The exact bytes a [`CapabilityIssuer`] signs and a `GatewayCore`
    /// verifies -- every field except `signature_b64` itself, in a fixed
    /// order (`serde_json` preserves struct field declaration order), so
    /// it is deterministic and does not include the signature it attests.
    pub(crate) fn signable_payload(&self) -> Vec<u8> {
        #[derive(Serialize)]
        struct Signable<'a> {
            subject: &'a str,
            resource: &'a str,
            effect: &'a str,
            bundle_hash: &'a str,
            issued_at_ms: u64,
            expires_at_ms: u64,
            nonce: &'a str,
        }
        serde_json::to_vec(&Signable {
            subject: &self.subject,
            resource: &self.resource,
            effect: &self.effect,
            bundle_hash: &self.bundle_hash,
            issued_at_ms: self.issued_at_ms,
            expires_at_ms: self.expires_at_ms,
            nonce: &self.nonce,
        })
        .expect("DecisionCapability's signable fields always serialize")
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
    /// PKCS#8 DER bytes, re-parsed into a keypair on each `mint` call
    /// (cheap; keeps this type plain `Vec<u8>`-backed rather than storing
    /// a non-trivial key-object type). `None` unless
    /// [`Self::with_signing_key`] was used -- see `crate::authority`'s
    /// module doc for why this is opt-in.
    signing_key_pkcs8: Option<Vec<u8>>,
    signing_public_key_b64: Option<String>,
}

impl CapabilityIssuer {
    pub fn new(bundle: PolicyBundle, ttl_ms: u64) -> Self {
        Self { bundle, ttl_ms, counter: AtomicU64::new(0), issued: Arc::new(Mutex::new(HashSet::new())), signing_key_pkcs8: None, signing_public_key_b64: None }
    }

    /// Like [`Self::new`], but every minted capability also carries a real
    /// Ed25519 signature over its own fields -- see `crate::authority`'s
    /// module doc. `signing_key_pkcs8` is raw PKCS#8 DER bytes (e.g. from
    /// [`crate::authority::generate_ed25519_keypair`] or a real
    /// operator-custodied key file).
    pub fn with_signing_key(bundle: PolicyBundle, ttl_ms: u64, signing_key_pkcs8: Vec<u8>) -> Result<Self, String> {
        use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
        use base64::Engine;
        use nirdosha_audit::crypto_backend::signature::{Ed25519KeyPair, KeyPair};

        let keypair = Ed25519KeyPair::from_pkcs8(&signing_key_pkcs8).map_err(|e| format!("not a valid Ed25519 PKCS#8 key: {e}"))?;
        let signing_public_key_b64 = BASE64_STANDARD.encode(keypair.public_key().as_ref());
        Ok(Self {
            bundle,
            ttl_ms,
            counter: AtomicU64::new(0),
            issued: Arc::new(Mutex::new(HashSet::new())),
            signing_key_pkcs8: Some(signing_key_pkcs8),
            signing_public_key_b64: Some(signing_public_key_b64),
        })
    }

    /// `None` unless this issuer was built with [`Self::with_signing_key`].
    /// A `GatewayCore` built from this issuer captures this at
    /// construction and, if present, requires and verifies every
    /// capability's signature against it.
    pub fn signing_public_key_b64(&self) -> Option<&str> {
        self.signing_public_key_b64.as_deref()
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
        let mut capability = DecisionCapability {
            subject: auth.user().to_string(),
            resource: resource.to_string(),
            effect: effect.to_string(),
            bundle_hash: self.bundle.bundle_hash.clone(),
            issued_at_ms: now_ms,
            expires_at_ms: now_ms + self.ttl_ms,
            nonce,
            signature_b64: None,
        };
        if let Some(pkcs8) = &self.signing_key_pkcs8 {
            use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
            use base64::Engine;
            use nirdosha_audit::crypto_backend::signature::Ed25519KeyPair;

            let keypair = Ed25519KeyPair::from_pkcs8(pkcs8).expect("this issuer's own pkcs8 was already validated in with_signing_key");
            let signature = keypair.sign(&capability.signable_payload());
            capability.signature_b64 = Some(BASE64_STANDARD.encode(signature.as_ref()));
        }
        Ok(capability)
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
    /// The gateway requires a signed capability (its issuer was built with
    /// [`CapabilityIssuer::with_signing_key`]) but this one carries none.
    SignatureMissing,
    /// A signature was present but did not verify against the gateway's
    /// trusted signing key -- forged or tampered.
    SignatureInvalid,
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
            GatewayError::SignatureMissing => write!(f, "this gateway requires a signed capability, but none was provided"),
            GatewayError::SignatureInvalid => write!(f, "capability signature does not verify against this gateway's trusted signing key"),
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
