//! The contract model and its portable JSON encoding.
//!
//! The encoding is a single-line doc string:
//!
//! ```text
//! nirdosha:contract {"effects":["pure"],"requires":{"role":"hr_staff"},"nfr":{"latency_ms":50,"concurrency_max":1000}}
//! ```
//!
//! `cargo-nirdosha` reads this form when scanning source; the attribute
//! macro emits it so the compiled crate carries its own contracts into
//! rustdoc JSON regardless of which compiler built it. Unknown keys are
//! rejected (`deny_unknown_fields`), so a typo in a hand-written contract
//! is a build error under the Nirdosha compiler, not a silent no-op.

use serde::{Deserialize, Serialize};

/// Every effect name the dialect currently understands.
/// Claiming an unknown effect is a build error under `cargo nirdosha`
/// (and a `compile_error!` under the attribute macro).
pub const KNOWN_EFFECTS: &[&str] = &[
    "pure", "io", "net", "db", "alloc", "clock", "random", "concurrent", "inference",
];

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contract {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effects: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires: Option<Requires>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ensures: Option<Ensures>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nfr: Option<Nfr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crud: Option<Crud>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<Resource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sequence: Option<Sequence>,
}

/// Either a role gate (`requires(role = "hr_staff")`, unforgeable-proof
/// injection, checked by the type system) or a numeric precondition
/// (`requires(idx < len)`, assumed before the body's own MIR proof
/// obligations and discharged by the same Z3 VC IR item 1 built —
/// issue #68). Exactly one of the two, never both or neither: a single
/// `requires(..)` clause is one claim, not a bundle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Requires {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expr: Option<String>,
}

/// `ensures(result >= 0)` — a postcondition over the function's own
/// return value (`result`) and parameters, checked against *every*
/// `return` MIR reaches, the same "claim about every path" semantics
/// `.nir`'s `contract_check.rs` already uses (issue #68).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ensures {
    pub expr: String,
}

/// Every CRUD operation name a `Crud` gate may name, and a `policy!`'s
/// `forbids` list may forbid.
pub const KNOWN_CRUD_OPS: &[&str] = &["create", "read", "update", "delete"];

/// `crud(op = "delete", policy = "financial_us")` — cross-checks this
/// function against a named [`crate::role::Role`]-style policy type at
/// build time (`crates/nirdosha-rt/src/policy.rs`'s `crud_forbidden`).
/// Real `rustc` const-evaluation, not a doc-comment scanner: see
/// `docs/nirdosha-rt-dialect.md`'s "no bespoke parser, ever" rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Crud {
    pub op: String,
    pub policy: String,
}

/// `resource(kind = "lock")` — this fn's own body must acquire and
/// release every value it gets from `nirdosha_rt::resource::acquire`
/// through `nirdosha_rt::resource::release` on *every* path, checked by
/// a real `rustc_mir_dataflow` forward analysis over pre-optimization
/// MIR (issue #69), the same "every path" semantics `requires`/`ensures`
/// (issue #68) already use. `kind` is a free-text label for diagnostics
/// today — every acquire/release pair in a `resource(..)`-claiming fn's
/// body is tracked regardless of the payload type; it does not yet
/// select among multiple concurrently-tracked resource kinds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resource {
    pub kind: String,
}

/// `sequence(before = "debit", after = "credit")` (issue #76) — this
/// fn's own body must never reach a call to `after` on any MIR path
/// where a call to `before` is not *guaranteed* to have already run.
/// Checked by a `rustc_mir_dataflow` "must" analysis over
/// pre-optimization MIR, the same framework and "every path" semantics
/// `resource(kind = ...)` (issue #69) already uses.
///
/// Names are matched by resolved callee (a `DefId`'s own last path
/// segment, not source text), the same real-name-resolution principle
/// `effects(pure)`'s driver checker uses — but, disclosed rather than
/// hidden: this is a **direct-call-only, subject-blind** check. It
/// does not follow into a local wrapper fn that itself calls `before`/
/// `after` transitively (matching `resource(..)`'s own precedent —
/// neither clause stitches interprocedural call graphs), and it does
/// not distinguish *which* value/account a call operates on: calling
/// `before`/`after` on two different subjects (`debit(&a); credit(&b)`)
/// is indistinguishable from calling them on the same one. Both are
/// real, bounded scope decisions, not partial bugs — full subject
/// tracking is a harder, disclosed follow-on (see the issue's own text:
/// "a debit-then-credit rule is an ordering constraint between two
/// named operations on a shared subject ... a different, harder
/// problem" than a single-value resource lifecycle).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sequence {
    pub before: String,
    pub after: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Nfr {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_rate_max: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub throughput_min_per_sec: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concurrency_max: Option<u64>,
}

impl Contract {
    /// The single-line `#[doc = "..."]` encoding of this contract.
    pub fn doc_string(&self) -> String {
        let json = serde_json::to_string(self).expect("contract serializes");
        format!("{} {}", crate::docparse::DOC_PREFIX, json)
    }

