// `#[classify]`'s and `lineage_query!`'s generated static/type names don't
// screaming-case the source item's own identifier (a pre-existing, purely
// cosmetic naming-convention gap in those macros, out of scope here).
//
// Some enum variants below (`Purpose`'s taxonomy) are declared for real but
// never constructed by this file itself — they're consumed as bare wire
// strings by `guard_policy!`'s `purpose(Ident)` clauses, not as `Purpose::`
// values.
#![allow(non_upper_case_globals, non_camel_case_types, dead_code)]

//! Corpus regression test: every `rust` code block from
//! `examples/rtm/roles-N-guard_policy.md`, concatenated in original order,
//! verbatim except for the `// SKIPPED (...)` markers below — each one
//! replaces content that fails for a reason *other* than the
//! `guard_policy!`/catalog-macro parser bugs this phase fixes. This is the
//! real acceptance bar for that fix: not a hand-picked example, the actual
//! 76 `guard_policy!` blocks (125 `PolicyRegistration`s after `action
//! in [...]`/`resource in [...]` fan-out) the doc ships today.
//!
//! Regenerate the non-skipped content with:
//! ```text
//! python3 -c "
//! import re
//! text = open('examples/rtm/roles-N-guard_policy.md').read()
//! blocks = re.findall(r'\`\`\`rust\n(.*?)\n\`\`\`', text, re.S)
//! print('\n\n'.join(blocks))
//! "
//! ```
//!
//! Graduated (previously skipped, now real): `purpose!`, `stream_port!`,
//! `window!`, `model_artifact!`, `matcher!`, `mcp_tools!` now exist as real
//! `#[proc_macro]`s in `nirdosha-guard-macros`, registering structured
//! records into `PURPOSES`/`PORTS`/`WINDOWS`/`MODELS`/`MATCHERS`/
//! `MCP_SERVERS` — see that crate's doc comments on each for the exact
//! grammar and what's structurally parsed vs. kept as opaque source text.
//! `purpose!` the function-like taxonomy macro is named `purpose_taxonomy!`
//! and called fully-qualified (`nirdosha_guard_macros::purpose_taxonomy!`)
//! for the same name-collision reason `workflow!` is qualified below — the
//! pre-existing `#[proc_macro_attribute] purpose` (a different,
//! load-bearing macro, see `guard_attributes.rs`) already owns the bare name.
//!
//! What's still skipped, and why (found by attempting exactly this compile):
//! - `#[reference(...)]` used as a bare six-line declarative list with no
//!   item attached, and with positional-path arguments
//!   (`customer.kyc_status`) rather than the real macro's `field = ...`
//!   keyword-argument grammar — a genuine grammar mismatch, not a bug in
//!   the existing attribute macro's own (different) contract.
//! - `#[relation(...)]`/`#[materialize(...)]` attached to semicolon-only,
//!   body-less `fn foo(...) -> T;` signatures — not a legal free-standing
//!   Rust item (no trait/extern block wraps them here), so
//!   `syn::parse::<syn::Item>` never produces a named item to attach to.
//! - The five `#[invariant]` functions in `00_core.nir`: each has a real
//!   content bug independent of macro parsing — `Money` has no `PartialOrd<i32>`,
//!   `CurrencyCode::is_iso_4217()` is never defined, `AlertUpdate` is never
//!   declared anywhere in the corpus, `SarBundle`/`Case` have no
//!   `.case()`/`.subjects()`/`.subject()` methods (only plain ID fields —
//!   the invariant wants a cross-entity join, not a local field read), and
//!   `Customer.legal_hold` is a field, called here as `.legal_hold()`.
//! - Every `#[dataset(...)]`-attributed struct body (`Transaction`, `Alert`,
//!   `Case`, `Customer`, `SarBundle`, `ScreeningHit`, `Payment`, `QaReview`,
//!   `Notification`, `UserProfile`, `SupportTicket`, `RefDataset`) —
//!   `10_domains.nir` references ~30 primitive ID/enum types
//!   (`TenantId`, `AccountId`, `CurrencyCode`, `Timestamp`, `CaseId`, ...)
//!   that are never declared anywhere in this doc set. That's expected
//!   content for a starter file set (a real implementer supplies them),
//!   not a macro-parser question, so it's out of scope here. The simple
//!   self-contained `#[classify]` tuple structs (`CardToken`, `Money`, ...)
//!   need no such types and are kept verbatim.
//! - `nirdosha_rt::workflow!` invocations are rewritten to the fully
//!   qualified `nirdosha_guard_macros::workflow!` — `nirdosha_rt`
//!   re-exports `nirdosha_macros::workflow` (an unrelated UI-screen-flow
//!   macro, real and load-bearing — see `crates/nirdosha-rt/tests/workflow.rs`)
//!   under the same name, so `nirdosha_rt::workflow!` cannot currently reach
//!   the RFC 0023 state-machine macro this file's `machine Name { ... }`
//!   syntax needs. Fixing the name collision is a separate decision (rename
//!   one of the two) that would change a public macro name other code
//!   already depends on, so it isn't made silently here.
//!
//! Everything else — all 66 `guard_policy!` blocks (`for <Role>`,
//! `action in [...]`, `resource in [...]`, `purpose(Ident)`, `field_policy`,
//! `requires`/`ensures`, `escalate to approval(...)`, `obligate ...`),
//! `roles!`, `approval_chain!` (7x in one file), `audit_sampling!`,
//! `audit_rules!`, `enumerate!`, `break_glass!`, `lineage_query!`,
//! `policy_simulation!`, `data_contract!`, and the now-graduated `purpose!`/
//! `stream_port!`/`window!`/`model_artifact!`/`matcher!`/`mcp_tools!` — is
//! verbatim.

// ============================================================================
// 00_core.nir — foundations. Serves: ALL modules.
// ============================================================================

// ---- Roles & principals ---- [RFC 0025 §6.2, §7.4; role set per screen-inventory Part 1]
nirdosha_rt::roles! {
    // Human roles
    role Analyst;              // L1 triage + L2 investigation        → M3–M7
    role ComplianceLead;       // team lead / four-eyes / quorum member → M4, M12
    role Mlro;                 // final SAR authority                 → M12  [NEW — RFCs name only ComplianceLead]
    role QaReviewer;           // QA sampling & scorecards            → M14  [NEW]
    role OpsAnalyst;           // real-time fraud desk, hold release  → M11  [NEW]
    role PolicyEngineer;       // scenario/model author (RMG)         → M8, M9
    role Admin;                // catalog, users, drivers, retention  → M18, M19
    role Auditor;              // read-only assurance                 → M20, M21.1
    role CsAgent;              // masked payment-status lookup only   → M21.3 [NEW]
    role RmUser;               // masked customer-status view         → M21.4 [NEW]
    role RegulatorViewer;      // usable ONLY via scoped delegation   → M21.2 [NEW]

    // Service principals — SPIFFE-backed, one identity per module (P2)
    principal SvcIngest    = "spiffe://acme/ns/rtm/sa/ingest";
    principal SvcFeatures  = "spiffe://acme/ns/rtm/sa/features";
    principal SvcScoring   = "spiffe://acme/ns/rtm/sa/scoring";
    principal SvcScreening = "spiffe://acme/ns/rtm/sa/screening";
    principal SvcAlerts    = "spiffe://acme/ns/rtm/sa/alerts";
    principal SvcCase      = "spiffe://acme/ns/rtm/sa/case";
    principal SvcBi        = "spiffe://acme/ns/rtm/sa/bi";
    principal McpCopilot   = "mcp:analyst-copilot";   // RFC 0024 §1 — separate principal
}

// ---- Purpose taxonomy (closed core; §1B / §17) ----
nirdosha_guard_macros::purpose_taxonomy! {
    enum Purpose {
        Operations,              // default operational purpose
        FraudMonitoring,         // "fraud_monitoring" — used by svc:* [RFC 0025 §8.1]
        AmlInvestigation,        // analyst work on alerts/cases          [NEW code]
        CustomerService,         // CsAgent lookups                       [NEW code]
        QaReview,                                                       // [NEW]
        Audit,                                                          // [NEW]
        RegulatoryInspection,                                           // [NEW]
        Analytics,                                                      // [NEW]
        ModelGovernance,                                                // [NEW]
        PlatformOperations,                                             // [NEW]
    }
}

