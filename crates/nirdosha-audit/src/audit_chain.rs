//! A real, minimal hash-chained audit log for the compiler's *own*
//! code-generation events -- `docs/API_TRUST_MODEL.md` §9 (T14 in its
//! decision matrix): "Business-*data* mutations already get a real
//! hash-chained audit trail (`finish_with_audit`); code-generation
//! events do not." Checked directly, not assumed: neither
//! `finish_with_audit` nor the `examples/trade-finance/trade_finance.nir`
//! file §9 cites, nor a `ledger.rs`/`capability.rs` `docs/goal.md` row 10
//! claims are "already in this codebase," actually exist in this
//! checkout (confirmed by search, 2026-09-15) -- so this is a real
//! from-scratch minimal version of the proposed mechanism, on the
//! tooling side, not an extension of something pre-existing.
//!
//! **Why this file, not a bigger rewrite of every self-modifying event
//! in one pass.** Two real code-generation-audit-shaped logs already
//! exist and were both flat, non-tamper-evident JSON Lines appends
//! before this file: `hint_cache.rs`'s `self_repair_hint_promotions.log`
//! (the self-repair loop teaching itself a new hint from a real failure
//! -- exactly T14's example shape: "the six-eyes threshold function was
//! auto-modified... because a proof found a counterexample") and
//! `hi_plugin.rs`'s `pack_signing_log.jsonl`. Rather than inventing a
//! third bespoke format, this module is the one hash-chaining primitive
//! both should use -- `hint_cache.rs` is wired to it (below); the pack-
//! signing log is real, separate follow-up (same shape, different call
//! site, not touched here to keep this change's blast radius honest).
//!
//! **The chain, concretely**: entry *n*'s `hash` is `sha256_hex(prev_hash
//! ++ content)`, where `content` is the entry's own canonical JSON
//! (everything except `hash` itself) and `prev_hash` is entry *n-1*'s
//! `hash` (`GENESIS_HASH`, 64 `'0'` characters, for the first entry).
//! This is the exact byte-concatenation-then-hash scheme
//! `crates/runtime-kernels/src/lib.rs`'s own `sha256_hex(prev_hash,
//! payload)` 2-arg chained form already implements for `.nir` programs
//! (`nir_sha256_hex`'s doc comment) -- same scheme, independent
//! implementation, so a `.nir` program and this tooling-side log could
//! in principle cross-verify each other's chains without translation.
//! Tampering with entry *n*'s stored content without recomputing every
//! later entry's `hash` is detectable by `verify_chain` -- the same
//! tamper-evidence property `verify_audit_chain` (§9's cited, not-yet-
//! real, `.nir`-level analogue) is meant to provide.

use std::io::Write;
use std::path::Path;

/// The first entry's `prev_hash` -- an explicit, documented sentinel
/// (not e.g. an empty string) so a genesis entry is unambiguously
/// distinguishable from a corrupted `prev_hash` field. 64 `'0'`
/// characters: the same length as a real sha256 hex digest, so it can
/// never be confused with a truncated real hash, pinned by
/// `genesis_hash_is_a_real_64_char_hex_length_sentinel` below.
pub const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// One entry in the chain, exactly as stored (one JSON object per line
/// in the log file, `serde_json` round-trippable).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct AuditEntry {
    pub seq: u64,
    pub timestamp: u64,
    /// Freeform, caller-defined payload -- for the self-repair-hint
    /// call site, `{"pattern": ..., "hint": ...}`; kept as an arbitrary
    /// `serde_json::Value` rather than a fixed struct so a second call
    /// site (e.g. the pack-signing log, real follow-up) can reuse this
    /// same chain primitive without this module knowing its shape.
    pub content: serde_json::Value,
    pub prev_hash: String,
    pub hash: String,
}

