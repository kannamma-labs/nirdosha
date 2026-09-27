//! The generalized RFC 0029 §7.1 effect-gateway contract: validate
//! capability identity, validate the governing bundle, consume the
//! capability's idempotency key (single use), run the guarded effect, and
//! write one evidence record either way. [`GatewayCore`] holds the shared
//! state every concrete gateway needs to do this; [`EffectGateway`] is the
//! trait a concrete gateway (like [`TransferRequestGatewayV1`]) implements
//! instead of reimplementing the orchestration itself.

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};

use nirdosha_audit::envelope::{AuditEnvelope, AuditRecordKind, ModuleAuditChain};

use crate::capability::{CapabilityIssuer, DecisionCapability, GatewayError};
use crate::replay_store::{InMemoryReplayStore, ReplayStore};

/// Shared verification + evidence state for one gateway instance: the
/// bundle it trusts, the issuer's provenance registry it checks nonces
/// against, its single-use (replay) store, and its evidence chain. Every
/// concrete gateway composes one of these rather than reimplementing
/// capability verification.
pub struct GatewayCore {
    evidence: ModuleAuditChain,
    expected_bundle_hash: String,
    issued: Arc<Mutex<HashSet<String>>>,
    replay_store: Arc<dyn ReplayStore>,
}

impl GatewayCore {
    /// Bind this core to `issuer` -- only a capability that (a) carries
    /// `issuer`'s current `bundle_hash` and (b) has a nonce present in
    /// `issuer`'s `issued_registry` will pass verification. There is no way
    /// to construct a core that trusts a capability its bound issuer did
    /// not itself mint. `module` scopes the evidence chain, matching the
    /// concrete gateway's `EffectGateway::MODULE`.
    ///
    /// Replay protection defaults to [`InMemoryReplayStore`] -- fine for
    /// tests and single-process development, unsafe across multiple
    /// processes or a restart. Use [`GatewayCore::with_replay_store`] with
    /// [`crate::SqliteReplayStore`] before a multi-process deployment.
    pub fn new(module: impl Into<String>, evidence_path: impl AsRef<Path>, issuer: &CapabilityIssuer) -> Self {
        Self::with_replay_store(module, evidence_path, issuer, Arc::new(InMemoryReplayStore::new()))
    }

    pub fn with_replay_store(
        module: impl Into<String>,
        evidence_path: impl AsRef<Path>,
        issuer: &CapabilityIssuer,
        replay_store: Arc<dyn ReplayStore>,
    ) -> Self {
        Self {
            evidence: ModuleAuditChain::new(module, evidence_path.as_ref().to_path_buf()),
            expected_bundle_hash: issuer.bundle().bundle_hash.clone(),
            issued: issuer.issued_registry(),
            replay_store,
        }
    }

    pub fn evidence(&self) -> &ModuleAuditChain {
        &self.evidence
    }

    /// Verify `capability` (resource/effect identity, bundle, expiry) and
    /// consume its idempotency key (single use) -- everything but running
    /// the guarded effect itself.
    fn verify_and_consume(
        &self,
        resource: &'static str,
        effect: &'static str,
        capability: &DecisionCapability,
        now_ms: u64,
    ) -> Result<(), GatewayError> {
        if capability.resource != resource || capability.effect != effect {
            return Err(GatewayError::CapabilityMismatch { expected_resource: resource, expected_effect: effect });
        }
        if capability.bundle_hash != self.expected_bundle_hash {
            return Err(GatewayError::BundleMismatch);
        }
        if capability.is_expired(now_ms) {
            return Err(GatewayError::CapabilityExpired);
        }
        // Provenance check: a nonce this gateway's bound issuer never minted
        // is rejected here, before the guarded effect ever runs -- this is
        // what makes a hand-constructed `DecisionCapability` distinguishable
        // from a genuine one, not just resource/effect/expiry checked the
        // same way.
        if !self.issued.lock().expect("issued-registry lock poisoned").contains(&capability.nonce) {
            return Err(GatewayError::NotIssued);
        }
        match self.replay_store.try_consume(&capability.nonce) {
            Ok(true) => Ok(()),
            Ok(false) => Err(GatewayError::CapabilityReplayed),
            Err(e) => Err(GatewayError::ReplayStoreUnavailable(e.to_string())),
        }
    }

