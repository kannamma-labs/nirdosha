# V2 guarantee migration — running implementation book

Owner: ongoing implementation in this workspace. Started 2026-09-16.
This book is the entry point for this effort; large historical documents
are not required reading. Update it with implementation, evidence, and
remaining work in the same change.

## Objective and decisions

Migrate to Rust-compatible source while preserving explicit Nirdosha
guarantees. A program compiling or matching expected output is insufficient
evidence for authorization, recovery, isolation, or effect safety.

1. Business-critical semantics belong in shared runtime implementations.
   Cargo and generated code must call the same authorization, transaction,
   and isolation machinery. Comments describe obligations; they do not
   silently upgrade business behavior under another reader.
2. Unsupported analysis must not satisfy a required guarantee.
3. Source checking, runtime measurements, formal proofs, and build provenance
   are separate evidence. Integrity hashes are not issuer authentication.
4. Native syntax may remain authoring sugar, with equivalence tests.

## Starting state (2026-09-16)

Existing example migration is uncommitted; preserve it. New work starts in
the verifier/certificate layer without replacing those edits.

- Source scanner handles contracts, not the general declaration layer.
  It has no macro expansion, type resolution, dependency analysis, or
  complete Cargo target/configuration coverage.
- MIR driver checks local calls but trusts broad portions of std/core/alloc
  and nirdosha_rt. It does not yet establish the proposed strict profile.
- `Auth::login` accepts caller-asserted roles; role-token privacy does not
  establish production authentication. Corpus identity is a fixture.
- Corpus sandbox uses threads. It cannot establish process isolation.
- Certificates bind listed source bytes and reports, not executable bytes,
  dependency closure, or authenticated issuer identity. Rehashing edited
  content is possible; signatures remain unimplemented.
- Output tests exist; cross-reader security/recovery equivalence does not.

## Delivery gates

| Gate | Acceptance evidence | Status |
|---|---|---|
| G1: honest evidence and consuming policy | Source scan cannot satisfy stronger required guarantees; missing/unknown evidence fails | Implemented; 31 tests pass locally |
| G2: accepted Rust subset | Expanded macros, resolved calls, dependency summaries, unknown dispatch rejection; adversarial fixtures | In progress: conservative local checker implemented |
| G3: shared authorization + durable transaction | Real authority boundary, role denial without side effects, restart/retry consistency | Adapter implemented; native acceptance tests pass; application migration pending |
| G4: shared isolation/cancellation | Process lifecycle and cleanup tests; no thread fixture accepted as isolation | Unix process runtime implemented and tested; application migration pending |
| G5: cross-reader equivalence | One enterprise flow under each implemented reader with matching state/audit/recovery outcomes | Pending |
| G6: build provenance | Artifact/configuration/dependency binding and authenticated issuer policy | Pending |

## Work log

### 2026-09-16 — initial audit and G1

Implemented:

- Bound `verification.coverage` with named guarantee outcomes, scope, method,
  and analysis limitations in package and workspace reports. Existing v1
  envelope preserved; older reports fail the new policy without breaking
  legacy integrity checks.
- `cargo nirdosha check-certificate <path> --root <package> --require <name>`.
  Requires an explicit policy, checks integrity/freshness, rejects unsupported
  guarantees and escaping paths. Currently consumes individual package
  source-scan certificates only; stronger profiles need explicit support.
- Regression cases: indirect helper and macro effects cannot earn purity;
  legacy, failed, stale, edited, artificially promoted, unknown and mixed
  requirements refuse. Coverage itself is bound by the certificate hash.
- Fixed a bug exposed by the new failed-report test: statement-position
  `println!(...);` escaped the impurity scanner because it visited only
  expression macros. The common macro visitor now checks both forms.
- Workspace report hashing now propagates read failures instead of silently
  substituting an empty source list.
- CLI wording distinguishes source inspection and hash integrity from
  verified semantics or authenticated provenance. Updated the smaller
  dialect document's certificate examples and claims.
- Added `.github/workflows/v2-guarantees.yml`. Workflow execution and branch
  protection are not established by local tests.

Evidence: `cargo test --locked -p nirdosha-contract-core -p cargo-nirdosha`
passed 31 tests (17 core, 8 existing verifier, 1 certificate, 5 new policy).
`cargo test --locked -p nirdosha-macros -p nirdosha-rt` also passed:
4 runtime tests and 2 executable documentation tests; 2 documentation
examples are ignored by their existing annotations.
Initial compilation exposed an existing NFR test initializer missing the
uncommitted model's two new fields; added explicit `None` values.

