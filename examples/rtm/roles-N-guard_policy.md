# The Policy Starter Set — `roles!` + `guard_policy!` + Macros Covering All 22 Modules

Below is the complete `crates/policy/rtm/` file set. Conventions:

- `// [RFC §x]` — derived from normative RFC text (verbatim or near-verbatim)
- `// [NEW]` — gap-fill the RFCs don't declare yet (flagged for review)
- `// → M#, #.#` — maps to the screen inventory module/screen
- Syntax follows RFC 0023 §1B canonical forms; where the exact grammar isn't pinned by an RFC (noted inline), the form is kept minimal and consistent with §7.4 of RFC 0025.
- Everything here compiles under plain `cargo` today (macros parse into registry records — RFC 0023 checklist: v2 smoke tests `[DONE]`); **lowering to executable plans is `[OPEN]`**, so these are the authoritative records the lowering must satisfy.

---

## `00_core.nir` — Roles, Purposes, Sampling, Classifications

```rust
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
nirdosha_rt::purpose! {
    enum Purpose {
        Operations,              // default operational purpose
        FraudMonitoring,         // "fraud_monitoring" — used by svc:* [RFC 0025 §8.1]
        AmlInvestigation,        // analyst work on alerts/cases          [NEW code]
        CustomerService,         // CsAgent lookups                       [NEW code]
        QaReview,                                                       [NEW]
        Audit,                                                          [NEW]
        RegulatoryInspection,                                           [NEW]
        Analytics,                                                      [NEW]
        ModelGovernance,                                                [NEW]
        PlatformOperations,                                             [NEW]
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
#[nirdosha_rt::invariant(name = "amount_positive", schema = "transaction")]
fn amount_positive(t: &Transaction) -> bool { t.amount > 0 }

#[nirdosha_rt::invariant(name = "currency_iso", schema = "transaction")]
fn currency_iso(t: &Transaction) -> bool { t.currency.is_iso_4217() }

#[nirdosha_rt::invariant(name = "rationale_present", schema = "alert")]       // → 3.3 [NEW]
fn rationale_present(a: &AlertUpdate) -> bool { !a.rationale.is_empty() }

#[nirdosha_rt::invariant(name = "sar_subject_in_case", schema = "sar_bundle")] // → 12.2 [NEW]
fn sar_subject_in_case(s: &SarBundle) -> bool { s.case().subjects().contains(s.subject()) }

#[nirdosha_rt::invariant(name = "no_legal_hold", schema = "customer")]        // → 18.9 [NEW]
fn no_legal_hold(c: &Customer) -> bool { !c.legal_hold() }
```

---

## `10_domains.nir` — Datasets, Classifications, Relations, References

