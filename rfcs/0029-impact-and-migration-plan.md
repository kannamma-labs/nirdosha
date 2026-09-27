# RFC 0029 — Responsibility Impact and Migration Plan

```
Companion to:  RFC 0029 — Domain-neutral policy admission, enforcement,
               and evidence
Status:        Planning document — no implementation claimed
Created:       2026-09-26
Purpose:       Identify the RFC, documentation, and implementation boundaries
               that must change if RFC 0029 is adopted and implemented
```

## 1. Why this document exists

RFC 0029 introduces an umbrella policy architecture. Several older RFCs
currently define local versions of policy admission, guarantee strength,
coverage, evidence, trust, or enforcement. Those implementations and designs
remain valuable, but their architectural responsibilities must be made
explicit to avoid multiple incompatible meanings of “policy,” “covered,”
“atomic,” “verified,” or “evidence.”

This document is the migration map. It answers:

- which RFCs lose generic responsibility to RFC 0029;
- which responsibilities each RFC retains;
- which RFCs merely need alignment;
- which source crates are probable implementation surfaces;
- how to sequence the migration without claiming unimplemented guarantees;
- how to decide that the responsibility migration is complete.

Landing RFC 0029 or this document does not itself amend older RFCs and does
not change runtime behavior. Every responsibility transfer below requires an
explicit amendment and, where applicable, implementation evidence.

## 2. Target responsibility hierarchy

```text
RFC 0029 — universal policy semantics and guarantee boundary
    │
    ├── RFC 0016 — domain-module packaging and mandatory invariants
    ├── RFC 0017 — guarantee artifact and certificate transport
    ├── RFC 0023 — data-policy specialization and store enforcement
    ├── RFC 0021.b — durable workflow and approval runtime
    ├── RFC 0021.c — project-graph representation of analysis findings
    ├── RFC 0026 — lineage and evidence relationships
    ├── RFC 0020 — Rust authoring and compile-time policy surface
    ├── RFC 0024 — agent/MCP enforcement-point integration
    └── RFC 0027 — stream-compute enforcement specialization
```

RFC 0029 owns the universal vocabulary: admission outcomes, enforcement
dimensions, platform tiers, effect coverage, bypass classifications,
obligation classes, fact provenance, policy lifecycle, evidence profiles,
and guarantee boundaries.

Specialized RFCs own concrete syntax, runtimes, gateways, storage lowering,
workflow engines, lineage stores, user interfaces, and domain-specific
conformance behavior.

## 3. RFCs with directly reduced responsibility

### 3.1 RFC 0016 — domain packs

**Responsibility moving to RFC 0029**

- universal policy admission;
- generic policy correctness versus enforcement boundary;
- generic trust and policy lifecycle semantics;
- universal waiver and emergency semantics;
- generic evidence and coverage terminology;
- generic domain-wide guarantee claims.

**Responsibility retained by RFC 0016**

- packaging domain vocabulary and invariant libraries;
- mandatory domain-module contents;
- domain-module signing and provenance inputs;
- installation and dependency resolution;
- domain-specific tests and invariant definitions;
- non-removability when an RFC 0029 meta-policy requires it.

**Required amendment**

Add a status note stating that an RFC 0016 domain pack is an RFC 0029 domain
module and receives no enforcement guarantee until it passes RFC 0029
admission. Replace independent guarantee, waiver, and trust terminology with
references to RFC 0029. Preserve historical implementation notes as
historical evidence rather than rewriting them as if they were designed under
RFC 0029.

### 3.2 RFC 0017 — guarantee manifests

**Responsibility moving to RFC 0029**

- meaning of enforcement strength;
- certified scope, assumptions, and exclusions;
- platform-tier semantics;
- effect-coverage and bypass terminology;
- prevention versus bounded-impact distinctions;
- generic evidence and replay requirements.

**Responsibility retained by RFC 0017**

