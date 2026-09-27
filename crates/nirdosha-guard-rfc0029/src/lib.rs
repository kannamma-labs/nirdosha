//! RFC 0029 §7.1 runtime capability + effect gateway.
//!
//! The generation-time admission gate (`cargo-nirdosha`'s
//! `run_service_admission`) proves a screen's `[screen.service]` wiring was
//! authorized once, at build time. It says nothing about what happens per
//! live HTTP request. This crate closes that gap for the `transfer_create`
//! vertical slice: a [`CapabilityIssuer`] mints a short-lived
//! [`DecisionCapability`] bound to one subject/resource/effect under a
//! governed [`PolicyBundle`]'s validity window, and [`TransferRequestGatewayV1`]
//! is the only thing that can consume one -- verifying resource/effect
//! identity, expiry, and single use (replay) before letting the caller's
//! real mutation run, and writing one evidence record either way via
//! `nirdosha_audit`'s hash-chained log.
//!
//! A gateway only accepts a capability whose nonce is present in its issuer's
//! own `issued_registry` (checked before `execute` ever runs) and whose
//! `bundle_hash` matches the bundle the gateway was bound to at construction
//! -- so a hand-built `DecisionCapability`, or one minted under a bundle the
//! gateway no longer trusts, is rejected outright rather than merely
//! resource/effect/expiry checked like a genuine one.
//!
//! Scope this crate does **not** claim: the capability carries no
//! cryptographic signature (no MAC over its fields) -- the issued-registry
//! check defends against forgery only *within the same process* that holds
//! the `Arc` the issuer and gateway share; it is not proof against a
//! malicious actor with the ability to fabricate arbitrary process state
//! (e.g. via unsafe code or a compromised dependency in the same address
//! space). The policy bundle's `approved_by`/`signed_by` are checked for
//! presence and the validity window against the clock -- not verified as a
//! real Ed25519 signature. And the issued/consumed registries are in-process
//! `HashSet`s, not a durable or cross-process store. All three are disclosed
//! scope boundaries for this first narrow slice (`funds_reserve`, shadow
//! mode, real capability signing, and a durable replay store are explicitly
//! later work), not silent gaps.

use std::collections::HashSet;
use std::fmt;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use nirdosha_audit::envelope::{AuditEnvelope, AuditRecordKind, ModuleAuditChain};
use nirdosha_rt::Auth;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// ---------------------------------------------------------------------------
// Governed policy bundle
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
struct RawBundleFile {
    bundle: RawBundle,
}

#[derive(Debug, Clone, Deserialize)]
struct RawBundle {
    authority_id: String,
    policy_owner: String,
    jurisdiction: String,
    approved_by: String,
    signed_by: String,
    effective_from: String,
    expires_at: String,
}

/// A domain authority's governed, versioned policy bundle -- the envelope a
/// service-catalog entry's admission is exercised under. `bundle_hash` is
/// always *computed* from the exact source bytes, never authored, so it
/// can't drift from the content it attests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyBundle {
    pub authority_id: String,
    pub policy_owner: String,
    pub jurisdiction: String,
    pub approved_by: String,
    pub signed_by: String,
    pub effective_from: String,
    pub expires_at: String,
    pub bundle_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BundleError {
    Parse(String),
    EmptyField(&'static str),
    InvertedWindow,
    UnparseableTimestamp(&'static str),
}

impl fmt::Display for BundleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BundleError::Parse(e) => write!(f, "policy bundle is invalid: {e}"),
            BundleError::EmptyField(name) => write!(f, "policy bundle field `{name}` must not be empty"),
            BundleError::InvertedWindow => {
                write!(f, "policy bundle effective_from must not be after expires_at")
            }
            BundleError::UnparseableTimestamp(name) => {
                write!(f, "policy bundle field `{name}` is not a valid `YYYY-MM-DDTHH:MM:SSZ` timestamp")
            }
        }
    }
}

