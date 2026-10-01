//! Observation + audit envelope (RFC 0026 §7.1–§7.2, I18, MP-3).
//!
//! Phase 1: a pure library. The invariant-bearing runtime collector — I18
//! "only the kernel appends", MP-2 batching, the MP-9 equivalence probe —
//! lands with the MIC in Phase 2. Nothing here may be described as
//! enforcement yet.

use nirdosha_guard_core::{Destination, LineageEntity, LineageFacts, PolicyVersion, Purpose};
use serde::{Deserialize, Serialize};

use crate::{
    Authority, DriverRef, EdgeType, EntityRef, FlowCompleteness, KeyRef, NodeId, TransformId,
    TraceId,
};

/// Audit-content discriminator for lineage records.
pub const RECORD_KIND: &str = "lineage";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CollectorConfig { pub enabled: bool }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservationContext {
    pub module: String,
    pub trace_id: TraceId,
    pub subject_id: String,
    pub tenant_id: String,
    pub policy_version: PolicyVersion,
    pub purpose: Purpose,
    pub destination: Destination,
    pub receipt_digest: [u8; 32],
}

#[derive(Debug, Clone, Copy)]
pub struct KernelCollector { pub config: CollectorConfig }

impl KernelCollector {
    pub fn observe(&self, facts: PlanFacts, driver_facts: Option<&LineageFacts>, context: ObservationContext) -> Option<LineageObservation> {
        if !self.config.enabled { return None; }
        let mut observation = LineageObservation::from_context(&context.module, &context.trace_id, &context.subject_id, &context.tenant_id, context.policy_version, context.purpose, context.destination, context.receipt_digest, facts);
        if let Some(driver_facts) = driver_facts { observation.enrich(driver_facts); }
        Some(observation)
    }
}

pub fn mp9_decision_equivalence<T>(mut run: impl FnMut(bool) -> Vec<T>) -> Result<(), String>
where
    T: PartialEq,
{
    let with_collector = run(true);
    let without_collector = run(false);
    if with_collector == without_collector { Ok(()) } else { Err("collector changed decision-visible output".into()) }
}

// ─────────────────────────────────────────────────────────────────────────────
// Kernel-side facts (plan IR + decision) — the collector's own knowledge
// ─────────────────────────────────────────────────────────────────────────────

/// Kernel-side facts. The driver cannot supply any of these — that is the
/// MP-3 trust argument. `sink` comes from plan IR; drivers may only refine
/// its keys via `LineageFacts::sink_keys`.
pub struct PlanFacts {
    pub edge_type: EdgeType,
    pub transformation: TransformId,
    pub driver: DriverRef,
    pub authority: Authority,
    pub completeness: FlowCompleteness,
    pub sampled: bool,
    pub degraded: bool,
    pub sink: LineageEntity,
}

// ─────────────────────────────────────────────────────────────────────────────
// The observation — one guarded execution, kernel-merged (I18 record)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineageObservation {
    pub trace_id: TraceId,
    pub module: String,
    pub subject_id: String,
    pub tenant_id: String,
    pub sources: Vec<EntityRef>,
    pub sink: EntityRef,
    pub edge_type: EdgeType,
    pub transformation: TransformId,
    pub policy_version: PolicyVersion,
    pub purpose: Purpose,
    pub destination: Destination,
    pub driver: DriverRef,
    pub authority: Authority,
    pub completeness: FlowCompleteness,
    pub sampled: bool,
    pub degraded: bool,
    pub receipt_digest: [u8; 32],
}

/// Deterministic core-id → graph-node resolution (collector-owned, §7.1).
/// Phase 1: the entity id becomes a Data-plane node id; kind/version inference
/// from the registry lands in Phase 2. Keys pass through opaquely — they are
/// pre-tokenized by contract (guard-core `LineageEntity`).
pub fn resolve_entity(e: &LineageEntity) -> EntityRef {
    EntityRef {
        node: NodeId::data(e.entity.clone(), None),
        keys: if e.keys.is_empty() {
            None
        } else {
            Some(e.keys.iter().map(|k| KeyRef { token: k.clone() }).collect())
        },
    }
}