// ---- Audit sampling table — §17 canonical form ----
// Hard floors the sampler cannot override (stated here as comments; enforced
// structurally): denials ALWAYS full (I2) · exports ALWAYS full (§2 Export) ·
// delegated/agent access ALWAYS full (I17) · lineage reads NEVER sampled (RFC 0026 §9.1).
nirdosha_rt::audit_sampling! {
    classification: PUBLIC       -> rate 0.01;
    classification: INTERNAL     -> rate 0.10;
    classification: CONFIDENTIAL -> rate 1.0;   // RTM: confidential = full by default
    classification: RESTRICTED   -> rate 1.0;
}

// ---- Approval chains ---- [RFC 0025 §8.6 + NEW; all timeouts resolve to DENY]
nirdosha_rt::approval_chain! {
    chain sar_release {                              // [RFC §8.6 verbatim]
        quorum(2, of = [ComplianceLead]);
        timeout(deny);
    }
}
nirdosha_rt::approval_chain! {
    chain case_review { quorum(2, of = [ComplianceLead]); timeout(deny); }   // → 4.11 [NEW]
}
nirdosha_rt::approval_chain! {
    chain policy_release { quorum(2, of = [PolicyEngineer, ComplianceLead]); timeout(deny); } // → 8.8 [NEW]
}
nirdosha_rt::approval_chain! {
    chain model_release { quorum(2, of = [PolicyEngineer, ComplianceLead]); timeout(deny); }  // → 9.1 [NEW]
}
nirdosha_rt::approval_chain! {
    chain override_release { quorum(1, of = [ComplianceLead]); timeout(deny); } // → 11.3 [NEW]
}
nirdosha_rt::approval_chain! {
    chain purge_release { quorum(2, of = [Admin, ComplianceLead]); timeout(deny); } // → 18.7 [NEW]
}
nirdosha_rt::approval_chain! {
    chain egress_release { quorum(1, of = [ComplianceLead]); timeout(deny); } // → 15.5, 21.1 [NEW]
}

// ---- Global invariant registrations (referenced by policies below) ----
// [RFC 0023 §2 Condition::Custom registry; §17; RFC 0026 §10.1 examples]
// SKIPPED (invariant body references types/methods the corpus never declares — see report): lines 73-74

// SKIPPED (invariant body references types/methods the corpus never declares — see report): lines 76-77

// SKIPPED (invariant body references types/methods the corpus never declares — see report): lines 79-80

// SKIPPED (invariant body references types/methods the corpus never declares — see report): lines 82-83

// SKIPPED (invariant body references types/methods the corpus never declares — see report): lines 85-86

// ============================================================================
// 10_domains.nir — entity declarations. Serves: all data screens.
// Field lists: [RFC 0025 §8.1/§8.5/§8.6] where declared; [NEW] otherwise.
// ============================================================================

// ---- Type classifications (mask registry is type-class driven, §17) ----
#[nirdosha_rt::classify(RESTRICTED)]   pub struct CardToken(pub String);   // PCI
#[nirdosha_rt::classify(RESTRICTED)]   pub struct NationalId(pub String);  // PII-hard
#[nirdosha_rt::classify(CONFIDENTIAL)] pub struct SubjectId(pub String);
#[nirdosha_rt::classify(CONFIDENTIAL)] pub struct DeviceId(pub String);
#[nirdosha_rt::classify(CONFIDENTIAL)] pub struct Geo(pub String);
#[nirdosha_rt::classify(CONFIDENTIAL)] pub struct CustomerName(pub String);
#[nirdosha_rt::classify(INTERNAL)]     pub struct Money(pub f64);

// ---- transaction — [RFC §8.1: exact required/allowed set] ----
// SKIPPED (#[dataset] struct body references primitive ID/enum types this doc set never declares — a real domain-model companion module, not a macro-parser question; see report): lines 98-112

// ---- txn_events — topic-as-dataset (features read this) [RFC §8.2] ----
// SKIPPED (#[dataset] struct body references primitive ID/enum types this doc set never declares — a real domain-model companion module, not a macro-parser question; see report): lines 115-116

// ---- alert — [RFC §8.5: required set; status is analyst-only] ----
// SKIPPED (#[dataset] struct body references primitive ID/enum types this doc set never declares — a real domain-model companion module, not a macro-parser question; see report): lines 119-134

// ---- case — [RFC §8.6: status machine below; alert_ids immutable] ----
// SKIPPED (#[dataset] struct body references primitive ID/enum types this doc set never declares — a real domain-model companion module, not a macro-parser question; see report): lines 137-147

// ---- customer — [NEW: full declaration; M5] ----
// SKIPPED (#[dataset] struct body references primitive ID/enum types this doc set never declares — a real domain-model companion module, not a macro-parser question; see report): lines 150-166

// ---- sar_bundle — [NEW: M12; existence itself is tipping-off-sensitive] ----
// SKIPPED (#[dataset] struct body references primitive ID/enum types this doc set never declares — a real domain-model companion module, not a macro-parser question; see report): lines 169-182

// ---- screening_hit — [NEW: M10] ----
// SKIPPED (#[dataset] struct body references primitive ID/enum types this doc set never declares — a real domain-model companion module, not a macro-parser question; see report): lines 185-193

// ---- payment (Mode B holds) — [NEW: M11; Pending entities live here] ----
// SKIPPED (#[dataset] struct body references primitive ID/enum types this doc set never declares — a real domain-model companion module, not a macro-parser question; see report): lines 196-209

// ---- qa_review — [NEW: M14] ----
// SKIPPED (#[dataset] struct body references primitive ID/enum types this doc set never declares — a real domain-model companion module, not a macro-parser question; see report): lines 212-221

// ---- notification — [NEW: M16; fed by notify(channel(...)) obligations] ----
// SKIPPED (#[dataset] struct body references primitive ID/enum types this doc set never declares — a real domain-model companion module, not a macro-parser question; see report): lines 224-227

// ---- user_profile — [NEW: M22.1 self-service] ----
// SKIPPED (#[dataset] struct body references primitive ID/enum types this doc set never declares — a real domain-model companion module, not a macro-parser question; see report): lines 230-232

// ---- support_ticket — [NEW: M22.2] ----
// SKIPPED (#[dataset] struct body references primitive ID/enum types this doc set never declares — a real domain-model companion module, not a macro-parser question; see report): lines 235-237

// ---- Reference datasets — [RFC 0023 §1C.2 pattern] ----
// SKIPPED (#[dataset] struct body references primitive ID/enum types this doc set never declares — a real domain-model companion module, not a macro-parser question; see report): lines 240-241

// SKIPPED (#[reference] used as a bare declarative list, not attached to an item — grammar mismatch, see report): lines 248-253

// ---- Relations — [§4: positive-only; negation = compile error] ----
// SKIPPED (body-less fn signature isn't a valid free-standing Rust item — grammar mismatch, see report): lines 256-264

// NOTE (M7): customer-level "network" (shared devices/counterparty rings) is a
// QUERY over transaction attributes (device_id, beneficiary), NOT a relation —
// cardinality unbounded. Relation lowering is reserved for bounded KYC graphs.

// ---- Stream ports — [RFC §8.1 + §6.5] ----
nirdosha_rt::stream_port! {
    port txn_in    { bind "card_network.rails";   semantics = at_least_once; }
    port txn_out   { publish "txn.authorized";    format = "avro"; schema = "transaction"; }
    port decisions { publish "guard.decisions";   format = "avro"; } // → M2.5 wall
}

// ---- Workflows — closed & deterministic; type-state (illegal transitions don't compile) ----
nirdosha_guard_macros::workflow! {
    machine CaseStatus {                                   // [RFC §8.6 verbatim]
        open -> investigating -> [confirmed_fraud, false_positive, escalate];
        confirmed_fraud -> sar_filed -> closed;
        false_positive -> closed;
    }
}
nirdosha_guard_macros::workflow! {                                   // [NEW → 3.3]
    machine AlertStatus {
        new -> in_progress -> [closed_false_positive, escalated, pending_info];
        pending_info -> [in_progress, escalated];          // → 3.7 RFI
        closed_false_positive -> in_progress;              // reopen = TL, audited
    }
}
nirdosha_guard_macros::workflow! {                                   // [NEW → M11]
    machine PaymentStatus {
        received -> held -> [released, blocked];           // transitions via I3 revalidate
        held -> held;                                      // no self-loop releases
    }
}
nirdosha_guard_macros::workflow! {                                   // [NEW → M12]
    machine SarStatus {
        draft -> in_review -> [filed, rejected, do_not_file];
        rejected -> draft;                                 // correction loop → 12.7
        filed -> continuation_due -> [filed, ceased];      // → 12.9
    }
}
nirdosha_guard_macros::workflow! {                                   // [NEW → M14]
    machine QaReviewStatus { assigned -> scored -> [closed, returned]; }
}

