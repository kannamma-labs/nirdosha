//! CTMS (Continuous Transaction Monitoring System).
//!
//! RFC 0029 treats CTMS as continuous *monitoring*: "the system detects and
//! records suspicious patterns within declared observation and response
//! bounds; it does not automatically prove that every suspicious
//! transaction was prevented or observed." Nothing in this crate claims
//! otherwise.
//!
//! ```text
//! synthetic Kafka event -> velocity rule -> AI model signal (advisory)
//!   -> alert.create -> RiskAnalyst alert queue -> case assignment
//!   -> RiskAnalyst escalation -> SeniorInvestigator review
//!   -> severity-routed disposition -> SAR draft -> compliance review
//!   -> MLRO approval -> mock regulator submission -> evidence/audit record
//! ```
//!
//! **Nothing in this crate hardcodes who is authorized to do what, or what
//! counts as suspicious.** Both are policy data an org supplies, loaded
//! from TOML rather than compiled into Rust `match`/`const` -- see
//! [`policy`] (case/SAR-workflow role routing) and [`rules`] (rule
//! thresholds).
//!
//! ## Modules
//!
//! - [`event`] -- one canonical `TransactionEvent` shape every source
//!   instrument normalizes into.
//! - [`stream`] -- a demo-mode, in-process stream adapter implementing the
//!   exact same [`nirdosha_ingestion_topic_kafka::StreamSource`]/
//!   [`nirdosha_ingestion_topic_kafka::StreamSink`] port the workspace's
//!   real `rskafka`-backed `KafkaTopicDriver` implements -- proven, not just
//!   claimed, against a live local Redpanda broker
//!   (`stream::tests::the_same_stream_adapter_processes_records_from_a_real_kafka_protocol_broker`,
//!   `#[ignore]`d by default since this sandbox has no broker running
//!   unless `docker-compose.dev.yml`'s `redpanda` service is up). Demonstrates
//!   dedup, malformed-record rejection, offset/partition recording,
//!   late-event detection, and consumer-restart replay.
//! - [`rules`] -- one deterministic rule shape (`velocity_24h` is this
//!   crate's own reference instance); thresholds load from a
//!   [`rules::RuleCatalog`] TOML, not a Rust constant.
//! - [`batch`] -- [`batch::run_batch`] replays a [`nirdosha_ingestion_topic_kafka::RawBatch`]
//!   through the *same* stream adapter, rule engine, and `alert.create`
//!   gateway realtime uses (`origin = "batch"`), idempotent both by
//!   `run_id` and by the alert idempotency key shared with realtime.
//! - [`model`] -- a real, local ONNX model (`nirdosha-scoring-model-onnx`)
//!   scoring this crate's own honestly-available features (own fixture,
//!   `scripts/gen_ctms_onnx.py`), attached to an alert as purely advisory
//!   metadata -- never consulted by any gateway's authorization decision.
//! - [`facts`] -- RFC 0029 §17 fact providers (KYC status, account status)
//!   with real issuer/freshness/revocation/disagreement/timeout handling,
//!   reusing `nirdosha_guard_rfc0029::FactEnvelope`. One honest in-memory
//!   implementation; no live vendor API (disclosed, not silently missing).
//! - [`alert`] -- `TransactionAlert` + the `alert.create` effect gateway.
//! - [`case`] -- `MonitoringCase` and the `case.assign`/`case.escalate`/
//!   `case.senior_review`/`case.disposition` effect gateways: a bounded
//!   escalation chain (`New -> Assigned -> Escalated -> SeniorReviewed ->
//!   {FalsePositive | SuspiciousActivity}`), state-invariant-protected
//!   (each step refuses to run out of order). Separation-of-duty rules
//!   ("an analyst cannot approve their own disposition", "only the
//!   assigned analyst may escalate their own case") are each enforced
//!   twice: a role gate at mint time (`service`, driven by
//!   [`policy::CaseWorkflowPolicy`]) and a same-actor check in the store
//!   itself.
//! - [`sar`] -- SAR `Draft -> ComplianceReviewed -> MlroApproved ->
//!   Submitted` (plus one correction/resubmission step), only ever for a
//!   case dispositioned `SuspiciousActivity`. Submission is real but mock:
//!   a genuine goAML XML envelope
//!   (`nirdosha_egress_report_goaml::ReportEnvelope`), validated against
//!   the real XSD schema, written to a local outbox -- never a real
//!   regulatory filing.
//! - [`policy`] -- [`policy::CaseWorkflowPolicy`]/[`policy::SarWorkflowPolicy`]:
//!   which role each workflow step requires, including the severity->role
//!   disposition routing table, loaded from TOML. An org supplies its own
//!   instead of using the crate's `default_v1()`.
//! - [`service`] -- role-gated facades (`assign_case`/`escalate_case`/
//!   `senior_review_case`/`disposition_case`) standing in for what a
//!   generated screen's capability gate does; [`sar`] has its own
//!   equivalent facades.
//! - [`durable_store`] -- `SqliteAlertStore`/`SqliteCaseStore`: real,
//!   restart-surviving implementations of `AlertStore`/`CaseStore`,
//!   swappable for the in-memory ones with no gateway changes.
//! - [`obligations`] -- a durable (SQLite-backed), restart-surviving
//!   post-commit obligation queue (e.g. "a high-severity alert needs
//!   someone to look at it"), atomically claimable by exactly one worker.
//! - [`health`] -- `PipelineHealthTracker`: consumer lag / malformed /
//!   late / duplicate counters and a degraded-status heuristic. A metrics
//!   snapshot, not an operational SLA/paging guarantee (this crate has no
//!   alerting infrastructure to page anyone).
//! - [`verify`] -- an automated static check that this crate's own source
//!   never calls a store's mutating method except from inside its
//!   guarding gateway (modeled on `nirdosha_guard_verify`'s `VerifyPass`
//!   shape). Scope stated exactly: proves this crate doesn't bypass
//!   itself; cannot prove an external crate importing the traits directly
//!   won't.
//!
//! `nirdosha_guard_rfc0029` itself gained, this pass, opt-in Ed25519
//! capability signing and detached policy-bundle signature verification
//! against an `AuthorityRegistry` (`authority` module there) -- additive,
//! every existing call site unaffected; see that crate's own module doc.
//!
//! ## Screen/`cargo-nirdosha` codegen wiring
//!
//! Real, not just planned: `crates/nirdosha-hi/templates/fintech/modules/transaction_monitoring/screens.toml`
//! screens 9.3 ("Alert Queue", `alert_create`) and 9.4 ("Case Assignment",
//! `case_assign`) are capability-gated by [`alert::CtmsAlertGatewayV1`]/
//! [`case::CtmsCaseAssignGatewayV1`] through `cargo-nirdosha`'s
//! `render_capability_gate`, generalized this pass beyond
//! `transfer_request_gateway_v1` to a small `(gateway, effect) -> Rust
//! type` table. Proven end to end, screen-to-gateway, over real HTTP
//! dispatch, by
//! `nirdosha_hi::hi_composer::tests::ctms_alert_and_case_screens_live_demo_screen_to_gateway`
//! (valid create, row round-trip, evidence write, replay rejection, wrong
//! role rejection -- for both effects). These two screens' own row
//! storage is still the generic one-row-per-resource table
//! `crud_screens!` always uses (identical to `transfer_create`'s own
//! division of responsibility): the gateway is the authorization +
//! evidence layer, [`alert::AlertStore`]/[`case::CaseStore`]'s own richer
//! domain semantics (idempotent creation, the escalation state machine)
//! are proven separately by this crate's own tests, not yet the literal
//! backing store for those two generated routes. `case.escalate`/
//! `case.senior_review`/`case.disposition`/SAR effects have Rust-type
//! wiring ready in `cargo-nirdosha` but no screen references them yet.
//!
//! ## What remains structurally out of reach here (disclosed, not silently missing)
//!
//! - **Real Kafka deployment**: single-partition round-trip against a
//!   live broker is proven (see [`stream`] above); consumer groups,
//!   partition ownership, rebalance, and multi-process coordination are
//!   not -- `rskafka` (this workspace's chosen driver) is a low-level
//!   per-partition client with no consumer-group protocol to wire.
//! - **External KYC/banking APIs**: [`facts`] gives every fact a real
//!   provenance envelope; there is no live vendor connection to attach it
//!   to.
//! - **Full RFC 0029 production policy pipeline**: this crate's
//!   `(gateway, effect)` capability gates are real, but the deeper chain
//!   (typed RFC 0029 IR -> full admission report -> a general PDP/PIP/PEP
//!   runtime) beyond what `cargo-nirdosha`'s existing admission engine
//!   already does is not built here.
//! - **Production key custody**: `AuthorityRegistry` verifies signatures
//!   against configured public keys; it says nothing about HSM/KMS-backed
//!   signing ceremonies or real multi-party authority onboarding.
//! - **Production guarantee artifacts and monitor-health SLAs**: these
//!   are organizational sign-off/on-call artifacts, not something a crate
//!   can honestly emit as code.
//! - **Distributed finality / multi-service workflow semantics**: would
//!   require a real, separately-deployed multi-service system to
//!   coordinate across (2PC/saga, etc.) -- there is exactly one process
//!   here, so this cannot be honestly demonstrated in this repository as
//!   it stands.

pub mod alert;
pub mod batch;
pub mod case;
pub mod durable_store;
pub mod event;
pub mod facts;
pub mod health;
pub mod model;
pub mod obligations;
pub mod policy;
pub mod rules;
pub mod sar;
pub mod service;
pub mod stream;
pub mod verify;
