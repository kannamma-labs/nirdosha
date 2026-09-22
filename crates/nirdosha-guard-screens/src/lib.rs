//! Bridges `nirdosha-guard-mic`'s `GuardClient`/`StoreDriver` plane into
//! `nirdosha-rt`'s screen-archetype macros (`crud_screens!` and friends)
//! -- see the RTM live-demo plan's "Confirmed evaluator gaps" section for
//! the five real holes (G1-G5) this crate exists to close.
//!
//! **Why `GuardedTable` doesn't just call `GuardClient::guarded_apply`/
//! `guarded_read`.** Both hardcode `PlanIr`/`ReadPlanIr.resource =
//! request.context.entity.clone()` -- the same field the evaluator
//! matches a policy's `resource ==` clause against
//! (`evaluator::matches_context`'s `context.entity == policy.resource`,
//! plain string equality). A corpus policy's `resource` is always the
//! bare entity *type* (`"transaction"`), so `context.entity` has to stay
//! `"transaction"` for evaluation to match anything at all -- but
//! `MemStoreDriver`/`PostgresStoreDriver` both key storage by that exact
//! same string (`resource TEXT PRIMARY KEY`), so every row of a given
//! type would collide on one storage key (G4). `GuardedTable` resolves
//! this by evaluating with `entity = "<type>"` (matches real policies)
//! but building its own `PlanIr`/`ReadPlanIr` with `resource =
//! "<type>:<row-id>"` (real per-row storage identity) -- which means it
//! calls `GuardClient::evaluate()` directly and drives `StoreDriver`
//! itself, rather than delegating to `guarded_apply`/`guarded_read`.
//! **Disclosed cost**: this reimplementation does not get `guarded_read`'s
//! I12/I15 checks, relation resolution, or lineage-observation emission
//! for free, and neither method's write-side idempotency/audit-envelope
//! shape is replicated yet -- `GuardClient::audit`/`idempotency` are
//! public fields specifically so a caller in this position can still
//! append the identical envelope shape by hand; doing so is a named
//! Phase B item, not attempted here.

pub mod entity;
pub mod error;
pub mod field_policy;
pub mod masking;
pub mod scope;

pub use entity::GuardedEntity;
pub use error::{guard_error_response, GuardScreenError};

use nirdosha_audit::envelope::{AuditEnvelope, AuditRecordKind};
use nirdosha_guard_core::approval_chain::{ApprovalChainDefinition, ApprovalChainError, ApprovalChainRuntime, EscalationOutcome, EscalationStatus};
use nirdosha_guard_core::evaluator::{EvaluationResult, PolicyCandidate};
use nirdosha_guard_core::{
    Action, Cap, Classification, Decision, Destination, EscalateTarget, Environment, EvaluationContext, FilterExpr,
    Obligation, PaginationMode, Purpose, QueryShape, QueryVerb, Subject, Tenant, Value, WriteAction,
};
use nirdosha_guard_mic::{EntityBytes, EvalRequest, GuardClient, PlanIr, ReadPlanIr, StoreDriver};
use nirdosha_guard_registry::{Effect, PolicyRecord};
use nirdosha_rt::Auth;
use std::collections::HashSet;
use std::marker::PhantomData;
use std::path::Path;
use std::sync::Mutex;

/// A still-open escalation `guarded_propose_escalated_update`/
/// `guarded_confirm_escalated_update` returned -- enough for the screen
/// layer to render "1 of 2 approvals; escalation `xyz` on chain
/// `case_review`" and to carry the three fields a later confirm call
/// needs (`chain`, `escalation_id`, and the row's own id, already known
/// to the caller).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingApproval {
    pub escalation_id: String,
    pub chain: String,
    pub quorum: u8,
    pub approvals_so_far: u8,
    /// `Some(ready_at_ms)` once quorum is real but the chain's
    /// `cooling(...)` window hasn't elapsed yet — `approvals_so_far ==
    /// quorum` in this state, but the write has NOT committed (T-04:
    /// "cooling period on approve"). `None` for an ordinary still-short
    /// pending escalation.
    pub cooling_ready_at: Option<u64>,
}

/// One row of a cross-entity approval inbox: everything
/// `approval_inbox!` needs to render an escalation without the caller
/// re-deriving quorum/status math itself. Built from
/// `ApprovalChainRuntime::list_pending`'s real per-table state -- not a
/// separate, weaker tracking structure.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PendingApprovalRow {
    pub escalation_id: String,
    pub chain: String,
    pub resource: String,
    pub proposer: String,
    pub opened_at: u64,
    pub deadline: u64,
    pub quorum: u8,
    pub approvals_so_far: u8,
    pub approver_ids: Vec<String>,
    /// One of: "pending", "cooling", "approved", "denied_timeout", "returned".
    pub status: String,
    pub cooling_ready_at: Option<u64>,
    pub return_reason: Option<String>,
}

/// The outcome of a two-step escalated write: either quorum was already
/// met (possibly on the very first call, for a `quorum(1, ...)` chain)
/// and the row committed for real, or it's still short and waiting on
/// more distinct, role-eligible approvers.
#[derive(Debug, Clone, PartialEq)]
pub enum EscalatedWrite<E> {
    Committed(E),
    Pending(PendingApproval),
}

pub struct GuardedTable<D: StoreDriver, E: GuardedEntity> {
    driver: D,
    client: Mutex<GuardClient>,
    /// A second, independent quorum runtime for this table's own
    /// escalating writes (e.g. `case-close-confirm`'s `escalate to
    /// approval(chain case_review)`) -- seeded from the same corpus
    /// `approval_chain!` definitions `GuardClient::with_approval_chains`
    /// would use, but tracked separately rather than reaching into
    /// `GuardClient`'s own private instance: `GuardClient::guarded_apply`'s
    /// escalation-open/commit path assumes `PlanIr.resource ==
    /// request.context.entity` (the bare type), the same G4 collision
    /// this crate's whole `<type>:<row-id>` storage-key convention exists
    /// to avoid (see this module's own doc comment) -- so a per-row
    /// escalation needs its own bookkeeping, not `GuardClient`'s.
    /// `ApprovalChainRuntime` itself is a real, independently-tested
    /// quorum tracker (`nirdosha-guard-core::approval_chain`); this isn't
    /// a second, weaker implementation of the same idea.
    approvals: Mutex<ApprovalChainRuntime>,
    /// The full corpus dump ("Gate 2" `PolicyRecord`s), kept alongside
    /// `client`'s evaluator-ready `PolicyCandidate`s -- `evaluate()`
    /// gives the real Allow/Deny/Escalate decision plus masks/caps/
    /// residual_filter, but G1 (field_policy)/G3 (filter_ref) need the
    /// record-level data `to_candidate()` never carries. See this
    /// crate's own module doc comment.
    records: Vec<PolicyRecord>,
    /// Wire role names this table checks `auth.has_role(...)` against to
    /// build `Subject.roles` -- `nirdosha_rt::Auth` has no public
    /// "every role this session holds" getter (`has_role`/`prove::<R>()`
    /// only), so the caller supplies the relevant vocabulary once at
    /// construction instead of this crate guessing at RTM's role names.
    role_universe: Vec<String>,
    tenant: String,
    dataset: String,
    /// Real `obligate notify(channel(...))` consumption (batch 3's real
    /// gap: `EvaluationResult`/`PolicyRecord` both already carry real
    /// `Obligation`s -- `to_candidate()` forwards them unlike
    /// `field_policy` (G1's whole point), so `evaluate()`'s own
    /// deny-overrides union already computes the right set; nothing
    /// downstream of that ever *read* it). Called with `(channel,
    /// "<RESOURCE>:<row_id>")` after a write this table's own evaluation
    /// found a matching `Obligation::Notify` for -- the one legitimate
    /// caller is `bridge.nir`'s `notification_table()` wiring, via
    /// `GuardedTable::system_write` (see that method's own doc comment
    /// for why this is a non-guarded write that isn't a policy-evasion
    /// hole). `None` (the default -- most tables never produce a
    /// `Notify` obligation) costs nothing.
    notify_hook: Option<Box<dyn Fn(&str, &str) + Send + Sync>>,
    _marker: PhantomData<E>,
}

impl<D: StoreDriver, E: GuardedEntity> GuardedTable<D, E> {
    pub fn new(
        driver: D,
        candidates: Vec<PolicyCandidate>,
        records: Vec<PolicyRecord>,
        module: impl Into<String>,
        audit_path: impl AsRef<Path>,
        role_universe: Vec<String>,
        tenant: impl Into<String>,
        approval_chains: Vec<ApprovalChainDefinition>,
    ) -> Self {
        Self {
            driver,
            client: Mutex::new(GuardClient::new(candidates, module, audit_path)),
            approvals: Mutex::new(ApprovalChainRuntime::new(approval_chains)),
            records,
            role_universe,
            tenant: tenant.into(),
            dataset: "guarded".into(),
            notify_hook: None,
            _marker: PhantomData,
        }
    }

    /// Additive builder: registers a callback invoked `(channel,
    /// "<RESOURCE>:<row_id>")` once per matched `Obligation::Notify`,
    /// after a write this table evaluated to `Allow`/`Approved` actually
    /// commits. Every existing `GuardedTable::new` call site is
    /// unaffected (defaults to no hook).
    pub fn with_notify_hook(mut self, hook: impl Fn(&str, &str) + Send + Sync + 'static) -> Self {
        self.notify_hook = Some(Box::new(hook));
        self
    }

    /// Real obligation consumption -- called once per `allow_records`
    /// after a successful commit (create/update/migrate/escalated-
    /// approve, everywhere `commit_action`/`guarded_insert_checked`
    /// reach a real `Allow`). Unions every matching Allow record's own
    /// `obligations` (mirrors `evaluator::evaluate`'s own union-across-
    /// matches behavior, at the `PolicyRecord` level since these callers
    /// already have `allow_records` in hand rather than a fresh
    /// `EvaluationResult`).
    fn dispatch_obligations(&self, allow_records: &[&PolicyRecord], row_id: &str) {
        let Some(hook) = &self.notify_hook else { return };
        let body_ref = format!("{}:{row_id}", E::RESOURCE);
        for record in allow_records {
            for obligation in &record.obligations {
                if let Obligation::Notify { channel } = obligation {
                    hook(channel, &body_ref);
                }
            }
        }
    }

    fn subject_roles(&self, auth: &Auth) -> Vec<String> {
        self.role_universe.iter().filter(|r| auth.has_role(r)).cloned().collect()
    }

    fn context(&self, auth: &Auth, action: Action, purpose: &str) -> EvaluationContext {
        EvaluationContext {
            subject: Subject { id: auth.user().to_string(), roles: self.subject_roles(auth), claims: vec![], clearance: Classification::Restricted },
            tenant: Tenant(self.tenant.clone()),
            entity: E::RESOURCE.to_string(),
            dataset: self.dataset.clone(),
            action,
            destination: Destination::Browser,
            environment: Environment { env: "demo".into(), ip: None, geo: None, device_posture: None, session_freshness: None },
            time_bucket: "demo".into(),
            query_shape: QueryShape { verbs: vec![QueryVerb::Select], aggregate: None, grouping_keys: vec![], subject_dimension: None, ordering: vec![], pagination: PaginationMode::LimitOnly { limit: 500 } },
            purpose: Purpose(purpose.to_string()),
            policy_version: "rtm-demo".into(),
        }
    }

    /// T-11: real audit-chain enforcement, closing this module's own
    /// doc-comment-disclosed gap ("neither method's write-side
    /// idempotency/audit-envelope shape is replicated yet ... a named
    /// Phase B item, not attempted here"). Every `evaluate()` call site
    /// in this file calls this immediately after, so `self.client`'s
    /// `GuardClient::audit` (`ModuleAuditChain`, the exact hash-chained
    /// log `GuardClient::guarded_read`/`guarded_apply` already write to,
    /// for a caller that goes through THAT api instead of this crate's
    /// own `evaluate()`-direct path) gets a real entry for every
    /// decision this crate makes too -- allow, deny, escalate, and
    /// pending alike, matching `GuardClient`'s own "audit the decision
    /// regardless of outcome" posture. This is what makes `AC.<domain>_chain`
    /// (T-11's `audit_projection`) a real per-table log instead of an
    /// always-empty one.
    fn record_audit_decision(&self, request: &EvalRequest, evaluation: &EvaluationResult) {
        let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
        let trace_id = format!("{}-{}-{now_ms}", E::RESOURCE, request.context.subject.id);
        let client = self.client.lock().expect("guard client lock poisoned");
        // `ModuleAuditChain::append` asserts `envelope.module ==
        // self.module` -- `client.audit.module` is whatever `module`
        // string `GuardedTable::new`'s constructor arg actually was
        // (RTM passes the literal `"rtm-demo"` at every call site, not a
        // per-domain name), so the envelope must echo that back, not
        // `E::RESOURCE` -- the per-domain distinction lives in WHICH
        // FILE this chain is (`audit_path`), not in the envelope content.
        let envelope = AuditEnvelope {
            trace_id,
            ts: now_ms.to_string(),
            module: client.audit.module.clone(),
            subject: request.context.subject.id.clone(),
            action: format!("{:?}", request.context.action),
            resource: request.context.entity.clone(),
            policy_versions: vec![request.context.policy_version.clone()],
            decision: format!("{:?}", evaluation.decision),
            obligations: evaluation.obligations.iter().map(|o| format!("{o:?}")).collect(),
            kind: AuditRecordKind::Decision,
            content: serde_json::json!({}),
        };
        client.audit.append(&envelope, now_ms);
    }