impl LineageObservation {
    /// I18-unconditional construction: a valid observation from kernel facts
    /// alone — no driver cooperation required. The Phase-2 runtime collector
    /// (MIC) calls this for every plan execution; `enrich` is optional.
    ///
    /// Deliberate shape: explicit context facts rather than
    /// `&EvaluationContext` — this crate stays decoupled from the context's
    /// exact field layout, and the runtime adapter maps ctx + snapshot
    /// version into these params in exactly one place.
    #[allow(clippy::too_many_arguments)]
    pub fn from_context(
        module: &str,
        trace_id: &TraceId,
        subject_id: &str,
        tenant_id: &str,
        policy_version: PolicyVersion,
        purpose: Purpose,
        destination: Destination,
        receipt_digest: [u8; 32],
        facts: PlanFacts,
    ) -> Self {
        LineageObservation {
            trace_id: trace_id.clone(),
            module: module.to_owned(),
            subject_id: subject_id.to_owned(),
            tenant_id: tenant_id.to_owned(),
            sources: Vec::new(),
            sink: resolve_entity(&facts.sink),
            edge_type: facts.edge_type,
            transformation: facts.transformation,
            policy_version,
            purpose,
            destination,
            driver: facts.driver,
            authority: facts.authority,
            completeness: facts.completeness,
            sampled: facts.sampled,
            degraded: facts.degraded,
            receipt_digest,
        }
    }

    /// Optional driver enrichment (guard-core `LineageFacts`). Kernel-owned
    /// fields are unreachable here by construction: the driver can add
    /// row-level sources and refine the sink's keys — nothing else. This is
    /// the code-level answer to "whose sink is it?": the kernel's, always.
    pub fn enrich(&mut self, facts: &LineageFacts) {
        for src in &facts.sources {
            self.sources.push(resolve_entity(src));
        }
        if facts.sink_keys.is_empty() {
            return;
        }
        let keys = self.sink.keys.get_or_insert_with(Vec::new);
        for k in &facts.sink_keys {
            if !keys.iter().any(|t| &t.token == k) {
                keys.push(KeyRef { token: k.clone() });
            }
        }
    }

    /// Audit-chain content (the payload inside the module's hash-chain
    /// envelope; the chain itself supplies seq/ts/prev_hash/hash).
    /// Serialization of these plain data shapes is infallible.
    pub fn to_audit_content(&self) -> serde_json::Value {
        let content = LineageAuditContent {
            kind: RECORD_KIND.to_owned(),
            trace_id: self.trace_id.clone(),
            module: self.module.clone(),
            policy_versions: vec![self.policy_version.clone()],
            observation: ObservationBody::from(self),
        };
        serde_json::to_value(&content).expect("lineage observation serialization is infallible")
    }

