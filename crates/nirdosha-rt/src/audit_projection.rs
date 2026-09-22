//! Unified audit-chain projection (T-11): `AC.all_chains` over the
//! per-domain hash-chained logs `GuardClient` already writes on every
//! guarded decision (`nirdosha_audit::envelope::ModuleAuditChain`, one
//! file per `bridge.nir` table). This module does not invent a second
//! audit mechanism — it reads those real chains back and merges them
//! (see [`project`]'s own doc comment for why it sorts directly instead
//! of via `nirdosha_audit`'s `ChainReconciler`).
//!
//! Immutability at the projection layer (T-11's own done-when) means: a
//! source chain that fails its own `verify()` is excluded from the
//! projection and reported as tampered, never silently included. The
//! projection itself has no update/delete path — only [`project`], which
//! rebuilds the merged view fresh from the source chains every time.

use nirdosha_audit::audit_chain::AuditEntry;
use nirdosha_audit::envelope::ModuleAuditChain;

/// One projected row — the flattened shape `bridge.nir`'s `AllChainsRow`
/// persists into its `GuardedTable` so `crud_screens!` screens (20.1, and
/// the `ac_timeline`/`timeline` combos on 3.10/12.11/14.5) can read it
/// like any other guarded dataset.
#[derive(Debug, Clone)]
pub struct ProjectedEntry {
    pub module: String,
    pub seq: u64,
    pub ts_ms: u64,
    pub trace_id: String,
    pub subject: String,
    pub action: String,
    pub resource: String,
    pub decision: String,
    pub kind: String,
    pub content: serde_json::Value,
}

