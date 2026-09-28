//! Coverage/bypass verification: a real, automated check (not a manual
//! review claim) that this crate's own code never calls a store's
//! mutating trait method except from inside the gateway that's supposed
//! to guard it. Modeled on `nirdosha_guard_verify`'s own `VerifyPass`/
//! `Finding` shape (`crates/nirdosha-guard-verify/src/lib.rs`) for
//! consistency with this workspace's other verification tooling, though
//! this one is a source-text scan, not that crate's registry-based
//! passes.
//!
//! **Exact scope, stated plainly**: this proves *this crate's own source*
//! never bypasses a gateway. It cannot and does not prove that some other
//! crate importing `AlertStore`/`CaseStore`/`SarStore` directly won't call
//! `create_or_get`/`assign_for_alert`/`draft`/etc. itself -- Rust's trait
//! visibility is `pub` by necessity (an app needs to choose which store
//! backend to construct), so nothing here can make that impossible at the
//! type level without breaking every test in this crate that
//! deliberately calls a store method directly to test the store's own
//! invariants in isolation from a gateway. That would be a real, larger
//! redesign (a private capability-token parameter only a gateway could
//! construct) -- not done here; disclosed rather than silently
//! unaddressed.

use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BypassFinding {
    pub file: String,
    pub line: usize,
    pub method: String,
}

/// The mutating trait methods that must only ever be called from inside
/// their own gateway's `impl` block (`crate::alert`/`crate::case`/
/// `crate::sar`) or `crate::durable_store` (which implements the trait
/// itself, so it legitimately calls e.g. `Self::save` / its own trait
/// methods aren't a bypass -- it *is* the store).
// Matched against the exact receiver spelling every gateway impl and test
// in this crate uses for a store parameter/binding (`store`, never e.g.
// `self` or a differently-named local) -- `store.escalate(` is a direct
// trait-method call; `gateway.escalate(` (a same-named *gateway* wrapper
// method, e.g. `crate::service::escalate_case` calling
// `CtmsCaseEscalateGatewayV1::escalate`) is not, and must not be flagged.
// This precision is why the check is receiver-prefixed rather than a bare
// method-name substring match.
const GUARDED_METHODS: &[&str] = &[
    "store.create_or_get_with_origin(",
    "store.assign_for_alert(",
    "store.escalate(",
    "store.senior_review(",
    "store.disposition(",
    "store.draft(",
    "store.compliance_review(",
    "store.mlro_approve(",
    "store.mark_submitted(",
];

/// Files allowed to call a [`GUARDED_METHODS`] method directly: the
/// gateway modules that exist to guard them, the durable-store module
/// that implements the traits, and this module itself (whose own source
/// literally quotes these method names as data, not calls). Any file's
/// own `#[cfg(test)]` module is separately exempted below (checking a
/// store's invariants in isolation from a gateway is exactly what those
/// tests are for -- see this module's own doc for why that isn't itself
/// a bypass).
const ALLOWED_FILES: &[&str] = &["alert.rs", "case.rs", "sar.rs", "durable_store.rs", "verify.rs"];

/// Scans every `.rs` file directly under `src_dir` (non-recursive -- this
/// crate's own layout is flat) for a guarded method call outside an
/// allowed file or a `#[cfg(test)]` block.
pub fn verify_no_gateway_bypass(src_dir: &Path) -> Vec<BypassFinding> {
    let mut findings = Vec::new();
    let Ok(entries) = std::fs::read_dir(src_dir) else {
        return findings;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
        if ALLOWED_FILES.contains(&file_name.as_str()) {
            continue;
        }
        let Ok(source) = std::fs::read_to_string(&path) else {
            continue;
        };
        let mut in_test_module = false;
        let mut test_module_brace_depth: i32 = -1;
        let mut brace_depth: i32 = 0;
        for (line_index, line) in source.lines().enumerate() {
            let opens = line.matches('{').count() as i32;
            let closes = line.matches('}').count() as i32;
            if line.contains("#[cfg(test)]") {
                in_test_module = true;
            } else if in_test_module && test_module_brace_depth < 0 && line.contains("mod ") && line.contains('{') {
                test_module_brace_depth = brace_depth;
            }
            brace_depth += opens - closes;
            if in_test_module && test_module_brace_depth >= 0 && brace_depth <= test_module_brace_depth {
                in_test_module = false;
                test_module_brace_depth = -1;
            }
            if in_test_module {
                continue;
            }
            for method in GUARDED_METHODS {
                if line.contains(method) {
                    findings.push(BypassFinding { file: file_name.clone(), line: line_index + 1, method: method.to_string() });
                }
            }
        }
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_crates_own_source_has_no_gateway_bypass() {
        let src_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let findings = verify_no_gateway_bypass(&src_dir);
        assert!(findings.is_empty(), "found gateway-bypassing call(s) outside an allowed file: {findings:?}");
    }

    #[test]
    fn detects_a_synthetic_bypass_in_a_temp_directory() {
        let dir = std::env::temp_dir().join(format!("ctms_verify_bypass_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("evil_service.rs"), "fn sneaky(store: &dyn crate::case::CaseStore) {\n    store.escalate(\"case-1\", \"mallory\", \"no capability needed\").unwrap();\n}\n").unwrap();
        let findings = verify_no_gateway_bypass(&dir);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].file, "evil_service.rs");
        assert_eq!(findings[0].method, "store.escalate(");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_direct_store_call_inside_a_cfg_test_module_is_not_flagged() {
        let dir = std::env::temp_dir().join(format!("ctms_verify_test_module_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("some_module.rs"),
            "pub fn real_code() {}\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn checks_store_invariant_directly() {\n        let store = crate::case::InMemoryCaseStore::new();\n        store.escalate(\"case-1\", \"priya\", \"reason\").unwrap_err();\n    }\n}\n",
        )
        .unwrap();
        let findings = verify_no_gateway_bypass(&dir);
        assert!(findings.is_empty(), "a direct call inside #[cfg(test)] must not be flagged: {findings:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
