//! Clause lowering — the "Gate 2" step `PolicyRegistration`'s own doc
//! comment promises ("Gate 2 expands this into the owned `PolicyRecord`
//! model") but nothing implemented until now.
//!
//! `PolicyRegistration` is a `const`-constructible struct (it has to be, to
//! live in a `linkme::distributed_slice` static) — it can only hold
//! `&'static str`/copy primitives, so the macro captures everything after
//! `action`/`resource`/`purpose` as one flattened, space-joined token
//! string (`clauses_json`, not actually JSON — see `lib.rs`). This module
//! re-tokenizes that string with `proc_macro2`/`syn` (the same tooling
//! `nirdosha-guard-macros` uses, just running at registry-consumption time
//! instead of macro-expansion time, where normal heap-allocating Rust code
//! is available) and parses it into the real `nirdosha_guard_core` types.
//!
//! The grammar covered here is exactly what
//! `examples/rtm/roles-N-guard_policy.md`'s 66 real `guard_policy!` blocks
//! use — cataloged by grepping the corpus, not designed in the abstract.
//! Anything encountered outside that grammar (a compound `requires`
//! expression like `field(status).transition_allowed()`, or
//! `field(score) >= model(rt_fraud_v1).threshold_alert`, which references a
//! dynamic model lookup no parse-time lowering can resolve) is preserved
//! as `Condition::Custom(InvariantId(<original text>))` rather than
//! silently dropped or guessed at — a later phase with the real
//! `EvaluationContext`/model registry available is the right place to
//! interpret it, not this one.

use nirdosha_guard_core::{
    AuditLevel, Cap, Classification, CompareOp, Condition, Destination, EscalateTarget,
    FieldMask, FieldPolicy, FilterExpr, InvariantId, MaskTransform, Obligation, Value,
};
use syn::parse::discouraged::Speculative;
use syn::parse::{ParseStream, Parser};
use syn::{Ident, LitInt, LitStr, Token};

/// Everything `lower()` can extract from a policy's clause text. Fields
/// mirror `PolicyRecord`'s clause-derived ones 1:1 (see `lib.rs`).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct LoweredClauses {
    pub caps: Vec<Cap>,
    pub affected_row_cap: Option<u64>,
    pub obligations: Vec<Obligation>,
    pub escalation: Option<EscalateTarget>,
    pub field_policy: Vec<FieldPolicy>,
    pub conditions: Vec<Condition>,
    pub filter: Option<FilterExpr>,
    pub filter_ref: Option<String>,
    pub masks: Vec<FieldMask>,
    pub reason: Option<String>,
    pub destination: Option<Destination>,
    pub destination_denied_above: Option<Classification>,
    pub grants: Vec<String>,
    /// Field names from `grant predicate_use(a, b, c)`, parsed out
    /// structurally rather than left as raw text in `grants` — I15
    /// ("masked fields excluded from filter/join/grouping/having/
    /// ordering/window unless granted") needs a real field list to check
    /// a `FilterExpr`'s fields against, not a string to re-parse at every
    /// call site.
    pub predicate_use: Vec<String>,
    /// `grant count_allowed` — same reasoning, parsed structurally since
    /// a caller needs a plain bool, not a string to compare.
    pub count_allowed: bool,
}

/// The closed set of clause-starting keywords this grammar recognizes.
/// Used both to dispatch and, inside `requires`/`filter`/`grant`'s
/// raw-tail capture, to know where an un-delimited clause body ends (these
/// three have no enclosing braces in the source — they just run until the
/// next clause keyword).
const CLAUSE_KEYWORDS: &[&str] = &[
    "mask", "cap", "requires", "ensures", "field_policy", "escalate", "obligate", "filter",
    "grant", "destination", "reason",
];

/// Parses `clauses_json` (`None` or malformed input just yields
/// `LoweredClauses::default()` — a policy with no clauses, or one whose
/// captured text somehow isn't valid Rust tokens, is not a reason to
/// panic the whole registry expansion).
pub fn lower(clauses_json: Option<&str>) -> LoweredClauses {
    let mut out = LoweredClauses::default();
    let Some(text) = clauses_json else { return out };
    let Ok(tokens) = text.parse::<proc_macro2::TokenStream>() else {
        return out;
    };
    let parser = |input: ParseStream| -> syn::Result<()> {
        lower_stream(input, &mut out)
    };
    // A parse failure partway through still leaves `out` with whatever was
    // lowered before the failure — better than discarding a policy's
    // already-recognized clauses over one unrecognized trailing one.
    let _ = parser.parse2(tokens);
    out
}

