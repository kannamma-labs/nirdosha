//! Parser for the portable doc encoding: `nirdosha:contract {json}`.
//!
//! This is the zero-dependency authoring form — a hand-written doc
//! comment on a plain Rust function. Stock rustc treats it as
//! documentation; the Nirdosha compiler treats it as a checked
//! declaration. Malformed JSON or unknown keys are build errors under
//! `cargo nirdosha`.

use crate::model::Contract;

pub const DOC_PREFIX: &str = "nirdosha:contract";

/// Parse a doc string.
/// - `Ok(None)` — this is not a Nirdosha contract doc.
/// - `Ok(Some(c))` — a well-formed contract.
/// - `Err(msg)` — starts like a contract doc but is malformed; this is a
///   verification failure, not a silent skip.
pub fn parse_doc(doc: &str) -> Result<Option<Contract>, String> {
    let trimmed = doc.trim_start();
    let Some(rest) = trimmed.strip_prefix(DOC_PREFIX) else {
        return Ok(None);
    };
    let rest = rest.trim();
    if rest.is_empty() {
        return Err(format!(
            "`{DOC_PREFIX}` found with no JSON payload — expected `{DOC_PREFIX} {{\"effects\":[\"pure\"], ...}}`"
        ));
    }
    serde_json::from_str::<Contract>(rest)
        .map(Some)
        .map_err(|e| format!("malformed `{DOC_PREFIX}` JSON: {e}"))
}