```rust
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
#[nirdosha_rt::dataset(entity = "transaction", store = "pg_primary", kind = "table")]
pub struct Transaction {
    pub tenant_id:  TenantId,                       // INTERNAL (required, TenantEq-attested)
    pub subject_id: SubjectId,                      // CONFIDENTIAL
    pub account_id: AccountId,                      // INTERNAL                [NEW field]
    pub amount:     Money,                          // INTERNAL (Money mask = partial)
    pub currency:   CurrencyCode,                   // INTERNAL
    pub status:     TxnStatus,                      // INTERNAL
    pub occurred_at: Timestamp,                     // INTERNAL
    pub channel:    TxnChannel,                     // INTERNAL (allowed)
    pub merchant_id: MerchantId,                    // INTERNAL (allowed)
    pub card_token: CardToken,                      // RESTRICTED (allowed)
    pub device_id:  DeviceId,                       // CONFIDENTIAL (allowed)
    pub geo:        Geo,                            // CONFIDENTIAL (allowed)
}

// ---- txn_events — topic-as-dataset (features read this) [RFC §8.2] ----
#[nirdosha_rt::dataset(entity = "txn_events", store = "kafka_main", kind = "topic")]
pub struct TxnEvent { /* mirrors transaction schema */ }

// ---- alert — [RFC §8.5: required set; status is analyst-only] ----
#[nirdosha_rt::dataset(entity = "alert", store = "pg_primary", kind = "table")]
pub struct Alert {
    pub tenant_id: TenantId,
    pub txn_id:    TxnId,
    pub score:     f64,
    pub rule_hits: Vec<RuleHit>,
    pub model_version: String,
    pub policy_version: PolicyVersion,
    pub status:    AlertStatus,        // analyst-only (forbidden to svc:alerts)
    pub assignee:  Option<UserId>,     //                                      [NEW]
    pub disposition_code: Option<DispositionCode>,   //                       [NEW]
    pub rationale: Option<String>,     //                                     [NEW]
    pub case_id:   Option<CaseId>,     //                                     [NEW]
    pub sar_linked: bool,              // tipping-off flag — RESTRICTED-class   [NEW]
    pub sla_due_at: Timestamp,         //                                      [NEW]
}

// ---- case — [RFC §8.6: status machine below; alert_ids immutable] ----
#[nirdosha_rt::dataset(entity = "case", store = "pg_primary", kind = "table")]
pub struct Case {
    pub tenant_id: TenantId,
    pub status: CaseStatus,
    pub assigned_to: UserId,
    pub alert_ids: Vec<AlertId>,       // composition — immutable post-create
    pub subject_ref: CustomerRef,      //                                       [NEW]
    pub sar_id: Option<SarId>,         // existence restricted (tipping-off)    [NEW]
    pub disposition: Option<CaseDisposition>,   //                             [NEW]
    pub rationale: Option<String>,     //                                       [NEW]
}

// ---- customer — [NEW: full declaration; M5] ----
#[nirdosha_rt::dataset(entity = "customer", store = "pg_primary", kind = "table")]
pub struct Customer {
    pub tenant_id: TenantId,
    pub name: CustomerName,                 // CONFIDENTIAL
    pub national_id: NationalId,            // RESTRICTED
    pub dob: Date,                          // CONFIDENTIAL
    pub address: Address,                   // CONFIDENTIAL
    pub phone: Phone,                       // CONFIDENTIAL
    pub occupation: String,                 // INTERNAL
    pub kyc_status: KycStatus,              // INTERNAL
    pub risk_rating: RiskBand,              // CONFIDENTIAL
    pub pep_flag: bool,                     // CONFIDENTIAL
    pub sanctions_status: ScreeningStatus,  // CONFIDENTIAL
    pub expected_profile: ActivityProfile,  // CONFIDENTIAL  → feeds 5.5
    pub restriction_status: RestrictionBand,// INTERNAL — the ONLY field RM/CS may see → M21
    pub legal_hold: bool,                   // INTERNAL     → DSAR gate
}

// ---- sar_bundle — [NEW: M12; existence itself is tipping-off-sensitive] ----
#[nirdosha_rt::dataset(entity = "sar_bundle", store = "pg_primary", kind = "table")]
pub struct SarBundle {
    pub tenant_id: TenantId,
    pub case_id: CaseId,                    // required — invariant-linked
    pub subject: SubjectRef,
    pub status: SarStatus,
    pub narrative: String,                  // RESTRICTED-class content
    pub activity_codes: Vec<ActivityCode>,
    pub amount_total: Money,
    pub txn_refs: Vec<TxnId>,
    pub filed_at: Option<Timestamp>,
    pub goaml_ref: Option<String>,
    pub next_review_at: Option<Timestamp>,  // continuing-activity (→ 12.9)
}

// ---- screening_hit — [NEW: M10] ----
#[nirdosha_rt::dataset(entity = "screening_hit", store = "pg_primary", kind = "table")]
pub struct ScreeningHit {
    pub tenant_id: TenantId,
    pub subject_ref: SubjectRef,
    pub list_id: ListId,                    // ofac_sdn | un_consolidated | eu_fsf | internal
    pub match_score: f64,
    pub disposition: Option<HitDisposition>, // true_match | false_match | needs_edd
    pub rationale: Option<String>,
}

// ---- payment (Mode B holds) — [NEW: M11; Pending entities live here] ----
#[nirdosha_rt::dataset(entity = "payment", store = "pg_primary", kind = "table")]
pub struct Payment {
    pub tenant_id: TenantId,
    pub rail_ref: String,                   // external payment reference
    pub originator: SubjectRef,
    pub beneficiary: SubjectRef,
    pub amount: Money,
    pub currency: CurrencyCode,
    pub status: PaymentStatus,              // workflow machine below
    pub hold_reason: Option<HoldReason>,
    pub hold_expires_at: Option<Timestamp>, // deny-by-timeout clock (§6.3)
    pub decision_by: Option<UserId>,
    pub decision_rationale: Option<String>,
}

// ---- qa_review — [NEW: M14] ----
#[nirdosha_rt::dataset(entity = "qa_review", store = "pg_primary", kind = "table")]
pub struct QaReview {
    pub tenant_id: TenantId,
    pub item_ref: ItemRef,                  // alert | case
    pub rubric_scores: Vec<Score>,
    pub result: QaResult,
    pub error_severity: Option<Severity>,
    pub retraining_flag: bool,
    pub feedback: Option<String>,
}

// ---- notification — [NEW: M16; fed by notify(channel(...)) obligations] ----
#[nirdosha_rt::dataset(entity = "notification", store = "pg_primary", kind = "table")]
pub struct Notification { pub tenant_id: TenantId, pub channel: ChannelId,
                          pub user_id: UserId, pub body_ref: String,
                          pub read: bool, pub entity_ref: Option<String> }

// ---- user_profile — [NEW: M22.1 self-service] ----
#[nirdosha_rt::dataset(entity = "user_profile", store = "pg_primary", kind = "table")]
pub struct UserProfile { pub user_id: UserId, pub locale: Locale, pub tz: Tz,
                         pub notif_prefs: NotifPrefs }

// ---- support_ticket — [NEW: M22.2] ----
#[nirdosha_rt::dataset(entity = "support_ticket", store = "pg_primary", kind = "table")]
pub struct SupportTicket { pub tenant_id: TenantId, pub category: TicketCategory,
                           pub priority: Severity, pub description: String }

// ---- Reference datasets — [RFC 0023 §1C.2 pattern] ----
#[nirdosha_rt::dataset(entity = "refdata", store = "refdata", kind = "table")]
pub struct RefDataset { pub code: String, pub label: String, pub class: Classification }

#[nirdosha_rt::reference(customer.kyc_status,    store = "refdata", missing = FailClosed)]
#[nirdosha_rt::reference(alert.disposition_code, store = "refdata", missing = FailClosed)] // → 3.3, 18.5
#[nirdosha_rt::reference(customer.risk_rating,   store = "refdata", missing = FailClosed)]
#[nirdosha_rt::reference(transaction.currency,   store = "refdata", missing = FailClosed)] // ISO 4217
#[nirdosha_rt::reference(case.disposition,       store = "refdata", missing = FailClosed)]
#[nirdosha_rt::reference(country_risk,           store = "refdata", missing = FailClosed)] // → M13.2

// ---- Relations — [§4: positive-only; negation = compile error] ----
#[nirdosha_rt::relation(source = "core_kyc", cardinality = 200, ttl = 60s)]
fn accounts_of(customer: CustomerId) -> Set<AccountId>;           // → 5.2, 6.3

#[nirdosha_rt::relation(source = "core_kyc", cardinality = 100, ttl = 300s)]
fn related_parties(customer: CustomerId) -> Set<CustomerId>;      // → 5.6

#[nirdosha_rt::relation(source = "org_registry", cardinality = 1000, ttl = 300s)]
#[nirdosha_rt::materialize(relation = ubo_closure)]               // Tier 2 → 5.6
fn ubo_closure(customer: CustomerId) -> Set<CustomerId>;

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
nirdosha_rt::workflow! {
    machine CaseStatus {                                   // [RFC §8.6 verbatim]
        open -> investigating -> [confirmed_fraud, false_positive, escalate];
        confirmed_fraud -> sar_filed -> closed;
        false_positive -> closed;
    }
}
nirdosha_rt::workflow! {                                   // [NEW → 3.3]
    machine AlertStatus {
        new -> in_progress -> [closed_false_positive, escalated, pending_info];
        pending_info -> [in_progress, escalated];          // → 3.7 RFI
        closed_false_positive -> in_progress;              // reopen = TL, audited
    }
}
nirdosha_rt::workflow! {                                   // [NEW → M11]
    machine PaymentStatus {
        received -> held -> [released, blocked];           // transitions via I3 revalidate
        held -> held;                                      // no self-loop releases
    }
}
nirdosha_rt::workflow! {                                   // [NEW → M12]
    machine SarStatus {
        draft -> in_review -> [filed, rejected, do_not_file];
        rejected -> draft;                                 // correction loop → 12.7
        filed -> continuation_due -> [filed, ceased];      // → 12.9
    }
}
nirdosha_rt::workflow! {                                   // [NEW → M14]
    machine QaReviewStatus { assigned -> scored -> [closed, returned]; }
}

// ---- Enumerations (get_options screens) — §8.4 / §1C.2: both hops guarded ----
nirdosha_rt::enumerate! { options from alert.disposition_code filter active == true cap(row_cap = 50) } // → 3.3
nirdosha_rt::enumerate! { options from transaction.channel      filter active == true cap(row_cap = 50) } // → 6.1
nirdosha_rt::enumerate! { options from country_risk             filter active == true cap(row_cap = 250) } // → 13.2
```