fn peek_is_known_keyword(input: ParseStream) -> bool {
    if !input.peek(Ident) {
        return false;
    }
    input
        .fork()
        .parse::<Ident>()
        .map(|ident| CLAUSE_KEYWORDS.contains(&ident.to_string().as_str()))
        .unwrap_or(false)
}

/// Reconstructs a delimited group's interior as one whitespace-collapsed
/// string — used for bare-word/dotted-path arguments like
/// `destination(export-file)` (`export-file` tokenizes as THREE tokens:
/// `export`, `-`, `file` — not one `Ident`, since `-` isn't valid inside an
/// identifier) and `reason(sod.ingestion_is_write_only)` (a dotted path,
/// also multiple tokens). Parsing these as a single `Ident` would reject
/// every real instance of either clause in the corpus.
fn group_to_compact_string(content: ParseStream) -> syn::Result<String> {
    let stream: proc_macro2::TokenStream = content.parse()?;
    Ok(stream.to_string().replace(' ', ""))
}

fn lower_stream(input: ParseStream, out: &mut LoweredClauses) -> syn::Result<()> {
    while !input.is_empty() {
        let kw: Ident = input.parse()?;
        match kw.to_string().as_str() {
            "mask" => lower_mask(input, out)?,
            "cap" => lower_cap(input, out)?,
            "requires" | "ensures" => lower_requires(input, out)?,
            "field_policy" => lower_field_policy(input, out)?,
            "escalate" => lower_escalate(input, out)?,
            "obligate" => lower_obligate(input, out)?,
            "filter" => lower_filter(input, out)?,
            "grant" => lower_grant(input, out)?,
            "destination" => lower_destination(input, out)?,
            "reason" => {
                let content;
                syn::parenthesized!(content in input);
                out.reason = Some(group_to_compact_string(&content)?);
            }
            _ => {
                // An unrecognized top-level word (e.g. a comment artifact
                // that survived stringification). Skip its trailing
                // delimited group if it has one; otherwise there is
                // nothing safe to skip, so just drop the word and
                // continue rather than looping forever on it.
                if input.peek(syn::token::Paren) {
                    let content;
                    syn::parenthesized!(content in input);
                    let _: proc_macro2::TokenStream = content.parse()?;
                } else if input.peek(syn::token::Brace) {
                    let content;
                    syn::braced!(content in input);
                    let _: proc_macro2::TokenStream = content.parse()?;
                }
            }
        }
    }
    Ok(())
}

/// `mask(field, transform)` — the only shape used both in the smoke test
/// (`mask(card_number, partial_last4)`, `mask(cvv, full)`) and the corpus
/// (`mask(subject_id, tokenized)`).
fn lower_mask(input: ParseStream, out: &mut LoweredClauses) -> syn::Result<()> {
    let content;
    syn::parenthesized!(content in input);
    let field: Ident = content.parse()?;
    let _: Token![,] = content.parse()?;
    let transform: Ident = content.parse()?;
    let transform = match transform.to_string().as_str() {
        "full" => MaskTransform::Full,
        "partial_last4" => MaskTransform::PartialLast4,
        "hash" => MaskTransform::Hash,
        "drop" => MaskTransform::Drop,
        "tokenized" => MaskTransform::Custom { name: "tokenized".into() },
        other => MaskTransform::Custom { name: other.into() },
    };
    out.masks.push(FieldMask { field: vec![field.to_string()], transform });
    Ok(())
}