- guarantee artifact schema and serialization;
- source, dependency, configuration, and binary binding;
- signing, attachment, distribution, and verification;
- consumer-side checking of required claims;
- compatibility and schema migration for guarantee artifacts.

**Required amendment**

Define RFC 0017 as the carrier of RFC 0029 results, not the source of their
meaning. Add explicit fields or a versioned extension for platform tier,
certified effect scope, enforcement mappings, assumptions, exclusions,
bypass classifications, evidence profile, and policy bundle hash. State that
the currently shipped source-scan certificate does not satisfy RFC 0029.

### 3.3 RFC 0023 — data guard

**Responsibility moving to RFC 0029**

- generic PAP/PIP/PDP/PEP definitions;
- universal decision and obligation lifecycle;
- generic policy admission and composition;
- universal provenance and assurance dimensions;
- platform tiers and generic bypass semantics.

**Responsibility retained by RFC 0023**

- data-read and data-write policy specialization;
- `AccessPlan`, `WritePlan`, filters, masks, caps, and cohort constraints;
- Cedar and other data-policy front ends;
- relation resolution and purpose-limited access;
- SQL/DataFusion/store-native lowering;
- federated reads and store-driver conformance;
- guarded data access for MCP and applications.

**Required amendment**

Describe the guard IR as a specialization/lowering of the RFC 0029 canonical
IR for data effects. Map every guard decision and obligation to an RFC 0029
decision/effect class. Remove or alias duplicate generic admission,
enforcement-tier, provenance, and evidence definitions.

### 3.4 RFC 0021.c — graph analysis

**Responsibility moving to RFC 0029**

- universal policy-to-effect coverage semantics;
- universal bypass classifications;
- generic evidence-freshness and certification meaning.

**Responsibility retained by RFC 0021.c**

- project graph nodes and edges for findings;
- checker-adapter interfaces;
- incremental finding lifecycle;
- reachability visualization and query surfaces;
- Hi/MCP presentation of findings and evidence references.

**Required amendment**

Make RFC 0021.c a projection of RFC 0029 coverage results. Define graph
representations for policy, effect, enforcement point, gateway, obligation,
fact authority, evidence, assumption, exclusion, and bypass classification.
Graph state must not independently upgrade an RFC 0029 result.

### 3.5 RFC 0026 — metadata and lineage plane

**Responsibility moving to RFC 0029**

- minimum decision-evidence contents;
- universal fact-provenance requirements;
- evidence profiles and replay classes;
- chain-of-custody requirements for policy decisions.

**Responsibility retained by RFC 0026**

- declared and observed data lineage;
- derivation and transformation relationships;
- lineage collection, projection, and storage;
- metadata queries and graph-store integration;
- declared-versus-observed delta.

**Required amendment**

Treat RFC 0029 decision/fact/obligation evidence as governed metadata inputs.
Provide lineage relationships without redefining whether the evidence is
sufficient for a guarantee. Preserve lineage-specific retention and query
semantics where they do not conflict with the evidence profile selected by
policy.

## 4. RFCs constrained or normalized by RFC 0029

These RFCs retain their main responsibilities but must map them into RFC
0029's canonical semantics.

### 4.1 RFC 0020 — v2 entity policy and encryption annotations

Retains `policy!`, `crud(...)`, categorical actions, encryption declarations,
and Rust compile-time enforcement. Its declarations become an authoring front
end that lowers into the RFC 0029 IR. Compile-time assertions must state their
actual enforcement class and may not imply platform-wide effect coverage.

### 4.2 RFC 0021.b — approval and workflow runtime

Retains distinct-person approval, quorum, durable ledger, outbox, workflow
state, and recovery. It becomes the primary implementation candidate for RFC
0029 `DurableWorkflow`, separation-of-duty, and human-judgment obligations.
It must expose pinning, invalidation, cancellation, compensation, evidence,
and terminal obligation states required by RFC 0029.

### 4.3 RFC 0024 — Hi and guard MCP integration