    /// Reconstruct from audit content. `None` for non-lineage records AND
    /// malformed lineage records — Phase 3's V10 mode-(a) replay distinguishes
    /// absent vs malformed (tracked in the 0026 checklist).
    pub fn from_audit_content(content: &serde_json::Value) -> Option<Self> {
        let parsed: LineageAuditContent = serde_json::from_value(content.clone()).ok()?;
        if parsed.kind != RECORD_KIND {
            return None;
        }
        Some(LineageObservation::from(parsed))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Audit envelope — RFC 0025 §6.3-compatible content shape
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LineageAuditContent {
    kind: String,
    trace_id: TraceId,
    module: String,
    /// Envelope copy for chain tooling; the canonical value lives in the body.
    #[serde(default)]
    #[allow(dead_code)]
    policy_versions: Vec<PolicyVersion>,
    observation: ObservationBody,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ObservationBody {
    subject_id: String,
    tenant_id: String,
    sources: Vec<EntityRef>,
    sink: EntityRef,
    edge_type: EdgeType,
    transformation: TransformId,
    policy_version: PolicyVersion,
    purpose: Purpose,
    destination: Destination,
    driver: DriverRef,
    authority: Authority,
    completeness: FlowCompleteness,
    sampled: bool,
    degraded: bool,
    receipt_digest: [u8; 32],
}

impl From<&LineageObservation> for ObservationBody {
    fn from(o: &LineageObservation) -> Self {
        ObservationBody {
            subject_id: o.subject_id.clone(),
            tenant_id: o.tenant_id.clone(),
            sources: o.sources.clone(),
            sink: o.sink.clone(),
            edge_type: o.edge_type,
            transformation: o.transformation.clone(),
            policy_version: o.policy_version.clone(),
            purpose: o.purpose.clone(),
            destination: o.destination.clone(),
            driver: o.driver.clone(),
            authority: o.authority,
            completeness: o.completeness,
            sampled: o.sampled,
            degraded: o.degraded,
            receipt_digest: o.receipt_digest,
        }
    }
}

impl From<LineageAuditContent> for LineageObservation {
    fn from(c: LineageAuditContent) -> Self {
        let b = c.observation;
        LineageObservation {
            trace_id: c.trace_id,
            module: c.module,
            subject_id: b.subject_id,
            tenant_id: b.tenant_id,
            sources: b.sources,
            sink: b.sink,
            edge_type: b.edge_type,
            transformation: b.transformation,
            policy_version: b.policy_version,
            purpose: b.purpose,
            destination: b.destination,
            driver: b.driver,
            authority: b.authority,
            completeness: b.completeness,
            sampled: b.sampled,
            degraded: b.degraded,
            receipt_digest: b.receipt_digest,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
pub(crate) mod test_support {
    //! Shared sample builder. Destination variant naming is consolidated here
    //! (single verify point against guard-core's actual enum).

    use nirdosha_guard_core::{Destination, LineageEntity, PolicyVersion, Purpose};

    use crate::{Authority, DriverRef, EdgeType, FlowCompleteness, TransformId};

    use super::{LineageObservation, PlanFacts};

    pub fn sample_observation(
        authority: Authority,
        purpose_code: &str,
        trace_id: &str,
    ) -> LineageObservation {
        let facts = PlanFacts {
            edge_type: EdgeType::Read,
            transformation: TransformId::Window("velocity_1h".to_owned()),
            driver: DriverRef {
                port: "StreamPort".to_owned(),
                vendor: "kafka".to_owned(),
                version: "0.12.0".to_owned(),
            },
            authority,
            completeness: FlowCompleteness::Full,
            sampled: false,
            degraded: false,
            sink: LineageEntity { entity: "txn_events".to_owned(), keys: vec![] },
        };
        let mut obs = LineageObservation::from_context(
            "svc:features",
            &trace_id.to_owned(),
            "user-42",
            "tenant-1",
            PolicyVersion::from("2025.11.4"),
            Purpose(purpose_code.to_owned()),
            Destination::ApiClient,
            [7u8; 32],
            facts,
        );
        // A read always has at least one source; from_context starts empty by
        // design (I18 unconditional), so tests attach one explicit source to
        // exercise edge projection.
        obs.enrich(&nirdosha_guard_core::LineageFacts {
            sources: vec![LineageEntity { entity: "txn_events".to_owned(), keys: vec![] }],
            sink_keys: vec![],
        });
        obs
    }
}

#[cfg(test)]
mod tests {
    use nirdosha_guard_core::LineageFacts;

    use super::test_support::sample_observation;
    use super::*;
    use crate::NodeKind;

    #[test]
    fn i18_unconditional_from_context() {
        // No driver facts anywhere — a valid observation must still exist.
        let facts = PlanFacts {
            edge_type: EdgeType::Read,
            transformation: TransformId::Policy("ingest-create-txn".to_owned()),
            driver: DriverRef {
                port: "StreamPort".to_owned(),
                vendor: "kafka".to_owned(),
                version: "0.12.0".to_owned(),
            },
            authority: Authority::KernelExecution,
            completeness: FlowCompleteness::Full,
            sampled: false,
            degraded: false,
            sink: LineageEntity { entity: "txn_events".to_owned(), keys: vec![] },
        };
        let obs = LineageObservation::from_context(
            "svc:ingest",
            &"trace-1".to_owned(),
            "user-42",
            "tenant-1",
            PolicyVersion::from("2025.11.4"),
            Purpose("fraud_monitoring".to_owned()),
            Destination::ApiClient,
            [7u8; 32],
            facts,
        );
        assert!(obs.sources.is_empty());
        assert_eq!(obs.sink.node.catalog_id, "txn_events");
        assert_eq!(obs.sink.node.kind, NodeKind::Data);
        assert_eq!(obs.policy_version, "2025.11.4");
    }

    #[test]
    fn enrich_adds_sources_and_sink_keys_but_never_the_sink_identity() {
        let mut obs = LineageObservation::from_context(
            "svc:ingest",
            &"trace-1".to_owned(),
            "user-42",
            "tenant-1",
            PolicyVersion::from("2025.11.4"),
            Purpose("fraud_monitoring".to_owned()),
            Destination::ApiClient,
            [7u8; 32],
            PlanFacts {
                edge_type: EdgeType::Read,
                transformation: TransformId::Policy("ingest-create-txn".to_owned()),
                driver: DriverRef {
                    port: "StreamPort".to_owned(),
                    vendor: "kafka".to_owned(),
                    version: "0.12.0".to_owned(),
                },
                authority: Authority::KernelExecution,
                completeness: FlowCompleteness::Full,
                sampled: false,
                degraded: false,
                sink: LineageEntity { entity: "txn_events".to_owned(), keys: vec![] },
            },
        );

        obs.enrich(&LineageFacts {
            sources: vec![LineageEntity {
                entity: "velocity_1h".to_owned(),
                keys: vec!["tok_subj_42".to_owned()],
            }],
            sink_keys: vec!["tok_sink_row_9".to_owned()],
        });

        assert_eq!(obs.sources.len(), 1);
        assert_eq!(obs.sources[0].node.catalog_id, "velocity_1h");
        assert_eq!(
            obs.sources[0].keys.as_ref().unwrap()[0].token,
            "tok_subj_42"
        );
        // Kernel sink identity stands; only keys were refined.
        assert_eq!(obs.sink.node.catalog_id, "txn_events");
        assert_eq!(obs.sink.keys.as_ref().unwrap()[0].token, "tok_sink_row_9");
    }

    #[test]
    fn audit_content_round_trips() {
        let obs = sample_observation(Authority::KernelExecution, "fraud_monitoring", "trace-1");
        let content = obs.to_audit_content();
        assert_eq!(content["kind"], "lineage");
        let back = LineageObservation::from_audit_content(&content)
            .expect("round trip must reconstruct");
        assert_eq!(obs, back);
    }

    #[test]
    fn non_lineage_content_is_filtered() {
        let other = serde_json::json!({ "kind": "deny", "trace_id": "t" });
        assert!(LineageObservation::from_audit_content(&other).is_none());
    }

    #[test]
    fn disabled_collector_emits_nothing_and_mp9_compares_outputs() {
        let disabled = KernelCollector { config: CollectorConfig { enabled: false } };
        let facts = PlanFacts {
            edge_type: EdgeType::Read,
            transformation: TransformId::Policy("v1".into()),
            driver: DriverRef { port: "store".into(), vendor: "memory".into(), version: "1".into() },
            authority: Authority::KernelExecution,
            completeness: FlowCompleteness::Full,
            sampled: false,
            degraded: false,
            sink: LineageEntity { entity: "orders".into(), keys: vec![] },
        };
        let context = ObservationContext { module: "m".into(), trace_id: "t".into(), subject_id: "u".into(), tenant_id: "tenant".into(), policy_version: "v1".into(), purpose: Purpose("p".into()), destination: Destination::ApiClient, receipt_digest: [0; 32] };
        assert!(disabled.observe(facts, None, context).is_none());
        assert!(mp9_decision_equivalence(|_| vec!["allow"]).is_ok());
        assert!(mp9_decision_equivalence(|enabled| vec![enabled]).is_err());
    }
}