// ---- Enumerations (get_options screens) — §8.4 / §1C.2: both hops guarded ----
nirdosha_rt::enumerate! { options from alert.disposition_code filter active == true cap(row_cap = 50) } // → 3.3
nirdosha_rt::enumerate! { options from transaction.channel      filter active == true cap(row_cap = 50) } // → 6.1
nirdosha_rt::enumerate! { options from country_risk             filter active == true cap(row_cap = 250) } // → 13.2

// ============================================================================
// 20_ingestion.nir — [RFC §8.1 verbatim policies] + analyst reads.
// ============================================================================

nirdosha_rt::guard_policy! {                                // [RFC §8.1 verbatim]
    allow "ingest-create-txn" for SvcIngest
    when action == "create" && resource == "transaction"
    purpose(FraudMonitoring)
    field_policy { required(tenant_id, subject_id, amount, currency, status, occurred_at)
                   allowed(channel, merchant_id, card_token, device_id, geo) }
    cap(affected_rows = 1)
    obligate audit(full)
}

nirdosha_rt::guard_policy! {                                // [RFC §8.1 verbatim — V7 SoD record]
    deny "ingest-no-readback" for SvcIngest
    when action == "read" && resource == "transaction"
    reason(sod.ingestion_is_write_only)
}

// → 6.1 Transaction search (Analyst) — listing; caps + predicate_use per I15
nirdosha_rt::guard_policy! {
    allow "analyst-search-transaction" for Analyst
    when action == "read" && resource == "transaction"
    purpose(AmlInvestigation)
    filter tenant_scope()
    cap(row_cap = 500, max_scan_rows = 100_000, max_execution = 10s)
    grant predicate_use(amount, currency, channel, merchant_id, device_id, geo)
    grant count_allowed                                     // → totals on 6.1
    obligate audit(sampled)                                 // CONFIDENTIAL ⇒ rate 1.0 via table
}

// → 6.2 Transaction detail — same surface, no listing shortcuts; masked fields
// (card_token) absent per classification unless subject has explicit grant (none here).
nirdosha_rt::guard_policy! {
    allow "analyst-open-transaction" for Analyst
    when action == "read" && resource == "transaction"
    purpose(AmlInvestigation)
    filter tenant_scope()
    cap(row_cap = 1)
    obligate audit(full)
}

// → 6.6 manual alert from txn (L2) — evidence-carrying write; svc:alerts remains
// the only *service* writer; this is the human path.
nirdosha_rt::guard_policy! {
    allow "analyst-flag-transaction" for Analyst
    when action == "update" && resource == "transaction"
    purpose(AmlInvestigation)
    requires invariant(amount_positive) && invariant(currency_iso)
    field_policy { allowed(analyst_flag, analyst_flag_reason)
                   forbidden(tenant_id, subject_id, amount, currency, status,
                             occurred_at, card_token) }
    cap(affected_rows = 1)
    obligate audit(full)
}

// → 19.3 exception replay / 19.2 reprocess — IT admin ops as guarded mutations (§6.6)
nirdosha_rt::guard_policy! {
    allow "ops-ingest-admin" for Admin
    when action in ["update", "migrate"] && resource == "ingest_admin"
    purpose(PlatformOperations)
    escalate to approval(chain policy_release)
    obligate audit(full)
}

// ============================================================================
// 30_features.nir — [RFC §8.2 verbatim] + model-input features.
// ============================================================================

nirdosha_rt::window! {                                      // [RFC §8.2 verbatim]
    feature velocity_1h(subject_id) =
        sliding(1h, keys = [subject_id], aggs = [count, sum(amount)]);
    feature impossible_travel(subject_id) =
        session(30m, keys = [subject_id], expr = geo_speed(geo) > 900 km/h);
}
nirdosha_rt::window! {                                      // [NEW — model inputs, §8.3]
    feature distinct_payees_7d(subject_id) =
        hopping(7d every 1h, keys = [subject_id], aggs = [count_distinct(beneficiary)]);
    feature amount_dev_30d(subject_id) =
        hopping(30d every 6h, keys = [subject_id], aggs = [stddev(amount)]);
}

nirdosha_rt::guard_policy! {                                // [RFC §8.2 verbatim + grants]
    allow "features-read" for SvcFeatures
    when action == "read" && resource == "txn_events"
    purpose(FraudMonitoring)
    destination(feature_pipeline)
    cap(max_scan_rows = 500_000, max_execution = 10s)
    grant predicate_use(merchant_id, geo, amount)           // I15: explicit, auditable
    obligate audit(sampled)
}

// ============================================================================
// 40_scoring.nir — [RFC §8.3/§8.4 verbatim] + hit disposition + list mgmt.
// ============================================================================

nirdosha_rt::model_artifact! {                              // [RFC §8.3 verbatim]
    model rt_fraud_v1 {
        format = onnx;
        inputs = [velocity_1h, distinct_payees_7d, amount_dev_30d, impossible_travel];
        outputs = [score: f64, explanation: vec[string]];
        threshold_alert = 0.85;
    }
}

nirdosha_rt::guard_policy! {                                // [RFC §8.3 verbatim — V7 SoD]
    deny "scoring-no-side-effects" for SvcScoring
    when action in ["create", "update"] && resource in ["alert", "case", "transaction", "payment"]
    reason(sod.pipeline_stage_isolation)
}

// → 9.1 model swap = guarded admin mutation, never a config edit [RFC §8.3]
nirdosha_rt::guard_policy! {
    allow "scoring-model-swap" for PolicyEngineer
    when action == "update" && resource == "model"
    purpose(ModelGovernance)
    escalate to approval(chain model_release)               // maker≠checker; shadow flag carried
    obligate audit(full)
}

// ---- Screening [RFC §8.4 verbatim matcher + policy] ----
nirdosha_rt::matcher! {
    matcher sanctions {
        algorithm = fuzzy_jaro_winkler;
        threshold = 0.92;
        lists = [ofac_sdn, un_consolidated, eu_fsf];
    }
}
nirdosha_rt::matcher! {                                     // [NEW → 10.4 internal/PEP lists]
    matcher internal_watchlist {
        algorithm = fuzzy_jaro_winkler;
        threshold = 0.95;
        lists = [internal_watch, pep_global];
    }
}

nirdosha_rt::guard_policy! {                                // [RFC §8.4 verbatim]
    allow "screening-check" for SvcScreening
    when action == "read" && resource == "sanctions_lists"
    purpose(FraudMonitoring)
    cap(max_scan_rows = 50)
    obligate audit(full)                                    // every check auditable
}

// svc:screening writes hits — sole writer [NEW]
nirdosha_rt::guard_policy! {
    allow "screening-write-hit" for SvcScreening
    when action == "create" && resource == "screening_hit"
    purpose(FraudMonitoring)
    field_policy { required(tenant_id, subject_ref, list_id, match_score)
                   forbidden(disposition, rationale) }      // analyst-only fields
    cap(affected_rows = 1)
    obligate audit(full)
}

// → 10.2 Hit disposition (Analyst) [NEW]
nirdosha_rt::guard_policy! {
    allow "analyst-disposition-hit" for Analyst
    when action == "update" && resource == "screening_hit"
    purpose(AmlInvestigation)
    field_policy { allowed(disposition, rationale)
                   forbidden(subject_ref, list_id, match_score) }
    requires invariant(rationale_present)
    cap(affected_rows = 1)
    obligate audit(full)
}

// → 10.3/10.4 list upload & config = migrate (maker-checker) [NEW]
nirdosha_rt::guard_policy! {
    allow "screening-list-migrate" for Admin
    when action == "migrate" && resource in ["sanctions_lists", "matcher"]
    purpose(ModelGovernance)
    escalate to approval(chain policy_release)              // V5 re-attestation on swap
    obligate audit(full)
}

// ============================================================================
// 50_alerts.nir — [RFC §8.5 verbatim raise] + full L1 triage surface.
// ============================================================================