/// `crypto_backend::sha256` (2026-09) -- `fips`-feature-aware, same
/// reason `hi_graph::sha256_hex` routes through it now instead of a
/// separate direct `sha2` call.
fn sha256_hex(bytes: &[u8]) -> String {
    crate::crypto_backend::sha256(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// `prev_hash ++ canonical_content_json`, hashed -- the one place both
/// `append_entry` and `verify_chain` compute an entry's `hash`, so they
/// can never drift into two different notions of "correct".
fn entry_hash(prev_hash: &str, seq: u64, timestamp: u64, content: &serde_json::Value) -> Result<String, String> {
    // `seq`/`timestamp` are folded into the hashed payload (not just
    // `content`) so reordering or retiming a real entry is also
    // detected, not only a tampered `content` field.
    let canonical = serde_json::json!({ "seq": seq, "timestamp": timestamp, "content": content });
    let canonical_bytes = serde_json::to_vec(&canonical).map_err(|e| format!("audit entry is not serializable: {e}"))?;
    let mut payload = Vec::with_capacity(prev_hash.len() + canonical_bytes.len());
    payload.extend_from_slice(prev_hash.as_bytes());
    payload.extend_from_slice(&canonical_bytes);
    Ok(sha256_hex(&payload))
}

/// Reads `path`'s last line (if any) to find the current chain tip,
/// computes the new entry against it, and appends -- best-effort on
/// disk I/O, matching every other audit-shaped log in this codebase
/// (`hint_cache.rs::record_success`, `hi_plugin.rs::
/// append_pack_signing_log`): a log write that can't be durably made
/// must never be the reason the actual self-repair/pack-install
/// decision it's recording fails. Returns the entry that was (or would
/// have been) appended either way, so a caller can still act on it
/// (e.g. log a one-line warning) even when the disk write itself failed.
pub fn append_entry(path: &Path, content: serde_json::Value, now: u64) -> AuditEntry {
    let prev_hash = last_hash(path).unwrap_or_else(|| GENESIS_HASH.to_string());
    let seq = last_seq(path).map(|s| s + 1).unwrap_or(0);
    // `entry_hash` only fails if `content` can't serialize at all
    // (never true for a `serde_json::Value` built from already-valid
    // JSON) -- falling back to a fixed sentinel hash rather than
    // panicking keeps this function's "never blocks the real
    // operation" contract even in that unreachable case.
    let hash = entry_hash(&prev_hash, seq, now, &content).unwrap_or_else(|_| "0".repeat(64));
    let entry = AuditEntry { seq, timestamp: now, content, prev_hash, hash };
    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return entry;
        }
    }
    if let Ok(line) = serde_json::to_string(&entry) {
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "{line}");
        }
    }
    entry
}

/// Public per T-11 (RTM `screens-plan.md` B8): the audit-projection layer
/// needs to read a module chain's entries back, not just verify their
/// count -- `verify_chain` alone can't feed `ChainReconciler::reconcile`.
/// Same file-format contract `verify_chain`/`append_entry` already share.
pub fn read_entries(path: &Path) -> Vec<AuditEntry> {
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    text.lines().filter(|l| !l.trim().is_empty()).filter_map(|l| serde_json::from_str(l).ok()).collect()
}

fn last_hash(path: &Path) -> Option<String> {
    read_entries(path).last().map(|e| e.hash.clone())
}

fn last_seq(path: &Path) -> Option<u64> {
    read_entries(path).last().map(|e| e.seq)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChainError {
    /// Entry at this index (0-based, file order) has a `hash` that
    /// doesn't match recomputing it from its own `seq`/`timestamp`/
    /// `content`/`prev_hash` -- its content was altered after the fact,
    /// or the hash field itself was.
    HashMismatch { index: usize, seq: u64 },
    /// Entry at this index's `prev_hash` doesn't match the previous
    /// entry's `hash` (or, for index 0, doesn't match `GENESIS_HASH`) --
    /// an entry was deleted, reordered, or spliced in.
    ChainBroken { index: usize, seq: u64 },
}

impl std::fmt::Display for ChainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChainError::HashMismatch { index, seq } => write!(f, "audit chain entry #{index} (seq {seq}) has been altered: its stored hash does not match its own content"),
            ChainError::ChainBroken { index, seq } => write!(f, "audit chain entry #{index} (seq {seq})'s prev_hash does not match the preceding entry's hash -- an entry was deleted, reordered, or inserted"),
        }
    }
}