/// A source chain named for the projection — `module` here is the label
/// used in the projected rows (e.g. `"alert"`), independent of whatever
/// module name the underlying `ModuleAuditChain` itself was constructed
/// with (`GuardClient::new`'s `module` arg is mostly used for envelope
/// assertions, not a naming contract this crate should assume matches).
pub struct ChainSource {
    pub label: &'static str,
    pub chain: ModuleAuditChain,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TamperedChain {
    pub label: String,
    pub error: String,
}

/// Reads every source chain, verifies each independently (T-11's
/// immutability requirement), and merges the intact ones into one
/// timestamp-ordered projection, tagging each row with its source
/// `label`. Tampered chains are reported, not merged in — a caller
/// decides how loudly to surface that (the rtm integration test asserts
/// a corrupted source chain is excluded and named, not silently
/// absorbed).
///
/// **Not `ChainReconciler::reconcile`.** That helper's secondary sort key
/// is the envelope's own `content.module` field — real for the two
/// hand-built chains its own unit test constructs, but every RTM
/// `GuardedTable` in `bridge.nir` is built via `GuardedTable::new(...,
/// module: "rtm-demo", ...)`, one literal string shared by every table
/// (`GuardClient`'s `module` constructor arg, not a per-domain one) — so
/// `content.module` is uniformly `"rtm-demo"` here and carries no
/// per-domain information to sort or filter by. This fn sorts on
/// `(timestamp, label, seq)` instead, where `label` is the caller-chosen
/// per-source tag (`"alert"`, `"case"`, ...) `ChainSource` already
/// carries — the real domain distinction for this corpus.
pub fn project(sources: &[ChainSource]) -> (Vec<ProjectedEntry>, Vec<TamperedChain>) {
    let mut tampered = Vec::new();
    let mut labeled: Vec<(&str, AuditEntry)> = Vec::new();
    for source in sources {
        match source.chain.verify() {
            Ok(_) => labeled.extend(source.chain.entries().into_iter().map(|e| (source.label, e))),
            Err(err) => tampered.push(TamperedChain { label: source.label.to_string(), error: err.to_string() }),
        }
    }
    labeled.sort_by(|(left_label, left), (right_label, right)| {
        left.timestamp.cmp(&right.timestamp).then_with(|| left_label.cmp(right_label)).then_with(|| left.seq.cmp(&right.seq))
    });

    let projected = labeled
        .into_iter()
        .map(|(label, entry)| {
            let trace_id = entry.content.get("trace_id").and_then(|v| v.as_str()).unwrap_or_default().to_string();
            let subject = entry.content.get("subject").and_then(|v| v.as_str()).unwrap_or_default().to_string();
            let action = entry.content.get("action").and_then(|v| v.as_str()).unwrap_or_default().to_string();
            let resource = entry.content.get("resource").and_then(|v| v.as_str()).unwrap_or_default().to_string();
            let decision = entry.content.get("decision").and_then(|v| v.as_str()).unwrap_or_default().to_string();
            let kind = entry.content.get("kind").and_then(|v| v.as_str()).unwrap_or_default().to_string();
            let content = entry.content.get("content").cloned().unwrap_or(serde_json::Value::Null);
            ProjectedEntry { module: label.to_string(), seq: entry.seq, ts_ms: entry.timestamp, trace_id, subject, action, resource, decision, kind, content }
        })
        .collect();
    (projected, tampered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nirdosha_audit::envelope::{AuditEnvelope, AuditRecordKind};

    fn envelope(module: &str, trace_id: &str, action: &str) -> AuditEnvelope {
        AuditEnvelope {
            trace_id: trace_id.into(),
            ts: "2026-09-22T00:00:00.000Z".into(),
            module: module.into(),
            subject: "analyst-1".into(),
            action: action.into(),
            resource: format!("{module}:1"),
            policy_versions: vec!["v1".into()],
            decision: "Allow".into(),
            obligations: vec![],
            kind: AuditRecordKind::Decision,
            content: serde_json::json!({}),
        }
    }

    fn temp_chain(module: &str, file_tag: &str) -> ModuleAuditChain {
        let path = std::env::temp_dir().join(format!("nirdosha_rt_audit_projection_test_{file_tag}_{}.jsonl", std::process::id()));
        let _ = std::fs::remove_file(&path);
        ModuleAuditChain::new(module, path)
    }

    #[test]
    fn projects_across_modules_in_timestamp_order() {
        let alert = temp_chain("alert", "alert_proj");
        let case = temp_chain("case", "case_proj");
        alert.append(&envelope("alert", "a1", "Read"), 200);
        case.append(&envelope("case", "c1", "Read"), 100);
        alert.append(&envelope("alert", "a2", "Update"), 300);

        let (rows, tampered) = project(&[
            ChainSource { label: "alert", chain: alert },
            ChainSource { label: "case", chain: case },
        ]);
        assert!(tampered.is_empty());
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].trace_id, "c1", "ts=100 sorts first across modules");
        assert_eq!(rows[1].trace_id, "a1");
        assert_eq!(rows[2].trace_id, "a2");
        assert_eq!(rows[0].module, "case");
        assert_eq!(rows[1].action, "Read");
    }

    #[test]
    fn a_tampered_source_chain_is_excluded_and_named_not_silently_merged() {
        let alert = temp_chain("alert", "alert_tamper");
        alert.append(&envelope("alert", "a1", "Read"), 100);
        alert.append(&envelope("alert", "a2", "Update"), 200);
        let path = alert.path().to_path_buf();
        let text = std::fs::read_to_string(&path).unwrap();
        let tampered_text = text.replacen("\"Read\"", "\"Delete\"", 1);
        std::fs::write(&path, tampered_text).unwrap();

        let case = temp_chain("case", "case_intact");
        case.append(&envelope("case", "c1", "Read"), 50);

        let (rows, tampered) = project(&[
            ChainSource { label: "alert", chain: alert },
            ChainSource { label: "case", chain: case },
        ]);
        assert_eq!(tampered.len(), 1);
        assert_eq!(tampered[0].label, "alert");
        assert_eq!(rows.len(), 1, "the tampered chain's entries must not appear in the projection");
        assert_eq!(rows[0].trace_id, "c1");
    }

    #[test]
    fn an_empty_or_missing_chain_projects_as_zero_rows_not_an_error() {
        let empty = temp_chain("empty", "never_written");
        let (rows, tampered) = project(&[ChainSource { label: "empty", chain: empty }]);
        assert!(tampered.is_empty());
        assert!(rows.is_empty());
    }
}