impl PolicyBundle {
    /// Parse and validate `policy-bundle.toml`'s source text. Fails closed
    /// on any empty governance field, an inverted validity window, or a
    /// timestamp this crate's minimal RFC 3339 parser can't read.
    pub fn from_toml_str(src: &str) -> Result<Self, BundleError> {
        let raw: RawBundleFile = toml::from_str(src).map_err(|e| BundleError::Parse(e.to_string()))?;
        let b = raw.bundle;
        for (name, value) in [
            ("authority_id", &b.authority_id),
            ("policy_owner", &b.policy_owner),
            ("jurisdiction", &b.jurisdiction),
            ("approved_by", &b.approved_by),
            ("signed_by", &b.signed_by),
        ] {
            if value.trim().is_empty() {
                return Err(BundleError::EmptyField(name));
            }
        }
        let effective_from_secs = parse_rfc3339_utc_seconds(&b.effective_from)
            .ok_or(BundleError::UnparseableTimestamp("effective_from"))?;
        let expires_at_secs = parse_rfc3339_utc_seconds(&b.expires_at)
            .ok_or(BundleError::UnparseableTimestamp("expires_at"))?;
        if effective_from_secs > expires_at_secs {
            return Err(BundleError::InvertedWindow);
        }
        let bundle_hash = format!("sha256:{:x}", Sha256::digest(src.as_bytes()));
        Ok(PolicyBundle {
            authority_id: b.authority_id,
            policy_owner: b.policy_owner,
            jurisdiction: b.jurisdiction,
            approved_by: b.approved_by,
            signed_by: b.signed_by,
            effective_from: b.effective_from,
            expires_at: b.expires_at,
            bundle_hash,
        })
    }

    /// Whether this bundle's governance window covers `now_ms` (epoch
    /// milliseconds). An unparseable window (should not happen after
    /// `from_toml_str` validated it, but this is also reachable via
    /// `#[derive(Deserialize)]` on `PolicyBundle` itself from a trusted
    /// source) is treated as inactive, not active-by-default.
    pub fn is_active(&self, now_ms: u64) -> bool {
        let (Some(from), Some(until)) = (
            parse_rfc3339_utc_seconds(&self.effective_from),
            parse_rfc3339_utc_seconds(&self.expires_at),
        ) else {
            return false;
        };
        let now_secs = now_ms / 1000;
        now_secs >= from && now_secs < until
    }
}

/// Minimal `YYYY-MM-DDTHH:MM:SSZ` (UTC, whole seconds, no offset) parser --
/// `policy-bundle.toml`'s own fixed format. Deliberately not a datetime
/// dependency: capability TTL arithmetic needs a real epoch (unlike
/// `rfc0029-conformance`'s plain string comparisons of the same shape),
/// but nothing here needs calendars beyond this one shape.
fn parse_rfc3339_utc_seconds(s: &str) -> Option<u64> {
    let s = s.strip_suffix('Z')?;
    let (date, time) = s.split_once('T')?;
    let mut d = date.split('-');
    let year: i64 = d.next()?.parse().ok()?;
    let month: i64 = d.next()?.parse().ok()?;
    let day: i64 = d.next()?.parse().ok()?;
    if d.next().is_some() {
        return None;
    }
    let mut t = time.split(':');
    let hour: i64 = t.next()?.parse().ok()?;
    let minute: i64 = t.next()?.parse().ok()?;
    let second: i64 = t.next()?.parse().ok()?;
    if t.next().is_some() {
        return None;
    }

    // Howard Hinnant's `days_from_civil`.
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = (month + 9) % 12; // [0, 11]
    let doy = (153 * mp + 2) / 5 + day - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    let days = era * 146097 + doe - 719468; // days since 1970-01-01

    let secs = days * 86400 + hour * 3600 + minute * 60 + second;
    u64::try_from(secs).ok()
}

// ---------------------------------------------------------------------------
// Decision capability
// ---------------------------------------------------------------------------

/// A short-lived, single-use authorization to exercise one `resource` +
/// `effect` under one governed `bundle_hash`. See the module doc comment
/// for what this does and does not defend against.
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
/// built via [`TransferRequestGatewayV1::new`] shares this same `Arc`, so it
/// can tell a genuinely minted capability apart from a hand-constructed one
/// with a fabricated nonce -- the latter is rejected before any effect runs.
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
            GatewayError::Execution(e) => write!(f, "gateway-executed effect failed: {e}"),
        }
    }
}

/// The one thing allowed to turn an admitted `transfer_create` capability
/// into a real mutation. There is no other entry point into the mutation
/// this gateway wraps from generated code -- see `crud_screens!`'s
/// `capability_gate` clause, which only ever calls `consume_and_execute`.
pub struct TransferRequestGatewayV1 {
    evidence: ModuleAuditChain,
    expected_bundle_hash: String,
    issued: Arc<Mutex<HashSet<String>>>,
    consumed_nonces: Mutex<HashSet<String>>,
}

