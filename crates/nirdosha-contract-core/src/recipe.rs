//! Core, deterministic parts of Nirdosha Recipe Format v1.
//!
//! This module intentionally does not implement Sigstore discovery or an
//! Ed25519 key store. Those are deployment trust-policy concerns. It does
//! implement the bytes that must be identical across runners: JCS, DSSE PAE,
//! self-nulled identifiers, result hashing, and aggregate flattening.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const MEDIA_TYPE: &str = "application/vnd.nirdosha.recipe.v1+json";
pub const RECIPE_VERSION: &str = "1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HashRef {
    pub hash: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Determinism {
    pub mechanism: String,
    pub value: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolverRef {
    pub name: String,
    pub version: String,
    pub hash: String,
    pub determinism: Determinism,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Command {
    pub args: Vec<String>,
    pub stdin: Option<String>,
    #[serde(default)]
    pub env: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvariantResult {
    pub id: String,
    pub result: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counterexample_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counterexample_url: Option<String>,
}

/// Flattening is normative and rejects an empty set: otherwise an empty
/// invariant list would receive a vacuous `unsat` claim.
pub fn aggregate(results: &[InvariantResult]) -> Result<&'static str, String> {
    if results.is_empty() {
        return Err("at least one invariant is required".into());
    }
    let mut previous: Option<&str> = None;
    for result in results {
        if !matches!(result.result.as_str(), "unsat" | "sat" | "unknown") {
            return Err(format!(
                "invariant `{}` has invalid result `{}`",
                result.id, result.result
            ));
        }
        if previous.is_some_and(|id| id >= result.id.as_str()) {
            return Err("per_invariant must be sorted by strictly ascending id".into());
        }
        previous = Some(&result.id);
    }
    if results.iter().any(|r| r.result == "sat") {
        Ok("sat")
    } else if results.iter().any(|r| r.result == "unknown") {
        Ok("unknown")
    } else {
        Ok("unsat")
    }
}

pub fn jcs(value: &Value) -> Result<Vec<u8>, String> {
    serde_jcs::to_vec(value).map_err(|e| format!("JCS encoding failed: {e}"))
}

pub fn sha256_prefixed(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

pub fn pae(typ: &str, body: &[u8]) -> Vec<u8> {
    pae_bytes(typ, body)
}

/// DSSE PAE over arbitrary bytes. The string implementation above is not
/// suitable for non-UTF-8 bodies, so this is the normative byte-preserving
/// implementation used by signing and verification.
pub fn pae_bytes(typ: &str, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(32 + typ.len() + body.len());
    out.extend_from_slice(b"DSSEv1 ");
    out.extend_from_slice(typ.len().to_string().as_bytes());
    out.push(b' ');
    out.extend_from_slice(typ.as_bytes());
    out.push(b' ');
    out.extend_from_slice(body.len().to_string().as_bytes());
    out.push(b' ');
    out.extend_from_slice(body);
    out
}

pub fn result_hash(result: &Value) -> Result<String, String> {
    Ok(sha256_prefixed(&jcs(result)?))
}

pub fn recipe_id(recipe: &Value) -> Result<String, String> {
    let mut copy = recipe.clone();
    let object = copy.as_object_mut().ok_or("recipe must be a JSON object")?;
    object.insert("recipe_id".into(), Value::Null);
    object.insert("signer_signature".into(), Value::Null);
    Ok(sha256_prefixed(&jcs(&copy)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregate_is_deterministic_and_rejects_empty_or_unsorted_results() {
        let a = vec![InvariantResult {
            id: "A".into(),
            result: "unsat".into(),
            counterexample_hash: None,
            counterexample_url: None,
        }];
        assert_eq!(aggregate(&a).unwrap(), "unsat");
        assert!(aggregate(&[]).is_err());
        let bad = vec![
            InvariantResult {
                id: "B".into(),
                result: "unsat".into(),
                counterexample_hash: None,
                counterexample_url: None,
            },
            InvariantResult {
                id: "A".into(),
                result: "unsat".into(),
                counterexample_hash: None,
                counterexample_url: None,
            },
        ];
        assert!(aggregate(&bad).is_err());
    }

    #[test]
    fn pae_preserves_binary_body() {
        let body = [0, 255, 1];
        let encoded = pae_bytes(MEDIA_TYPE, &body);
        assert!(encoded.ends_with(&body));
        assert_eq!(&encoded[..7], b"DSSEv1 ");
    }

    #[test]
    fn recipe_id_ignores_only_self_referential_fields() {
        let a = serde_json::json!({"recipe_id":"old", "signer_signature":{"sig":"old"}, "x":1});
        let b = serde_json::json!({"recipe_id":"new", "signer_signature":{"sig":"new"}, "x":1});
        assert_eq!(recipe_id(&a).unwrap(), recipe_id(&b).unwrap());
    }
}