Retains delegation, agent-tool authorization, evaluate-then-act, and
maker-checker policy mutations. MCP tool invocation becomes an RFC 0029 effect
gateway with explicit agent identity, authority chain, decision validity,
obligations, and evidence.

### 4.4 RFC 0025 — RTM ecosystem

Retains the financial-crime domain vocabulary, guards, workflows, provider
ports, screens, and conformance cases. It becomes a domain-module composition
and worked vertical application under RFC 0029 rather than a source of
universal policy architecture.

### 4.5 RFC 0027 — verified stream compute

Retains stream operators, partitioning, checkpointing, replay, state, and
delivery semantics. It must map each stream effect to RFC 0029 atomicity,
workflow, or continuous-assurance claims and preserve the distinction between
exactly-once state mutation, at-least-once external delivery, and
bounded-impact monitoring.

### 4.6 RFC 0015 — keyed guard

Retains its proposed keyed mutual-exclusion mechanism. It becomes one possible
gateway implementation. Holding a key or lock alone does not establish RFC
0029 `Atomic`; the policy check, relevant state, and protected effect must
share the declared atomic boundary.

### 4.7 RFC 0007 — APM runtime kernel

Retains resource admission, runtime controls, and NFR monitoring. It becomes
an operational/resource enforcement mechanism and must report whether a
control is preventive, runtime guarded, or continuous bounded-impact.

### 4.8 RFC 0010 — route exposure

Retains deny-by-default HTTP route exposure and role/claim-facing behavior.
Routes and handlers become entry points/PEPs in the RFC 0029 coverage graph.
Route coverage does not by itself prove downstream database, message, file,
or provider effects are covered.

## 5. RFCs largely unaffected

The following RFCs retain their primary responsibility. They may expose
effect identities, preserve policy metadata, or consume evidence, but RFC
0029 does not subsume their architecture:

- RFC 0001 — package manifests;
- RFC 0002 — editor and LSP tooling;
- RFCs 0003–0005 — native plugin ABI and boundary design;
- RFC 0008 — plugin discovery;
- RFC 0009 — UI catalog and screen generation;
- RFCs 0012–0014 — Hi, Realm, and generative console;
- RFC 0018 — syntax ergonomics;
- RFC 0019 — OMNISCOPE analysis engine;
- RFC 0022 — web transport hardening;
- RFC 0022.b — screen map and application shell;
- RFC 0028 — mobile client generation.

Native plugins, generated clients, and analysis engines can still introduce
or invoke effects. Their presence in this section means their main design
responsibility is unchanged, not that they are automatically inside RFC
0029's certified scope.

## 6. Documentation amendment matrix

| Document | Required change | Completion evidence |
| --- | --- | --- |
| RFC 0016 | Declare domain-module role and defer universal admission/trust semantics | Cross-reference plus no conflicting active definitions |
| RFC 0017 | Define artifact-carrier role and versioned RFC 0029 fields | Schema, compatibility tests, consumer rejection tests |
| RFC 0020 | Define syntax-to-canonical-IR lowering | Compiled examples and IR snapshots |
| RFC 0021.b | Map runtime to `DurableWorkflow` and obligation lifecycle | Recovery, pinning, invalidation, compensation tests |
| RFC 0021.c | Define graph projection of coverage/evidence | Adapter and stale-finding tests |
| RFC 0023 | Restrict to data-policy specialization | IR mapping and store conformance matrix |
| RFC 0024 | Define agent-tool gateway behavior | Delegation, expiry, deny-without-effect tests |
| RFC 0025 | Declare RTM a domain-module composition | Domain admission and end-to-end evidence report |
| RFC 0026 | Consume policy evidence as metadata/lineage | Provenance projection and access-control tests |
| RFC 0027 | Map stream guarantees to RFC 0029 classes | Failure/replay/external-effect tests |
| docs/V2_GUARANTEES.md | Separate current guarantees from RFC 0029 targets | No unsupported guarantee accepted by policy gate |
| docs/nirdosha-rt-dialect.md | Explain current syntax as partial front end, not full RFC 0029 | Examples and explicit limitations |
| docs/ROADMAP.md | Track every implementation phase honestly | Status backed by tests/artifacts |