    /// `PolicyRecord`s (not candidates) whose `subjects`/`action`/
    /// `resource`/`purpose` match this request the same way
    /// `evaluator::matches_context` does, restricted to `effect ==
    /// Allow` (by the time a caller reaches here, `evaluate()` already
    /// resolved a final Allow decision -- deny-overrides already ran --
    /// so only the winning Allow record(s)' own field_policy/filter_ref
    /// are relevant to check further).
    fn matching_allow_records<'a>(&'a self, roles: &[String], action: &str, purpose: &str) -> Vec<&'a PolicyRecord> {
        self.records
            .iter()
            .filter(|r| {
                matches!(r.effect, Effect::Allow)
                    && (r.subjects.is_empty() || r.subjects.iter().any(|s| roles.iter().any(|role| role == s)))
                    && r.action == action
                    && r.resource == E::RESOURCE
                    && r.purpose.as_deref().is_none_or(|p| p == purpose)
            })
            .collect()
    }

    /// **Real, corpus-level cap ambiguity, not a bug in this crate**:
    /// `20_ingestion.nir`'s `analyst-search-transaction` (`row_cap =
    /// 500`, intended for the list screen) and `analyst-open-transaction`
    /// (`row_cap = 1`, intended for the detail screen) both match the
    /// *identical* `(subject=Analyst, action=read, resource=transaction,
    /// purpose=AmlInvestigation)` tuple -- the guard DSL has no way to
    /// say "this policy is for a list query, that one is for a single-id
    /// lookup," so `evaluate()` (`nirdosha-guard-core`) unions both
    /// matching policies' `Cap`s, and which `RowCap` a naive
    /// `find_map`-first-match would apply depends on registration order,
    /// not intent. `widest_caps` resolves each cap *kind* to its
    /// maximum value among matches for a scan like this one -- the
    /// direction that avoids one policy's narrower grant silently
    /// truncating another's broader one, since this fn's whole job is
    /// "list every row the caller is allowed to see." `guarded_get`
    /// (below) sidesteps the ambiguity entirely for a single-id lookup,
    /// which never legitimately needs the widest grant in the first
    /// place.
    pub fn guarded_snapshot(&self, auth: &Auth, purpose: &str) -> Result<Vec<E>, GuardScreenError> {
        self.scan(auth, Action::Read, purpose)
    }

    /// Same shape as `guarded_snapshot`, evaluated as `Action::Aggregate`
    /// instead of `Action::Read` -- real corpus policies like
    /// `80_consumers.nir`'s `lead-dashboard` (`ComplianceLead`) grant
    /// `action == "aggregate"` on `alert`/`case`/`sar_bundle_stats` and
    /// grant no plain `read` at all, so a dashboard widget built on
    /// `guarded_snapshot` would (correctly) always deny that role. The
    /// caller reduces the returned rows to whatever metric/chart shape it
    /// needs -- this crate has no separate server-side aggregation
    /// engine, matching `GuardedTable`'s existing "decode to typed rows,
    /// let the caller finish the job" posture everywhere else.
    pub fn guarded_aggregate(&self, auth: &Auth, purpose: &str) -> Result<Vec<E>, GuardScreenError> {
        self.scan(auth, Action::Aggregate, purpose)
    }

    /// T-12 (I15): a `?q=` search, real rather than decorative. `q` is
    /// matched case-insensitively, ORed only across whichever of
    /// `candidate_fields` (the screen's own displayed field list) the
    /// winning Allow record(s) actually granted via `grant
    /// predicate_use(...)` -- never a field the policy didn't name, so
    /// a masked/forbidden field can never be searched into existence.
    /// Applied in-process, after decode -- the same "coarse driver
    /// pushdown + fine in-process filter" posture
    /// [`Self::apply_subject_scope_in_process`] already establishes for
    /// `subject_scope()`: `read_scope_clauses`'s own doc comment names
    /// the real reason (`MemStoreDriver`/`PostgresStoreDriver`'s flat
    /// `resource`/`tenant`/`payload` schema can't push an arbitrary
    /// payload-field predicate to the driver yet -- a separate,
    /// pre-existing gap this ticket doesn't newly introduce or silently
    /// paper over). `q = None`/empty short-circuits to the plain
    /// `guarded_snapshot` result. A non-empty `q` naming zero eligible
    /// fields (none of `candidate_fields` are granted) is a named deny,
    /// not a silent empty result -- I15's "reject filters on
    /// non-predicate_use fields" half.
    pub fn guarded_search(&self, auth: &Auth, purpose: &str, q: Option<&str>, candidate_fields: &[&str]) -> Result<Vec<E>, GuardScreenError> {
        let request = EvalRequest { context: self.context(auth, Action::Read, purpose) };
        let evaluation = { self.client.lock().expect("guard client lock poisoned").evaluate(&request) };
        self.record_audit_decision(&request, &evaluation);
        match evaluation.decision {
            Decision::Deny { reason } => Err(GuardScreenError::Denied(reason)),
            Decision::Escalate { to } => Err(GuardScreenError::Escalated(format!("{to:?}"))),
            Decision::Pending { .. } => Err(GuardScreenError::Store("read cannot be pending here".into())),
            Decision::Allow => {
                let rows = self.scan_allowed(&request, purpose, &evaluation)?;
                let Some(needle) = q.map(str::trim).filter(|s| !s.is_empty()) else { return Ok(rows) };
                let eligible: Vec<&str> = candidate_fields
                    .iter()
                    .copied()
                    .filter(|f| evaluation.predicate_use.iter().any(|p| p == f))
                    .collect();
                if eligible.is_empty() {
                    return Err(GuardScreenError::Denied(format!(
                        "search not permitted: none of this screen's fields are granted for predicate use (I15) under purpose `{purpose}`"
                    )));
                }
                let needle_lower = needle.to_lowercase();
                Ok(rows
                    .into_iter()
                    .filter(|row| {
                        let Ok(json) = serde_json::to_value(row) else { return false };
                        eligible.iter().any(|f| {
                            json.get(f)
                                .map(|v| nirdosha_rt::screens::value_display(v).to_lowercase().contains(&needle_lower))
                                .unwrap_or(false)
                        })
                    })
                    .collect())
            }
        }
    }

    fn scan(&self, auth: &Auth, action: Action, purpose: &str) -> Result<Vec<E>, GuardScreenError> {
        let request = EvalRequest { context: self.context(auth, action, purpose) };
        let evaluation = { self.client.lock().expect("guard client lock poisoned").evaluate(&request) };
        self.record_audit_decision(&request, &evaluation);
        match evaluation.decision {
            Decision::Deny { reason } => Err(GuardScreenError::Denied(reason)),
            Decision::Escalate { to } => Err(GuardScreenError::Escalated(format!("{to:?}"))),
            Decision::Pending { .. } => Err(GuardScreenError::Store("read cannot be pending here".into())),
            Decision::Allow => self.scan_allowed(&request, purpose, &evaluation),
        }
    }

    /// The "already decided `Allow`" half of a scan -- factored out of
    /// [`Self::scan`] so [`Self::guarded_propose_escalated_export`]/
    /// [`Self::guarded_confirm_escalated_export`] can reuse the identical
    /// query-building/decode path once THEIR OWN escalation quorum (not
    /// `scan`'s plain evaluate) reaches `Allow`/`Approved` -- one real
    /// read-plan implementation, not a second copy for the escalated
    /// case.
    fn scan_allowed(&self, request: &EvalRequest, purpose: &str, evaluation: &EvaluationResult) -> Result<Vec<E>, GuardScreenError> {
        let action_str = action_wire_str(request.context.action.clone());
        let clauses = self.read_scope_clauses(request, action_str, purpose, &evaluation.residual_filter)?;
        let plan = ReadPlanIr {
            resource: E::RESOURCE.to_string(),
            dataset: self.dataset.clone(),
            filter: Some(FilterExpr::And(clauses)),
            caps: widest_caps(&evaluation.caps),
            pagination: request.context.query_shape.pagination.clone(),
            policy_version: request.context.policy_version.clone(),
        };
        let query_result = self.driver.query(&plan).map_err(|e| GuardScreenError::Store(format!("{e:?}")))?;
        let masks = self.read_masks(request, action_str, purpose, &evaluation.masks);
        let rows = self.decode_rows(query_result.rows, &masks)?;
        let rows = self.apply_subject_scope_in_process(request, action_str, purpose, rows);
        Ok(self.apply_condition_filter_in_process(request, action_str, purpose, rows))
    }

    /// Real, batch-B-final-round fix: a matching Allow record's
    /// `Condition::Expr` clauses (`requires field(x) == "y"`, the
    /// `clauses.rs::lower_condition` shape [`field_policy::check_conditions`]
    /// already checks on the write side) were never consulted on a SCAN
    /// at all -- `sar-export`'s real `requires field(status) ==
    /// "confirmed_fraud"` is the corpus's first export policy with such a
    /// condition, and without this, an export scan would return every
    /// `SarBundle` row regardless of status, silently ignoring a
    /// compliance-critical gate (a SAR can only leave the building once
    /// truly confirmed). Treated the same way a `filter` clause narrows a
    /// scan, not as a pass/fail on the whole request -- rows failing the
    /// condition are excluded from the result rather than denying the
    /// entire export, matching how a real export screen would behave
    /// (export the eligible rows, not zero-or-all). `Condition::Custom`
    /// (invariant/transition-allowed) conditions are NOT applied here --
    /// those need a typed before/after pair `check_custom_conditions`
    /// requires, which has no meaning for a read-only scan; a dataset
    /// that ever needs a custom condition to gate reads would be a real,
    /// new gap to disclose, not silently skipped by this fn.
    fn apply_condition_filter_in_process(&self, request: &EvalRequest, action: &str, purpose: &str, rows: Vec<E>) -> Vec<E> {
        let records = self.matching_allow_records(&request.context.subject.roles, action, purpose);
        if records.iter().all(|r| r.conditions.is_empty()) {
            return rows;
        }
        rows.into_iter()
            .filter(|row| {
                let Ok(json) = serde_json::to_value(row) else { return false };
                records.iter().all(|record| field_policy::check_conditions(record, &json).is_ok())
            })
            .collect()
    }

    /// `evaluation.masks` (real `mask(...)`-granted `FieldMask`s) unioned
    /// with the real read-side `field_policy { forbidden(...) }` masks
    /// [`field_policy::read_masks_from_field_policy`] synthesizes for
    /// every matching Allow record -- see that fn's own doc comment for
    /// why this union is needed at all (until this fix, `field_policy`
    /// on a read policy was never enforced, only `mask(...)` was).
    fn read_masks(&self, request: &EvalRequest, action: &str, purpose: &str, granted: &[nirdosha_guard_core::FieldMask]) -> Vec<nirdosha_guard_core::FieldMask> {
        let mut masks = granted.to_vec();
        for record in self.matching_allow_records(&request.context.subject.roles, action, purpose) {
            masks.extend(field_policy::read_masks_from_field_policy(record, E::field_policy_exempt()));
        }
        masks
    }

    /// Detail-by-id, kept independent of `guarded_snapshot`'s own scan
    /// rather than filtering *its* result -- a targeted exact-key lookup
    /// (this row's own storage key, ANDed onto the same tenant/scope
    /// clauses a list read uses) has an inherent result size of one and
    /// so is never subject to `widest_caps`' row_cap resolution at all,
    /// unlike a call that goes through `guarded_snapshot`'s own
    /// `RowCap` (see that fn's doc comment: two real corpus policies,
    /// `analyst-search-transaction` and `analyst-open-transaction`,
    /// match the *identical* (subject, action, resource, purpose) tuple
    /// with different intended row counts -- a single scan-then-filter
    /// implementation of "get one row" would be at the mercy of
    /// whichever grant's cap the driver happened to apply, which could
    /// silently return "not found" for a row that exists whenever the
    /// scan truncated before reaching it).
    pub fn guarded_get(&self, auth: &Auth, purpose: &str, row_id: &str) -> Result<Option<E>, GuardScreenError> {
        let request = EvalRequest { context: self.context(auth, Action::Read, purpose) };
        let evaluation = { self.client.lock().expect("guard client lock poisoned").evaluate(&request) };
        self.record_audit_decision(&request, &evaluation);
        match evaluation.decision {
            Decision::Deny { reason } => Err(GuardScreenError::Denied(reason)),
            Decision::Escalate { to } => Err(GuardScreenError::Escalated(format!("{to:?}"))),
            Decision::Pending { .. } => Err(GuardScreenError::Store("read cannot be pending here".into())),
            Decision::Allow => {
                let mut clauses = self.read_scope_clauses(&request, "read", purpose, &evaluation.residual_filter)?;
                clauses.push(FilterExpr::Eq { field: vec!["resource".into()], value: Value::Str(scope::storage_key(E::RESOURCE, row_id)) });
                let plan = ReadPlanIr {
                    resource: E::RESOURCE.to_string(),
                    dataset: self.dataset.clone(),
                    filter: Some(FilterExpr::And(clauses)),
                    caps: vec![],
                    pagination: PaginationMode::LimitOnly { limit: 1 },
                    policy_version: request.context.policy_version.clone(),
                };
                let query_result = self.driver.query(&plan).map_err(|e| GuardScreenError::Store(format!("{e:?}")))?;
                let masks = self.read_masks(&request, "read", purpose, &evaluation.masks);
                Ok(self.decode_rows(query_result.rows, &masks)?.into_iter().next())
            }
        }
    }

    /// The tenant/subject/other-scope clauses `scan`/`guarded_get` both
    /// need, shared so the two never drift on what "the same read-like
    /// policy" is allowed to see. `action` is the wire string the
    /// evaluation itself ran under (`"read"` or `"aggregate"` today) --
    /// `matching_allow_records` must be asked about the *same* action,
    /// since a policy's own `filter_ref` is declared per-action, not
    /// per-resource (e.g. `lead-dashboard`'s `filter tenant_scope()` is
    /// only ever registered under `action == "aggregate"`).
    fn read_scope_clauses(&self, request: &EvalRequest, action: &str, purpose: &str, residual_filter: &Option<FilterExpr>) -> Result<Vec<FilterExpr>, GuardScreenError> {
        let mut clauses = vec![scope::resource_prefix_filter(E::RESOURCE)];
        if let Some(filter) = residual_filter {
            clauses.push(filter.clone());
        }
        for record in self.matching_allow_records(&request.context.subject.roles, action, purpose) {
            if let Some(filter_ref) = &record.filter_ref {
                // Real, batch-3-discovered driver limitation, not routed
                // around silently: `MemStoreDriver`/`PostgresStoreDriver`'s
                // own flat `resource`/`tenant`/`payload` schema means
                // `field_str` (confirmed by direct read of
                // `nirdosha-guard-mic::field_str`) only resolves the
                // literal `["resource"]`/`["tenant"]` field paths -- an
                // `Eq{field: [subject_scope_field], ...}` clause pushed
                // into `ReadPlanIr.filter` would silently match ZERO rows
                // (`field_str` returns `None` for any other field, and
                // `FilterExpr::Eq`'s own match arm treats that as
                // non-matching), the exact same class of gap the G5
                // "payload-field-filter" disclosure already names for
                // fine-grained predicates. `subject_scope()` is skipped
                // HERE (not pushed to the driver at all) and applied
                // in-process instead, by `scan_allowed`, after decode --
                // same "coarse driver pushdown + fine in-process filter"
                // posture G5 already establishes, not a second, weaker
                // implementation of scoping.
                if filter_ref.trim() == "subject_scope()" {
                    continue;
                }
                match scope::resolve_filter_ref(filter_ref, &self.tenant, &request.context.subject.id, E::subject_scope_field()) {
                    Some(resolved) => clauses.push(resolved),
                    None => return Err(GuardScreenError::Store(format!("unresolved scope function `{filter_ref}` on policy `{}` (Phase B item)", record.id))),
                }
            }
        }
        Ok(clauses)
    }

    /// The in-process half of `subject_scope()` -- see
    /// `read_scope_clauses`'s own doc comment for why this can't be a
    /// driver-pushed filter. Applied only when a real matching Allow
    /// record actually declared `filter subject_scope()` (never
    /// unconditionally -- a dataset with no such policy must not have
    /// its rows silently narrowed by a scope nothing asked for).
    fn apply_subject_scope_in_process(&self, request: &EvalRequest, action: &str, purpose: &str, rows: Vec<E>) -> Vec<E> {
        let Some(field) = E::subject_scope_field() else { return rows };
        let needs_subject_scope = self
            .matching_allow_records(&request.context.subject.roles, action, purpose)
            .iter()
            .any(|r| r.filter_ref.as_deref().map(str::trim) == Some("subject_scope()"));
        if !needs_subject_scope {
            return rows;
        }
        let subject_id = request.context.subject.id.clone();
        rows.into_iter()
            .filter(|row| serde_json::to_value(row).ok().and_then(|v| v.get(field).and_then(|f| f.as_str().map(str::to_string))) == Some(subject_id.clone()))
            .collect()
    }

    fn decode_rows(&self, rows: Vec<EntityBytes>, masks: &[nirdosha_guard_core::FieldMask]) -> Result<Vec<E>, GuardScreenError> {
        let mut out = Vec::with_capacity(rows.len());
        for bytes in rows {
            let mut value: serde_json::Value = serde_json::from_slice(&bytes.0).map_err(|e| GuardScreenError::Store(format!("decode: {e}")))?;
            masking::apply_masks(&mut value, masks);
            let entity: E = serde_json::from_value(value).map_err(|e| GuardScreenError::Store(format!("entity mismatch: {e}")))?;
            out.push(entity);
        }
        Ok(out)
    }

    pub fn guarded_insert(&self, auth: &Auth, purpose: &str, entity: E) -> Result<E, GuardScreenError> {
        let mut submitted = field_names(&entity)?;
        for exempt in E::create_field_policy_exempt() {
            submitted.remove(*exempt);
        }
        self.guarded_insert_checked(auth, purpose, &submitted, entity)
    }

    /// Like [`Self::guarded_insert`], but `submitted` is the caller's own
    /// explicit "which field names did this write actually touch" set,
    /// not derived from `entity`'s full serialized key set. **Real fix,
    /// found by this batch's own tests**: `guarded_insert`'s original
    /// `field_names(&entity)` always returns every struct field (a
    /// `Default`-filled entity has no way to represent "this key was
    /// never submitted" vs. "submitted as its default/empty value"), so
    /// a `field_policy { required(...) }` check built on it can never
    /// actually fire for a missing field -- `analyst-create-case`
    /// submitting an empty `rationale=` should be rejected as a missing
    /// required field and, before this method existed, silently wasn't.
    /// `crud_screens!`'s guarded create route (the one real caller that
    /// knows the actual submitted form-field-name set) uses this; every
    /// existing `guarded_insert` test-fixture call site is unaffected.
    pub fn guarded_insert_checked(&self, auth: &Auth, purpose: &str, submitted: &HashSet<String>, entity: E) -> Result<E, GuardScreenError> {
        self.guarded_insert_action(auth, purpose, Action::Create, "create", submitted, entity)
    }

    /// `guarded_insert_checked` generalized over the guard `Action`/wire
    /// action-string pair -- `admin-mint-delegation`'s real corpus policy
    /// (`90_ops_admin.nir`) is `action == "delegate"`, not `"create"`;
    /// same create-shaped write (no pre-existing row, no escalation --
    /// this action has none), different action kind, matching the same
    /// `commit_update`→`commit_action` generalization pattern already
    /// established for the update side.
    pub fn guarded_insert_action(&self, auth: &Auth, purpose: &str, action: Action, action_str: &str, submitted: &HashSet<String>, entity: E) -> Result<E, GuardScreenError> {
        let request = EvalRequest { context: self.context(auth, action, purpose) };
        let allow_records = self.matching_allow_records(&request.context.subject.roles, action_str, purpose);
        for record in &allow_records {
            field_policy::check_field_policy(record, &submitted).map_err(GuardScreenError::FieldPolicyViolation)?;
        }
        // G2, create side: no "before" state exists yet, so `before ==
        // after == entity` -- see `check_custom_conditions`'s own doc
        // comment. No corpus create policy uses
        // `field(status).transition_allowed()` today; if one did, its
        // dataset's `transition_allowed` impl would need to accept the
        // self-transition this produces.
        let after_json = serde_json::to_value(&entity).map_err(|e| GuardScreenError::Store(format!("encode: {e}")))?;
        for record in &allow_records {
            field_policy::check_conditions(record, &after_json).map_err(GuardScreenError::ConditionFailed)?;
            field_policy::check_custom_conditions(record, &entity, &entity, &requested_status_value(&after_json)).map_err(GuardScreenError::ConditionFailed)?;
        }
        let evaluation = { self.client.lock().expect("guard client lock poisoned").evaluate(&request) };
        self.record_audit_decision(&request, &evaluation);
        match evaluation.decision {
            Decision::Deny { reason } => Err(GuardScreenError::Denied(reason)),
            Decision::Escalate { to } => Err(GuardScreenError::Escalated(format!("{to:?}"))),
            Decision::Pending { .. } => Err(GuardScreenError::Store("write cannot be pending here".into())),
            Decision::Allow => {
                let payload = serde_json::to_vec(&entity).map_err(|e| GuardScreenError::Store(format!("encode: {e}")))?;
                let resource = scope::storage_key(E::RESOURCE, &entity.row_id());
                let plan = PlanIr {
                    resource,
                    dataset: self.dataset.clone(),
                    filter: Some(FilterExpr::TenantEq { value: Value::Str(self.tenant.clone()) }),
                    row_scope: None,
                    action: WriteAction::Create,
                    affected_row_cap: evaluation.affected_row_cap,
                    policy_version: request.context.policy_version.clone(),
                };
                let prepared = self.driver.prepare(&plan).map_err(|e| GuardScreenError::Store(format!("{e:?}")))?;
                self.driver.commit(prepared, EntityBytes(payload)).map_err(|e| GuardScreenError::Store(format!("{e:?}")))?;
                self.dispatch_obligations(&allow_records, &entity.row_id());
                Ok(entity)
            }
        }
    }

    /// The internal, unauthorized-by-itself counterpart to
    /// `guarded_snapshot`: fetches exactly one row by its real storage
    /// key, unmasked, with no policy evaluation of its own. Only ever
    /// called from inside `guarded_update`/`guarded_delete` *after* the
    /// Update/Delete action itself already evaluated to `Allow` -- the
    /// same posture `PlanIr::row_scope`'s own doc comment describes for
    /// the driver-level existence/precondition check: reading the
    /// pre-existing row to fulfil a write that was already authorized
    /// isn't a second `read` the guard policy plane needs to separately
    /// grant, any more than a SQL `UPDATE ... WHERE` needs its own
    /// `SELECT` grant.
    /// Test-fixture seeding only: writes a row directly to the driver,
    /// bypassing policy evaluation entirely -- for the (real, corpus-
    /// accurate) cases where NO write policy exists to arrange a given
    /// starting state through (e.g. `customer` has no create policy at
    /// all; `case`'s only create policy, `analyst-create-case`, forbids
    /// the `status` field, so a test needing a case pre-seeded at
    /// `confirmed_fraud` has no guarded path to get there). Matches the
    /// same "arrange state outside the boundary under test" posture
    /// `nirdosha-guard-mic`'s own driver tests use when seeding directly
    /// against `MemStoreDriver`. Not for anything this crate's own
    /// runtime code should ever call.
    pub fn raw_driver_seed(&self, tenant: &str, entity: &E) {
        self.write_unauthorized(tenant, entity)
    }

    /// The one legitimate *runtime* caller of an unauthorized write: a
    /// system-generated side effect of an action that was ALREADY
    /// authorized elsewhere (an `Obligation::Notify` consumed by
    /// `dispatch_obligations` above), not a new authorization decision of
    /// its own -- there is no `guard_policy!` that could even govern it
    /// (`notification` names no `resource ==`/`resource in [...]` clause
    /// anywhere in the corpus, confirmed by grep), so evaluating one
    /// would be inventing a policy, not enforcing a real one. Shares
    /// `raw_driver_seed`'s exact mechanics (same "no evaluation, direct
    /// driver write" shape) under a name that says what it's really for,
    /// so a reader grepping call sites can tell "test fixture" from "real
    /// runtime side effect" apart.
    pub fn system_write(&self, tenant: &str, entity: &E) {
        self.write_unauthorized(tenant, entity)
    }

    fn write_unauthorized(&self, tenant: &str, entity: &E) {
        let resource = scope::storage_key(E::RESOURCE, &entity.row_id());
        let plan = PlanIr {
            resource,
            dataset: self.dataset.clone(),
            filter: Some(FilterExpr::TenantEq { value: Value::Str(tenant.to_string()) }),
            row_scope: None,
            action: WriteAction::Create,
            affected_row_cap: None,
            policy_version: "rtm-demo".into(),
        };
        let prepared = self.driver.prepare(&plan).expect("unauthorized write: prepare");
        let payload = serde_json::to_vec(entity).expect("unauthorized write: encode");
        self.driver.commit(prepared, EntityBytes(payload)).expect("unauthorized write: commit");
    }

    /// Real, unguarded read counterpart to [`Self::system_write`] -- for
    /// the same "no `guard_policy!` governs this resource at all" cases
    /// (`notification`), where the screen route gates access by role
    /// directly (`Auth::has_role`) rather than a policy this crate could
    /// evaluate. Scoped to this table's own tenant + resource prefix,
    /// same as a real guarded scan -- just without an `evaluate()` call
    /// there is genuinely no policy to make.
    pub fn system_scan(&self) -> Result<Vec<E>, GuardScreenError> {
        let plan = ReadPlanIr {
            resource: E::RESOURCE.to_string(),
            dataset: self.dataset.clone(),
            filter: Some(FilterExpr::And(vec![scope::resource_prefix_filter(E::RESOURCE), FilterExpr::TenantEq { value: Value::Str(self.tenant.clone()) }])),
            caps: vec![],
            pagination: PaginationMode::LimitOnly { limit: 1000 },
            policy_version: "rtm-demo".into(),
        };
        let query_result = self.driver.query(&plan).map_err(|e| GuardScreenError::Store(format!("{e:?}")))?;
        self.decode_rows(query_result.rows, &[])
    }

    fn fetch_raw(&self, storage_key: &str) -> Result<Option<EntityBytes>, GuardScreenError> {
        let plan = ReadPlanIr {
            resource: E::RESOURCE.to_string(),
            dataset: self.dataset.clone(),
            filter: Some(FilterExpr::Eq { field: vec!["resource".into()], value: Value::Str(storage_key.to_string()) }),
            caps: vec![],
            pagination: PaginationMode::LimitOnly { limit: 1 },
            policy_version: "rtm-demo".into(),
        };
        let result = self.driver.query(&plan).map_err(|e| GuardScreenError::Store(format!("{e:?}")))?;
        Ok(result.rows.into_iter().next())
    }

    /// `changed_fields` is exactly the set of field names this update is
    /// attempting to write (e.g. an HTML form's submitted keys) -- G1's
    /// `field_policy` is checked against *that*, not the merged row's
    /// full field set, matching every real corpus update policy's own
    /// shape (`analyst-flag-transaction`'s `forbidden(tenant_id, amount,
    /// ...)` would trip on every single update if "submitted" meant "the
    /// whole row", since those fields are always present on the merged
    /// entity whether or not this write touched them). `apply` mutates a
    /// clone of the existing row in place; the pre-mutation clone is
    /// `check_custom_conditions`'s `before` (e.g. `sar-edit`'s `requires
    /// field(status) == "draft"` gates what the row already *is*, not
    /// what this write turns it into) and the post-mutation value is
    /// `after` (e.g. `amount_positive` validates the row being
    /// committed).
    pub fn guarded_update(&self, auth: &Auth, purpose: &str, row_id: &str, changed_fields: &HashSet<String>, apply: impl FnOnce(&mut E)) -> Result<E, GuardScreenError> {
        let request = EvalRequest { context: self.context(auth, Action::Update, purpose) };
        let mut submitted = changed_fields.clone();
        for exempt in E::field_policy_exempt() {
            submitted.remove(*exempt);
        }
        let allow_records = self.matching_allow_records(&request.context.subject.roles, "update", purpose);
        for record in &allow_records {
            field_policy::check_field_policy(record, &submitted).map_err(GuardScreenError::FieldPolicyViolation)?;
        }

        let evaluation = { self.client.lock().expect("guard client lock poisoned").evaluate(&request) };
        self.record_audit_decision(&request, &evaluation);
        match evaluation.decision {
            Decision::Deny { reason } => Err(GuardScreenError::Denied(reason)),
            Decision::Escalate { to } => self.downgrade_if_escalation_condition_fails(row_id, &allow_records, apply, evaluation.affected_row_cap, &request, to),
            Decision::Pending { .. } => Err(GuardScreenError::Store("write cannot be pending here".into())),
            Decision::Allow => self.commit_update(row_id, &allow_records, apply, evaluation.affected_row_cap, &request.context.policy_version, &request.context.subject.id),
        }
    }

    /// Real fix: `evaluator::evaluate` decides Allow vs. Escalate purely
    /// from `EvaluationContext` (role/action/resource/purpose) -- it has
    /// no row data, so it cannot evaluate a matching record's own
    /// `requires field(x) > y` condition before deciding escalation wins
    /// over a plain Allow record matching the identical tuple.
    /// `ops-decide-hold-highvalue`'s real `requires field(amount) >
    /// 100_000` is exactly this shape: without this check, EVERY payment
    /// update would escalate through `override_release` regardless of
    /// amount, since the condition is invisible to `matches_context`.
    /// Re-checks each escalating matching record's conditions against
    /// the row this specific write targets; if none actually hold, the
    /// escalation doesn't apply to this row and the write proceeds under
    /// whichever non-escalating Allow record(s) still match (also
    /// re-verified at commit time by `commit_action`'s own
    /// `check_conditions`/`check_custom_conditions` loop, so this is a
    /// narrowing of which records apply, not a bypass of what they
    /// require). If no row exists yet, fails toward the stricter
    /// (escalate) outcome rather than guessing.
    fn downgrade_if_escalation_condition_fails(&self, row_id: &str, allow_records: &[&PolicyRecord], apply: impl FnOnce(&mut E), affected_row_cap: Option<u64>, request: &EvalRequest, to: EscalateTarget) -> Result<E, GuardScreenError> {
        let resource = scope::storage_key(E::RESOURCE, row_id);
        let existing = self.fetch_raw(&resource)?;
        let escalating: Vec<&&PolicyRecord> = allow_records.iter().filter(|r| r.escalation.is_some()).collect();
        let escalation_genuinely_required = match &existing {
            Some(bytes) => {
                let row_json: serde_json::Value = serde_json::from_slice(&bytes.0).map_err(|e| GuardScreenError::Store(format!("decode: {e}")))?;
                escalating.iter().any(|r| field_policy::check_conditions(r, &row_json).is_ok())
            }
            None => true,
        };
        if escalation_genuinely_required {
            return Err(GuardScreenError::Escalated(format!("{to:?}")));
        }
        let non_escalating: Vec<&PolicyRecord> = allow_records.iter().copied().filter(|r| r.escalation.is_none()).collect();
        if non_escalating.is_empty() {
            return Err(GuardScreenError::Escalated(format!("{to:?}")));
        }
        self.commit_update(row_id, &non_escalating, apply, affected_row_cap, &request.context.policy_version, &request.context.subject.id)
    }

    /// The write-side commit `guarded_update` performs once a plain
    /// `Decision::Allow` is in hand -- factored out so
    /// `guarded_confirm_escalated_update` (below) can reuse the exact
    /// same field_policy/invariant/commit path once a real quorum is
    /// reached, instead of a second, drifting copy of it. Thin wrapper
    /// over `commit_action` (`WriteAction::Update`) -- see that method's
    /// own doc comment for why the write-kind became a parameter (M8/M13
    /// both need the identical field_policy/invariant/commit path for a
    /// real `migrate`-action escalation, not a second copy of it).
    fn commit_update(&self, row_id: &str, allow_records: &[&PolicyRecord], apply: impl FnOnce(&mut E), affected_row_cap: Option<u64>, policy_version: &str, subject_id: &str) -> Result<E, GuardScreenError> {
        self.commit_action(row_id, allow_records, apply, affected_row_cap, policy_version, WriteAction::Update, subject_id)
    }

    /// `commit_update` generalized over `WriteAction` -- added for M8
    /// (`threshold-migrate`) and M13 (`refdata-migrate`), whose real
    /// corpus policies use `action == "migrate"`, not `"update"`.
    /// `PlanIr::commit`'s own driver contract doesn't care which
    /// `WriteAction` a plan carries beyond record-keeping (see
    /// `MemStoreDriver`/`PostgresStoreDriver`'s `commit` -- neither
    /// branches on it), so this is a real, not cosmetic, generalization:
    /// the same field_policy/invariant/commit path now serves both write
    /// kinds instead of a second, drifting copy for `migrate`.
    ///
    /// **Real bug found and fixed wiring M22**: a matching Allow record's
    /// `filter subject_scope()` (RTM's `self-update-profile`: "own row
    /// only") was evaluated for READS (`apply_subject_scope_in_process`)
    /// but never checked on the WRITE path at all -- `guarded_update`
    /// would happily let any subject holding the role update ANY row_id,
    /// not just their own, silently ignoring the one clause the policy
    /// exists to add over a bare role grant. `subject_id` (the caller's
    /// own identity, threaded in from every call site's own
    /// `EvaluationContext.subject.id`) is checked against `E::
    /// subject_scope_field()`'s value on the EXISTING row before any
    /// mutation runs, for exactly the matching records that actually
    /// declared `filter subject_scope()` -- same "only enforce what a
    /// real matching policy asked for" posture `apply_subject_scope_in_process`
    /// already established on the read side, not a blanket check every
    /// table now pays for.
    fn commit_action(&self, row_id: &str, allow_records: &[&PolicyRecord], apply: impl FnOnce(&mut E), affected_row_cap: Option<u64>, policy_version: &str, write_action: WriteAction, subject_id: &str) -> Result<E, GuardScreenError> {
        let resource = scope::storage_key(E::RESOURCE, row_id);
        let existing = self.fetch_raw(&resource)?.ok_or_else(|| GuardScreenError::Store(format!("update: row `{row_id}` not found")))?;
        let before: E = serde_json::from_slice(&existing.0).map_err(|e| GuardScreenError::Store(format!("decode: {e}")))?;
        let before_json = serde_json::to_value(&before).map_err(|e| GuardScreenError::Store(format!("encode: {e}")))?;

        let needs_subject_scope = allow_records.iter().any(|r| r.filter_ref.as_deref().map(str::trim) == Some("subject_scope()"));
        if needs_subject_scope {
            let field = E::subject_scope_field().ok_or_else(|| GuardScreenError::Store(format!("policy declares filter subject_scope() but {} has no subject_scope_field() (Phase B item)", E::RESOURCE)))?;
            let owner = before_json.get(field).and_then(|v| v.as_str());
            if owner != Some(subject_id) {
                return Err(GuardScreenError::Denied("subject_scope(): this row does not belong to the requesting subject".into()));
            }
        }

        let mut after = before.clone();
        apply(&mut after);
        let after_json = serde_json::to_value(&after).map_err(|e| GuardScreenError::Store(format!("encode: {e}")))?;

        for record in allow_records {
            field_policy::check_conditions(record, &before_json).map_err(GuardScreenError::ConditionFailed)?;
            field_policy::check_custom_conditions(record, &before, &after, &requested_status_value(&after_json)).map_err(GuardScreenError::ConditionFailed)?;
        }

        let payload = serde_json::to_vec(&after).map_err(|e| GuardScreenError::Store(format!("encode: {e}")))?;
        let plan = PlanIr {
            resource,
            dataset: self.dataset.clone(),
            filter: Some(FilterExpr::TenantEq { value: Value::Str(self.tenant.clone()) }),
            row_scope: Some(FilterExpr::TenantEq { value: Value::Str(self.tenant.clone()) }),
            action: write_action,
            affected_row_cap,
            policy_version: policy_version.to_string(),
        };
        let prepared = self.driver.prepare(&plan).map_err(|e| GuardScreenError::Store(format!("{e:?}")))?;
        self.driver.commit(prepared, EntityBytes(payload)).map_err(|e| GuardScreenError::Store(format!("{e:?}")))?;
        self.dispatch_obligations(allow_records, row_id);
        Ok(after)
    }

    /// First half of a real two-step, quorum-gated write (RTM's
    /// `case-close-confirm`: `escalate to approval(chain case_review)`).
    /// On a plain `Decision::Allow` this commits immediately (nothing to
    /// escalate); on `Decision::Escalate { to: Approval { chain } }` it
    /// opens a real pending escalation in this table's own
    /// `ApprovalChainRuntime` and records the *proposer's own* approval
    /// against it -- for a `quorum(2, of = [ComplianceLead])` chain, that
    /// consumes one of two required slots, so reaching quorum needs one
    /// MORE, distinct, role-eligible approver: `ApprovalChainRuntime::
    /// approve`'s own duplicate-approver rejection is what makes this
    /// self-review-blocked, not a separate check here.
    pub fn guarded_propose_escalated_update(&self, auth: &Auth, purpose: &str, row_id: &str, changed_fields: &HashSet<String>, apply: impl FnOnce(&mut E), now_ms: u64) -> Result<EscalatedWrite<E>, GuardScreenError> {
        self.guarded_propose_escalated_action(auth, purpose, Action::Update, "update", WriteAction::Update, row_id, changed_fields, apply, now_ms)
    }

    /// `guarded_propose_escalated_update` generalized over the guard
    /// `Action`/wire action-string/`WriteAction` triple -- M8's
    /// `threshold-migrate` and M13's `refdata-migrate` are real corpus
    /// `escalate to approval(chain ...)` policies on `action == "migrate"`,
    /// not `"update"`; this is the same two-step quorum machinery
    /// (`ApprovalChainRuntime`, self-review blocked by its own
    /// duplicate-approver rejection), not a second, weaker copy of it.
    #[allow(clippy::too_many_arguments)]
    pub fn guarded_propose_escalated_action(&self, auth: &Auth, purpose: &str, action: Action, action_str: &str, write_action: WriteAction, row_id: &str, changed_fields: &HashSet<String>, apply: impl FnOnce(&mut E), now_ms: u64) -> Result<EscalatedWrite<E>, GuardScreenError> {
        let request = EvalRequest { context: self.context(auth, action, purpose) };
        let mut submitted = changed_fields.clone();
        for exempt in E::field_policy_exempt() {
            submitted.remove(*exempt);
        }
        let allow_records = self.matching_allow_records(&request.context.subject.roles, action_str, purpose);
        for record in &allow_records {
            field_policy::check_field_policy(record, &submitted).map_err(GuardScreenError::FieldPolicyViolation)?;
        }
        let evaluation = { self.client.lock().expect("guard client lock poisoned").evaluate(&request) };
        self.record_audit_decision(&request, &evaluation);
        match evaluation.decision {
            Decision::Deny { reason } => Err(GuardScreenError::Denied(reason)),
            Decision::Pending { .. } => Err(GuardScreenError::Store("write cannot be pending here".into())),
            Decision::Allow => self.commit_action(row_id, &allow_records, apply, evaluation.affected_row_cap, &request.context.policy_version, write_action, &request.context.subject.id).map(EscalatedWrite::Committed),
            Decision::Escalate { to: EscalateTarget::Approval { chain } } => {
                let mut approvals = self.approvals.lock().expect("approval runtime lock poisoned");
                let definition = approvals
                    .definition(&chain)
                    .cloned()
                    .ok_or_else(|| GuardScreenError::Store(format!("chain `{chain}` has no registered approval_chain! definition — check the corpus's own approval_chain! block and this table's constructor")))?;
                // The proposer consumes their own quorum slot ONLY when
                // they hold a role the chain itself accepts as an
                // approver (M4's `case-close-confirm`: `ComplianceLead`
                // proposes AND is in `case_review`'s own `of =
                // [ComplianceLead]` list -- "four-eyes among peers"). A
                // real maker-checker split, where the proposing role is
                // deliberately NOT an eligible approver at all (M13's
                // `refdata-migrate`: `Admin` proposes, but `policy_release`'s
                // `of = [PolicyEngineer, ComplianceLead]` names neither
                // `Admin` nor any role `Admin` might also hold), is not
                // an error -- it's the whole point of maker≠checker.
                // Found live wiring M13: the original version of this
                // method treated "proposer isn't chain-eligible" as a
                // hard `Denied`, which would have made `refdata-migrate`
                // uninvokable by the one role the corpus actually grants
                // it to. Opening with zero approvals and requiring
                // `quorum` DISTINCT real approvers from `approver_roles`
                // (none of whom need be the proposer) is the correct
                // general case; the proposer-is-also-an-approver path
                // above is the special case, not the other way around.
                let proposer_role = definition.approver_roles.iter().find(|role| auth.has_role(role)).cloned();
                let escalation_id = format!("{}:{row_id}:{now_ms}", E::RESOURCE);
                let resource = scope::storage_key(E::RESOURCE, row_id);
                let deadline = now_ms + 24 * 60 * 60 * 1000;
                approvals.open(escalation_id.clone(), &chain, resource, auth.user(), now_ms, deadline).map_err(|e| GuardScreenError::Store(format!("{e:?}")))?;
                let (approvals_so_far, quorum, cooling_ready_at) = match proposer_role {
                    Some(role) => {
                        let status = approvals.approve(&escalation_id, auth.user(), &role, now_ms).map_err(|e| GuardScreenError::Store(format!("{e:?}")))?;
                        match status {
                            EscalationStatus::Pending { approvals_so_far, quorum } => (approvals_so_far, quorum, None),
                            EscalationStatus::Approved => (definition.quorum, definition.quorum, None), // quorum == 1, no cooling: proposer alone satisfies it
                            EscalationStatus::Cooling { ready_at } => (definition.quorum, definition.quorum, Some(ready_at)), // quorum == 1, cooling: proposer alone reached quorum but window hasn't elapsed
                            EscalationStatus::DeniedTimeout => return Err(GuardScreenError::Store("escalation timed out immediately — check the chain's own deadline arithmetic".into())),
                            EscalationStatus::Returned { .. } => return Err(GuardScreenError::Store("a freshly-opened escalation cannot already be returned".into())),
                        }
                    }
                    None => (0, definition.quorum, None),
                };
                Ok(EscalatedWrite::Pending(PendingApproval { escalation_id, chain, quorum, approvals_so_far, cooling_ready_at }))
            }
            Decision::Escalate { to } => Err(GuardScreenError::Escalated(format!("{to:?}"))),
        }
    }

    /// Second half: a *different*, role-eligible approver confirms a
    /// pending escalation `guarded_propose_escalated_update` opened.
    /// `chain`/`escalation_id`/`row_id` are exactly what that call
    /// returned in its `PendingApproval` -- carried by the caller (e.g.
    /// as hidden form fields) rather than recovered from
    /// `ApprovalChainRuntime`'s own state, which has no chain-by-id
    /// lookup today (a real, small, disclosed limitation, not an
    /// oversight: adding one is a `nirdosha-guard-core` change this
    /// table's own crate doesn't need to make for RTM's one real use of
    /// this path). `apply` must match the SAME mutation the proposal
    /// described -- this method has no way to verify that on its own; the
    /// caller (the screen route) is responsible for re-deriving the exact
    /// same closure from the same submitted fields, the same way
    /// `guarded_update`'s own caller always has.
    pub fn guarded_confirm_escalated_update(&self, auth: &Auth, purpose: &str, row_id: &str, chain: &str, escalation_id: &str, changed_fields: &HashSet<String>, apply: impl FnOnce(&mut E), now_ms: u64) -> Result<EscalatedWrite<E>, GuardScreenError> {
        self.guarded_confirm_escalated_action(auth, purpose, Action::Update, "update", WriteAction::Update, row_id, chain, escalation_id, changed_fields, apply, now_ms)
    }

    /// `guarded_confirm_escalated_update` generalized the same way
    /// `guarded_propose_escalated_action` generalizes its propose-side
    /// counterpart -- see that method's own doc comment.
    #[allow(clippy::too_many_arguments)]
    pub fn guarded_confirm_escalated_action(&self, auth: &Auth, purpose: &str, action: Action, action_str: &str, write_action: WriteAction, row_id: &str, chain: &str, escalation_id: &str, changed_fields: &HashSet<String>, apply: impl FnOnce(&mut E), now_ms: u64) -> Result<EscalatedWrite<E>, GuardScreenError> {
        let (quorum, status) = {
            let mut approvals = self.approvals.lock().expect("approval runtime lock poisoned");
            let definition = approvals
                .definition(chain)
                .cloned()
                .ok_or_else(|| GuardScreenError::Store(format!("chain `{chain}` has no registered approval_chain! definition")))?;
            let role = definition
                .approver_roles
                .iter()
                .find(|role| auth.has_role(role))
                .cloned()
                .ok_or_else(|| GuardScreenError::Denied(format!("no role held by `{}` is eligible to approve chain `{chain}`", auth.user())))?;
            let status = approvals.approve(escalation_id, auth.user(), &role, now_ms).map_err(|e| match e {
                ApprovalChainError::DuplicateApprover { .. } => GuardScreenError::Denied("self-review blocked: this approver already acted on this escalation".into()),
                ApprovalChainError::Expired => GuardScreenError::Denied("escalation timed out before reaching quorum (I3: timeout resolves to deny, never auto-approve)".into()),
                other => GuardScreenError::Store(format!("{other:?}")),
            })?;
            (definition.quorum, status)
        };
        match status {
            EscalationStatus::Approved => {
                let purpose_request = EvalRequest { context: self.context(auth, action, purpose) };
                let mut submitted = changed_fields.clone();
                for exempt in E::field_policy_exempt() {
                    submitted.remove(*exempt);
                }
                let allow_records = self.matching_allow_records(&purpose_request.context.subject.roles, action_str, purpose);
                self.commit_action(row_id, &allow_records, apply, None, &purpose_request.context.policy_version, write_action, &purpose_request.context.subject.id).map(EscalatedWrite::Committed)
            }
            EscalationStatus::Cooling { ready_at } => Ok(EscalatedWrite::Pending(PendingApproval { escalation_id: escalation_id.to_string(), chain: chain.to_string(), quorum, approvals_so_far: quorum, cooling_ready_at: Some(ready_at) })),
            EscalationStatus::Pending { approvals_so_far, .. } => Ok(EscalatedWrite::Pending(PendingApproval { escalation_id: escalation_id.to_string(), chain: chain.to_string(), quorum, approvals_so_far, cooling_ready_at: None })),
            EscalationStatus::DeniedTimeout => Err(GuardScreenError::Denied("escalation timed out before reaching quorum (I3: timeout resolves to deny)".into())),
            EscalationStatus::Returned { reason } => Err(GuardScreenError::Denied(format!("escalation was returned: {reason}"))),
        }
    }

    /// Finalizes a `Cooling` escalation once its window has elapsed --
    /// the counterpart to `guarded_confirm_escalated_action` for chains
    /// with `cooling(...)`. Does not require a fresh distinct approval
    /// (quorum was already real); any role eligible on the chain may
    /// call this once `now_ms` has passed the cooling deadline, same
    /// `apply` contract as confirm (caller re-derives the identical
    /// mutation).
    #[allow(clippy::too_many_arguments)]
    pub fn guarded_finalize_escalated_action(&self, auth: &Auth, purpose: &str, action: Action, action_str: &str, write_action: WriteAction, row_id: &str, chain: &str, escalation_id: &str, changed_fields: &HashSet<String>, apply: impl FnOnce(&mut E), now_ms: u64) -> Result<EscalatedWrite<E>, GuardScreenError> {
        let (quorum, status) = {
            let mut approvals = self.approvals.lock().expect("approval runtime lock poisoned");
            let definition = approvals
                .definition(chain)
                .cloned()
                .ok_or_else(|| GuardScreenError::Store(format!("chain `{chain}` has no registered approval_chain! definition")))?;
            definition
                .approver_roles
                .iter()
                .find(|role| auth.has_role(role))
                .ok_or_else(|| GuardScreenError::Denied(format!("no role held by `{}` is eligible on chain `{chain}`", auth.user())))?;
            let status = approvals.finalize(escalation_id, now_ms).map_err(|e| GuardScreenError::Store(format!("{e:?}")))?;
            (definition.quorum, status)
        };
        match status {
            EscalationStatus::Approved => {
                let purpose_request = EvalRequest { context: self.context(auth, action, purpose) };
                let mut submitted = changed_fields.clone();
                for exempt in E::field_policy_exempt() {
                    submitted.remove(*exempt);
                }
                let allow_records = self.matching_allow_records(&purpose_request.context.subject.roles, action_str, purpose);
                self.commit_action(row_id, &allow_records, apply, None, &purpose_request.context.policy_version, write_action, &purpose_request.context.subject.id).map(EscalatedWrite::Committed)
            }
            EscalationStatus::Cooling { ready_at } => Ok(EscalatedWrite::Pending(PendingApproval { escalation_id: escalation_id.to_string(), chain: chain.to_string(), quorum, approvals_so_far: quorum, cooling_ready_at: Some(ready_at) })),
            EscalationStatus::Pending { approvals_so_far, .. } => Ok(EscalatedWrite::Pending(PendingApproval { escalation_id: escalation_id.to_string(), chain: chain.to_string(), quorum, approvals_so_far, cooling_ready_at: None })),
            EscalationStatus::DeniedTimeout => Err(GuardScreenError::Denied("escalation timed out before reaching quorum (I3: timeout resolves to deny)".into())),
            EscalationStatus::Returned { reason } => Err(GuardScreenError::Denied(format!("escalation was returned: {reason}"))),
        }
    }

    /// An eligible approver actively returns (rejects) a still-open
    /// escalation with a mandatory reason -- T-04's return-with-reason
    /// invariant, machine-checked at this layer (empty reason is a
    /// named deny, not a silent no-op).
    pub fn guarded_return_escalated(&self, auth: &Auth, chain: &str, escalation_id: &str, reason: &str, now_ms: u64) -> Result<(), GuardScreenError> {
        let mut approvals = self.approvals.lock().expect("approval runtime lock poisoned");
        let definition = approvals
            .definition(chain)
            .cloned()
            .ok_or_else(|| GuardScreenError::Store(format!("chain `{chain}` has no registered approval_chain! definition")))?;
        let role = definition
            .approver_roles
            .iter()
            .find(|role| auth.has_role(role))
            .cloned()
            .ok_or_else(|| GuardScreenError::Denied(format!("no role held by `{}` is eligible to return chain `{chain}`", auth.user())))?;
        approvals.return_with_reason(escalation_id, auth.user(), &role, reason, now_ms).map_err(|e| match e {
            ApprovalChainError::EmptyReturnReason => GuardScreenError::Denied("a return requires a non-empty reason".into()),
            ApprovalChainError::AlreadyResolved(_) => GuardScreenError::Denied("escalation already resolved".into()),
            ApprovalChainError::Expired => GuardScreenError::Denied("escalation timed out before it could be returned".into()),
            other => GuardScreenError::Store(format!("{other:?}")),
        })?;
        Ok(())
    }

    /// This table's own pending/cooling/recently-resolved escalations --
    /// the real per-entity slice an `approval_inbox!` screen merges
    /// across the several `GuardedTable`s it names (case/rule-catalog/
    /// payment/sar_bundle/...), the same merge-projection shape B8's
    /// `AC.all_chains` already established for audit chains.
    pub fn list_pending_approvals(&self) -> Vec<PendingApprovalRow> {
        let approvals = self.approvals.lock().expect("approval runtime lock poisoned");
        approvals
            .list_pending()
            .into_iter()
            .map(|escalation| {
                let quorum = approvals.definition(&escalation.chain).map(|d| d.quorum).unwrap_or(0);
                let (status, cooling_ready_at, return_reason) = match &escalation.outcome {
                    Some(EscalationOutcome::Approved) => ("approved".to_string(), None, None),
                    Some(EscalationOutcome::DeniedTimeout) => ("denied_timeout".to_string(), None, None),
                    Some(EscalationOutcome::Returned { reason, .. }) => ("returned".to_string(), None, Some(reason.clone())),
                    None => match escalation.quorum_reached_at {
                        Some(reached_at) => {
                            let cooling_ms = approvals.definition(&escalation.chain).map(|d| d.cooling_period_ms).unwrap_or(0);
                            ("cooling".to_string(), Some(reached_at + cooling_ms), None)
                        }
                        None => ("pending".to_string(), None, None),
                    },
                };
                PendingApprovalRow {
                    escalation_id: escalation.id,
                    chain: escalation.chain,
                    resource: escalation.resource,
                    proposer: escalation.subject_id,
                    opened_at: escalation.opened_at,
                    deadline: escalation.deadline,
                    quorum,
                    approvals_so_far: escalation.approvals.len() as u8,
                    approver_ids: escalation.approvals.iter().map(|a| a.approver_id.clone()).collect(),
                    status,
                    cooling_ready_at,
                    return_reason,
                }
            })
            .collect()
    }

    /// Export's own propose/confirm pair -- real corpus policies
    /// (`governed-export`, `dsar-export`, and later batches'
    /// `sar-export`/`auditor-export`) all share the same
    /// `escalate to approval(chain egress_release)` shape M4's per-row
    /// update escalation already proved, but export has no single "row"
    /// to fetch-then-mutate (`commit_action`'s own shape) -- it's a
    /// guard-enforced SCAN producing an artifact, so "committing" once
    /// quorum is met means running that scan for real (via
    /// [`Self::scan_allowed`]), not writing anything. The escalation's
    /// own tracking key is `"<RESOURCE>:export"` (a resource-wide
    /// scan has no per-row identity to key on) -- unlike the update/
    /// migrate path, concurrent export proposals for the same resource
    /// necessarily share one pending escalation, which matches the real
    /// shape of "the whole export needs approval," not any one row.
    pub fn guarded_propose_escalated_export(&self, auth: &Auth, purpose: &str, action: Action, action_str: &str, now_ms: u64) -> Result<EscalatedWrite<Vec<E>>, GuardScreenError> {
        let _ = action_str; // signature symmetry with the confirm half / the update-side propose/confirm pair; scan_allowed derives its own wire string via action_wire_str(request.context.action)
        let request = EvalRequest { context: self.context(auth, action, purpose) };
        let evaluation = { self.client.lock().expect("guard client lock poisoned").evaluate(&request) };
        self.record_audit_decision(&request, &evaluation);
        match evaluation.decision {
            Decision::Deny { reason } => Err(GuardScreenError::Denied(reason)),
            Decision::Pending { .. } => Err(GuardScreenError::Store("export cannot be pending here".into())),
            Decision::Allow => self.scan_allowed(&request, purpose, &evaluation).map(EscalatedWrite::Committed),
            Decision::Escalate { to: EscalateTarget::Approval { chain } } => {
                let mut approvals = self.approvals.lock().expect("approval runtime lock poisoned");
                let definition = approvals
                    .definition(&chain)
                    .cloned()
                    .ok_or_else(|| GuardScreenError::Store(format!("chain `{chain}` has no registered approval_chain! definition")))?;
                // Same real maker≠checker handling as
                // `guarded_propose_escalated_action`'s own doc comment
                // (`dsar-export`: `Admin` proposes, `egress_release`'s
                // `of = [ComplianceLead]` doesn't name `Admin`).
                let proposer_role = definition.approver_roles.iter().find(|role| auth.has_role(role)).cloned();
                let escalation_id = format!("{}:export:{now_ms}", E::RESOURCE);
                let resource = format!("{}:export", E::RESOURCE);
                let deadline = now_ms + 24 * 60 * 60 * 1000;
                approvals.open(escalation_id.clone(), &chain, resource, auth.user(), now_ms, deadline).map_err(|e| GuardScreenError::Store(format!("{e:?}")))?;
                let (approvals_so_far, quorum, cooling_ready_at) = match proposer_role {
                    Some(role) => {
                        let status = approvals.approve(&escalation_id, auth.user(), &role, now_ms).map_err(|e| GuardScreenError::Store(format!("{e:?}")))?;
                        match status {
                            EscalationStatus::Pending { approvals_so_far, quorum } => (approvals_so_far, quorum, None),
                            EscalationStatus::Approved => (definition.quorum, definition.quorum, None),
                            EscalationStatus::Cooling { ready_at } => (definition.quorum, definition.quorum, Some(ready_at)),
                            EscalationStatus::DeniedTimeout => return Err(GuardScreenError::Store("escalation timed out immediately".into())),
                            EscalationStatus::Returned { .. } => return Err(GuardScreenError::Store("a freshly-opened escalation cannot already be returned".into())),
                        }
                    }
                    None => (0, definition.quorum, None),
                };
                Ok(EscalatedWrite::Pending(PendingApproval { escalation_id, chain, quorum, approvals_so_far, cooling_ready_at }))
            }
            Decision::Escalate { to } => Err(GuardScreenError::Escalated(format!("{to:?}"))),
        }
    }

    /// Second half of [`Self::guarded_propose_escalated_export`] -- same
    /// real, distinct-approver quorum check
    /// `guarded_confirm_escalated_action` uses, "committing" on
    /// `Approved` by running the real scan instead of a row write.
    pub fn guarded_confirm_escalated_export(&self, auth: &Auth, purpose: &str, action: Action, action_str: &str, chain: &str, escalation_id: &str, now_ms: u64) -> Result<EscalatedWrite<Vec<E>>, GuardScreenError> {
        let (quorum, status) = {
            let mut approvals = self.approvals.lock().expect("approval runtime lock poisoned");
            let definition = approvals
                .definition(chain)
                .cloned()
                .ok_or_else(|| GuardScreenError::Store(format!("chain `{chain}` has no registered approval_chain! definition")))?;
            let role = definition
                .approver_roles
                .iter()
                .find(|role| auth.has_role(role))
                .cloned()
                .ok_or_else(|| GuardScreenError::Denied(format!("no role held by `{}` is eligible to approve chain `{chain}`", auth.user())))?;
            let status = approvals.approve(escalation_id, auth.user(), &role, now_ms).map_err(|e| match e {
                ApprovalChainError::DuplicateApprover { .. } => GuardScreenError::Denied("self-review blocked: this approver already acted on this escalation".into()),
                ApprovalChainError::Expired => GuardScreenError::Denied("escalation timed out before reaching quorum (I3: timeout resolves to deny)".into()),
                other => GuardScreenError::Store(format!("{other:?}")),
            })?;
            (definition.quorum, status)
        };
        match status {
            EscalationStatus::Approved => {
                let _ = action_str; // signature symmetry with the propose half; scan_allowed derives its own wire string via action_wire_str(request.context.action)
                let request = EvalRequest { context: self.context(auth, action, purpose) };
                // Real `evaluate()`, not reused from the propose step (a
                // distinct approver's own session evaluates this, and
                // `evaluate()` is cheap/pure) -- ignoring `.decision`
                // deliberately: it will legitimately come back
                // `Escalate` again (the underlying corpus policy still
                // carries that clause), but `EvaluationResult`'s own
                // `obligations`/`residual_filter`/`caps`/`masks`/
                // `affected_row_cap` are accumulated from the SAME
                // matching-Allow-candidates union before the escalation
                // check even runs (confirmed against `evaluator::
                // evaluate`'s own source: the `Escalate` arm returns
                // those fields verbatim from that union) -- the quorum
                // having already been met (tracked in THIS table's own
                // `ApprovalChainRuntime`, not re-derived from the
                // decision) is what makes proceeding to the real scan
                // correct here, not a fabricated `Allow`.
                let evaluation = { self.client.lock().expect("guard client lock poisoned").evaluate(&request) };
                self.record_audit_decision(&request, &evaluation);
                self.scan_allowed(&request, purpose, &evaluation).map(EscalatedWrite::Committed)
            }
            EscalationStatus::Cooling { ready_at } => Ok(EscalatedWrite::Pending(PendingApproval { escalation_id: escalation_id.to_string(), chain: chain.to_string(), quorum, approvals_so_far: quorum, cooling_ready_at: Some(ready_at) })),
            EscalationStatus::Pending { approvals_so_far, .. } => Ok(EscalatedWrite::Pending(PendingApproval { escalation_id: escalation_id.to_string(), chain: chain.to_string(), quorum, approvals_so_far, cooling_ready_at: None })),
            EscalationStatus::DeniedTimeout => Err(GuardScreenError::Denied("escalation timed out before reaching quorum (I3: timeout resolves to deny)".into())),
            EscalationStatus::Returned { reason } => Err(GuardScreenError::Denied(format!("escalation was returned: {reason}"))),
        }
    }

    pub fn guarded_delete(&self, auth: &Auth, purpose: &str, row_id: &str) -> Result<(), GuardScreenError> {
        let request = EvalRequest { context: self.context(auth, Action::Delete, purpose) };
        let evaluation = { self.client.lock().expect("guard client lock poisoned").evaluate(&request) };
        self.record_audit_decision(&request, &evaluation);
        match evaluation.decision {
            Decision::Deny { reason } => Err(GuardScreenError::Denied(reason)),
            Decision::Escalate { to } => Err(GuardScreenError::Escalated(format!("{to:?}"))),
            Decision::Pending { .. } => Err(GuardScreenError::Store("write cannot be pending here".into())),
            Decision::Allow => {
                let resource = scope::storage_key(E::RESOURCE, row_id);
                let plan = PlanIr {
                    resource,
                    dataset: self.dataset.clone(),
                    filter: Some(FilterExpr::TenantEq { value: Value::Str(self.tenant.clone()) }),
                    row_scope: Some(FilterExpr::TenantEq { value: Value::Str(self.tenant.clone()) }),
                    action: WriteAction::Delete,
                    affected_row_cap: evaluation.affected_row_cap,
                    policy_version: request.context.policy_version.clone(),
                };
                let prepared = self.driver.prepare(&plan).map_err(|e| GuardScreenError::Store(format!("{e:?}")))?;
                self.driver.commit(prepared, EntityBytes(Vec::new())).map_err(|e| GuardScreenError::Store(format!("{e:?}")))?;
                Ok(())
            }
        }
    }
}