---

## `20_ingestion.nir` — Ingest + Analyst Transaction Access (M6, parts of M19)

```rust
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
```

---

## `30_features.nir` — Feature Engine (M8 partial, M6.6)

```rust
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
```

---

## `40_scoring.nir` — Model, Screening, ML Governance (M8, M9, M10)

```rust
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
```

---

## `50_alerts.nir` — Alert Generation + Triage (M3)

```rust
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
    grant predicate_use(status, assignee, model_version, policy_version, txn_id)  // I15: 3.1's ?q= search
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
```

---

## `60_case_management.nir` — Cases (M4)

```rust
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
```

---

## `65_sar.nir` — SAR / Regulatory Reporting (M12)

```rust
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
```

---

## `70_mcp.nir` — Analyst Copilot (M3–M7 copilot variants)

```rust
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
```

---

## `75_lineage.nir` — Lineage Explorer, V10, Simulation (M7, M20, M8.6–8.7)

```rust
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

// → M7 network graph link queries (7.1/7.2/7.3) + graph export (7.5) [NEW]
//   `menus.toml` declares `action = "link_query", resource = "network_of"` and
//   `action = "export", resource = "link_edge"`; these are the policy records the
//   V5 route-guard invariant resolves to.
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

// → 8.6 simulation read-only view for ComplianceLead (menu V5 coverage) [NEW]
nirdosha_rt::guard_policy! {
    allow "lead-view-simulation" for ComplianceLead
    when action == "simulate" && resource == "policy_simulation"
    purpose(ModelGovernance)
    obligate audit(full)
}

// → 8.3 threshold edits = migrate (dry-run mandatory at runtime, §2) [NEW]
nirdosha_rt::guard_policy! {
    allow "threshold-migrate" for PolicyEngineer
    when action == "migrate" && resource in ["window", "policy", "matcher", "model"]
    purpose(ModelGovernance)
    escalate to approval(chain policy_release)
    obligate audit(full)
}

// → 8.3/8.10 policy/window migrate for ComplianceLead (menu V5 coverage) [NEW]
nirdosha_rt::guard_policy! {
    allow "lead-migrate-policy" for ComplianceLead
    when action == "migrate" && resource in ["policy", "window"]
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
```