Formatting: new Rust files formatted. Repository-wide `git diff --check`
reports a pre-existing extra blank line at the end of the corpus Cargo.toml;
the example migration is left intact. No huge Markdown documents read.

Remaining limits: G2–G6 are pending. Strong guarantees deliberately remain
unsupported in the consumer policy. The optional MIR driver does not yet
publish stronger bound evidence; a source report must not be upgraded merely
because a `--deep` invocation was requested. This delivery is G1, not the full
authorization/transaction migration.

## Next implementation sequence

### 2026-09-16 — G2 deep-check hardening

Removed crate-wide purity trust, including the runtime exemption. Inspects
pre-optimization MIR and expanded HIR; rejects unresolved dispatch, drops,
static state, reference writes and unsafe code. Root-local traversal avoids
reusing incomplete results within recursive components. Added real compiler
differential tests at O0/O3 and a pinned nightly CI job. Iterator/NFR pure
demos now need explicit summaries; ordinary Cargo behavior is unchanged.
All 9 driver tests pass, including trait-default dispatch regression and
header checks, with both O0 and O3 invocations.

Runtime inventory: native identity already verifies JWT signatures and
issuer/audience in runtime-kernels; time validity is intentionally a separate
input-driven check. Native transactions already use a WAL/FULL SQLite log
and replay registry, with known site-version and early-network-state limits.
Adapters must reuse these mechanisms and expose the limits, not invent
parallel protocols.

### 2026-09-16 — G3 shared runtime adapters (in progress)

Added opt-in `nirdosha-rt/native`, using runtime-kernels as an ordinary Rust
dependency. Explicit workspace exclusions resolve Cargo's nested-workspace
auto-enrollment error; kernel build.rs isolation remains intact.

Added authority-bound identity over the existing signature verifier. Session
roles are extracted from verified JSON; authorization rechecks expiry and
authority identity. `AuthorizedSaga` denies requests before any log insertion
or external call and recovers pending work before accepting requests.

Added a safe saga adapter to the existing WAL/FULL SQLite log and instance
lock. Native ABI initialization and Rust initialization now share connection
setup. Persisted input/protocol identity prevents changed-request retries or
dispatch to an incompatible protocol version. Reserved negative site IDs
separate safe-adapter replay from native numeric trampolines.

Network, commit and compensation operations are retried with the same ID;
providers MUST implement durable idempotency. This cannot promise exactly-once
effects against arbitrary third-party APIs. Real kill/restart tests cover
each persistence boundary and effect-before-log window against a provider
that atomically stores its idempotency receipt and business mutation.

Important correction from source inspection: the native LLVM backend still
rejects `sandbox`; corpus comments claiming the proprietary compiler already
has process isolation are stale. G4 must implement the process runtime and
must distinguish process separation from filesystem/network confinement.

Evidence: `cargo test -p nirdosha-rt --features native --test native_guarantees
--offline` passes 7 tests, including 12 kill/restart boundaries, denied request
side-effect checks, authority/expiry checks, protocol mismatch and log locks.
The shared native ABI transaction tests are being run separately because the
kernel remains in its own Cargo workspace.

### 2026-09-16 — G4 owned process runtime

`WorkerProcess` launches an explicit executable in a fresh Unix process group;
stop and Drop kill the group and reap the leader. No closure fork from a
multithreaded Rust process. Tests verify stop/Drop release a durable-log lock
held by the worker. This supplies separate address spaces and owned lifecycle,
not filesystem/network restrictions or containment of hostile descendants
that deliberately leave the process group. Non-Unix launch refuses until
equivalent job-object lifecycle support exists. The thread prelude remains
explicitly fixture-only; migration callers must choose the real API.

Close MIR trust gaps with adversarial tests before granting
an effects guarantee. Inventory existing native transaction and identity
runtime APIs before designing adapters; avoid a second independent WAL or
authentication implementation. G3 must include kill/restart tests at each
persistence boundary and duplicate transaction IDs, plus negative role
tests. Production identity must have an explicit trusted authority boundary.

Crash correctness is defined by permitted recovered business states and
idempotency of effects, not by matching console output. Cross-reader tests
must fail or mark a reader unavailable when v2 consumption is unimplemented;
they must never substitute the same Cargo invocation and call it equivalence.