/// **Real bug, found by this batch's own `AlertRow`/`CaseRow` tests**
/// (the first entities in this crate to actually implement
/// `transition_allowed` for real — `TransactionRow` never wired it, so
/// this never fired before): every real `field(x).transition_allowed()`
/// condition in this corpus names `status` (confirmed against every
/// occurrence in `examples/rtm/src/*.nir`, and already noted by
/// `check_custom_conditions`'s own doc comment) — but this crate's
/// `guarded_insert_checked`/`commit_update` were passing the WHOLE
/// encoded row as `requested_status`, not that field's own value, so
/// `GuardedEntity::transition_allowed`'s `.as_str()` extraction always
/// saw a JSON object and always got `None`/failed. Extracts `status`
/// when the row has one, otherwise passes the value through unchanged
/// (harmless: a dataset with no `transition_allowed` override returns
/// `None` regardless of what it's handed, correctly failing closed with
/// a named Phase-B-item reason, never silently passing).
/// The wire action-string a real `guard_policy! { when action == "..." }`
/// clause uses, for every `Action` a read-shaped (`scan`/export) path in
/// this crate can be asked to evaluate under -- `governed-export`/
/// `dsar-export`'s real `action == "export"` needs this to reach
/// `matching_allow_records`'s own string comparison the same way `read`/
/// `aggregate` already do; unhandled variants fall back to `"read"`
/// rather than panicking, matching this fn's one caller's own prior
/// inline default.
fn action_wire_str(action: Action) -> &'static str {
    // `Action` has no `Copy`/`Eq` derive here worth relying on across a
    // shared reference (its own crate keeps it non-`Copy` deliberately --
    // see `nirdosha-guard-core`'s own derive list); callers pass an owned
    // clone.
    match action {
        Action::Read => "read",
        Action::Aggregate => "aggregate",
        Action::Export => "export",
        _ => "read",
    }
}