nirdosha_rt::guard_policy! {                                // [RFC §8.5 verbatim]
    allow "alert-raise" for SvcAlerts
    when action == "create" && resource == "alert"
    requires field(score) >= model(rt_fraud_v1).threshold_alert
          or field(rule_hits).nonempty()
    field_policy {
        required(tenant_id, txn_id, score, model_version, policy_version)
        forbidden(status)                    // analyst-only field — guard rejects writes to it
    }
    cap(affected_rows = 1)
    obligate audit(full)
    obligate notify(channel("analyst-inbox"))
}

// SoD: alerts service never mutates what it created status-wise [NEW — V7 record]
nirdosha_rt::guard_policy! {
    deny "alerts-no-edit" for SvcAlerts
    when action == "update" && resource == "alert"
    reason(sod.alert_writer_is_blind_to_status)
}

// → 3.1/3.2/3.4 queue + detail + related (Analyst)
nirdosha_rt::guard_policy! {
    allow "analyst-read-alert" for Analyst
    when action == "read" && resource == "alert"
    purpose(AmlInvestigation)
    filter tenant_scope()
    cap(row_cap = 200, max_scan_rows = 50_000, max_execution = 5s)
    field_policy { forbidden(sar_linked) }                  // tipping-off: absent, not masked
    grant count_allowed
    obligate audit(sampled)
}

// → 3.3 disposition; 3.6 reassign; 3.7 RFI status; 3.4 merge/dupe-link [NEW]
nirdosha_rt::guard_policy! {
    allow "analyst-disposition-alert" for Analyst
    when action == "update" && resource == "alert"
    purpose(AmlInvestigation)
    requires field(status).transition_allowed()             // AlertStatus machine
    requires invariant(rationale_present)                   // → min-rationale gate
    field_policy {
        allowed(status, assignee, disposition_code, rationale, duplicate_of, case_id)
        forbidden(txn_id, score, rule_hits, model_version, policy_version, tenant_id, sar_linked)
    }
    cap(affected_rows = 1)                                  // bulk ops loop per-row; per-record receipts
    obligate audit(full)
}

// → 3.5 bulk reprioritize (TL) — one identity, per-row audit [NEW]
nirdosha_rt::guard_policy! {
    allow "lead-bulk-alert" for ComplianceLead
    when action == "update" && resource == "alert"
    purpose(Operations)
    field_policy { allowed(assignee, severity, tags) forbidden(everything_else) }
    cap(affected_rows = 100)
    obligate audit(full)
}

// → 3.9 SLA monitor (aggregate; cohort floor — no analyst-level rows leak) [NEW]
nirdosha_rt::guard_policy! {
    allow "lead-sla-aggregate" for ComplianceLead
    when action == "aggregate" && resource == "alert"
    purpose(Operations)
    filter time_range(created_at, within, retention_window())
    cap(cohort_floor = 5, max_scan_rows = 1_000_000, max_execution = 30s)
    mask(subject_id, tokenized)
    obligate audit(full)
}

// ============================================================================
// 60_case_management.nir — [RFC §8.6 verbatim core] + create/read/merge/RFI.
// ============================================================================

nirdosha_rt::guard_policy! {                                // [NEW — 3.8 escalation path]
    allow "analyst-create-case" for Analyst
    when action == "create" && resource == "case"
    purpose(AmlInvestigation)
    field_policy { required(tenant_id, subject_ref, alert_ids, rationale)
                   forbidden(status, sar_id, disposition) } // svc:case seeds status
    cap(affected_rows = 1)
    obligate audit(full)
    obligate notify(channel("case-inbox"))
}

nirdosha_rt::guard_policy! {                                // → 4.2/4.3 read [NEW]
    allow "case-read" for Analyst
    when action == "read" && resource == "case"
    purpose(AmlInvestigation)
    filter tenant_scope()
    field_policy { forbidden(sar_id) }                      // tipping-off
    cap(row_cap = 200, max_scan_rows = 50_000)
    grant count_allowed
    obligate audit(sampled)
}

nirdosha_rt::guard_policy! {                                // [RFC §8.6 verbatim — 4.10/4.13]
    allow "case-transition" for Analyst
    when action == "update" && resource == "case"
    requires field(status).transition_allowed()
    field_policy { allowed(status, assigned_to, disposition, rationale, merged_into)
                   forbidden(alert_ids) }                   // composition immutable
    requires invariant(rationale_present)
    cap(affected_rows = 1)
    obligate audit(full)
}

// → 4.11 four-eyes on suspicious closure [NEW — chain from 00_core]
nirdosha_rt::guard_policy! {
    allow "case-close-confirm" for ComplianceLead
    when action == "update" && resource == "case"
    requires field(status) in ["confirmed_fraud", "false_positive"]
    escalate to approval(chain case_review)
    field_policy { allowed(review_verdict, review_note) forbidden(everything_else) }
    cap(affected_rows = 1)
    obligate audit(full)
}

// svc:case internal transitions (status seeding, sar_id attach) [NEW]
nirdosha_rt::guard_policy! {
    allow "case-svc-write" for SvcCase
    when action == "update" && resource == "case"
    purpose(AmlInvestigation)
    field_policy { allowed(status, sar_id) forbidden(alert_ids, rationale, assigned_to) }
    cap(affected_rows = 1)
    obligate audit(full)
}

// ============================================================================
// 65_sar.nir — SAR lifecycle. Tipping-off posture: existence + subjects are
// RESTRICTED-class; llm_context denied above CONFIDENTIAL everywhere here.
// ============================================================================

nirdosha_rt::guard_policy! {                                // → 12.2/12.3/12.4 draft [NEW]
    allow "analyst-draft-sar" for Analyst
    when action == "create" && resource == "sar_bundle"
    purpose(AmlInvestigation)
    requires invariant(sar_subject_in_case)
    field_policy { required(case_id, subject) forbidden(status, goaml_ref, filed_at) }
    cap(affected_rows = 1)
    obligate audit(full)
    destination(llm_context) denied_above(CONFIDENTIAL)     // copilot: no SAR content
}

nirdosha_rt::guard_policy! {                                // → 12.2–12.5 edit [NEW]
    allow "analyst-edit-sar" for Analyst
    when action == "update" && resource == "sar_bundle"
    requires field(status) == "draft"
    field_policy { allowed(narrative, activity_codes, amount_total, txn_refs, next_review_at)
                   forbidden(case_id, subject, status, goaml_ref) }
    cap(affected_rows = 1)
    obligate audit(full)
}

nirdosha_rt::guard_policy! {                                // → 12.6 MLRO decision [NEW]
    allow "mlro-decide-sar" for Mlro
    when action == "update" && resource == "sar_bundle"
    purpose(AmlInvestigation)
    requires field(status).transition_allowed()             // SarStatus machine
    field_policy { allowed(status, mlro_rationale) forbidden(narrative, txn_refs) }
    cap(affected_rows = 1)
    obligate audit(full)
}

nirdosha_rt::guard_policy! {                                // [RFC §8.6 verbatim shape → 12.10]
    allow "sar-export" for ComplianceLead
    when action == "export" && resource == "sar_bundle"
    requires field(status) == "confirmed_fraud"             // NOTE: RFC machine files from
                                                            // confirmed_fraud; align via
                                                            // sar_filed state in practice
    escalate to approval(chain sar_release)                 // quorum(2); timeout(deny)
    destination(export-file)
    obligate audit(full)                                    // egress: NEVER sampled (§2)
    obligate notify(channel("regulatory-log"))
}

// → 12.7 submission receipts written by filing driver [NEW]
nirdosha_rt::guard_policy! {
    allow "sar-submission-write" for SvcCase
    when action == "create" && resource == "sar_submission"
    purpose(AmlInvestigation)
    cap(affected_rows = 1)
    obligate audit(full)
}

// Tipping-off explicit denies — denial records are themselves V-pass evidence [NEW]
nirdosha_rt::guard_policy! {
    deny "cs-no-sar" for CsAgent
    when action in ["read", "aggregate"] && resource in ["sar_bundle", "case", "alert"]
    reason(tipping_off.sar_existence_invisible)
}
nirdosha_rt::guard_policy! {
    deny "rm-no-sar" for RmUser
    when action in ["read", "aggregate"] && resource in ["sar_bundle", "case", "alert"]
    reason(tipping_off.sar_existence_invisible)
}

// ============================================================================
// 70_mcp.nir — [RFC §8.8 verbatim structure]. I17 defaults are non-negotiable.
// ============================================================================