Historical text should not be silently rewritten. Add amendment headers or
clearly marked replacement sections so readers can distinguish original
design claims from current responsibility boundaries.

## 7. Probable implementation surfaces

This table is a planning inventory, not authorization to implement all listed
changes in one patch.

| Surface | Expected RFC 0029 responsibility |
| --- | --- |
| `crates/nirdosha-contract-core` | Shared policy model fragments, canonical encoding, admission diagnostics, proof/evidence types where appropriate |
| New policy-core crate or carefully scoped expansion of an existing core | Canonical IR, type system, composition algebra, decidability checks |
| `crates/nirdosha-macros` | Rust authoring front ends and registration; never the sole source of universal semantics |
| `crates/nirdosha-driver` | Tier-1-capable derivations only for an explicitly closed subset; effect-path analysis and coverage fragments |
| `crates/cargo-nirdosha` | Admission/build orchestration, bundle selection, certificate emission, policy gates |
| `crates/nirdosha-guard-core` | Data-policy lowering and RFC 0029 data gateway specialization |
| `crates/nirdosha-guard-registry` | Registered data-policy descriptors; eventual canonical policy/effect references |
| `crates/nirdosha-guard-mic` | PEP/gateway execution for store effects and atomic adapter contracts |
| `crates/nirdosha-guard-verify` | Data-specialized conformance and coverage fragments |
| `crates/nirdosha-workflow` | Durable workflow, approval, pinning, invalidation, obligations, compensation |
| `crates/nirdosha-graph` | Coverage/evidence graph storage and finding projection |
| `crates/nirdosha-lineage` | Fact/evidence derivation relationships and observed metadata |
| `crates/nirdosha-audit` | Integrity primitives and signed evidence support, without overstating unsigned hash chains |
| `crates/nirdosha-guard-mcp` | Agent-tool PEP and delegated effect gateway |
| Store/provider crates | Concrete gateway atomicity, isolation, idempotency, authority, and evidence conformance |
| `nirdosha-hi` | Policy authoring/visualization assistance; never independent policy authority |

Before selecting a new crate boundary, implementation must inventory existing
types to avoid creating a second policy model beside `nirdosha-contract-core`
or a second guard IR beside `nirdosha-guard-core`.

## 8. Migration sequence

### Stage 0 — adopt responsibility boundaries

- Approve or revise RFC 0029.
- Add amendment headers to RFCs 0016, 0017, 0020, 0021.b, 0021.c, 0023,
  0024, 0025, 0026, and 0027.
- Establish one glossary for policy, effect, PEP, gateway, obligation,
  certified scope, exclusion, and evidence.
- Mark duplicate or conflicting terminology as legacy.

No runtime behavior changes in this stage.

### Stage 1 — formal core and canonical bundle

- Deliver grammar, canonical encoding, type system, and initial policy-kind
  semantics.
- Define admission result and stable diagnostic schemas.
- Define policy bundle and RFC 0017 guarantee-artifact linkage.
- Establish meta-policy and trust-root validation.
- Deliver the twelve cross-domain hardening semantics in RFC 0029 §36 before
  freezing the canonical IR: resource/effect closure, decision capabilities,
  evidence/distributed finality, safety posture, authority conflict,
  transition revalidation, aggregate consistency, monitor health,
  policy-plane confidentiality, and applicability.

Do not migrate front ends until the canonical core is stable enough to avoid
each subsystem inventing its own mapping.

### Stage 2 — map existing mechanisms

- Lower RFC 0020 declarations into the canonical IR.
- Map RFC 0023 guard decisions, plans, and obligations.
- Map RFC 0021.b workflows and approvals.
- Map RFC 0024 agent-tool effects.
- Map RFC 0027 stream effects.
- Identify unsupported and opaque constructs explicitly.