/// `cap(key = value, key = value, ...)`. Values are Rust integer literals,
/// sometimes with a unit suffix the corpus uses informally (`60s`, `5GB`) —
/// these aren't real Rust literal suffixes, but `syn::LitInt` tokenizes and
/// exposes them via `.suffix()` regardless, since nothing here ever asks
/// rustc to type-check them as an actual typed literal.
fn lower_cap(input: ParseStream, out: &mut LoweredClauses) -> syn::Result<()> {
    let content;
    syn::parenthesized!(content in input);
    while !content.is_empty() {
        let key: Ident = content.parse()?;
        let _: Token![=] = content.parse()?;
        let lit: LitInt = content.parse()?;
        let raw: u64 = lit.base10_parse().unwrap_or(0);
        let suffix = lit.suffix();
        let value = match suffix {
            "s" => raw.saturating_mul(1000), // seconds -> ms, for max_execution
            "GB" => raw.saturating_mul(1_000_000_000), // decimal GB -> bytes
            _ => raw,
        };
        match key.to_string().as_str() {
            "row_cap" => out.caps.push(Cap::RowCap(value)),
            "max_scan_rows" => out.caps.push(Cap::MaxScanRows(value)),
            "max_scan_bytes" => out.caps.push(Cap::MaxScanBytes(value)),
            "max_execution" | "max_execution_time" | "max_execution_time_ms" => {
                out.caps.push(Cap::MaxExecutionTimeMs(value))
            }
            "max_result_bytes" => out.caps.push(Cap::MaxResultBytes(value)),
            "cohort_floor" => out.caps.push(Cap::CohortFloor(value)),
            "max_depth" => out.caps.push(Cap::MaxDepth(value)),
            "max_nodes" => out.caps.push(Cap::MaxNodes(value)),
            // `affected_rows` is WritePlan::affected_row_cap, a different
            // concept from the read-side `Cap` enum (rows touched by a
            // mutation, not rows scanned by a read) — kept as its own
            // field rather than forced into `Cap`.
            "affected_rows" => out.affected_row_cap = Some(value),
            _ => {} // unrecognized cap key: drop rather than guess
        }
        if content.peek(Token![,]) {
            let _: Token![,] = content.parse()?;
        }
    }
    Ok(())
}

/// `field_policy { required(a, b) allowed(c) forbidden(d, e) }`.
fn lower_field_policy(input: ParseStream, out: &mut LoweredClauses) -> syn::Result<()> {
    let content;
    syn::braced!(content in input);
    while !content.is_empty() {
        let kind: Ident = content.parse()?;
        let paren;
        syn::parenthesized!(paren in content);
        while !paren.is_empty() {
            let field: Ident = paren.parse()?;
            let path = vec![field.to_string()];
            out.field_policy.push(match kind.to_string().as_str() {
                "required" => FieldPolicy::Required(path),
                "allowed" => FieldPolicy::Allowed(path),
                "forbidden" => FieldPolicy::Forbidden(path),
                _ => FieldPolicy::Allowed(path),
            });
            if paren.peek(Token![,]) {
                let _: Token![,] = paren.parse()?;
            }
        }
    }
    Ok(())
}

/// `escalate to approval(chain name)` / `escalate to step_up(method)`.
fn lower_escalate(input: ParseStream, out: &mut LoweredClauses) -> syn::Result<()> {
    let to: Ident = input.parse()?;
    if to != "to" {
        return Err(syn::Error::new(to.span(), "expected `to` after `escalate`"));
    }
    let kind: Ident = input.parse()?;
    let content;
    syn::parenthesized!(content in input);
    out.escalation = match kind.to_string().as_str() {
        "approval" => {
            let chain_kw: Ident = content.parse()?;
            if chain_kw != "chain" {
                return Err(syn::Error::new(chain_kw.span(), "expected `chain` inside approval(...)"));
            }
            let name: Ident = content.parse()?;
            Some(EscalateTarget::Approval { chain: name.to_string() })
        }
        "step_up" => {
            let method: Ident = content.parse()?;
            Some(EscalateTarget::StepUp { method: method.to_string() })
        }
        "materialize" | "materialization" => {
            let job: Ident = content.parse()?;
            Some(EscalateTarget::Materialization { job: job.to_string() })
        }
        _ => None,
    };
    Ok(())
}