nirdosha_rt::mcp_tools! {
    server analyst_copilot {
        identity = "mcp:analyst-copilot";
        tools = [query_records(Transaction, Alert, Case, Customer),
                 get_options(AlertStatus, CaseStatus, TxnChannel, DispositionCode),
                 evaluate,             // dry-run first — mandatory for writes
                 submit_write];        // gated: requires matching evaluate (plan-hash)
        delegation { bind user + agent; ttl = 30m; max_tool_calls = 60; rate = 20/min; }
        defaults {
            audit = full;              // I17: agent access is never sampled
            row_cap = 50;
            destination = llm_context; // policy still governs per field/class
        }                              // masked fields are ABSENT, not masked-in-place
    }
}

// Copilot SAR protection (belt-and-braces over 65_sar denies) [NEW]
nirdosha_rt::guard_policy! {
    deny "copilot-no-sar" for McpCopilot
    when action == "read" && resource == "sar_bundle"
    reason(tipping_off.llm_context_denied_for_restricted)
}

// ============================================================================
// 75_lineage.nir — [RFC 0026 §9–10]. Phases 1–4 of RFC 0026.
// ============================================================================

nirdosha_rt::lineage_query! {                               // [RFC 0026 §9.2 verbatim]
    view downstream_of(start: NodeId, depth: u32) -> DownstreamRow {
        node: NodeId,
        edge_type: EdgeType [filter],
        policy_version: PolicyVersion [filter],
        authority: Authority [filter],
    };
    view upstream_of(start: NodeId, depth: u32) -> UpstreamRow {
        node: NodeId,
        edge_type: EdgeType [filter],
        purpose: Purpose [filter],
        destination: Destination [filter],
    };
    view provenance_of(trace_id: TraceId) -> ProvenanceRow {
        step: u32, src: NodeId, edge_type: EdgeType, dst: NodeId,
        policy_version: PolicyVersion, receipt_digest: [u8; 32],
    };
}

nirdosha_rt::guard_policy! {                                // [RFC 0026 §9.1 verbatim → M7]
    allow "lineage-explore" for Analyst
    when action == "lineage_query" && resource == "downstream_of"
    filter tenant_scope()
    cap(max_depth = 5, max_nodes = 1_000, max_execution = 5s)
    destination(llm_context) denied_above(CONFIDENTIAL)
    obligate audit(full)                                    // lineage reads are never sampled
}
nirdosha_rt::guard_policy! {                                // [NEW → 20.1 auditor scope]
    allow "lineage-audit" for Auditor
    when action == "lineage_query" && resource in ["downstream_of", "upstream_of", "provenance_of"]
    purpose(Audit)
    cap(max_depth = 8, max_nodes = 10_000, max_execution = 30s)
    obligate audit(full)
}

// → 8.6/8.7 threshold simulation — dry-run at scale [RFC 0026 §10.3; Phase 4]
nirdosha_rt::policy_simulation! {
    simulation sim_threshold_tune {
        window = 7d;
        candidate_policy = "2025.12.1-candidate";
        baseline_policy = current;
        measure { alert_volume, denied_flows, escalation_load, case_load };
        obligate audit(full);
    }
}
nirdosha_rt::guard_policy! {
    allow "policy-simulate" for PolicyEngineer
    when action == "simulate" && resource == "policy_simulation"
    purpose(ModelGovernance)
    obligate audit(full)                                    // read-only results; V10-subject
}

// → 8.3 threshold edits = migrate (dry-run mandatory at runtime, §2) [NEW]
nirdosha_rt::guard_policy! {
    allow "threshold-migrate" for PolicyEngineer
    when action == "migrate" && resource in ["window", "policy", "matcher", "model"]
    purpose(ModelGovernance)
    escalate to approval(chain policy_release)
    obligate audit(full)
}

// → V10 findings read (dormant/undeclared/issued-unconsumed/pending) [NEW → new screens]
nirdosha_rt::guard_policy! {
    allow "v10-findings-read" for PolicyEngineer
    when action == "read" && resource == "lineage_findings"
    purpose(ModelGovernance)
    obligate audit(full)
}

// ============================================================================
// 80_consumers.nir — [RFC §8.7 verbatim] + leadership aggregates + egress.
// ============================================================================

nirdosha_rt::guard_policy! {                                // [RFC §8.7 verbatim — 15.1/15.4]
    allow "bi-aggregate" for SvcBi
    when action == "aggregate" && resource == "transaction"
    filter time_range(occurred_at, within, retention_window())
    cap(cohort_floor = 10, max_scan_bytes = 5GB, max_execution = 60s)
    mask(subject_id, tokenized)                             // k-anonymity via tokenization
    destination(warehouse)
    obligate audit(full)                                    // aggregates are still audited (I9)
}

// → 2.2–2.4, 2.6 dashboards — browser aggregate reads [NEW]
nirdosha_rt::guard_policy! {
    allow "lead-dashboard" for ComplianceLead
    when action == "aggregate" && resource in ["alert", "case", "sar_bundle_stats"]
    purpose(Operations)
    filter tenant_scope()
    cap(cohort_floor = 5, max_execution = 30s)
    mask(subject_id, tokenized)
    obligate audit(full)
}

// → 15.5 governed export center [NEW — egress action, §2]
nirdosha_rt::guard_policy! {
    allow "governed-export" for Analyst
    when action == "export" && resource in ["alert", "case", "transaction"]
    purpose(AmlInvestigation)
    destination(export-file)
    escalate to approval(chain egress_release)              // above-threshold rows → approval
    cap(row_cap = 50_000, max_result_bytes = 100MB)
    obligate audit(full)                                    // export: most-audited action
}

// ============================================================================
// 85_intervention.nir — Mode B holds. timeout ⇒ DENY, never auto-release.
// ============================================================================

nirdosha_rt::guard_policy! {                                // svc:ingest parks the hold [NEW]
    allow "ingest-create-hold" for SvcIngest
    when action == "create" && resource == "payment"
    purpose(FraudMonitoring)
    field_policy { required(tenant_id, rail_ref, originator, beneficiary, amount,
                             currency, status, hold_reason, hold_expires_at)
                   forbidden(decision_by, decision_rationale) }
    cap(affected_rows = 1)
    obligate audit(full)
}

// → 11.1/11.2 desk queue + decision [NEW]
nirdosha_rt::guard_policy! {
    allow "ops-read-holds" for OpsAnalyst
    when action == "read" && resource == "payment"
    purpose(FraudMonitoring)
    filter tenant_scope()
    cap(row_cap = 100, max_scan_rows = 10_000, max_execution = 5s)
    grant count_allowed
    obligate audit(sampled)
}

nirdosha_rt::guard_policy! {
    allow "ops-decide-hold" for OpsAnalyst
    when action == "update" && resource == "payment"
    purpose(FraudMonitoring)
    requires field(status).transition_allowed()             // PaymentStatus machine
    requires invariant(rationale_present)
    field_policy { allowed(status, decision_by, decision_rationale)
                   forbidden(rail_ref, originator, beneficiary, amount, hold_reason) }
    cap(affected_rows = 1)
    obligate audit(full)
    // release revalidates at commit (I3) — runtime, not policy
}
// Above-authority releases escalate: same policy family, higher band [NEW]
nirdosha_rt::guard_policy! {
    allow "ops-decide-hold-highvalue" for OpsAnalyst
    when action == "update" && resource == "payment"
    requires field(amount) > 100_000
    escalate to approval(chain override_release)            // → 11.3; + step_up above 1M:
    // escalate to step_up(mfa) — combine via two obligations on the same block in practice
    obligate audit(full)
}

// → 11.4 explicit deny of silent auto-release [NEW — documents the guard's stance]
nirdosha_rt::guard_policy! {
    deny "auto-release-on-timeout" for SvcIngest
    when action == "update" && resource == "payment"
    requires field(status) == "released" && expired(hold_expires_at)
    reason(policy.timeout_resolves_to_deny)
}

// → 11.5 outcomes stats [NEW]
nirdosha_rt::guard_policy! {
    allow "hold-stats" for ComplianceLead
    when action == "aggregate" && resource == "payment"
    purpose(Operations)
    cap(cohort_floor = 5, max_execution = 30s)
    mask(subject_id, tokenized)
    obligate audit(full)
}

// ============================================================================
// 90_ops_admin.nir — everything-is-a-catalog-mutation (P6). No runtime YAML (N-A2).
// ============================================================================

// → M18.1/18.2 runtime user↔role grants (roles themselves are compiled; this is assignment)
nirdosha_rt::guard_policy! {
    allow "admin-grant-role" for Admin
    when action in ["create", "update", "delete"] && resource == "user_role"
    purpose(PlatformOperations)
    escalate to approval(chain policy_release)              // maker≠checker
    cap(affected_rows = 1)
    obligate audit(full)
}