---

## `80_consumers.nir` — BI, Dashboards, Export Center (M2, M15)

```rust
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
```

---

## `85_intervention.nir` — Real-Time Holds (M11)

```rust
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
```

---

## `90_ops_admin.nir` — Catalog, Users, Reference Data, Retention, DSAR (M13, M18, M19, M22)

```rust
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

// → M18.1 user-management read [NEW — screens-plan A2; mirrors src/90_ops_admin.nir]
//   The read beside the grant write path; backs menus.toml nav.users'
//   `read user_role` guard (18.1's screen roles are Admin only).
nirdosha_rt::guard_policy! {
    allow "user-role-read" for Admin
    when action == "read" && resource == "user_role"
    purpose(PlatformOperations)
    cap(row_cap = 500)
    obligate audit(sampled)
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

// → M22.1 self-profile read [NEW]
//   `menus.toml` chrome `profile` and route `/preferences/profile`
//   declare `read`/`user_profile` with `filter = "subject_scope"`.
nirdosha_rt::guard_policy! {
    allow "self-read-profile" for Analyst, ComplianceLead, Mlro, QaReviewer, OpsAnalyst, PolicyEngineer, Admin, Auditor, CsAgent, RmUser
    when action == "read" && resource == "user_profile"
    filter subject_scope()
    cap(row_cap = 1)
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

// → M16 notification feed [NEW]
//   `menus.toml` declares `read`/`notification` for the bell chrome and
//   `/notifications` route; this is the policy record V5 resolves to.
nirdosha_rt::guard_policy! {
    allow "notification-read" for Analyst, ComplianceLead, Mlro, QaReviewer, OpsAnalyst, PolicyEngineer, Admin, Auditor, CsAgent, RmUser
    when action == "read" && resource == "notification"
    purpose(Operations)
    filter tenant_scope()
    cap(row_cap = 200)
    obligate audit(sampled)
}

// → M17 static knowledge [NEW]
//   `menus.toml` declares `read`/`static_docs` for 17.1-17.3 and the
//   About route (`/about`, 22.3).
nirdosha_rt::guard_policy! {
    allow "static-docs-read" for Analyst, ComplianceLead, Mlro, QaReviewer, OpsAnalyst, PolicyEngineer, Admin, Auditor, CsAgent, RmUser
    when action == "read" && resource == "static_docs"
    purpose(Operations)
    cap(row_cap = 100)
    obligate audit(sampled)
}

// → M2 saved views [NEW]
//   `menus.toml` chrome `saved_views` + route `/preferences/saved-views`
//   declare `read`/`saved_view`.
nirdosha_rt::guard_policy! {
    allow "saved-view-read" for Analyst, ComplianceLead, Mlro, QaReviewer, OpsAnalyst, PolicyEngineer, Admin, Auditor, CsAgent, RmUser
    when action == "read" && resource == "saved_view"
    purpose(Operations)
    cap(row_cap = 50)
    obligate audit(sampled)
}

// → M16 shift handover [NEW]
//   `menus.toml` `/handover` declares `read`/`handover` for Analyst,
//   ComplianceLead and OpsAnalyst.
nirdosha_rt::guard_policy! {
    allow "handover-read" for Analyst, ComplianceLead, OpsAnalyst
    when action == "read" && resource == "handover"
    purpose(Operations)
    cap(row_cap = 100)
    obligate audit(sampled)
}

// → M20 audit log [NEW]
//   `menus.toml` 20.1/20.2/20.3 and sub-route `/alerts/{id}/audit`
//   declare `read`/`audit_record` for Auditor, Admin and Mlro.
nirdosha_rt::guard_policy! {
    allow "audit-record-read" for Auditor, Admin, Mlro
    when action == "read" && resource == "audit_record"
    purpose(Audit)
    filter tenant_scope()
    cap(row_cap = 1_000, max_scan_rows = 500_000, max_execution = 60s)
    obligate audit(full)
}
```