Existing behavior remains available, but only mapped constructs receive RFC
0029 evidence.

### Stage 3 — gateway and obligation conformance

- Certify at least one guarded gateway and one atomic gateway.
- Add transaction-bound proof/evidence for atomic guards.
- Add durable pre-effect/post-commit obligation handling.
- Add invalidation, decision expiry, revocation, and resource-version tests.
- Add cross-service compensation and non-atomicity disclosures.

### Stage 4 — coverage and evidence integration

- Emit coverage fragments from compiler, runtime, gateways, and deployment
  controls.
- Project them through RFC 0021.c and RFC 0026.
- Emit versioned RFC 0017 artifacts containing scope, assumptions,
  exclusions, bypass classifications, and policy bundle hash.
- Make consumers reject stronger claims unsupported by those artifacts.

### Stage 5 — constrained Tier 1

- Define a closed, capability-mediated effect subset.
- Prove all in-scope paths use admitted gateways.
- Bind compiler, runtime, bundle, dependencies, and deployment attestation.
- Red-team native, migration, debug, reflection, message, file, network, and
  administrative bypasses.

The ordinary Rust dialect must not be labeled Tier 1 outside this constrained
and attested subset.

## 9. Per-RFC completion checklist

An affected RFC is considered migrated only when:

- [ ] its status/header names RFC 0029's responsibility boundary;
- [ ] generic concepts it no longer owns are removed, marked historical, or
      normatively delegated;
- [ ] retained responsibilities are stated positively;
- [ ] its data types map to the canonical IR or are explicitly opaque;
- [ ] its enforcement mechanisms declare Axis A and per-decision Axis B;
- [ ] external facts declare authority, freshness, revocation, and failure;
- [ ] obligations identify blocking versus post-commit behavior;
- [ ] evidence maps to a declared profile and replay class;
- [ ] bypasses and exclusions use RFC 0029 terminology;
- [ ] tests demonstrate the claimed mechanism and negative paths;
- [ ] certificates do not advertise a stronger guarantee than the evidence;
- [ ] roadmap status matches implementation reality.

## 10. Cross-cutting acceptance criteria

The responsibility migration is complete only when:

1. there is one canonical policy IR and one canonical encoding;
2. all authoring surfaces either lower to it or are labeled opaque;
3. RFCs 0016, 0017, 0021.c, 0023, and 0026 no longer define competing
   universal admission, coverage, or evidence semantics;
4. every enforcement claim names platform tier, scope, assumptions, effect
   class, and mechanism;
5. `DetectedAndResponded` and `AcceptedRisk` never appear as prevention or
   enforcement-complete coverage;
6. every atomic claim is backed by a conformant transaction boundary;
7. every durable-workflow claim has recovery, idempotency, invalidation, and
   obligation evidence;
8. RFC 0017 artifacts bind the exact policy bundle and carry exclusions;
9. RFC 0021.c and RFC 0026 project, rather than redefine, policy evidence;
10. the consumer-side policy gate rejects unsupported or stale claims;
11. current source-scan certificates remain honestly labeled as source-scan
    evidence;
12. docs and roadmap distinguish shipped mechanisms from RFC 0029 targets.
13. all twelve §36 hardening requirements have formal semantics, admission
    checks, evidence fields, and negative conformance tests before their
    associated guarantee is enabled.
14. RFC 0029.a's model identity, status, receipt, promise, effective-state,
    monitor, and impact interfaces are implemented and profile-conformant
    before any AI assurance guarantee is enabled.

## 11. Explicit non-actions

Adopting RFC 0029 does not require deleting older RFCs or implementations.
It does not make all existing `policy!`, guard, workflow, certificate, or
lineage behavior invalid. It changes where universal semantics live and
requires each specialization to state how it conforms.

Historical RFC evidence remains useful. The migration should preserve dated
status notes, shipped behavior, and known limitations rather than flattening
the repository history into a fictional single design.