/// Independently verifies every entry in `path`'s log, in file order:
/// each entry's `hash` must be a correct recomputation of its own
/// content, and each entry's `prev_hash` must equal the previous
/// entry's `hash` (or `GENESIS_HASH` for the first). Returns the number
/// of entries verified on success. An empty or missing file verifies
/// trivially as `Ok(0)` -- "no audit events yet" is not itself evidence
/// of tampering.
pub fn verify_chain(path: &Path) -> Result<usize, ChainError> {
    let entries = read_entries(path);
    let mut expected_prev = GENESIS_HASH.to_string();
    for (index, entry) in entries.iter().enumerate() {
        if entry.prev_hash != expected_prev {
            return Err(ChainError::ChainBroken { index, seq: entry.seq });
        }
        let recomputed = entry_hash(&entry.prev_hash, entry.seq, entry.timestamp, &entry.content).unwrap_or_default();
        if recomputed != entry.hash {
            return Err(ChainError::HashMismatch { index, seq: entry.seq });
        }
        expected_prev = entry.hash.clone();
    }
    Ok(entries.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_log_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("nirdosha_audit_chain_test_{name}_{}_{}.jsonl", std::process::id(), name.len()))
    }

    #[test]
    fn a_fresh_chain_starts_from_genesis_and_verifies() {
        let path = temp_log_path("fresh");
        let _ = std::fs::remove_file(&path);
        let e1 = append_entry(&path, serde_json::json!({"pattern": "p1", "hint": "h1"}), 100);
        assert_eq!(e1.seq, 0);
        assert_eq!(e1.prev_hash, GENESIS_HASH);
        let e2 = append_entry(&path, serde_json::json!({"pattern": "p2", "hint": "h2"}), 200);
        assert_eq!(e2.seq, 1);
        assert_eq!(e2.prev_hash, e1.hash, "entry 2 must chain onto entry 1's real hash");
        assert_eq!(verify_chain(&path), Ok(2));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn tampering_with_a_stored_entrys_content_is_detected() {
        let path = temp_log_path("tamper_content");
        let _ = std::fs::remove_file(&path);
        append_entry(&path, serde_json::json!({"pattern": "p1", "hint": "h1"}), 100);
        append_entry(&path, serde_json::json!({"pattern": "p2", "hint": "h2"}), 200);
        // Tamper: rewrite the first line's `content.hint` without
        // recomputing its `hash` -- exactly what an attacker editing
        // the file by hand, or a buggy tool, would do.
        let text = std::fs::read_to_string(&path).unwrap();
        let tampered = text.replacen("\"h1\"", "\"tampered\"", 1);
        std::fs::write(&path, tampered).unwrap();
        match verify_chain(&path) {
            Err(ChainError::HashMismatch { index: 0, .. }) => {}
            other => panic!("expected a HashMismatch at index 0, got {other:?}"),
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn deleting_an_entry_breaks_the_chain_for_the_next_one() {
        let path = temp_log_path("tamper_delete");
        let _ = std::fs::remove_file(&path);
        append_entry(&path, serde_json::json!({"n": 1}), 100);
        append_entry(&path, serde_json::json!({"n": 2}), 200);
        append_entry(&path, serde_json::json!({"n": 3}), 300);
        // Delete the middle line -- entry 3's `prev_hash` now points at
        // a hash that's no longer the immediately preceding entry's.
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        let spliced = format!("{}\n{}\n", lines[0], lines[2]);
        std::fs::write(&path, spliced).unwrap();
        match verify_chain(&path) {
            Err(ChainError::ChainBroken { index: 1, .. }) => {}
            other => panic!("expected a ChainBroken at index 1, got {other:?}"),
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_missing_file_verifies_as_zero_entries_not_an_error() {
        let path = temp_log_path("missing");
        let _ = std::fs::remove_file(&path);
        assert_eq!(verify_chain(&path), Ok(0));
    }

    #[test]
    fn genesis_hash_is_a_real_64_char_hex_length_sentinel() {
        assert_eq!(GENESIS_HASH.len(), 64, "sha256 hex digests are 64 chars -- the genesis sentinel must be the same length so it's never confused with a truncated real hash");
        assert!(GENESIS_HASH.chars().all(|c| c == '0'));
    }
}