---

## `95_qa.nir` — Quality Assurance (M14) `[NEW — no RFC machinery exists]`

```rust
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

// → 14.5 Overturned Decisions Log (T-11): ComplianceLead/Mlro read the
// full qa_review set (not subject-scoped like `self-read-qa`, not
// closed-only like `qa-read-closed`) to see every `returned` (overturned)
// review, cross-referenced against the `audit_chain` alert history.
nirdosha_rt::guard_policy! {
    allow "compliance-lead-read-qa-review" for ComplianceLead
    when action == "read" && resource == "qa_review"
    purpose(QaReview)
    filter tenant_scope()
    cap(row_cap = 500, max_scan_rows = 50_000)
    obligate audit(full)
}
nirdosha_rt::guard_policy! {
    allow "mlro-read-qa-review" for Mlro
    when action == "read" && resource == "qa_review"
    purpose(QaReview)
    filter tenant_scope()
    cap(row_cap = 500, max_scan_rows = 50_000)
    obligate audit(full)
}
```

---

## `96_restricted_views.nir` — CS / RM / Auditor / Regulator (M21)

```rust
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
// T-11 (B8): "audit_chain"/"sar_visibility" joined this list — the
// AC.all_chains projection and PG.sar_visibility tables `bridge.nir`
// builds for 20.1/20.2/12.11/14.5, same broad-Audit-purpose read posture
// as every other resource here.
nirdosha_rt::guard_policy! {
    allow "auditor-read" for Auditor
    when action == "read" && resource in ["alert", "case", "transaction",
                                          "customer", "sar_bundle", "payment",
                                          "screening_hit", "qa_review",
                                          "audit_chain", "sar_visibility"]
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

// → 20.1/20.2/14.5 (T-11): Mlro/ComplianceLead/Admin also read the
// unified chain projection — same broad Audit-purpose grant `auditor-read`
// gives Auditor, for the other roles those three screens name.
nirdosha_rt::guard_policy! {
    allow "mlro-audit-chain-read" for Mlro
    when action == "read" && resource in ["audit_chain", "sar_visibility"]
    purpose(Audit)
    filter tenant_scope()
    cap(row_cap = 1_000, max_scan_rows = 500_000, max_execution = 60s)
    obligate audit(full)
}
nirdosha_rt::guard_policy! {
    allow "compliance-lead-audit-chain-read" for ComplianceLead
    when action == "read" && resource == "audit_chain"
    purpose(Audit)
    filter tenant_scope()
    cap(row_cap = 1_000, max_scan_rows = 500_000, max_execution = 60s)
    obligate audit(full)
}
nirdosha_rt::guard_policy! {
    allow "admin-audit-chain-read" for Admin
    when action == "read" && resource == "audit_chain"
    purpose(Audit)
    filter tenant_scope()
    cap(row_cap = 1_000, max_scan_rows = 500_000, max_execution = 60s)
    obligate audit(full)
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
```