impl TransferRequestGatewayV1 {
    pub const RESOURCE: &'static str = "TransferRequest";
    pub const EFFECT: &'static str = "transfer.request";
    const MODULE: &'static str = "rfc0029:transfer_request_gateway_v1";

    /// Bind this gateway to `issuer` -- only a capability that (a) carries
    /// `issuer`'s current `bundle_hash` and (b) has a nonce present in
    /// `issuer`'s `issued_registry` will pass verification. There is no way
    /// to construct a `TransferRequestGatewayV1` that trusts a capability
    /// its bound issuer did not itself mint.
    pub fn new(evidence_path: impl AsRef<Path>, issuer: &CapabilityIssuer) -> Self {
        Self {
            evidence: ModuleAuditChain::new(Self::MODULE, evidence_path.as_ref().to_path_buf()),
            expected_bundle_hash: issuer.bundle().bundle_hash.clone(),
            issued: issuer.issued_registry(),
            consumed_nonces: Mutex::new(HashSet::new()),
        }
    }

    /// Verify `capability` (resource/effect identity, expiry, single use)
    /// and only then run `execute` -- the real guarded mutation. Writes one
    /// evidence entry either way: a rejected effect is still a decision
    /// that happened, and the evidence trail should say so.
    pub fn consume_and_execute<T>(
        &self,
        capability: &DecisionCapability,
        now_ms: u64,
        execute: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, GatewayError> {
        if let Err(e) = self.verify_and_consume(capability, now_ms) {
            self.record(capability, now_ms, "rejected", &e.to_string());
            return Err(e);
        }
        match execute() {
            Ok(value) => {
                self.record(capability, now_ms, "accepted", "effect executed");
                Ok(value)
            }
            Err(msg) => {
                self.record(capability, now_ms, "rejected", &msg);
                Err(GatewayError::Execution(msg))
            }
        }
    }

    fn verify_and_consume(&self, capability: &DecisionCapability, now_ms: u64) -> Result<(), GatewayError> {
        if capability.resource != Self::RESOURCE || capability.effect != Self::EFFECT {
            return Err(GatewayError::CapabilityMismatch {
                expected_resource: Self::RESOURCE,
                expected_effect: Self::EFFECT,
            });
        }
        if capability.bundle_hash != self.expected_bundle_hash {
            return Err(GatewayError::BundleMismatch);
        }
        if capability.is_expired(now_ms) {
            return Err(GatewayError::CapabilityExpired);
        }
        // Provenance check: a nonce this gateway's bound issuer never minted
        // is rejected here, before `execute` ever runs -- this is what makes
        // a hand-constructed `DecisionCapability` distinguishable from a
        // genuine one, not just resource/effect/expiry checked the same way.
        if !self.issued.lock().expect("issued-registry lock poisoned").contains(&capability.nonce) {
            return Err(GatewayError::NotIssued);
        }
        let mut consumed = self.consumed_nonces.lock().expect("consumed-nonces lock poisoned");
        if !consumed.insert(capability.nonce.clone()) {
            return Err(GatewayError::CapabilityReplayed);
        }
        Ok(())
    }

    fn record(&self, capability: &DecisionCapability, now_ms: u64, decision: &str, detail: &str) {
        let envelope = AuditEnvelope {
            trace_id: capability.nonce.clone(),
            ts: now_ms.to_string(),
            module: Self::MODULE.to_string(),
            subject: capability.subject.clone(),
            action: capability.effect.clone(),
            resource: capability.resource.clone(),
            policy_versions: vec![capability.bundle_hash.clone()],
            decision: decision.to_string(),
            obligations: vec![],
            kind: AuditRecordKind::Decision,
            content: serde_json::json!({
                "detail": detail,
                "nonce": capability.nonce,
                "issued_at_ms": capability.issued_at_ms,
                "expires_at_ms": capability.expires_at_ms,
            }),
        };
        self.evidence.append(&envelope, now_ms);
    }

    pub fn evidence(&self) -> &ModuleAuditChain {
        &self.evidence
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        // Comfortably inside the fixture's validity window.
        parse_rfc3339_utc_seconds("2026-06-01T00:00:00Z").unwrap() * 1000
    }

    #[test]
    fn parses_known_epoch_anchors() {
        assert_eq!(parse_rfc3339_utc_seconds("1970-01-01T00:00:00Z"), Some(0));
        let next_year = parse_rfc3339_utc_seconds("2027-01-01T00:00:00Z").unwrap();
        let this_year = parse_rfc3339_utc_seconds("2026-01-01T00:00:00Z").unwrap();
        assert_eq!(next_year - this_year, 365 * 86400, "2026 is not a leap year");
    }

    #[test]
    fn rejects_empty_governance_field() {
        let src = VALID_BUNDLE.replace("approved_by = \"banking-domain-authority\"", "approved_by = \"\"");
        assert_eq!(PolicyBundle::from_toml_str(&src).unwrap_err(), BundleError::EmptyField("approved_by"));
    }

    #[test]
    fn rejects_inverted_window() {
        let src = VALID_BUNDLE.replace("expires_at = \"2027-01-01T00:00:00Z\"", "expires_at = \"2025-01-01T00:00:00Z\"");
        assert_eq!(PolicyBundle::from_toml_str(&src).unwrap_err(), BundleError::InvertedWindow);
    }

    fn issuer() -> CapabilityIssuer {
        let bundle = PolicyBundle::from_toml_str(VALID_BUNDLE).unwrap();
        CapabilityIssuer::new(bundle, 5 * 60 * 1000)
    }

    #[test]
    fn mint_fails_when_bundle_window_does_not_cover_now() {
        let issuer = issuer();
        let auth = Auth::login("alice", &["Customer"]);
        let before_window = parse_rfc3339_utc_seconds("2025-06-01T00:00:00Z").unwrap() * 1000;
        let err = issuer
            .mint(&auth, TransferRequestGatewayV1::RESOURCE, TransferRequestGatewayV1::EFFECT, before_window)
            .unwrap_err();
        assert_eq!(err, GatewayError::BundleNotActive);
    }

    #[test]
    fn valid_mint_and_consume_succeeds_and_writes_one_evidence_entry() {
        let dir = std::env::temp_dir().join(format!("rfc0029_gw_test_{}", std::process::id()));
        let evidence_path = dir.join("evidence.jsonl");
        let _ = std::fs::remove_file(&evidence_path);
        let issuer = issuer();
        let gateway = TransferRequestGatewayV1::new(&evidence_path, &issuer);
        let auth = Auth::login("alice", &["Customer"]);
        let now = ms_2026_06_01();
        let cap = issuer
            .mint(&auth, TransferRequestGatewayV1::RESOURCE, TransferRequestGatewayV1::EFFECT, now)
            .expect("mint should succeed inside the bundle window");

        let result = gateway.consume_and_execute(&cap, now, || Ok::<_, String>("row-1".to_string()));
        assert_eq!(result, Ok("row-1".to_string()));

        let entries = gateway.evidence().entries();
        assert_eq!(entries.len(), 1);
        let entry_json = serde_json::to_string(&entries[0].content).unwrap();
        assert!(entry_json.contains(&cap.bundle_hash), "{entry_json}");
        assert!(entry_json.contains("\"decision\":\"accepted\"") || entry_json.contains(&cap.nonce), "{entry_json}");
        let _ = std::fs::remove_file(&evidence_path);
    }

    #[test]
    fn expired_capability_is_rejected() {
        let dir = std::env::temp_dir().join(format!("rfc0029_gw_test_expired_{}", std::process::id()));
        let issuer = issuer();
        let gateway = TransferRequestGatewayV1::new(&dir, &issuer);
        let auth = Auth::login("alice", &["Customer"]);
        let now = ms_2026_06_01();
        let cap = issuer.mint(&auth, TransferRequestGatewayV1::RESOURCE, TransferRequestGatewayV1::EFFECT, now).unwrap();
        let after_ttl = cap.expires_at_ms + 1;
        let err = gateway
            .consume_and_execute(&cap, after_ttl, || Ok::<_, String>(()))
            .unwrap_err();
        assert_eq!(err, GatewayError::CapabilityExpired);
    }

    #[test]
    fn replayed_capability_is_rejected() {
        let dir = std::env::temp_dir().join(format!("rfc0029_gw_test_replay_{}", std::process::id()));
        let issuer = issuer();
        let gateway = TransferRequestGatewayV1::new(&dir, &issuer);
        let auth = Auth::login("alice", &["Customer"]);
        let now = ms_2026_06_01();
        let cap = issuer.mint(&auth, TransferRequestGatewayV1::RESOURCE, TransferRequestGatewayV1::EFFECT, now).unwrap();

        assert!(gateway.consume_and_execute(&cap, now, || Ok::<_, String>(())).is_ok());
        let err = gateway.consume_and_execute(&cap, now, || Ok::<_, String>(())).unwrap_err();
        assert_eq!(err, GatewayError::CapabilityReplayed);
    }

    #[test]
    fn resource_effect_mismatch_is_rejected() {
        let dir = std::env::temp_dir().join(format!("rfc0029_gw_test_mismatch_{}", std::process::id()));
        let issuer = issuer();
        let gateway = TransferRequestGatewayV1::new(&dir, &issuer);
        let auth = Auth::login("alice", &["Customer"]);
        let now = ms_2026_06_01();
        let cap = issuer.mint(&auth, "Account", "funds.reserve", now).unwrap();
        let err = gateway.consume_and_execute(&cap, now, || Ok::<_, String>(())).unwrap_err();
        assert!(matches!(err, GatewayError::CapabilityMismatch { .. }));
    }

    #[test]
    fn a_hand_constructed_capability_the_issuer_never_minted_is_rejected_before_execute_runs() {
        // "Direct gateway call without capability" is structurally
        // impossible to express -- there is no unchecked path, only this
        // one. Earlier this crate only checked resource/effect/expiry/replay,
        // which a forged capability with the right shape could satisfy; this
        // now also requires the nonce to be present in the bound issuer's
        // own `issued_registry`, so a value that never went through `mint`
        // is rejected outright.
        let dir = std::env::temp_dir().join(format!("rfc0029_gw_test_forged_{}", std::process::id()));
        let issuer = issuer();
        let gateway = TransferRequestGatewayV1::new(&dir, &issuer);
        let now = ms_2026_06_01();
        let forged = DecisionCapability {
            subject: "mallory".into(),
            resource: TransferRequestGatewayV1::RESOURCE.into(),
            effect: TransferRequestGatewayV1::EFFECT.into(),
            bundle_hash: issuer.bundle().bundle_hash.clone(),
            issued_at_ms: now,
            expires_at_ms: now + 1,
            nonce: "not-from-an-issuer".into(),
        };
        let mut executed = false;
        let err = gateway
            .consume_and_execute(&forged, now, || {
                executed = true;
                Ok::<_, String>(())
            })
            .unwrap_err();
        assert_eq!(err, GatewayError::NotIssued);
        assert!(!executed, "the guarded effect must never run for an unminted capability");

        let entries = gateway.evidence().entries();
        assert_eq!(entries.len(), 1, "rejection is still evidence-recorded");
        let entry_json = serde_json::to_string(&entries[0]).unwrap();
        assert!(entry_json.contains("\"decision\":\"rejected\""), "{entry_json}");
    }

    #[test]
    fn capability_minted_under_a_bundle_the_gateway_no_longer_trusts_is_rejected() {
        // Simulates bundle rotation: the capability carries a real,
        // well-formed `bundle_hash`, and a nonce format identical to a real
        // one, but not the hash of the bundle this particular gateway was
        // constructed against.
        let dir = std::env::temp_dir().join(format!("rfc0029_gw_test_bundle_rotate_{}", std::process::id()));
        let old_issuer = issuer();
        let auth = Auth::login("alice", &["Customer"]);
        let now = ms_2026_06_01();
        let cap = old_issuer
            .mint(&auth, TransferRequestGatewayV1::RESOURCE, TransferRequestGatewayV1::EFFECT, now)
            .unwrap();

        let rotated_bundle = PolicyBundle::from_toml_str(
            &VALID_BUNDLE.replace("policy_owner = \"payments-platform-team\"", "policy_owner = \"payments-platform-team-v2\""),
        )
        .unwrap();
        let new_issuer = CapabilityIssuer::new(rotated_bundle, 5 * 60 * 1000);
        let gateway = TransferRequestGatewayV1::new(&dir, &new_issuer);

        let err = gateway.consume_and_execute(&cap, now, || Ok::<_, String>(())).unwrap_err();
        assert_eq!(err, GatewayError::BundleMismatch);
    }
}
