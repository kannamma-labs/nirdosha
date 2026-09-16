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
    pub nfr: Option<Nfr>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Requires {
    pub role: String,
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
            if let Err(msg) = crate::role::validate_role_name(&req.role) {
                issues.push(format!("requires(role = ...) — {msg}"));
            }
        }
        if let Some(nfr) = &self.nfr {
            if nfr.latency_ms.is_some_and(|v| v <= 0.0) {
                issues.push("nfr: latency_ms must be > 0".into());
            }
            if nfr.concurrency_max.is_some_and(|v| v == 0) {
                issues.push("nfr: concurrency_max must be >= 1".into());
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
                role: "hr_staff".into(),
            }),
            nfr: Some(Nfr {
                latency_ms: Some(50.0),
                error_rate_max: None,
                throughput_min_per_sec: None,
                concurrency_max: Some(1000),
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