fn requested_status_value(row_json: &serde_json::Value) -> serde_json::Value {
    row_json.get("status").cloned().unwrap_or_else(|| row_json.clone())
}

fn field_names<E: serde::Serialize>(entity: &E) -> Result<HashSet<String>, GuardScreenError> {
    match serde_json::to_value(entity).map_err(|e| GuardScreenError::Store(format!("encode: {e}")))? {
        serde_json::Value::Object(map) => Ok(map.keys().cloned().collect()),
        _ => Ok(HashSet::new()),
    }
}

/// Collapses possibly-repeated `Cap` kinds (one matching `evaluate()`
/// candidate per corpus policy, several of which may legitimately match
/// the same request -- see `guarded_snapshot`'s own doc comment) down to
/// at most one of each kind, keeping the maximum value seen. `CohortFloor`
/// is the one kind where "widest" is the *minimum*, not the maximum --
/// it is a k-anonymity floor (a smaller number is the *stricter*
/// requirement), the opposite of every other cap here being a ceiling.
fn widest_caps(caps: &[Cap]) -> Vec<Cap> {
    use std::collections::HashMap;
    fn kind(c: &Cap) -> u8 {
        match c {
            Cap::RowCap(_) => 0,
            Cap::MaxScanRows(_) => 1,
            Cap::MaxScanBytes(_) => 2,
            Cap::MaxExecutionTimeMs(_) => 3,
            Cap::MaxResultBytes(_) => 4,
            Cap::CohortFloor(_) => 5,
            Cap::MaxDepth(_) => 6,
            Cap::MaxNodes(_) => 7,
        }
    }
    fn value(c: &Cap) -> u64 {
        match *c {
            Cap::RowCap(n) | Cap::MaxScanRows(n) | Cap::MaxScanBytes(n) | Cap::MaxExecutionTimeMs(n) | Cap::MaxResultBytes(n) | Cap::CohortFloor(n) | Cap::MaxDepth(n) | Cap::MaxNodes(n) => n,
        }
    }
    let mut best: HashMap<u8, Cap> = HashMap::new();
    for cap in caps {
        best.entry(kind(cap))
            .and_modify(|existing| {
                let keep_new = if matches!(cap, Cap::CohortFloor(_)) { value(cap) < value(existing) } else { value(cap) > value(existing) };
                if keep_new {
                    *existing = cap.clone();
                }
            })
            .or_insert_with(|| cap.clone());
    }
    best.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nirdosha_guard_core::evaluator::PolicyEffect;
    use nirdosha_guard_core::FieldMask;
    use nirdosha_guard_mic::MemStoreDriver;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    struct Widget {
        widget_id: String,
        tenant_id: String,
        name: String,
        // `Option`, not `String` -- T-02's `Drop` transform truly removes
        // a `field_policy { forbidden(secret_note) }` key rather than
        // masking it, so a read under such a policy must decode to
        // `None`, not error out on a missing required field (mirrors
        // `bridge.nir`'s `AlertRow::sar_linked`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        secret_note: Option<String>,
    }

    impl GuardedEntity for Widget {
        const RESOURCE: &'static str = "widget";
        fn row_id(&self) -> String { self.widget_id.clone() }
    }

    fn scratch_dir(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("guard-screens-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    fn candidate(id: &str, effect: PolicyEffect, subjects: &[&str], action: Action, purpose: Option<&str>, filter: Option<FilterExpr>, masks: Vec<FieldMask>) -> PolicyCandidate {
        PolicyCandidate {
            effect,
            subjects: subjects.iter().map(|s| s.to_string()).collect(),
            action,
            resource: Widget::RESOURCE.to_string(),
            purpose: purpose.map(str::to_string),
            conditions: vec![],
            filter,
            obligations: vec![],
            escalation: None,
            caps: vec![],
            masks,
            affected_row_cap: None,
            predicate_use: vec![],
            id: id.to_string(),
        }
    }

    fn record(id: &str, action: &str, field_policy: Vec<nirdosha_guard_core::FieldPolicy>, filter_ref: Option<&str>) -> PolicyRecord {
        PolicyRecord {
            // Empty subjects == "any subject", matching
            // `matches_context`/`matching_allow_records`'s shared
            // convention -- these fixture records exist only to carry
            // field_policy/filter_ref for whichever role each test logs
            // in as, not to test role-matching itself (the candidates
            // above already cover that).
            id: id.to_string(), effect: Effect::Allow, subjects: vec![], action: action.into(),
            resource: Widget::RESOURCE.to_string(), purpose: Some("Ops".into()), caps: vec![], affected_row_cap: None,
            obligations: vec![], escalation: None, field_policy, conditions: vec![], filter: None,
            filter_ref: filter_ref.map(str::to_string), masks: vec![], reason: None, destination: None,
            destination_denied_above: None, grants: vec![], predicate_use: vec![], count_allowed: false,
            policy_src: String::new(), line: 0,
        }
    }

    #[test]
    fn g3_tenant_scope_filter_isolates_rows_by_tenant() {
        let root = scratch_dir("g3-tenant-scope");
        let driver = MemStoreDriver::new();
        // A row belonging to a different tenant, seeded directly against
        // the raw driver (bypassing GuardedTable, which only ever writes
        // under its own construction-time tenant) -- proves the read-side
        // TenantEq filter this test is really about, not just that a
        // freshly-seeded row round-trips.
        let foreign_plan = PlanIr { resource: scope::storage_key(Widget::RESOURCE, "w-foreign"), dataset: "guarded".into(), filter: Some(FilterExpr::TenantEq { value: Value::Str("tenant-b".into()) }), row_scope: None, action: WriteAction::Create, affected_row_cap: None, policy_version: "rtm-demo".into() };
        let foreign_prepared = driver.prepare(&foreign_plan).expect("prepare foreign row");
        driver.commit(foreign_prepared, EntityBytes(serde_json::to_vec(&Widget { widget_id: "w-foreign".into(), tenant_id: "tenant-b".into(), name: "not-mine".into(), secret_note: Some("x".into()) }).unwrap())).expect("commit foreign row");

        let read_candidate = candidate("widget-read", PolicyEffect::Allow, &["Analyst"], Action::Read, Some("Ops"), None, vec![]);
        let write_candidate = candidate("widget-write", PolicyEffect::Allow, &["Ingest"], Action::Create, Some("Ops"), None, vec![]);
        let records = vec![record("widget-read", "read", vec![], Some("tenant_scope()"))];
        let table = GuardedTable::<_, Widget>::new(driver, vec![read_candidate, write_candidate], records, "test", root.join("audit.jsonl"), vec!["Analyst".into(), "Ingest".into()], "tenant-a", vec![]);

        table.guarded_insert(&Auth::login("svc", &["Ingest"]), "Ops", Widget { widget_id: "w-1".into(), tenant_id: "tenant-a".into(), name: "in-tenant".into(), secret_note: Some("x".into()) }).expect("seed in tenant-a");

        let rows = table.guarded_snapshot(&Auth::login("an", &["Analyst"]), "Ops").expect("analyst read must succeed");
        assert_eq!(rows.len(), 1, "the tenant-b row must not leak through: {rows:?}");
        assert_eq!(rows[0].widget_id, "w-1");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn g1_field_policy_forbidden_field_blocks_a_guarded_insert() {
        let root = scratch_dir("g1-field-policy");
        let driver = MemStoreDriver::new();
        let write_candidate = candidate("widget-write", PolicyEffect::Allow, &["Ingest"], Action::Create, Some("Ops"), None, vec![]);
        let records = vec![record("widget-write", "create", vec![nirdosha_guard_core::FieldPolicy::Forbidden(vec!["secret_note".into()])], None)];
        let table = GuardedTable::<_, Widget>::new(driver, vec![write_candidate], records, "test", root.join("audit.jsonl"), vec!["Ingest".into()], "tenant-a", vec![]);

        let result = table.guarded_insert(&Auth::login("svc", &["Ingest"]), "Ops", Widget { widget_id: "w-1".into(), tenant_id: "tenant-a".into(), name: "n".into(), secret_note: Some("should be forbidden".into()) });
        assert!(matches!(result, Err(GuardScreenError::FieldPolicyViolation(_))), "{result:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn g5_masking_redacts_a_field_before_it_reaches_the_typed_entity() {
        let root = scratch_dir("g5-masking");
        let driver = MemStoreDriver::new();
        let masks = vec![FieldMask { field: vec!["secret_note".into()], transform: nirdosha_guard_core::MaskTransform::Full }];
        let read_candidate = candidate("widget-read", PolicyEffect::Allow, &["Analyst"], Action::Read, Some("Ops"), None, masks);
        let write_candidate = candidate("widget-write", PolicyEffect::Allow, &["Ingest"], Action::Create, Some("Ops"), None, vec![]);
        let table = GuardedTable::<_, Widget>::new(driver, vec![read_candidate, write_candidate], vec![], "test", root.join("audit.jsonl"), vec!["Analyst".into(), "Ingest".into()], "tenant-a", vec![]);

        table.guarded_insert(&Auth::login("svc", &["Ingest"]), "Ops", Widget { widget_id: "w-1".into(), tenant_id: "tenant-a".into(), name: "n".into(), secret_note: Some("real-secret".into()) }).expect("seed");
        let rows = table.guarded_snapshot(&Auth::login("an", &["Analyst"]), "Ops").expect("read");
        assert_eq!(rows[0].secret_note, Some("[REDACTED]".to_string()));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Real bug, found wiring RTM's M21 (`cs-payment-status`/
    /// `rm-restriction-view`'s real `field_policy { forbidden(...) }` on
    /// a READ policy): before this fix, only `mask(...)`-granted
    /// `FieldMask`s reached a decoded row at all -- `field_policy`'s own
    /// `forbidden(...)` was checked on WRITE (`check_field_policy`) but
    /// never on READ, so a policy expressing "hide this field" purely via
    /// `field_policy` (no `mask(...)` clause at all, exactly
    /// `cs-payment-status`'s real shape) returned every field in the
    /// clear. See `field_policy::read_masks_from_field_policy`'s own doc
    /// comment for the fix.
    ///
    /// T-02 update: `read_masks_from_field_policy` now synthesizes
    /// `MaskTransform::Drop` for `forbidden(...)`, not `Full` -- the
    /// field must come back `None` (absent), not a `"[REDACTED]"`
    /// placeholder string (tipping-off: existence itself must be hidden).
    #[test]
    fn g1_read_side_forbidden_field_policy_is_now_dropped_not_placeholder_masked() {
        let root = scratch_dir("g1-read-field-policy");
        let driver = MemStoreDriver::new();
        let write_candidate = candidate("widget-write", PolicyEffect::Allow, &["Ingest"], Action::Create, Some("Ops"), None, vec![]);
        // No `mask(...)` on this candidate at all -- the read grant is
        // real (`evaluate()` must return `Allow`), but carries zero
        // `FieldMask`s of its own; only the `PolicyRecord`'s
        // `field_policy` says anything about `secret_note`.
        let read_candidate = candidate("widget-read", PolicyEffect::Allow, &["CsAgent"], Action::Read, Some("Ops"), None, vec![]);
        let records = vec![record("widget-read", "read", vec![nirdosha_guard_core::FieldPolicy::Allowed(vec!["name".into()]), nirdosha_guard_core::FieldPolicy::Forbidden(vec!["secret_note".into()])], None)];
        let table = GuardedTable::<_, Widget>::new(driver, vec![write_candidate, read_candidate], records, "test", root.join("audit.jsonl"), vec!["Ingest".into(), "CsAgent".into()], "tenant-a", vec![]);
        table.guarded_insert(&Auth::login("svc", &["Ingest"]), "Ops", Widget { widget_id: "w-1".into(), tenant_id: "tenant-a".into(), name: "visible".into(), secret_note: Some("must-not-leak".into()) }).expect("seed");

        let rows = table.guarded_snapshot(&Auth::login("cs", &["CsAgent"]), "Ops").expect("read must succeed (the grant itself is real)");
        assert_eq!(rows[0].name, "visible", "an allowed field must still come through untouched");
        assert_eq!(rows[0].secret_note, None, "T-02: a field_policy-forbidden field on a READ policy must be absent (Drop), not a masked placeholder or the real value: {:?}", rows[0]);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Regression for a real bug this crate's own dev caught building
    /// RTM's M6: two corpus policies (`analyst-search-transaction`,
    /// `row_cap=500`, and `analyst-open-transaction`, `row_cap=1`) can
    /// both match the identical (subject, action, resource, purpose)
    /// tuple -- `evaluate()` unions both policies' caps, and a naive
    /// first-match `RowCap` pick would silently truncate a multi-row
    /// list to one row (see `guarded_snapshot`'s own doc comment).
    #[test]
    fn guarded_snapshot_is_not_truncated_by_an_unrelated_narrower_row_cap_from_another_matching_policy() {
        let root = scratch_dir("row-cap-ambiguity");
        let driver = MemStoreDriver::new();
        let write_candidate = candidate("widget-write", PolicyEffect::Allow, &["Ingest"], Action::Create, Some("Ops"), None, vec![]);
        // Two Allow candidates matching the identical (role, action,
        // resource, purpose) tuple, one with a wide row_cap (the "list"
        // policy's intent) and one with row_cap=1 (the "detail" policy's
        // intent) -- exactly RTM's `analyst-search-transaction` /
        // `analyst-open-transaction` shape.
        let mut wide = candidate("widget-list", PolicyEffect::Allow, &["Analyst"], Action::Read, Some("Ops"), None, vec![]);
        wide.caps = vec![nirdosha_guard_core::Cap::RowCap(500)];
        let mut narrow = candidate("widget-detail", PolicyEffect::Allow, &["Analyst"], Action::Read, Some("Ops"), None, vec![]);
        narrow.caps = vec![nirdosha_guard_core::Cap::RowCap(1)];
        let table = GuardedTable::<_, Widget>::new(driver, vec![write_candidate, wide, narrow], vec![], "test", root.join("audit.jsonl"), vec!["Ingest".into(), "Analyst".into()], "tenant-a", vec![]);
        for i in 0..3 {
            table
                .guarded_insert(&Auth::login("svc", &["Ingest"]), "Ops", Widget { widget_id: format!("w-{i}"), tenant_id: "tenant-a".into(), name: "n".into(), secret_note: Some("x".into()) })
                .expect("seed");
        }

        let rows = table.guarded_snapshot(&Auth::login("an", &["Analyst"]), "Ops").expect("list read must succeed");
        assert_eq!(rows.len(), 3, "the widest matching row_cap must win for a list read, not whichever policy's cap the driver saw first: {rows:?}");
    }

    #[test]
    fn guarded_update_applies_an_allowed_field_and_leaves_others_untouched() {
        let root = scratch_dir("update-allowed");
        let driver = MemStoreDriver::new();
        let write_candidate = candidate("widget-write", PolicyEffect::Allow, &["Ingest"], Action::Create, Some("Ops"), None, vec![]);
        let update_candidate = candidate("widget-update", PolicyEffect::Allow, &["Analyst"], Action::Update, Some("Ops"), None, vec![]);
        let records = vec![record("widget-update", "update", vec![nirdosha_guard_core::FieldPolicy::Allowed(vec!["name".into()])], None)];
        let table = GuardedTable::<_, Widget>::new(driver, vec![write_candidate, update_candidate], records, "test", root.join("audit.jsonl"), vec!["Ingest".into(), "Analyst".into()], "tenant-a", vec![]);
        table.guarded_insert(&Auth::login("svc", &["Ingest"]), "Ops", Widget { widget_id: "w-1".into(), tenant_id: "tenant-a".into(), name: "old".into(), secret_note: Some("keep-me".into()) }).expect("seed");

        let changed: HashSet<String> = ["name".into()].into();
        let updated = table
            .guarded_update(&Auth::login("an", &["Analyst"]), "Ops", "w-1", &changed, |w| w.name = "new".into())
            .expect("allowed field update must succeed");
        assert_eq!(updated.name, "new");
        assert_eq!(updated.secret_note, Some("keep-me".to_string()), "an untouched field must survive the merge");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn guarded_update_rejects_a_forbidden_field_and_leaves_the_row_unchanged() {
        let root = scratch_dir("update-forbidden");
        let driver = MemStoreDriver::new();
        let write_candidate = candidate("widget-write", PolicyEffect::Allow, &["Ingest"], Action::Create, Some("Ops"), None, vec![]);
        let update_candidate = candidate("widget-update", PolicyEffect::Allow, &["Analyst"], Action::Update, Some("Ops"), None, vec![]);
        let records = vec![record("widget-update", "update", vec![nirdosha_guard_core::FieldPolicy::Forbidden(vec!["secret_note".into()])], None)];
        let table = GuardedTable::<_, Widget>::new(driver, vec![write_candidate, update_candidate], records, "test", root.join("audit.jsonl"), vec!["Ingest".into(), "Analyst".into()], "tenant-a", vec![]);
        table.guarded_insert(&Auth::login("svc", &["Ingest"]), "Ops", Widget { widget_id: "w-1".into(), tenant_id: "tenant-a".into(), name: "old".into(), secret_note: Some("original".into()) }).expect("seed");

        let changed: HashSet<String> = ["secret_note".into()].into();
        let result = table.guarded_update(&Auth::login("an", &["Analyst"]), "Ops", "w-1", &changed, |w| w.secret_note = Some("tampered".into()));
        assert!(matches!(result, Err(GuardScreenError::FieldPolicyViolation(_))), "{result:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    struct OwnedWidget {
        widget_id: String,
        tenant_id: String,
        owner_user_id: String,
        name: String,
    }

    impl GuardedEntity for OwnedWidget {
        const RESOURCE: &'static str = "owned_widget";
        fn row_id(&self) -> String { self.widget_id.clone() }
        fn subject_scope_field() -> Option<&'static str> { Some("owner_user_id") }
    }

    /// Real bug, found wiring RTM's M22 (`self-update-profile`'s real
    /// `filter subject_scope()`): the write path never checked it at
    /// all before this fix -- any subject holding `Analyst` could
    /// `guarded_update` ANY row_id, not just their own. See
    /// `commit_action`'s own doc comment for the fix.
    #[test]
    fn guarded_update_enforces_subject_scope_and_denies_editing_someone_elses_row() {
        let root = scratch_dir("write-subject-scope");
        let driver = MemStoreDriver::new();
        let mut write_candidate = candidate("owned-write", PolicyEffect::Allow, &["Ingest"], Action::Create, Some("Ops"), None, vec![]);
        write_candidate.resource = OwnedWidget::RESOURCE.to_string();
        let mut update_candidate = candidate("owned-update", PolicyEffect::Allow, &["Analyst"], Action::Update, Some("Ops"), None, vec![]);
        update_candidate.resource = OwnedWidget::RESOURCE.to_string();
        let records = vec![PolicyRecord {
            id: "owned-update".into(), effect: Effect::Allow, subjects: vec![], action: "update".into(),
            resource: OwnedWidget::RESOURCE.to_string(), purpose: Some("Ops".into()), caps: vec![], affected_row_cap: None,
            obligations: vec![], escalation: None, field_policy: vec![], conditions: vec![], filter: None,
            filter_ref: Some("subject_scope()".into()), masks: vec![], reason: None, destination: None,
            destination_denied_above: None, grants: vec![], predicate_use: vec![], count_allowed: false,
            policy_src: String::new(), line: 0,
        }];
        let table = GuardedTable::<_, OwnedWidget>::new(driver, vec![write_candidate, update_candidate], records, "test", root.join("audit.jsonl"), vec!["Ingest".into(), "Analyst".into()], "tenant-a", vec![]);
        table.guarded_insert(&Auth::login("svc", &["Ingest"]), "Ops", OwnedWidget { widget_id: "w-1".into(), tenant_id: "tenant-a".into(), owner_user_id: "alice".into(), name: "old".into() }).expect("seed");

        let changed: HashSet<String> = ["name".into()].into();
        let owner_edit = table.guarded_update(&Auth::login("alice", &["Analyst"]), "Ops", "w-1", &changed, |w| w.name = "mine".into());
        assert!(owner_edit.is_ok(), "the row's own subject must still be able to edit it: {owner_edit:?}");

        let stranger_edit = table.guarded_update(&Auth::login("bob", &["Analyst"]), "Ops", "w-1", &changed, |w| w.name = "hijacked".into());
        assert!(matches!(&stranger_edit, Err(GuardScreenError::Denied(reason)) if reason.contains("subject_scope")), "a different subject must be denied, not silently allowed to edit someone else's row: {stranger_edit:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn guarded_delete_removes_the_row() {
        let root = scratch_dir("delete");
        let driver = MemStoreDriver::new();
        let write_candidate = candidate("widget-write", PolicyEffect::Allow, &["Ingest"], Action::Create, Some("Ops"), None, vec![]);
        let read_candidate = candidate("widget-read", PolicyEffect::Allow, &["Admin"], Action::Read, Some("Ops"), None, vec![]);
        let delete_candidate = candidate("widget-delete", PolicyEffect::Allow, &["Admin"], Action::Delete, Some("Ops"), None, vec![]);
        let table = GuardedTable::<_, Widget>::new(driver, vec![write_candidate, read_candidate, delete_candidate], vec![], "test", root.join("audit.jsonl"), vec!["Ingest".into(), "Admin".into()], "tenant-a", vec![]);
        table.guarded_insert(&Auth::login("svc", &["Ingest"]), "Ops", Widget { widget_id: "w-1".into(), tenant_id: "tenant-a".into(), name: "n".into(), secret_note: Some("s".into()) }).expect("seed");
        assert_eq!(table.guarded_snapshot(&Auth::login("admin", &["Admin"]), "Ops").expect("read before delete").len(), 1);

        table.guarded_delete(&Auth::login("admin", &["Admin"]), "Ops", "w-1").expect("delete must succeed");
        assert!(table.guarded_snapshot(&Auth::login("admin", &["Admin"]), "Ops").expect("read after delete").is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn deny_by_default_when_no_policy_matches() {
        let root = scratch_dir("deny-by-default");
        let driver = MemStoreDriver::new();
        let table = GuardedTable::<_, Widget>::new(driver, vec![], vec![], "test", root.join("audit.jsonl"), vec!["Analyst".into()], "tenant-a", vec![]);
        let result = table.guarded_snapshot(&Auth::login("an", &["Analyst"]), "Ops");
        assert!(matches!(result, Err(GuardScreenError::Denied(_))), "{result:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Regression for the exact shape `80_consumers.nir`'s `lead-dashboard`
    /// needs: a role granted `action == "aggregate"` but no plain
    /// `action == "read"` at all. `guarded_snapshot` (Action::Read) must
    /// still deny by default; only `guarded_aggregate` (Action::Aggregate)
    /// may see the rows.
    #[test]
    fn guarded_aggregate_evaluates_under_the_aggregate_action_not_read() {
        let root = scratch_dir("aggregate-action");
        let driver = MemStoreDriver::new();
        let write_candidate = candidate("widget-write", PolicyEffect::Allow, &["Ingest"], Action::Create, Some("Ops"), None, vec![]);
        let aggregate_candidate = candidate("widget-aggregate", PolicyEffect::Allow, &["ComplianceLead"], Action::Aggregate, Some("Operations"), None, vec![]);
        let table = GuardedTable::<_, Widget>::new(driver, vec![write_candidate, aggregate_candidate], vec![], "test", root.join("audit.jsonl"), vec!["Ingest".into(), "ComplianceLead".into()], "tenant-a", vec![]);
        table.guarded_insert(&Auth::login("svc", &["Ingest"]), "Ops", Widget { widget_id: "w-1".into(), tenant_id: "tenant-a".into(), name: "n".into(), secret_note: Some("s".into()) }).expect("seed");

        let lead = Auth::login("lead", &["ComplianceLead"]);
        let read_result = table.guarded_snapshot(&lead, "Operations");
        assert!(matches!(read_result, Err(GuardScreenError::Denied(_))), "a role with only an aggregate grant must still be denied a plain read: {read_result:?}");

        let aggregate_result = table.guarded_aggregate(&lead, "Operations").expect("the real aggregate grant must succeed");
        assert_eq!(aggregate_result.len(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    fn case_review_chain() -> ApprovalChainDefinition {
        ApprovalChainDefinition { name: "case_review".into(), quorum: 2, approver_roles: vec!["ComplianceLead".into()], cooling_period_ms: 0 }
    }

    /// `60_case_management.nir`'s real shape: `case-close-confirm` is
    /// `for ComplianceLead`, `escalate to approval(chain case_review)`
    /// (`quorum(2, of = [ComplianceLead])`). Proposing consumes one slot;
    /// the same proposer confirming again must NOT reach quorum on their
    /// own (self-review blocked) -- `ApprovalChainRuntime::approve`'s
    /// duplicate-approver rejection is the real mechanism, mapped here to
    /// a named `Denied`, not a generic store error.
    #[test]
    fn escalated_update_blocks_self_review_and_leaves_it_pending() {
        let root = scratch_dir("escalate-self-review");
        let driver = MemStoreDriver::new();
        let write_candidate = candidate("widget-write", PolicyEffect::Allow, &["Ingest"], Action::Create, Some("Ops"), None, vec![]);
        let mut close_candidate = candidate("widget-close-confirm", PolicyEffect::Allow, &["ComplianceLead"], Action::Update, Some("Ops"), None, vec![]);
        close_candidate.escalation = Some(EscalateTarget::Approval { chain: "case_review".into() });
        let table = GuardedTable::<_, Widget>::new(driver, vec![write_candidate, close_candidate], vec![], "test", root.join("audit.jsonl"), vec!["Ingest".into(), "ComplianceLead".into()], "tenant-a", vec![case_review_chain()]);
        table.guarded_insert(&Auth::login("svc", &["Ingest"]), "Ops", Widget { widget_id: "w-1".into(), tenant_id: "tenant-a".into(), name: "old".into(), secret_note: Some("s".into()) }).expect("seed");

        let lead_a = Auth::login("lead-a", &["ComplianceLead"]);
        let changed: HashSet<String> = ["name".into()].into();
        let proposed = table.guarded_propose_escalated_update(&lead_a, "Ops", "w-1", &changed, |w| w.name = "closed".into(), 1_000).expect("propose must open a real escalation");
        let pending = match proposed {
            EscalatedWrite::Pending(p) => p,
            EscalatedWrite::Committed(_) => panic!("quorum(2) must not commit on the proposer's own approval alone"),
        };
        assert_eq!((pending.approvals_so_far, pending.quorum), (1, 2));

        let self_confirm = table.guarded_confirm_escalated_update(&lead_a, "Ops", "w-1", &pending.chain, &pending.escalation_id, &changed, |w| w.name = "closed".into(), 2_000);
        assert!(matches!(&self_confirm, Err(GuardScreenError::Denied(reason)) if reason.contains("self-review")), "{self_confirm:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// `90_ops_admin.nir`'s real shape: `refdata-migrate` is `for Admin`,
    /// `escalate to approval(chain policy_release)` (`quorum(2, of =
    /// [PolicyEngineer, ComplianceLead])`) -- `Admin` itself is NOT in
    /// the approver list at all, a genuine maker≠checker split (the
    /// proposer can never be one of the two required sign-offs). Found
    /// live wiring M13: before this fix, `guarded_propose_escalated_action`
    /// unconditionally required the proposer to hold an eligible
    /// approver role, which would have made this real corpus policy
    /// uninvokable by the one role actually granted it.
    #[test]
    fn maker_neq_checker_lets_a_non_approver_role_propose_without_self_approving() {
        let root = scratch_dir("maker-neq-checker");
        let driver = MemStoreDriver::new();
        let write_candidate = candidate("widget-write", PolicyEffect::Allow, &["Ingest"], Action::Create, Some("Ops"), None, vec![]);
        let mut migrate_candidate = candidate("widget-migrate", PolicyEffect::Allow, &["Admin"], Action::Migrate, Some("Ops"), None, vec![]);
        migrate_candidate.escalation = Some(EscalateTarget::Approval { chain: "policy_release".into() });
        let chain = ApprovalChainDefinition { name: "policy_release".into(), quorum: 2, approver_roles: vec!["PolicyEngineer".into(), "ComplianceLead".into()], cooling_period_ms: 0 };
        let table = GuardedTable::<_, Widget>::new(driver, vec![write_candidate, migrate_candidate], vec![], "test", root.join("audit.jsonl"), vec!["Ingest".into(), "Admin".into(), "PolicyEngineer".into(), "ComplianceLead".into()], "tenant-a", vec![chain]);
        table.guarded_insert(&Auth::login("svc", &["Ingest"]), "Ops", Widget { widget_id: "w-1".into(), tenant_id: "tenant-a".into(), name: "old".into(), secret_note: Some("s".into()) }).expect("seed");

        let admin = Auth::login("admin-1", &["Admin"]);
        let changed: HashSet<String> = ["name".into()].into();
        let proposed = table
            .guarded_propose_escalated_action(&admin, "Ops", Action::Migrate, "migrate", WriteAction::Migrate, "w-1", &changed, |w| w.name = "migrated".into(), 1_000)
            .expect("Admin must be able to open the escalation even though they hold no approver-eligible role");
        let pending = match proposed {
            EscalatedWrite::Pending(p) => p,
            EscalatedWrite::Committed(_) => panic!("quorum(2) must not commit on a proposal with zero approvals"),
        };
        assert_eq!((pending.approvals_so_far, pending.quorum), (0, 2), "the proposer consumed no slot: they were never eligible to fill one");

        let engineer = Auth::login("pe-1", &["PolicyEngineer"]);
        let first = table
            .guarded_confirm_escalated_action(&engineer, "Ops", Action::Migrate, "migrate", WriteAction::Migrate, "w-1", &pending.chain, &pending.escalation_id, &changed, |w| w.name = "migrated".into(), 2_000)
            .expect("first real approver");
        assert!(matches!(first, EscalatedWrite::Pending(ref p) if (p.approvals_so_far, p.quorum) == (1, 2)));

        let lead = Auth::login("lead-1", &["ComplianceLead"]);
        let second = table
            .guarded_confirm_escalated_action(&lead, "Ops", Action::Migrate, "migrate", WriteAction::Migrate, "w-1", &pending.chain, &pending.escalation_id, &changed, |w| w.name = "migrated".into(), 3_000)
            .expect("second real approver reaches quorum");
        match second {
            EscalatedWrite::Committed(row) => assert_eq!(row.name, "migrated"),
            EscalatedWrite::Pending(p) => panic!("quorum(2) with two distinct real approvers must commit: {p:?}"),
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn escalated_update_commits_once_a_second_distinct_approver_confirms() {
        let root = scratch_dir("escalate-quorum-met");
        let driver = MemStoreDriver::new();
        let write_candidate = candidate("widget-write", PolicyEffect::Allow, &["Ingest"], Action::Create, Some("Ops"), None, vec![]);
        let read_candidate = candidate("widget-read", PolicyEffect::Allow, &["ComplianceLead"], Action::Read, Some("Ops"), None, vec![]);
        let mut close_candidate = candidate("widget-close-confirm", PolicyEffect::Allow, &["ComplianceLead"], Action::Update, Some("Ops"), None, vec![]);
        close_candidate.escalation = Some(EscalateTarget::Approval { chain: "case_review".into() });
        let table = GuardedTable::<_, Widget>::new(driver, vec![write_candidate, read_candidate, close_candidate], vec![], "test", root.join("audit.jsonl"), vec!["Ingest".into(), "ComplianceLead".into()], "tenant-a", vec![case_review_chain()]);
        table.guarded_insert(&Auth::login("svc", &["Ingest"]), "Ops", Widget { widget_id: "w-1".into(), tenant_id: "tenant-a".into(), name: "old".into(), secret_note: Some("s".into()) }).expect("seed");

        let lead_a = Auth::login("lead-a", &["ComplianceLead"]);
        let lead_b = Auth::login("lead-b", &["ComplianceLead"]);
        let changed: HashSet<String> = ["name".into()].into();
        let pending = match table.guarded_propose_escalated_update(&lead_a, "Ops", "w-1", &changed, |w| w.name = "closed".into(), 1_000).expect("propose") {
            EscalatedWrite::Pending(p) => p,
            EscalatedWrite::Committed(_) => panic!("must be pending after one approval"),
        };

        let confirmed = table
            .guarded_confirm_escalated_update(&lead_b, "Ops", "w-1", &pending.chain, &pending.escalation_id, &changed, |w| w.name = "closed".into(), 2_000)
            .expect("a second, distinct ComplianceLead must reach quorum and commit");
        match confirmed {
            EscalatedWrite::Committed(row) => assert_eq!(row.name, "closed"),
            EscalatedWrite::Pending(p) => panic!("quorum(2) with two distinct approvers must commit: {p:?}"),
        }

        let rows = table.guarded_snapshot(&lead_a, "Ops").expect("read back after commit");
        assert_eq!(rows[0].name, "closed", "the committed row must actually be persisted, not just returned in-memory: {rows:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    fn case_review_chain_with_cooling() -> ApprovalChainDefinition {
        ApprovalChainDefinition { name: "case_review".into(), quorum: 2, approver_roles: vec!["ComplianceLead".into()], cooling_period_ms: 1_000 }
    }

    /// T-04/B10: a chain with `cooling(...)` doesn't commit the instant
    /// quorum is reached -- `guarded_confirm_escalated_update` surfaces
    /// `Cooling` as `Pending` (real, not a silent auto-approve), and only
    /// `guarded_finalize_escalated_action`, called once the window has
    /// genuinely elapsed, commits.
    #[test]
    fn escalated_update_with_cooling_stays_pending_until_finalized_after_the_window() {
        let root = scratch_dir("escalate-cooling");
        let driver = MemStoreDriver::new();
        let write_candidate = candidate("widget-write", PolicyEffect::Allow, &["Ingest"], Action::Create, Some("Ops"), None, vec![]);
        let mut close_candidate = candidate("widget-close-confirm", PolicyEffect::Allow, &["ComplianceLead"], Action::Update, Some("Ops"), None, vec![]);
        close_candidate.escalation = Some(EscalateTarget::Approval { chain: "case_review".into() });
        let table = GuardedTable::<_, Widget>::new(driver, vec![write_candidate, close_candidate], vec![], "test", root.join("audit.jsonl"), vec!["Ingest".into(), "ComplianceLead".into()], "tenant-a", vec![case_review_chain_with_cooling()]);
        table.guarded_insert(&Auth::login("svc", &["Ingest"]), "Ops", Widget { widget_id: "w-1".into(), tenant_id: "tenant-a".into(), name: "old".into(), secret_note: Some("s".into()) }).expect("seed");

        let lead_a = Auth::login("lead-a", &["ComplianceLead"]);
        let lead_b = Auth::login("lead-b", &["ComplianceLead"]);
        let changed: HashSet<String> = ["name".into()].into();
        let pending = match table.guarded_propose_escalated_update(&lead_a, "Ops", "w-1", &changed, |w| w.name = "closed".into(), 1_000).expect("propose") {
            EscalatedWrite::Pending(p) => p,
            EscalatedWrite::Committed(_) => panic!("must be pending after one approval"),
        };

        let after_quorum = table
            .guarded_confirm_escalated_update(&lead_b, "Ops", "w-1", &pending.chain, &pending.escalation_id, &changed, |w| w.name = "closed".into(), 2_000)
            .expect("quorum reached");
        match after_quorum {
            EscalatedWrite::Pending(p) => assert_eq!(p.cooling_ready_at, Some(3_000), "quorum(2) at t=2000 + cooling(1000ms)"),
            EscalatedWrite::Committed(row) => panic!("cooling(...) must not commit on the same call quorum was reached: {row:?}"),
        }

        let too_early = table
            .guarded_finalize_escalated_action(&lead_a, "Ops", Action::Update, "update", WriteAction::Update, "w-1", &pending.chain, &pending.escalation_id, &changed, |w| w.name = "closed".into(), 2_500)
            .expect("finalize before ready_at");
        assert!(matches!(too_early, EscalatedWrite::Pending(_)), "still cooling at t=2500 (ready_at=3000): {too_early:?}");

        let finalized = table
            .guarded_finalize_escalated_action(&lead_a, "Ops", Action::Update, "update", WriteAction::Update, "w-1", &pending.chain, &pending.escalation_id, &changed, |w| w.name = "closed".into(), 3_000)
            .expect("finalize at ready_at");
        match finalized {
            EscalatedWrite::Committed(row) => assert_eq!(row.name, "closed"),
            EscalatedWrite::Pending(p) => panic!("cooling window elapsed at t=3000; finalize must commit: {p:?}"),
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// T-04/B10: `guarded_return_escalated` is real and generic -- an
    /// eligible approver returns a cooling escalation with a mandatory
    /// reason, and it never reaches `Approved` even once the clock would
    /// otherwise have finalized it.
    #[test]
    fn guarded_return_escalated_blocks_finalize_and_requires_a_real_reason() {
        let root = scratch_dir("escalate-return");
        let driver = MemStoreDriver::new();
        let write_candidate = candidate("widget-write", PolicyEffect::Allow, &["Ingest"], Action::Create, Some("Ops"), None, vec![]);
        let mut close_candidate = candidate("widget-close-confirm", PolicyEffect::Allow, &["ComplianceLead"], Action::Update, Some("Ops"), None, vec![]);
        close_candidate.escalation = Some(EscalateTarget::Approval { chain: "case_review".into() });
        let table = GuardedTable::<_, Widget>::new(driver, vec![write_candidate, close_candidate], vec![], "test", root.join("audit.jsonl"), vec!["Ingest".into(), "ComplianceLead".into()], "tenant-a", vec![case_review_chain_with_cooling()]);
        table.guarded_insert(&Auth::login("svc", &["Ingest"]), "Ops", Widget { widget_id: "w-1".into(), tenant_id: "tenant-a".into(), name: "old".into(), secret_note: Some("s".into()) }).expect("seed");

        let lead_a = Auth::login("lead-a", &["ComplianceLead"]);
        let lead_b = Auth::login("lead-b", &["ComplianceLead"]);
        let changed: HashSet<String> = ["name".into()].into();
        let pending = match table.guarded_propose_escalated_update(&lead_a, "Ops", "w-1", &changed, |w| w.name = "closed".into(), 1_000).expect("propose") {
            EscalatedWrite::Pending(p) => p,
            EscalatedWrite::Committed(_) => panic!("must be pending after one approval"),
        };
        table.guarded_confirm_escalated_update(&lead_b, "Ops", "w-1", &pending.chain, &pending.escalation_id, &changed, |w| w.name = "closed".into(), 2_000).expect("quorum reached, now cooling");

        let empty_reason = table.guarded_return_escalated(&lead_a, &pending.chain, &pending.escalation_id, "   ", 2_100);
        assert_eq!(empty_reason, Err(GuardScreenError::Denied("a return requires a non-empty reason".into())));

        table.guarded_return_escalated(&lead_a, &pending.chain, &pending.escalation_id, "wrong widget targeted", 2_200).expect("real reason returns it");

        let after_return = table
            .guarded_finalize_escalated_action(&lead_b, "Ops", Action::Update, "update", WriteAction::Update, "w-1", &pending.chain, &pending.escalation_id, &changed, |w| w.name = "closed".into(), 5_000)
            .expect_err("a returned escalation must never finalize to Approved, even long past the original ready_at");
        assert!(matches!(after_return, GuardScreenError::Denied(_)), "{after_return:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// T-04/B10: `list_pending_approvals` -- the real per-table read
    /// `approval_inbox!`'s merged cross-entity view is built from.
    #[test]
    fn list_pending_approvals_reflects_cooling_and_resolved_state() {
        let root = scratch_dir("list-pending");
        let driver = MemStoreDriver::new();
        let write_candidate = candidate("widget-write", PolicyEffect::Allow, &["Ingest"], Action::Create, Some("Ops"), None, vec![]);
        let mut close_candidate = candidate("widget-close-confirm", PolicyEffect::Allow, &["ComplianceLead"], Action::Update, Some("Ops"), None, vec![]);
        close_candidate.escalation = Some(EscalateTarget::Approval { chain: "case_review".into() });
        let table = GuardedTable::<_, Widget>::new(driver, vec![write_candidate, close_candidate], vec![], "test", root.join("audit.jsonl"), vec!["Ingest".into(), "ComplianceLead".into()], "tenant-a", vec![case_review_chain_with_cooling()]);
        table.guarded_insert(&Auth::login("svc", &["Ingest"]), "Ops", Widget { widget_id: "w-1".into(), tenant_id: "tenant-a".into(), name: "old".into(), secret_note: Some("s".into()) }).expect("seed");

        let lead_a = Auth::login("lead-a", &["ComplianceLead"]);
        let lead_b = Auth::login("lead-b", &["ComplianceLead"]);
        let changed: HashSet<String> = ["name".into()].into();
        let pending = match table.guarded_propose_escalated_update(&lead_a, "Ops", "w-1", &changed, |w| w.name = "closed".into(), 1_000).expect("propose") {
            EscalatedWrite::Pending(p) => p,
            EscalatedWrite::Committed(_) => panic!("must be pending"),
        };
        let before_quorum = table.list_pending_approvals();
        assert_eq!(before_quorum.len(), 1);
        assert_eq!(before_quorum[0].status, "pending");
        assert_eq!(before_quorum[0].proposer, "lead-a");

        table.guarded_confirm_escalated_update(&lead_b, "Ops", "w-1", &pending.chain, &pending.escalation_id, &changed, |w| w.name = "closed".into(), 2_000).expect("quorum reached");
        let cooling = table.list_pending_approvals();
        assert_eq!(cooling[0].status, "cooling");
        assert_eq!(cooling[0].cooling_ready_at, Some(3_000));
        assert_eq!(cooling[0].approver_ids, vec!["lead-a".to_string(), "lead-b".to_string()]);

        table.guarded_finalize_escalated_action(&lead_a, "Ops", Action::Update, "update", WriteAction::Update, "w-1", &pending.chain, &pending.escalation_id, &changed, |w| w.name = "closed".into(), 3_000).expect("finalize");
        let resolved = table.list_pending_approvals();
        assert_eq!(resolved[0].status, "approved");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn obligate_notify_fires_the_registered_hook_on_a_real_committed_write() {
        let root = scratch_dir("obligate-notify");
        let driver = MemStoreDriver::new();
        let write_candidate = candidate("widget-write", PolicyEffect::Allow, &["Ingest"], Action::Create, Some("Ops"), None, vec![]);
        let mut write_record = record("widget-write", "create", vec![], None);
        write_record.obligations = vec![Obligation::Notify { channel: "test-inbox".into() }];
        let seen: std::sync::Arc<Mutex<Vec<(String, String)>>> = std::sync::Arc::new(Mutex::new(Vec::new()));
        let seen_for_hook = seen.clone();
        let table = GuardedTable::<_, Widget>::new(driver, vec![write_candidate], vec![write_record], "test", root.join("audit.jsonl"), vec!["Ingest".into()], "tenant-a", vec![])
            .with_notify_hook(move |channel, body_ref| seen_for_hook.lock().unwrap().push((channel.to_string(), body_ref.to_string())));

        table.guarded_insert(&Auth::login("svc", &["Ingest"]), "Ops", Widget { widget_id: "w-1".into(), tenant_id: "tenant-a".into(), name: "n".into(), secret_note: Some("s".into()) }).expect("insert");

        let fired = seen.lock().unwrap();
        assert_eq!(*fired, vec![("test-inbox".to_string(), "widget:w-1".to_string())], "a real Obligation::Notify from a matching PolicyRecord must reach the registered hook exactly once, with the real resource:row_id body_ref: {fired:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_write_with_no_notify_obligation_never_calls_the_hook() {
        let root = scratch_dir("no-notify");
        let driver = MemStoreDriver::new();
        let write_candidate = candidate("widget-write", PolicyEffect::Allow, &["Ingest"], Action::Create, Some("Ops"), None, vec![]);
        let seen: std::sync::Arc<Mutex<Vec<(String, String)>>> = std::sync::Arc::new(Mutex::new(Vec::new()));
        let seen_for_hook = seen.clone();
        let table = GuardedTable::<_, Widget>::new(driver, vec![write_candidate], vec![], "test", root.join("audit.jsonl"), vec!["Ingest".into()], "tenant-a", vec![])
            .with_notify_hook(move |channel, body_ref| seen_for_hook.lock().unwrap().push((channel.to_string(), body_ref.to_string())));

        table.guarded_insert(&Auth::login("svc", &["Ingest"]), "Ops", Widget { widget_id: "w-1".into(), tenant_id: "tenant-a".into(), name: "n".into(), secret_note: Some("s".into()) }).expect("insert");

        assert!(seen.lock().unwrap().is_empty(), "no PolicyRecord carried a Notify obligation, so the hook must not fire");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn system_write_and_system_scan_round_trip_with_no_policy_evaluation_at_all() {
        let root = scratch_dir("system-write-scan");
        let driver = MemStoreDriver::new();
        // No candidates/records at all -- proves this path genuinely
        // never consults `evaluate()`, matching `notification`'s real
        // corpus shape (no `guard_policy!` names that resource).
        let table = GuardedTable::<_, Widget>::new(driver, vec![], vec![], "test", root.join("audit.jsonl"), vec![], "tenant-a", vec![]);
        table.system_write("tenant-a", &Widget { widget_id: "sys-1".into(), tenant_id: "tenant-a".into(), name: "n".into(), secret_note: Some("s".into()) });
        let rows = table.system_scan().expect("system_scan needs no auth/policy to succeed");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].widget_id, "sys-1");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn escalated_export_blocks_self_review_and_commits_once_a_second_distinct_approver_confirms() {
        let root = scratch_dir("escalated-export");
        let driver = MemStoreDriver::new();
        let write_candidate = candidate("widget-write", PolicyEffect::Allow, &["Ingest"], Action::Create, Some("Ops"), None, vec![]);
        let mut export_candidate = candidate("widget-export", PolicyEffect::Allow, &["Analyst"], Action::Export, Some("Ops"), None, vec![]);
        export_candidate.escalation = Some(EscalateTarget::Approval { chain: "egress_release".into() });
        let egress_chain = ApprovalChainDefinition { name: "egress_release".into(), quorum: 2, approver_roles: vec!["ComplianceLead".into()], cooling_period_ms: 0 };
        let table = GuardedTable::<_, Widget>::new(driver, vec![write_candidate, export_candidate], vec![], "test", root.join("audit.jsonl"), vec!["Ingest".into(), "Analyst".into(), "ComplianceLead".into()], "tenant-a", vec![egress_chain]);
        table.guarded_insert(&Auth::login("svc", &["Ingest"]), "Ops", Widget { widget_id: "w-1".into(), tenant_id: "tenant-a".into(), name: "n".into(), secret_note: Some("s".into()) }).expect("seed");

        // Analyst proposes but holds no `egress_release`-eligible role --
        // real maker≠checker, same shape `refdata-migrate` exercises.
        let analyst = Auth::login("an", &["Analyst"]);
        let pending = match table.guarded_propose_escalated_export(&analyst, "Ops", Action::Export, "export", 1_000).expect("propose") {
            EscalatedWrite::Pending(p) => p,
            EscalatedWrite::Committed(rows) => panic!("must be pending, zero approvals yet: {rows:?}"),
        };
        assert_eq!(pending.approvals_so_far, 0);

        let lead = Auth::login("lead", &["ComplianceLead"]);
        // Self-review block: the SAME approver confirming twice must not
        // reach quorum(2) alone.
        let still_pending = match table.guarded_confirm_escalated_export(&lead, "Ops", Action::Export, "export", &pending.chain, &pending.escalation_id, 2_000).expect("first confirm") {
            EscalatedWrite::Pending(p) => p,
            EscalatedWrite::Committed(rows) => panic!("quorum(2) with one approver must still be pending: {rows:?}"),
        };
        let lead_b = Auth::login("lead-b", &["ComplianceLead"]);
        let committed = table.guarded_confirm_escalated_export(&lead_b, "Ops", Action::Export, "export", &still_pending.chain, &still_pending.escalation_id, 3_000).expect("second, distinct approver reaches quorum");
        match committed {
            EscalatedWrite::Committed(rows) => {
                assert_eq!(rows.len(), 1, "a real scan must run once quorum is met, returning the actual exportable rows: {rows:?}");
                assert_eq!(rows[0].widget_id, "w-1");
            }
            EscalatedWrite::Pending(p) => panic!("quorum(2) with two distinct approvers must commit: {p:?}"),
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// `sar-export`-shaped regression: a matching Allow record's
    /// `Condition::Expr` (`requires field(status) == "confirmed_fraud"`)
    /// must narrow a scan/export to only the rows that satisfy it, not
    /// leak every row regardless of state.
    #[test]
    fn condition_expr_on_a_read_policy_narrows_a_scan_to_matching_rows_only() {
        let root = scratch_dir("condition-filter-scan");
        let driver = MemStoreDriver::new();
        let mut read_candidate = candidate("widget-read", PolicyEffect::Allow, &["Analyst"], Action::Read, Some("Ops"), None, vec![]);
        read_candidate.conditions = vec![nirdosha_guard_core::Condition::Expr(FilterExpr::Eq { field: vec!["name".into()], value: Value::Str("eligible".into()) })];
        let write_candidate = candidate("widget-write", PolicyEffect::Allow, &["Ingest"], Action::Create, Some("Ops"), None, vec![]);
        let mut read_record = record("widget-read", "read", vec![], None);
        read_record.conditions = vec![nirdosha_guard_core::Condition::Expr(FilterExpr::Eq { field: vec!["name".into()], value: Value::Str("eligible".into()) })];
        let table = GuardedTable::<_, Widget>::new(driver, vec![read_candidate, write_candidate], vec![read_record], "test", root.join("audit.jsonl"), vec!["Analyst".into(), "Ingest".into()], "tenant-a", vec![]);

        let svc = Auth::login("svc", &["Ingest"]);
        table.guarded_insert(&svc, "Ops", Widget { widget_id: "w-yes".into(), tenant_id: "tenant-a".into(), name: "eligible".into(), secret_note: Some("x".into()) }).expect("seed eligible row");
        table.guarded_insert(&svc, "Ops", Widget { widget_id: "w-no".into(), tenant_id: "tenant-a".into(), name: "not-yet".into(), secret_note: Some("x".into()) }).expect("seed ineligible row");

        let rows = table.guarded_snapshot(&Auth::login("an", &["Analyst"]), "Ops").expect("read must succeed");
        assert_eq!(rows.len(), 1, "only the row satisfying the condition should be returned: {rows:?}");
        assert_eq!(rows[0].widget_id, "w-yes");
        let _ = std::fs::remove_dir_all(&root);
    }
}