// → M18.8 delegate/proxy = scoped delegation mint [RFC 0026 §7.5 semantics; NEW policy]
nirdosha_rt::guard_policy! {
    allow "admin-mint-delegation" for Admin
    when action == "delegate" && resource == "delegation_token"
    purpose(PlatformOperations)
    cap(ttl_max = 90d)                                      // delegate cover ≤ 90d
    obligate audit(full)                                    // I11: delegation fully audited
}
// Regulator inspection = same mint path with RegulatorViewer principal,
// exact-scope + TTL + full audit per §9.4 (→ M21.2).

// → M13.2/13.5 reference data & global params [NEW]
nirdosha_rt::guard_policy! {
    allow "refdata-migrate" for Admin
    when action == "migrate" && resource in ["refdata", "country_risk", "global_params"]
    purpose(PlatformOperations)
    escalate to approval(chain policy_release)
    obligate audit(full)
}

// → M18.7 retention purge — data_contract declares; purge executes dual-controlled
// [RFC 0026 §10.1 — Phase 4 macro; retention floor enforced by contract]
nirdosha_rt::data_contract! {
    contract txn_v2 {
        entity = "transaction";
        owner = "payments-platform";
        schema_version = "2.1";
        quality { invariant(amount_positive), invariant(currency_iso) };
        retention = 7y;
        downstream_consumers = [
            node(runtime, "svc:bi"),
            node(runtime, "regulatory_archive"),
            node(data, "rt_fraud_v1", "1.3.0"),
        ];
    }
}
nirdosha_rt::guard_policy! {
    allow "retention-purge" for Admin
    when action == "delete" && resource == "refdata_archive"
    purpose(PlatformOperations)
    requires invariant(no_legal_hold)
    escalate to approval(chain purge_release)               // quorum(2); type-"PURGE" confirm at runtime
    cap(affected_rows = 1_000_000)
    obligate audit(full)
}

// → M18.9 DSAR = export-with-hold-check [NEW]
nirdosha_rt::guard_policy! {
    allow "dsar-export" for Admin
    when action == "export" && resource == "customer"
    purpose(Operations)
    requires invariant(no_legal_hold)
    destination(export-file)
    escalate to approval(chain egress_release)
    obligate audit(full)
}

// → M19 driver install/upgrade/rollback = catalog mutation [RFC §7.5; §11 swap procedure]
nirdosha_rt::guard_policy! {
    allow "driver-migrate" for Admin
    when action == "migrate" && resource == "driver"
    purpose(PlatformOperations)
    escalate to approval(chain policy_release)              // V5/V8 re-run gates the apply
    obligate audit(full)
}

// → M22.1 self-profile [NEW]
nirdosha_rt::guard_policy! {
    allow "self-update-profile" for Analyst
    when action == "update" && resource == "user_profile"
    filter subject_scope()                                  // own row only
    field_policy { allowed(locale, tz, notif_prefs) forbidden(user_id) }
    cap(affected_rows = 1)
    obligate audit(sampled)
}

// → M22.2 support ticket [NEW]
//   Opened to all human roles so the chrome `profile → Help & Support`
//   route (`/help/support`, guard = create/support_ticket, AllHuman)
//   resolves to a real policy record.
nirdosha_rt::guard_policy! {
    allow "any-create-ticket" for Analyst, ComplianceLead, Mlro, QaReviewer, OpsAnalyst, PolicyEngineer, Admin, Auditor, CsAgent, RmUser
    when action == "create" && resource == "support_ticket"
    purpose(Operations)
    field_policy { required(tenant_id, category, priority, description) }
    cap(affected_rows = 1)
    obligate audit(sampled)
}

// ============================================================================
// 95_qa.nir — QA role surface + SoD denies both directions.
// ============================================================================

nirdosha_rt::guard_policy! {                                // → 14.1/14.2 read closed items
    allow "qa-read-closed" for QaReviewer
    when action == "read" && resource in ["alert", "case"]
    purpose(QaReview)
    filter tenant_scope()
    field_policy { forbidden(sar_linked) }
    cap(row_cap = 100, max_scan_rows = 50_000)
    obligate audit(sampled)
}

nirdosha_rt::guard_policy! {                                // → 14.2/14.4 scorecard + feedback
    allow "qa-write-review" for QaReviewer
    when action in ["create", "update"] && resource == "qa_review"
    requires field(status).transition_allowed()             // QaReviewStatus machine
    field_policy { required(item_ref, rubric_scores, result)
                   forbidden(item_ref_content) }            // reads originals; never edits them
    cap(affected_rows = 1)
    obligate audit(full)
    obligate notify(channel("qa-inbox"))
}

// SoD: QA never dispositions; analysts never score [NEW — conformance SoD probes]
nirdosha_rt::guard_policy! {
    deny "qa-no-disposition" for QaReviewer
    when action == "update" && resource == "alert"
    reason(sod.qa_separation)
}
nirdosha_rt::guard_policy! {
    deny "analyst-no-qa" for Analyst
    when action in ["create", "update"] && resource == "qa_review"
    reason(sod.qa_separation)
}

// → 14.3 analyst sees own scorecard [NEW]
nirdosha_rt::guard_policy! {
    allow "self-read-qa" for Analyst
    when action == "read" && resource == "qa_review"
    filter subject_scope()
    cap(row_cap = 50)
    obligate audit(full)
}

// ============================================================================
// 96_restricted_views.nir — coarse, masked, destination-controlled views.
// ============================================================================

// → 21.3 CS status lookup: status band + scripted actions ONLY.
// national_id, name, amounts — absent. sar/case/alert: denied in 65_sar.
nirdosha_rt::guard_policy! {
    allow "cs-payment-status" for CsAgent
    when action == "read" && resource == "payment"
    purpose(CustomerService)
    filter tenant_scope()
    cap(row_cap = 1)
    field_policy { allowed(status, hold_expires_at, currency)          // coarse band only
                   forbidden(amount, rail_ref, originator, beneficiary,
                             hold_reason, decision_by, decision_rationale) }
    obligate audit(full)                                    // every CS peek is auditable
}

// → 21.4 RM view: restriction band + required action only
nirdosha_rt::guard_policy! {
    allow "rm-restriction-view" for RmUser
    when action == "read" && resource == "customer"
    purpose(CustomerService)
    filter tenant_scope()
    cap(row_cap = 1)
    field_policy { allowed(restriction_status)              // single field
                   forbidden(national_id, name, dob, address, phone, risk_rating,
                             pep_flag, sanctions_status, expected_profile) }
    obligate audit(full)
}

// → 21.1 Auditor: broad READ, zero WRITE, egress gated
nirdosha_rt::guard_policy! {
    allow "auditor-read" for Auditor
    when action == "read" && resource in ["alert", "case", "transaction",
                                          "customer", "sar_bundle", "payment",
                                          "screening_hit", "qa_review"]
    purpose(Audit)
    filter tenant_scope()
    field_policy { forbidden(sar_linked) }                  // existence still controlled
    cap(row_cap = 1_000, max_scan_rows = 500_000, max_execution = 60s)
    obligate audit(full)                                    // read-only assurance, fully audited
}
nirdosha_rt::guard_policy! {
    deny "auditor-no-write" for Auditor
    when action in ["create", "update", "delete", "migrate"]
    reason(audit.read_only_principal)
}
nirdosha_rt::guard_policy! {                                // auditor pack export → 20.3
    allow "auditor-export" for Auditor
    when action == "export" && resource in ["audit_pack", "sar_bundle"]
    purpose(Audit)
    destination(export-file)
    escalate to approval(chain egress_release)
    obligate audit(full)
}

// → 21.2 Regulator: reachable ONLY through scoped delegation mint (90_ops_admin);
// base role has no grants — deny-by-default is the control. Explicit read window:
nirdosha_rt::guard_policy! {
    allow "regulator-scoped-read" for RegulatorViewer
    when action == "read" && resource in ["case", "sar_bundle", "transaction"]
    purpose(RegulatoryInspection)
    filter delegation_scope()                               // exact-scope per §9.4.1
    cap(row_cap = 100, max_scan_rows = 10_000)
    obligate audit(full)                                    // §9.4.5: full, never sampled
}
nirdosha_rt::guard_policy! {
    deny "regulator-no-export" for RegulatorViewer
    when action == "export"
    reason(regulatory.session_recording_only)
}