---

## `99_break_glass.nir` — Emergency Access (M18 break-glass screens; RFC 0023 §14)

```rust
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
```

---

## Coverage Matrix — Module → File → Policies → Screens

| Module | File | Key policies | Screens covered |
|---|---|---|---|
| M1 Auth | 00/96 | `roles!`, deny-by-default, GuardError surfacing | 1.1–1.6 |
| M2 Dashboards | 80/90 | `lead-dashboard`, `decisions` port, `saved-view-read` | 2.1–2.7 |
| M3 Alerts | 50 | `alert-raise`, read/disposition/bulk/SLA | 3.1–3.10 |
| M4 Cases | 60 | create/read/transition/close-confirm | 4.1–4.14 |
| M5 Customer | 10/90 | `customer` dataset, relations, DSAR | 5.1–5.10 |
| M6 Transactions | 20 | search/open/flag, ledger via filter | 6.1–6.8 |
| M7 Network/Lineage | 75 | `lineage_query!`, `network-link-query`, `network-export` | 7.1–7.5 |
| M8 Rules | 75/40 | `threshold-migrate`, `lead-migrate-policy`, `policy_simulation!`, `lead-view-simulation` | 8.1–8.12 |
| M9 ML | 40 | `scoring-model-swap`, `model_release` | 9.1–9.6 |
| M10 Screening | 40 | matcher, hits, list-migrate | 10.1–10.6 |
| M11 Intervention | 85 | holds, decide, override, auto-release deny | 11.1–11.5 |
| M12 SAR | 65 | draft/edit/decide/export/submissions | 12.1–12.11 |
| M13 Risk Config | 90 | `refdata-migrate`, contracts | 13.1–13.5 |
| M14 QA | 95 | qa read/write + SoD pair | 14.1–14.5 |
| M15 Reporting | 80 | `bi-aggregate`, `governed-export` | 15.1–15.5 |
| M16 Notifications | 10/50/60/90 | `notify(channel(...))` obligations, `notification-read`, `handover-read` | 16.1–16.5 |
| M17 Knowledge | 90 | signed build-time artifacts (`.nirpkg`), `static-docs-read` | 17.1–17.4 |
| M18 Admin | 90 | role-grant, delegation, purge, DSAR | 18.1–18.10 |
| M19 IT Ops | 20/90 | `ops-ingest-admin`, `driver-migrate` | 19.1–19.8 |
| M20 Audit | 75/96/90 | `lineage-audit`, `auditor-read/export`, `audit-record-read` | 20.1–20.3 |
| M21 Restricted | 96/65 | cs/rm/auditor/regulator blocks | 21.1–21.4 |
| M22 Misc | 90 | `self-update-profile`, `self-read-profile`, `any-create-ticket` | 22.1–22.3 |