/// `obligate audit(full | sampled)` / `obligate notify(channel("..."))`.
fn lower_obligate(input: ParseStream, out: &mut LoweredClauses) -> syn::Result<()> {
    let kind: Ident = input.parse()?;
    let content;
    syn::parenthesized!(content in input);
    match kind.to_string().as_str() {
        "audit" => {
            let level: Ident = content.parse()?;
            let level = if level == "full" {
                AuditLevel::Full
            } else {
                // "sampled" defers to the classification-driven table
                // (`audit_sampling!`) for the real rate; 1000 (100%) here
                // would be dishonest, 0 would be too — record the *kind*
                // (sampled, not full) and let the sampling table (a
                // separate registration) supply the rate. Using the
                // lowest permille as a placeholder would misrepresent
                // "sampled" as "never audited"; mid-scale is the least
                // wrong placeholder pending that join.
                AuditLevel::Sample { rate_permille: 500 }
            };
            out.obligations.push(Obligation::Audit { level });
        }
        "notify" => {
            let channel_kw: Ident = content.parse()?;
            let inner;
            syn::parenthesized!(inner in content);
            if channel_kw == "channel" || channel_kw == "topic" {
                if let Ok(lit) = inner.parse::<LitStr>() {
                    out.obligations.push(Obligation::Notify { channel: lit.value() });
                } else {
                    let raw = group_to_compact_string(&inner)?;
                    out.obligations.push(Obligation::Notify { channel: raw });
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// `filter tenant_scope()` / `subject_scope()` / `delegation_scope()` /
/// `time_range(field, within, retention_window())`. These name a
/// *request-context-dependent* scope — `tenant_scope()` means "bind to
/// whatever tenant the live `EvaluationContext` carries," which has no
/// concrete `Value` at lowering time. Recording the literal source text in
/// `filter_ref` (rather than inventing a placeholder `FilterExpr`) keeps
/// this honest: a later phase with a real `EvaluationContext` in hand is
/// where `tenant_scope()` actually becomes `FilterExpr::TenantEq { value:
/// <the real tenant> }`.
fn lower_filter(input: ParseStream, out: &mut LoweredClauses) -> syn::Result<()> {
    let raw = capture_clause_tail(input)?;
    out.filter_ref = Some(raw);
    Ok(())
}

/// `grant predicate_use(a, b, c)` / `grant count_allowed`.
fn lower_grant(input: ParseStream, out: &mut LoweredClauses) -> syn::Result<()> {
    let raw = capture_clause_tail(input)?;
    if raw == "count_allowed" {
        out.count_allowed = true;
    } else if let Some(fields) = raw.strip_prefix("predicate_use(").and_then(|s| s.strip_suffix(')')) {
        out.predicate_use.extend(fields.split(',').map(|f| f.trim().to_string()).filter(|f| !f.is_empty()));
    }
    out.grants.push(raw);
    Ok(())
}

/// `destination(name)` optionally followed by `denied_above(CLASS)`.
fn lower_destination(input: ParseStream, out: &mut LoweredClauses) -> syn::Result<()> {
    let content;
    syn::parenthesized!(content in input);
    let raw = group_to_compact_string(&content)?;
    out.destination = match raw.as_str() {
        "browser" => Some(Destination::Browser),
        "api_client" | "api-client" => Some(Destination::ApiClient),
        "llm_context" => Some(Destination::LlmContext),
        "export-file" | "export_file" => Some(Destination::ExportFile),
        "webhook" => Some(Destination::Webhook),
        "feature_pipeline" => Some(Destination::FeaturePipeline),
        "warehouse" => Some(Destination::Warehouse),
        _ => None,
    };
    if input.peek(Ident) {
        let forked = input.fork();
        if forked.parse::<Ident>().map(|i| i == "denied_above").unwrap_or(false) {
            let _: Ident = input.parse()?;
            let content;
            syn::parenthesized!(content in input);
            let class: Ident = content.parse()?;
            out.destination_denied_above = parse_classification(&class.to_string());
        }
    }
    Ok(())
}

fn parse_classification(name: &str) -> Option<Classification> {
    match name.to_ascii_uppercase().as_str() {
        "PUBLIC" => Some(Classification::Public),
        "INTERNAL" => Some(Classification::Internal),
        "CONFIDENTIAL" => Some(Classification::Confidential),
        "RESTRICTED" => Some(Classification::Restricted),
        _ => None,
    }
}

/// `requires`/`ensures`: one or more `&&`-joined conjuncts. Each conjunct
/// is lowered structurally when it matches a known shape
/// (`field(x) == "lit"`, `field(x) in [...]`, `field(x) </>/<=/>= N`,
/// `invariant(name)`) and preserved as `Condition::Custom(InvariantId(raw))`
/// otherwise (`field(status).transition_allowed()`,
/// `field(score) >= model(rt_fraud_v1).threshold_alert`,
/// `field(status) == "released" && expired(hold_expires_at)`'s second
/// conjunct — none of these are resolvable without a live
/// `EvaluationContext`/model registry).
fn lower_requires(input: ParseStream, out: &mut LoweredClauses) -> syn::Result<()> {
    loop {
        let condition = capture_one_conjunct(input)?;
        out.conditions.push(condition);
        if input.peek(Token![&&]) {
            let _: Token![&&] = input.parse()?;
            continue;
        }
        break;
    }
    Ok(())
}

fn capture_one_conjunct(input: ParseStream) -> syn::Result<Condition> {
    if input.peek(Ident) {
        let head_fork = input.fork();
        if let Ok(head) = head_fork.parse::<Ident>() {
            if head == "invariant" && head_fork.peek(syn::token::Paren) {
                let _: Ident = input.parse()?;
                let content;
                syn::parenthesized!(content in input);
                let name: Ident = content.parse()?;
                return Ok(Condition::Custom(InvariantId(name.to_string())));
            }
            if head == "field" && head_fork.peek(syn::token::Paren) {
                if let Some(condition) = try_field_condition(input)? {
                    return Ok(condition);
                }
            }
        }
    }
    // Fallback: raw-capture until `&&` or the next known clause keyword.
    let mut parts = Vec::new();
    while !input.is_empty() && !input.peek(Token![&&]) && !peek_is_known_keyword(input) {
        let tt: proc_macro2::TokenTree = input.parse()?;
        parts.push(tt.to_string());
    }
    Ok(Condition::Custom(InvariantId(parts.join(" "))))
}

/// Tries the structured `field(name) <op> <literal>` shapes on a fork,
/// committing to `input` only on a full match (op + a literal RHS, with
/// nothing unexpected trailing inside the parens). Returns `Ok(None)` for
/// any shape it doesn't recognize (`.transition_allowed()`, a non-literal
/// RHS like `model(...).threshold_alert`) — the caller falls back to raw
/// capture in that case, so a partially-matched speculative parse never
/// loses tokens.
fn try_field_condition(input: ParseStream) -> syn::Result<Option<Condition>> {
    let checkpoint = input.fork();
    let _: Ident = checkpoint.parse()?; // "field", already confirmed by the caller's peek
    let content;
    syn::parenthesized!(content in checkpoint);
    let field: Ident = content.parse()?;
    if !content.is_empty() {
        return Ok(None);
    }
    let field_path = vec![field.to_string()];

    if checkpoint.peek(Token![==]) {
        let _: Token![==] = checkpoint.parse()?;
        if let Ok(lit) = checkpoint.parse::<LitStr>() {
            input.advance_to(&checkpoint);
            return Ok(Some(Condition::Expr(FilterExpr::Eq {
                field: field_path,
                value: Value::Str(lit.value()),
            })));
        }
        return Ok(None);
    }
    if checkpoint.peek(Token![in]) {
        let _: Token![in] = checkpoint.parse()?;
        let bracket;
        syn::bracketed!(bracket in checkpoint);
        let mut values = Vec::new();
        while !bracket.is_empty() {
            let lit: LitStr = bracket.parse()?;
            values.push(Value::Str(lit.value()));
            if bracket.peek(Token![,]) {
                let _: Token![,] = bracket.parse()?;
            }
        }
        input.advance_to(&checkpoint);
        return Ok(Some(Condition::Expr(FilterExpr::In { field: field_path, values })));
    }
    let op = if checkpoint.peek(Token![>=]) {
        let _: Token![>=] = checkpoint.parse()?;
        Some(CompareOp::Ge)
    } else if checkpoint.peek(Token![<=]) {
        let _: Token![<=] = checkpoint.parse()?;
        Some(CompareOp::Le)
    } else if checkpoint.peek(Token![>]) {
        let _: Token![>] = checkpoint.parse()?;
        Some(CompareOp::Gt)
    } else if checkpoint.peek(Token![<]) {
        let _: Token![<] = checkpoint.parse()?;
        Some(CompareOp::Lt)
    } else {
        None
    };
    if let Some(op) = op {
        if let Ok(lit) = checkpoint.parse::<LitInt>() {
            let value: i64 = lit.base10_parse().unwrap_or(0);
            input.advance_to(&checkpoint);
            return Ok(Some(Condition::Expr(FilterExpr::Compare {
                field: field_path,
                op,
                value: Value::Int(value),
            })));
        }
        return Ok(None);
    }
    Ok(None)
}

/// Raw-capture helper for the two clause kinds with no enclosing delimiter
/// at all in the source (`filter`, `grant`) — they just run until the next
/// recognized clause keyword or end of input.
fn capture_clause_tail(input: ParseStream) -> syn::Result<String> {
    let mut parts = Vec::new();
    while !input.is_empty() && !peek_is_known_keyword(input) {
        let tt: proc_macro2::TokenTree = input.parse()?;
        parts.push(tt.to_string());
    }
    Ok(parts.join(" ").replace(" . ", ".").replace(" (", "(").replace(" )", ")").replace("( ", "(").replace(" ,", ","))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lower_str(s: &str) -> LoweredClauses {
        lower(Some(s))
    }

    #[test]
    fn none_input_yields_default() {
        assert_eq!(lower(None), LoweredClauses::default());
    }

    #[test]
    fn lowers_mask() {
        let out = lower_str(r#"mask(subject_id, tokenized)"#);
        assert_eq!(out.masks.len(), 1);
        assert_eq!(out.masks[0].field, vec!["subject_id".to_string()]);
        assert_eq!(out.masks[0].transform, MaskTransform::Custom { name: "tokenized".into() });
    }

    #[test]
    fn lowers_cap_with_unit_suffixes() {
        let out = lower_str(r#"cap(cohort_floor = 10, max_scan_bytes = 5GB, max_execution = 60s)"#);
        assert!(out.caps.contains(&Cap::CohortFloor(10)));
        assert!(out.caps.contains(&Cap::MaxScanBytes(5_000_000_000)));
        assert!(out.caps.contains(&Cap::MaxExecutionTimeMs(60_000)));
    }

    #[test]
    fn lowers_affected_rows_cap_separately_from_cap_enum() {
        let out = lower_str(r#"cap(affected_rows = 100)"#);
        assert_eq!(out.affected_row_cap, Some(100));
        assert!(out.caps.is_empty());
    }

    #[test]
    fn lowers_field_policy_block() {
        let out = lower_str(
            r#"field_policy { allowed(status, sar_id) forbidden(alert_ids, rationale, assigned_to) }"#,
        );
        assert!(out.field_policy.contains(&FieldPolicy::Allowed(vec!["status".into()])));
        assert!(out.field_policy.contains(&FieldPolicy::Allowed(vec!["sar_id".into()])));
        assert!(out.field_policy.contains(&FieldPolicy::Forbidden(vec!["alert_ids".into()])));
        assert!(out.field_policy.contains(&FieldPolicy::Forbidden(vec!["rationale".into()])));
        assert!(out.field_policy.contains(&FieldPolicy::Forbidden(vec!["assigned_to".into()])));
    }

    #[test]
    fn lowers_escalate_to_approval_chain() {
        let out = lower_str(r#"escalate to approval(chain sar_release)"#);
        assert_eq!(out.escalation, Some(EscalateTarget::Approval { chain: "sar_release".into() }));
    }

    #[test]
    fn lowers_obligate_audit_full() {
        let out = lower_str(r#"obligate audit(full)"#);
        assert_eq!(out.obligations, vec![Obligation::Audit { level: AuditLevel::Full }]);
    }

    #[test]
    fn lowers_obligate_notify_channel() {
        let out = lower_str(r#"obligate notify(channel("regulatory-log"))"#);
        assert_eq!(out.obligations, vec![Obligation::Notify { channel: "regulatory-log".into() }]);
    }

    #[test]
    fn lowers_reason_dotted_path() {
        let out = lower_str(r#"reason(sod.ingestion_is_write_only)"#);
        assert_eq!(out.reason.as_deref(), Some("sod.ingestion_is_write_only"));
    }

    #[test]
    fn lowers_destination_with_hyphen_and_denied_above() {
        let out = lower_str(r#"destination(llm_context) denied_above(CONFIDENTIAL)"#);
        assert_eq!(out.destination, Some(Destination::LlmContext));
        assert_eq!(out.destination_denied_above, Some(Classification::Confidential));

        let out2 = lower_str(r#"destination(export-file)"#);
        assert_eq!(out2.destination, Some(Destination::ExportFile));
    }

    #[test]
    fn lowers_filter_scope_function_as_a_reference_not_a_fake_value() {
        let out = lower_str(r#"filter tenant_scope()"#);
        assert_eq!(out.filter, None);
        assert_eq!(out.filter_ref.as_deref(), Some("tenant_scope()"));
    }

    #[test]
    fn lowers_grant_predicate_use() {
        let out = lower_str(r#"grant predicate_use(amount, currency, channel)"#);
        assert_eq!(out.grants, vec!["predicate_use(amount, currency, channel)".to_string()]);
    }

    #[test]
    fn lowers_requires_field_equals_literal() {
        let out = lower_str(r#"requires field(status) == "confirmed_fraud""#);
        assert_eq!(
            out.conditions,
            vec![Condition::Expr(FilterExpr::Eq {
                field: vec!["status".into()],
                value: Value::Str("confirmed_fraud".into()),
            })]
        );
    }

    #[test]
    fn lowers_requires_field_in_list() {
        let out = lower_str(r#"requires field(status) in ["confirmed_fraud", "false_positive"]"#);
        assert_eq!(
            out.conditions,
            vec![Condition::Expr(FilterExpr::In {
                field: vec!["status".into()],
                values: vec![Value::Str("confirmed_fraud".into()), Value::Str("false_positive".into())],
            })]
        );
    }

    #[test]
    fn lowers_requires_field_compare() {
        let out = lower_str(r#"requires field(amount) > 100_000"#);
        assert_eq!(
            out.conditions,
            vec![Condition::Expr(FilterExpr::Compare {
                field: vec!["amount".into()],
                op: CompareOp::Gt,
                value: Value::Int(100_000),
            })]
        );
    }

    #[test]
    fn lowers_requires_invariant_conjunction() {
        let out = lower_str(r#"requires invariant(amount_positive) && invariant(currency_iso)"#);
        assert_eq!(
            out.conditions,
            vec![
                Condition::Custom(InvariantId("amount_positive".into())),
                Condition::Custom(InvariantId("currency_iso".into())),
            ]
        );
    }

    #[test]
    fn falls_back_to_raw_text_for_unrecognizable_requires_shapes() {
        let out = lower_str(r#"requires field(status).transition_allowed()"#);
        assert_eq!(out.conditions.len(), 1);
        assert!(matches!(&out.conditions[0], Condition::Custom(InvariantId(raw)) if raw.contains("transition_allowed")));

        let out2 = lower_str(r#"requires field(score) >= model(rt_fraud_v1).threshold_alert"#);
        assert_eq!(out2.conditions.len(), 1);
        assert!(matches!(&out2.conditions[0], Condition::Custom(_)));
    }

    #[test]
    fn requires_then_next_clause_does_not_swallow_it() {
        let out = lower_str(r#"requires invariant(rationale_present) cap(affected_rows = 1)"#);
        assert_eq!(out.conditions, vec![Condition::Custom(InvariantId("rationale_present".into()))]);
        assert_eq!(out.affected_row_cap, Some(1));
    }

    #[test]
    fn full_ingest_create_txn_policy_lowers_every_clause() {
        // The corpus's own "ingest-create-txn" tail, verbatim.
        let out = lower_str(
            r#"purpose(FraudMonitoring)
               field_policy { required(tenant_id, subject_id, amount, currency, status, occurred_at)
                              allowed(channel, merchant_id, card_token, device_id, geo) }
               cap(affected_rows = 1)
               obligate audit(full)"#,
        );
        assert!(out.field_policy.contains(&FieldPolicy::Required(vec!["tenant_id".into()])));
        assert!(out.field_policy.contains(&FieldPolicy::Allowed(vec!["device_id".into()])));
        assert_eq!(out.affected_row_cap, Some(1));
        assert_eq!(out.obligations, vec![Obligation::Audit { level: AuditLevel::Full }]);
    }
}
