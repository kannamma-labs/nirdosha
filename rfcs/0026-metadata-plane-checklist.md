# RFC 0026 — Metadata Plane Implementation Checklist

This checklist records verified implementation status for RFC 0026 and its
required RFC 0023/0024/0025 substrate. Items move to `[DONE]` only after the
touched crates compile and their focused tests pass.

Status legend: `[DONE]`, `[PARTIAL]`, `[OPEN]`, `[BLOCKED: reason]`.

Last updated: 2026-09-19.

## Hygiene

- `[DONE]` Remove duplicate `nirdosha-lineage-store-embedded` and
  `nirdosha-lineage-store-remote` workspace members.
- `[DONE]` Create all crates listed by the workspace manifest and keep Cargo
  metadata/loadability green across all implementation phases.

## Phase A — Substrate foundations

### A1. Guard-core behavior

- `[DONE]` Decision and fragment caches with policy-safe cache admission.
- `[DONE]` Guard-down configuration, kill switch, degraded audit, and
  reconciliation inbox.
- `[DONE]` Delegation credentials, scope checks, expiry/revocation, and
  reaper.
- `[DONE]` Break-glass grants, dual approval, ledger, and review tasks.
- `[DONE]` Policy snapshots and change callbacks.
- `[DONE]` Relation lowering tiers, negative-relation rejection, and the
  exact-match deny-by-default evaluator.

### A2. Audit envelope

- `[DONE]` Record kinds and RFC 0025-compatible audit envelope.
- `[DONE]` Module audit chain adapter and chain reconciliation.

### A3. Guard registry

- `[DONE]` Registry record types and distributed-slice declarations.
- `[DONE]` Registry JSON dump and coverage matrix.
- `[DONE]` Cross-crate `linkme` collection proof.

## Phase B — Guard macros

- `[DONE]` Real policy matcher parsing and attribute catalog statics;
  full clause lowering and slice-fed macro registration.
- `[DONE]` Roles, workflows, approval chains, break-glass, and audit macros.
- `[DONE]` Positive and negative macro tests.

## Phase C — Guard MIC

- `[DONE]` Guard client, context, evaluation, and cache integration.
- `[DONE]` Synchronous StoreDriver SPI and receipts.
- `[DONE]` Guarded apply, audit-before-commit ordering, idempotency, and unforgeable plans.
- `[DONE]` In-memory driver; L1 net, write caps, masks, and federation.

## Phase D — Lineage emission

- `[DONE]` Lineage observation, projection, GraphStore port, and tokenized
  driver facts.
- `[DONE]` MIC emission hook and I18 collection for single-store applies,
  federated/degraded observations, MP-9 decision equivalence.

## Phase E — Verification

- `[DONE]` V1–V8 pass engine and owned registry view.
- `[DONE]` `cargo nirdosha verify --guard --registry-json <path>` plumbing.

## Phase F — Conformance and load shedding

- `[DONE]` Conformance probes, controlled clocks, and kill-switch probes.
- `[DONE]` Admission controller with non-sheddable audit/L1 work and
  conformance verification.

## Phase G — Guard MCP

- `[DONE]` Fail-closed evaluation and agent-safe defaults: subject/policy/
  destination binding, confidential-data denial, full audit, and row cap.
- `[DONE]` Delegation, submit-write gate, dual audit streams, and registry tools.

## Phase H — Lineage plane completion

- `[DONE]` Query vocabulary and deterministic projection.
- `[DONE]` Query macro expansion (`lineage_query!`) and composition macros.
- `[DONE]` Cursor projection service, declared graph join, and V10 replay pure checker.
- `[DONE]` Remote GraphStore transport and two LineageSink drivers (JSONL and HTTP).

## Phase I — Proof and documentation

- `[DONE]` End-to-end guarded apply, audit replay, and lineage proof (`crates/nirdosha-rt/tests/e2e_data_guard.rs`).
- `[DONE]` Update RFC 0023/0025 checklist and amendment notes.
- `[DONE]` Full workspace tests and final conformance proof.

## Verified deferrals

- `[OPEN]` Live vendor capability attestation and production store drivers.
- `[OPEN]` Cedar frontend integration and Gate-3 rustc metadata collection.
- `[OPEN]` Async/networked policy store and MIC uplift.
- `[OPEN]` Guarded `link_external_lineage` admission for external claims.