---

## What Polices This File Set (verify + runtime)

| Gate | What it catches here |
|---|---|
| **V1** inventory | every mutating path above has a policy record |
| **V2** coherence | `field_policy` fields ⊆ dataset schemas (e.g., typo `merchnt_id` = compile error) |
| **V3** coverage | every (subject, action, resource) triple above classified |
| **V4** relations | `ubo_closure` etc. positive-only; negation would fail compile |
| **V5** capability | `features-read` volume vs driver manifests; Mode B `event_time` for PaymentStatus |
| **V6** MCP sync | `mcp_tools!` tool list ↔ catalog entities |
| **V7** SoD | the explicit deny records (`ingest-no-readback`, `scoring-no-side-effects`, `alerts-no-edit`, `qa-no-disposition`, `cs-no-sar`, `rm-no-sar`, `copilot-no-sar`, `auditor-no-write`, `regulator-no-export`, `auto-release-on-timeout`) |
| **V8** two-driver | sanctions `ListProvider`, stream, store ports referenced above |
| **I-invariants** | I1 (`audit(full)` before commit everywhere sensitive), I3 (hold release), I9 (`cohort_floor` on all aggregates), I10 (`enumerate!` caps, opaque cursors runtime-enforced), I11 (delegation mint audited), I15 (`predicate_use` grants), I17 (copilot defaults) |

---

## Phase Gates & Open Items

| Block group | Compiles/runs from |
|---|---|
| 00/10/50/60/65/80 core policies | Today as records; executable at **RFC 0025 Phase 2–3** (plans land) |
| 85 intervention, workflow machines | Phase 3 (Mode B holds, I3) |
| 40 model/list drivers, 70 MCP | Phase 4–5 |
| 75 lineage (RFC 0026) | `lineage_query!` Phase 1; GraphStore/V10 Phase 2–3; `data_contract!`/`policy_simulation!` Phase 4 |
| 95/96/99 | Policy work only — runnable as soon as their underlying entities have drivers (Phase 2) |

**Decisions this file set forces (review before merging):** (1) `Mlro` as separate role vs. 2nd ComplianceLead; (2) SAR export `requires` state — `sar_filed` (machine) vs `confirmed_fraud` (RFC verbatim) — flagged inline in `65_sar.nir`; (3) the 100k override threshold in `85_intervention.nir`; (4) sampling rates for INTERNAL (0.10) — defense-in-depth above the RFC floor; (5) QA soD scope.