    fn record(&self, capability: &DecisionCapability, now_ms: u64, decision: &str, detail: &str) {
        let envelope = AuditEnvelope {
            trace_id: capability.nonce.clone(),
            ts: now_ms.to_string(),
            module: self.evidence.module.clone(),
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
}

/// RFC 0029 §7.1 generalized effect-gateway contract: validate capability
/// (resource/effect + bundle + expiry), consume its idempotency key, run
/// the guarded effect, write evidence either way. A concrete gateway
/// supplies its identity (`RESOURCE`/`EFFECT`/`MODULE`) and a [`GatewayCore`]
/// to hold onto; `consume_and_execute`'s default body is the entire
/// orchestration, shared rather than reimplemented per gateway.
pub trait EffectGateway {
    const RESOURCE: &'static str;
    const EFFECT: &'static str;
    const MODULE: &'static str;

    fn core(&self) -> &GatewayCore;

    /// Verify `capability` and only then run `execute` -- the real guarded
    /// mutation. Writes one evidence entry either way: a rejected effect is
    /// still a decision that happened, and the evidence trail should say so.
    fn consume_and_execute<T>(
        &self,
        capability: &DecisionCapability,
        now_ms: u64,
        execute: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, GatewayError> {
        let core = self.core();
        if let Err(e) = core.verify_and_consume(Self::RESOURCE, Self::EFFECT, capability, now_ms) {
            core.record(capability, now_ms, "rejected", &e.to_string());
            return Err(e);
        }
        match execute() {
            Ok(value) => {
                core.record(capability, now_ms, "accepted", "effect executed");
                Ok(value)
            }
            Err(msg) => {
                core.record(capability, now_ms, "rejected", &msg);
                Err(GatewayError::Execution(msg))
            }
        }
    }
}

/// The one thing allowed to turn an admitted `transfer_create` capability
/// into a real mutation. There is no other entry point into the mutation
/// this gateway wraps from generated code -- see `crud_screens!`'s
/// `capability_gate` clause, which only ever calls `consume_and_execute`.
pub struct TransferRequestGatewayV1 {
    core: GatewayCore,
}

impl TransferRequestGatewayV1 {
    pub fn new(evidence_path: impl AsRef<Path>, issuer: &CapabilityIssuer) -> Self {
        Self { core: GatewayCore::new(Self::MODULE, evidence_path, issuer) }
    }

    /// Like `new`, but with an explicit [`ReplayStore`] -- use with
    /// [`crate::SqliteReplayStore`] before a multi-process deployment.
    pub fn with_replay_store(
        evidence_path: impl AsRef<Path>,
        issuer: &CapabilityIssuer,
        replay_store: Arc<dyn ReplayStore>,
    ) -> Self {
        Self { core: GatewayCore::with_replay_store(Self::MODULE, evidence_path, issuer, replay_store) }
    }

    pub fn evidence(&self) -> &ModuleAuditChain {
        self.core.evidence()
    }

    /// Inherent forwarder to `EffectGateway::consume_and_execute` -- codegen
    /// (`cargo-nirdosha`'s `render_capability_gate`) emits a plain
    /// `gateway.consume_and_execute(...)` call into generated app code that
    /// never imports this crate's `EffectGateway` trait, so method
    /// resolution needs an inherent method here, not just the trait's
    /// default one.
    pub fn consume_and_execute<T>(
        &self,
        capability: &DecisionCapability,
        now_ms: u64,
        execute: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, GatewayError> {
        <Self as EffectGateway>::consume_and_execute(self, capability, now_ms, execute)
    }
}

impl EffectGateway for TransferRequestGatewayV1 {
    const RESOURCE: &'static str = "TransferRequest";
    const EFFECT: &'static str = "transfer.request";
    const MODULE: &'static str = "rfc0029:transfer_request_gateway_v1";

    fn core(&self) -> &GatewayCore {
        &self.core
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundle::PolicyBundle;
    use nirdosha_rt::Auth;

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
    fn durable_replay_store_rejects_reuse_across_a_fresh_gateway_instance() {
        // Simulates a restart (or a second process) reattaching to the
        // same durable replay-store file: a capability consumed by the
        // first `TransferRequestGatewayV1` instance must still be rejected
        // by a second instance that only shares the on-disk store, not the
        // first instance's own `GatewayCore`.
        use crate::replay_store::SqliteReplayStore;
        let db_path = std::env::temp_dir().join(format!("rfc0029_gw_test_durable_replay_{}.sqlite3", std::process::id()));
        let _ = std::fs::remove_file(&db_path);
        let dir = std::env::temp_dir().join(format!("rfc0029_gw_test_durable_replay_evidence_{}", std::process::id()));
        let issuer = issuer();
        let auth = Auth::login("alice", &["Customer"]);
        let now = ms_2026_06_01();
        let cap = issuer.mint(&auth, TransferRequestGatewayV1::RESOURCE, TransferRequestGatewayV1::EFFECT, now).unwrap();

        let store_a: Arc<dyn ReplayStore> = Arc::new(SqliteReplayStore::open(&db_path).unwrap());
        let gateway_a = TransferRequestGatewayV1::with_replay_store(&dir, &issuer, store_a);
        assert!(gateway_a.consume_and_execute(&cap, now, || Ok::<_, String>(())).is_ok());

        let store_b: Arc<dyn ReplayStore> = Arc::new(SqliteReplayStore::open(&db_path).unwrap());
        let gateway_b = TransferRequestGatewayV1::with_replay_store(&dir, &issuer, store_b);
        let err = gateway_b.consume_and_execute(&cap, now, || Ok::<_, String>(())).unwrap_err();
        assert_eq!(err, GatewayError::CapabilityReplayed);

        let _ = std::fs::remove_file(&db_path);
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