    pub fn claims_pure(&self) -> bool {
        self.effects
            .as_ref()
            .is_some_and(|e| e.iter().any(|x| x == "pure"))
    }

    /// Semantic validation shared by the macro and the checker.
    /// Returns human-readable issues; non-empty means the contract is
    /// rejected at build time.
    pub fn validate(&self) -> Vec<String> {
        let mut issues = Vec::new();
        if let Some(effects) = &self.effects {
            if effects.is_empty() {
                issues.push("effects() is empty — list effects or remove the clause".into());
            }
            for e in effects {
                if !KNOWN_EFFECTS.contains(&e.as_str()) {
                    issues.push(format!(
                        "unknown effect `{e}` — known effects: {KNOWN_EFFECTS:?}"
                    ));
                }
            }
        }
        if let Some(req) = &self.requires {
            match (&req.role, &req.expr) {
                (Some(role), None) => {
                    if let Err(msg) = crate::role::validate_role_name(role) {
                        issues.push(format!("requires(role = ...) — {msg}"));
                    }
                }
                (None, Some(expr)) => {
                    if let Err(msg) = crate::predicate::parse_predicate(expr) {
                        issues.push(format!("requires({expr}) — {msg}"));
                    }
                }
                (Some(_), Some(_)) => {
                    issues.push("requires(..) takes a role or an expression, not both".into())
                }
                (None, None) => {
                    issues.push("requires(..) needs a role or a boolean expression".into())
                }
            }
        }
        if let Some(ens) = &self.ensures
            && let Err(msg) = crate::predicate::parse_predicate(&ens.expr)
        {
            issues.push(format!("ensures({}) — {msg}", ens.expr));
        }
        if let Some(nfr) = &self.nfr {
            if nfr.latency_ms.is_some_and(|v| v <= 0.0) {
                issues.push("nfr: latency_ms must be > 0".into());
            }
            if nfr.concurrency_max.is_some_and(|v| v == 0) {
                issues.push("nfr: concurrency_max must be >= 1".into());
            }
        }
        if let Some(crud) = &self.crud {
            if !KNOWN_CRUD_OPS.contains(&crud.op.as_str()) {
                issues.push(format!(
                    "crud(op = ...) — unknown op `{}`, known ops: {KNOWN_CRUD_OPS:?}",
                    crud.op
                ));
            }
            if let Err(msg) = crate::role::validate_role_name(&crud.policy) {
                issues.push(format!("crud(policy = ...) — {msg}"));
            }
        }
        if let Some(resource) = &self.resource
            && resource.kind.is_empty()
        {
            issues.push("resource(kind = ...) — kind must not be empty".into());
        }
        if let Some(sequence) = &self.sequence {
            if sequence.before.is_empty() || sequence.after.is_empty() {
                issues.push("sequence(before = ..., after = ...) — neither name may be empty".into());
            } else if sequence.before == sequence.after {
                issues.push("sequence(before = ..., after = ...) — before and after must name different fns".into());
            }
        }
        issues
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_encoding_roundtrips() {
        let c = Contract {
            effects: Some(vec!["pure".into()]),
            requires: Some(Requires {
                role: Some("hr_staff".into()),
                expr: None,
            }),
            ensures: Some(Ensures {
                expr: "result >= 0".into(),
            }),
            nfr: Some(Nfr {
                latency_ms: Some(50.0),
                error_rate_max: None,
                throughput_min_per_sec: None,
                concurrency_max: Some(1000),
            }),
            crud: Some(Crud {
                op: "delete".into(),
                policy: "financial_us".into(),
            }),
            resource: Some(Resource {
                kind: "lock".into(),
            }),
            sequence: Some(Sequence {
                before: "debit".into(),
                after: "credit".into(),
            }),
        };
        let doc = c.doc_string();
        assert!(doc.starts_with("nirdosha:contract {"));
        match crate::docparse::parse_doc(&doc) {
            Ok(Some(back)) => assert_eq!(back, c),
            _ => panic!("doc contract did not roundtrip: {doc}"),
        }
    }

    #[test]
    fn crud_rejects_unknown_op() {
        let c = Contract {
            crud: Some(Crud {
                op: "wipe".into(),
                policy: "financial_us".into(),
            }),
            ..Default::default()
        };
        assert!(!c.validate().is_empty());
    }

    #[test]
    fn sequence_rejects_identical_before_and_after() {
        let c = Contract {
            sequence: Some(Sequence {
                before: "debit".into(),
                after: "debit".into(),
            }),
            ..Default::default()
        };
        assert!(!c.validate().is_empty());
    }

    #[test]
    fn unknown_keys_are_rejected() {
        let doc = r#"nirdosha:contract {"effekts":["pure"]}"#;
        match crate::docparse::parse_doc(doc) {
            Err(msg) => assert!(msg.contains("effekts"), "should name the bad key: {msg}"),
            _ => panic!("typo'd contract key must be rejected"),
        }
    }

    #[test]
    fn validate_catches_unknown_effect() {
        let c = Contract {
            effects: Some(vec!["purr".into()]),
            ..Default::default()
        };
        assert!(!c.validate().is_empty());
    }
}