// ============================================================================
// 99_break_glass.nir — scoped, time-boxed, dual-approved, reconciled. Never global.
// ============================================================================

nirdosha_rt::break_glass! {                                 // → SAR emergency egress
    scope(resource("sar_bundle"), action("export"))
    ttl(1h)
    dual_approve above(severity(2))
    obligate audit(full)
    // auto-creates post-hoc review task; V10 holds finding open until reconciliation
}
nirdosha_rt::break_glass! {                                 // → payment release above authority
    scope(resource("payment"), action("update"))
    ttl(30m)
    dual_approve above(severity(2))
    obligate audit(full)
}
// NOTE: guard-down emergency access is OUT-OF-BAND by design (RFC 0023 §14):
// no break-glass macro path exists when the guard is down; dual-control paper
// procedure + mandatory reconciliation on recovery. Not expressible here — correct.

// ============================================================================
// Blocker D additions — policies added after the original doc so the
// `menus.toml` route-guard declarations resolve to real `guard_policy!`
// records.
// ============================================================================

// → M7 network graph link queries (7.1/7.2/7.3) + graph export (7.5)
nirdosha_rt::guard_policy! {
    allow "network-link-query" for Analyst, ComplianceLead
    when action == "link_query" && resource == "network_of"
    purpose(AmlInvestigation)
    cap(max_depth = 5, max_nodes = 1_000, max_execution = 5s)
    destination(llm_context) denied_above(CONFIDENTIAL)
    obligate audit(full)
}
nirdosha_rt::guard_policy! {
    allow "network-export" for Analyst
    when action == "export" && resource == "link_edge"
    purpose(AmlInvestigation)
    destination(export-file)
    escalate to approval(chain egress_release)
    cap(row_cap = 50_000, max_result_bytes = 100MB)
    obligate audit(full)
}

// → 8.6 simulation read-only view for ComplianceLead
nirdosha_rt::guard_policy! {
    allow "lead-view-simulation" for ComplianceLead
    when action == "simulate" && resource == "policy_simulation"
    purpose(ModelGovernance)
    obligate audit(full)
}

// → 8.3/8.10 policy/window migrate for ComplianceLead
nirdosha_rt::guard_policy! {
    allow "lead-migrate-policy" for ComplianceLead
    when action == "migrate" && resource in ["policy", "window"]
    purpose(ModelGovernance)
    escalate to approval(chain policy_release)
    obligate audit(full)
}

// → M16 notification feed, M16 handover, M17 static knowledge, M2 saved views
nirdosha_rt::guard_policy! {
    allow "notification-read" for Analyst, ComplianceLead, Mlro, QaReviewer, OpsAnalyst, PolicyEngineer, Admin, Auditor, CsAgent, RmUser
    when action == "read" && resource == "notification"
    purpose(Operations)
    filter tenant_scope()
    cap(row_cap = 200)
    obligate audit(sampled)
}
nirdosha_rt::guard_policy! {
    allow "static-docs-read" for Analyst, ComplianceLead, Mlro, QaReviewer, OpsAnalyst, PolicyEngineer, Admin, Auditor, CsAgent, RmUser
    when action == "read" && resource == "static_docs"
    purpose(Operations)
    cap(row_cap = 100)
    obligate audit(sampled)
}
nirdosha_rt::guard_policy! {
    allow "saved-view-read" for Analyst, ComplianceLead, Mlro, QaReviewer, OpsAnalyst, PolicyEngineer, Admin, Auditor, CsAgent, RmUser
    when action == "read" && resource == "saved_view"
    purpose(Operations)
    cap(row_cap = 50)
    obligate audit(sampled)
}
nirdosha_rt::guard_policy! {
    allow "handover-read" for Analyst, ComplianceLead, OpsAnalyst
    when action == "read" && resource == "handover"
    purpose(Operations)
    cap(row_cap = 100)
    obligate audit(sampled)
}

// → M20 audit log
nirdosha_rt::guard_policy! {
    allow "audit-record-read" for Auditor, Admin, Mlro
    when action == "read" && resource == "audit_record"
    purpose(Audit)
    filter tenant_scope()
    cap(row_cap = 1_000, max_scan_rows = 500_000, max_execution = 60s)
    obligate audit(full)
}

// → M22 support ticket (all human roles) + self-profile read
nirdosha_rt::guard_policy! {
    allow "self-read-profile" for Analyst, ComplianceLead, Mlro, QaReviewer, OpsAnalyst, PolicyEngineer, Admin, Auditor, CsAgent, RmUser
    when action == "read" && resource == "user_profile"
    filter subject_scope()
    cap(row_cap = 1)
    obligate audit(sampled)
}

#[test]
fn corpus_compiles_and_registers_policies() {
    let count = nirdosha_guard_registry::POLICIES.len();
    println!("registered policy records: {count}");
    assert!(count >= 76, "expected at least 76 registrations (one per guard_policy! block, more after action/resource-in-list fan-out), got {count}");
}

/// Phase 3 acceptance bar: `records()` on the real corpus produces
/// non-trivial, correctly-lowered `PolicyRecord`s — not just "the macro
/// didn't error." Each assertion below is a real clause from the doc,
/// checked against the specific structured value it should lower to.
#[test]
fn corpus_policies_lower_to_real_structured_records() {
    let records = nirdosha_guard_registry::records();
    assert_eq!(records.len(), nirdosha_guard_registry::POLICIES.len());

    let find = |id: &str| -> Vec<&nirdosha_guard_registry::PolicyRecord> {
        records.iter().filter(|r| r.id == id).collect()
    };

    // "ingest-create-txn": purpose + field_policy + affected_rows cap +
    // full audit obligation, all from one guard_policy! block.
    let ingest = find("ingest-create-txn");
    assert_eq!(ingest.len(), 1);
    let ingest = ingest[0];
    assert_eq!(ingest.subjects, vec!["SvcIngest".to_string()]);
    assert_eq!(ingest.affected_row_cap, Some(1));
    assert!(ingest
        .field_policy
        .contains(&nirdosha_guard_core::FieldPolicy::Required(vec!["tenant_id".into()])));
    assert!(ingest
        .field_policy
        .contains(&nirdosha_guard_core::FieldPolicy::Allowed(vec!["device_id".into()])));
    assert!(ingest.obligations.contains(&nirdosha_guard_core::Obligation::Audit {
        level: nirdosha_guard_core::AuditLevel::Full
    }));

    // "sar-export": quorum escalation + egress destination + notify.
    let sar_export = find("sar-export");
    assert_eq!(sar_export.len(), 1);
    let sar_export = sar_export[0];
    assert_eq!(
        sar_export.escalation,
        Some(nirdosha_guard_core::EscalateTarget::Approval { chain: "sar_release".into() })
    );
    assert_eq!(sar_export.destination, Some(nirdosha_guard_core::Destination::ExportFile));
    assert!(sar_export
        .obligations
        .contains(&nirdosha_guard_core::Obligation::Notify { channel: "regulatory-log".into() }));

    // "case-transition": requires field(status).transition_allowed() (not
    // structurally lowerable — must survive as a named condition, not be
    // silently dropped) plus a real invariant() conjunct.
    let case_transition = find("case-transition");
    assert_eq!(case_transition.len(), 1);
    assert!(!case_transition[0].conditions.is_empty());

    // "screening-write-hit": forbidden field_policy fields.
    let hit = find("screening-write-hit");
    assert_eq!(hit.len(), 1);
    assert!(hit[0]
        .field_policy
        .contains(&nirdosha_guard_core::FieldPolicy::Forbidden(vec!["disposition".into()])));

    // "auditor-no-write": one deny per fanned-out action, each with the
    // same reason code.
    let auditor_no_write = find("auditor-no-write");
    assert_eq!(auditor_no_write.len(), 4);
    for record in &auditor_no_write {
        assert_eq!(record.effect, nirdosha_guard_registry::Effect::Deny);
        assert_eq!(record.reason.as_deref(), Some("audit.read_only_principal"));
    }

    // "lead-sla-aggregate": cohort_floor cap + tokenized mask.
    let sla = find("lead-sla-aggregate");
    assert_eq!(sla.len(), 1);
    assert!(sla[0].caps.contains(&nirdosha_guard_core::Cap::CohortFloor(5)));
    assert_eq!(sla[0].masks.len(), 1);
    assert_eq!(sla[0].masks[0].field, vec!["subject_id".to_string()]);
}

/// Graduation proof for the six macros this phase unblocked: the real
/// corpus's own `purpose_taxonomy!`/`stream_port!`/`window!`/
/// `model_artifact!`/`matcher!`/`mcp_tools!` blocks (above, no longer
/// `// SKIPPED`) register real structured records — same bar as
/// `corpus_policies_lower_to_real_structured_records`, applied to the newly
/// unblocked macros instead of `guard_policy!`.
#[test]
fn corpus_newly_unblocked_macros_register_real_records() {
    let dump = nirdosha_guard_registry::dump();

    assert!(dump.purposes.iter().any(|p| p.code == "FraudMonitoring"));
    assert!(dump.purposes.iter().any(|p| p.code == "PlatformOperations"));

    let txn_in = dump.ports.iter().find(|p| p.name == "txn_in").expect("txn_in port must be registered");
    assert_eq!(txn_in.direction, "bind");
    assert_eq!(txn_in.target, "card_network.rails");

    let velocity = dump.windows.iter().find(|w| w.name == "velocity_1h").expect("velocity_1h window must be registered");
    assert_eq!(velocity.kind, "sliding");
    assert_eq!(velocity.key, "subject_id");

    let model = dump.models.iter().find(|m| m.name == "rt_fraud_v1").expect("rt_fraud_v1 model must be registered");
    assert_eq!(model.threshold_alert, Some(0.85));
    assert_eq!(model.inputs.len(), 4);

    let matcher = dump.matchers.iter().find(|m| m.name == "sanctions").expect("sanctions matcher must be registered");
    assert_eq!(matcher.lists, vec!["ofac_sdn", "un_consolidated", "eu_fsf"]);

    let copilot = dump.mcp_servers.iter().find(|s| s.name == "analyst_copilot").expect("analyst_copilot MCP server must be registered");
    assert_eq!(copilot.identity, "mcp:analyst-copilot");
    assert_eq!(copilot.max_tool_calls, Some(60));
}

/// The convention `cargo nirdosha verify --guard` (Plan Phase 6) looks
/// for: a `#[test]`, any name, any file, that calls
/// `write_dump_from_env()`. `cargo test` finds it by content
/// (`cargo test nirdosha_guard_dump`), not a fixed path — see that
/// function's doc comment in `nirdosha-guard-registry` for why this has
/// to run inside a test/run process rather than be read from source.
#[test]
fn nirdosha_guard_dump() {
    nirdosha_guard_registry::write_dump_from_env().expect("registry dump must write");
}

/// End-to-end: registry dump -> `RegistryView` -> `verify()`, on the real
/// corpus. This is the actual `cargo nirdosha verify --guard` pipeline
/// (minus Phase 6's build-time wiring), run against real content for the
/// first time since any of it existed.
#[test]
fn corpus_registry_dump_round_trips_through_verify() {
    let json = nirdosha_guard_registry::dump_json().expect("registry must serialize");
    let registry =
        nirdosha_guard_verify::RegistryView::from_json(&json).expect("dump must deserialize into RegistryView");
    assert_eq!(registry.policies.len(), nirdosha_guard_registry::POLICIES.len());

    let findings = nirdosha_guard_verify::verify(&registry);
    let by_pass = |pass: &str| findings.iter().filter(|f| f.pass == pass).count();
    println!("V1={} V2={} V3={} V4={} V5={} V6={} V7={} V8={}",
        by_pass("V1"), by_pass("V2"), by_pass("V3"), by_pass("V4"),
        by_pass("V5"), by_pass("V6"), by_pass("V7"), by_pass("V8"));
    for finding in &findings {
        println!("  {} {:?}: {} ({:?})", finding.pass, finding.severity, finding.message, finding.item);
    }

    // V1/V2/V3 must be clean: every registration has an id/action/resource,
    // effect is allow/deny, and action is one of the canonical wire
    // strings (Phase 1's for/in-list parsing plus Phase 3's lowering
    // produced well-formed records for all 125).
    assert_eq!(by_pass("V1"), 0, "V1 findings: {findings:?}");
    assert_eq!(by_pass("V2"), 0, "V2 findings: {findings:?}");
    assert_eq!(by_pass("V3"), 0, "V3 findings: {findings:?}");
    // V4: none of the corpus's real filters/conditions negate a relation.
    assert_eq!(by_pass("V4"), 0, "V4 findings: {findings:?}");
    // V5 needs driver manifests, which this corpus (pure policy/catalog
    // declarations, no #[dataset]-attached store drivers) never registers
    // — nothing to find either way.
    assert_eq!(by_pass("V5"), 0);
    // V8 *does* have something to check now that `stream_port!` is real:
    // the corpus declares 3 real ports (`txn_in`, `txn_out`, `decisions`)
    // but no `DRIVER_MANIFESTS` for any of them (driver installation is
    // separate infra work this policy/catalog corpus never does) — V8
    // correctly flags all 3 as under-manifested. This is V8 doing its job
    // now that there's real port data to check, not a regression.
    let v8 = findings.iter().filter(|f| f.pass == "V8").collect::<Vec<_>>();
    assert_eq!(v8.len(), 3, "V8 findings: {findings:?}");
    for port in ["txn_in", "txn_out", "decisions"] {
        assert!(
            v8.iter().any(|f| f.item.as_deref() == Some(port)),
            "expected a V8 finding for port `{port}`: {findings:?}"
        );
    }
}

/// Plan Phase 15: `approval_chain!` must register a real, structured
/// `ApprovalChainRecord` per chain — not just raw source text into
/// `CATALOG` — and `nirdosha_guard_core::approval_chain::ApprovalChainRuntime`
/// built from those real records must let `sar_release` actually escalate,
/// require two distinct `ComplianceLead` approvals, and deny on timeout,
/// end to end from the real `00_core.nir` corpus declarations (not a
/// hand-constructed fixture standing in for them).
#[test]
fn corpus_approval_chains_are_really_registered_and_sar_release_escalates_end_to_end() {
    let dump = nirdosha_guard_registry::dump();
    assert_eq!(dump.approval_chains.len(), 7, "all 7 real approval_chain! blocks in 00_core.nir must register a structured record: {:?}", dump.approval_chains);

    let sar_release = dump.approval_chains.iter().find(|chain| chain.name == "sar_release").expect("sar_release must be registered");
    assert_eq!(sar_release.quorum, 2);
    assert_eq!(sar_release.approvers, vec!["ComplianceLead".to_string()]);

    let policy_release = dump.approval_chains.iter().find(|chain| chain.name == "policy_release").expect("policy_release must be registered");
    assert_eq!(policy_release.approvers, vec!["PolicyEngineer".to_string(), "ComplianceLead".to_string()]);

    // End-to-end runtime proof against the real, corpus-derived definitions.
    let definitions: Vec<nirdosha_guard_core::approval_chain::ApprovalChainDefinition> = dump
        .approval_chains
        .iter()
        .map(|record| nirdosha_guard_core::approval_chain::ApprovalChainDefinition { name: record.name.clone(), quorum: record.quorum, approver_roles: record.approvers.clone(), cooling_period_ms: record.cooling_ms })
        .collect();
    let mut runtime = nirdosha_guard_core::approval_chain::ApprovalChainRuntime::new(definitions);

    runtime.open("sar-esc-1", "sar_release", "sar-bundle-42", "analyst-1", 0, 1_000).expect("a real registered chain must accept a real escalation");
    let first = runtime.approve("sar-esc-1", "lead-a", "ComplianceLead", 10).unwrap();
    assert_eq!(first, nirdosha_guard_core::approval_chain::EscalationStatus::Pending { approvals_so_far: 1, quorum: 2 });
    let second = runtime.approve("sar-esc-1", "lead-b", "ComplianceLead", 20).unwrap();
    assert_eq!(second, nirdosha_guard_core::approval_chain::EscalationStatus::Approved);

    // A second, independent escalation on the same real chain that never
    // reaches quorum must deny on timeout (I3: "all timeouts resolve to
    // DENY", the corpus's own comment above the approval_chain! blocks).
    runtime.open("sar-esc-2", "sar_release", "sar-bundle-43", "analyst-2", 0, 1_000).unwrap();
    runtime.approve("sar-esc-2", "lead-a", "ComplianceLead", 10).unwrap();
    let timed_out = runtime.check_timeout("sar-esc-2", 1_500).unwrap();
    assert_eq!(timed_out, nirdosha_guard_core::approval_chain::EscalationStatus::DeniedTimeout);
}
