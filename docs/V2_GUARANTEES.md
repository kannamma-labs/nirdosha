# V2 guarantee profile

Implementation status and evidence: [running book](V2_IMPLEMENTATION_BOOK.md).
This is a target profile, not a claim that all Rust programs meet it.

## Required semantics

| Guarantee identifier | Required mechanism and evidence |
|---|---|
| `effects_pure` | Resolved, transitive effect checks including expanded macros and dependencies; unknown targets reject certification |
| `deadlock_freedom` | Restricted concurrency with a justified progress argument; Rust memory safety alone is insufficient |
| `deterministic_execution` | Controlled clocks, randomness, scheduling and external input under a stated execution model |
| `authenticated_identity` | Trusted identity validation, role proof checks, and denied-operation tests without protected effects |
| `durable_transactions` | Shared persistence/recovery protocol, crash-boundary tests and idempotent retries |
| `process_isolation` | Process-backed runtime and cancellation/cleanup tests; thread fixtures excluded |
| `build_provenance` | Binding of executable, toolchain, configuration and dependency closure, plus issuer authentication policy |
| `source_scan` | Listed source files pass the current syntactic scanner; explicitly limited evidence |

## Accepted subset: enforcement target

- Safe Rust only in application code. Unsafe/FFI enters through named,
  reviewed runtime boundaries; trust assumptions must be recorded.
- Macros are checked after expansion. An unexpanded source scan cannot
  certify generated bodies, including proc macros and build-generated code.
- Dependencies require version/content-bound effect summaries or explicit
  trusted boundary declarations. Crate-name-wide purity is insufficient.
- Dynamic dispatch/function pointers require a complete target set with
  checked effects; otherwise the effect claim is unsupported.
- Concurrency uses approved runtime primitives. Blocking, callbacks,
  destructors, async execution, and cancellation require analysis before
  they are admitted under relevant guarantees.
- Cargo targets, features, cfg flags, generated sources and dependency
  versions must be included in stronger certification scope.

These restrictions are not all implemented. The initial policy gate must
reject stronger guarantees rather than infer them from scanner success.

### Deep checker: conservative local subset (implemented)

The driver traverses local calls separately for every pure-claiming root,
using expanded HIR and runtime MIR before optimization. It accepts scalar
arithmetic, branches, local helpers and recursion. This checks effects; it
does not prove termination or prevent arithmetic-overflow panics. Numeric assertion proofs now let guarded division
and indexing pass: see [MIR numeric proofs](MIR_NUMERIC_PROOFS.md). These
per-check records do not upgrade this to a whole-program guarantee.

It rejects external calls without effect summaries; crate names alone never
grant trust. A curated, `DefId`-resolved effect table
(`nirdosha-driver/src/std_effects.rs`, issue #71) now covers a real but
bounded std/core/alloc surface — `Vec`/`Option`/`Result`/`String`/`HashMap`
inherent methods, `Iterator` adaptors (`map`/`filter`/`fold`/...), and the
dialect's own injected `nfr(..)` guard — so ordinary iterator-based code and
NFR-instrumented pure fns are no longer blocked outright. A higher-order
call (`.map(closure)`) is trusted for the call itself but still requires the
closure/fn-item argument's own body to check out, so a real effect hidden
inside a callback passed to a trusted adaptor is still rejected. Grown
incrementally, one table entry plus a test per addition; anything not in the
table still falls back to blanket rejection. Deliberately **not** trusted by
path name, even though real code hits them constantly: `Clone::clone`,
`Deref`/`DerefMut`, `Mul`/`Add`/comparison operators, and other traits a
downstream type routinely reimplements — `def_path_str` resolves a trait
method call to the trait's own declared path regardless of which type
implements it (confirmed empirically: a hand-written `impl Mul<i64> for
Loud` with a real side effect resolves to the exact same
`std::ops::Mul::mul` string a primitive multiply would), so trusting these
by name would silently accept a lying `effects(pure)` claim. A sound fix
needs the call's resolved `Self` type checked against Rust's orphan-rule
guarantee first — real, and worth doing, but its own follow-on. Do not
restore crate-wide exemptions to make a demo pass; version-bound summaries
and explicit instrumentation boundaries are the route to broader
acceptance.

Separately: any value needing drop glue — even a plain `Vec`/`String` with
no custom `Drop` impl, just ordinary deallocation — is *also* rejected
outright (`"destructor effects lack a verified summary"`,
`TerminatorKind::Drop` in `main.rs`), independent of this table. That gap is
tracked separately; this fix does not touch it.

Unresolved function pointers and trait dispatch (including default trait
bodies), destructors, static state, writes through references/pointers,
unsafe blocks/functions and async functions cannot pass this subset.
Non-contract Rust remains a pass-through. Tests compare stock rustc and the
driver at optimization levels 0 and 3. The CI job pins nightly-2026-08-18;
the locally installed compiler reports commit 8fa1c96cf (2026-08-17).

This is narrower coverage, not a completed `effects_pure` attestation:
bound MIR reports, dependency summaries and resolved Cargo build scope are
still required before the certificate policy can accept that guarantee.

## Reader contract

Plain Cargo executes explicit runtime calls. `cargo nirdosha` checks claims
at its stated analysis level before delegating. The v2 generation path is
pending; generated code must call the shared runtime and preserve business
semantics. UI/scaffolding generation may add surfaces, but authorization and
durability must not depend on comments being interpreted by a paid reader.

## Evidence contract

Source-scan coverage lives inside the certificate's hash-bound verification
payload. Existing v1 certificates without coverage remain integrity-auditable
but cannot pass the new coverage policy. `passed` means the named method
completed without findings that fail its gate; it does not mean a theorem
was proven. Unsupported guarantees are explicit.

A source integrity audit only compares the listed files and certificate
binding. It does not authenticate an issuer, detect all newly added files,
or establish executable provenance. Consumers must obtain certificates from
a trusted verification invocation until authenticated provenance exists.

## Commands (implemented G1 gate)

From the package root, `cargo nirdosha verify` emits coverage in
`verification.coverage` inside its existing v1 certificate. A consumer runs:

```sh
cargo nirdosha check-certificate target/nirdosha/contract-report-example.json \
  --root /path/to/example --require source_scan
```

Use the actual report path printed by verify (workspace targets often live
outside the package directory). Repeat `--require` for multiple guarantees;
all must pass. `--require effects_pure`, `authenticated_identity`, or any
other stronger guarantee currently refuses with exit 1, even if the source
scan found zero violations. Unknown requirements, missing coverage, failed
reports, stale sources, and source paths escaping the root also refuse.
The command does not invoke Cargo metadata. Workspace certificates currently
require checking their individual package certificates with separate roots.

The gate is intentionally conservative: `--deep` does not yet emit a bound
MIR coverage report, so it does not upgrade the source certificate. Legacy
reports still support the old integrity audit but fail this consuming gate.
The `v2-guarantees` CI workflow runs the rejection tests; making the job a
branch-protection requirement is repository-host configuration, not YAML.

## Shared native runtime (implemented adapters)

Enable `nirdosha-rt`'s `native` feature for `native::Authority`,
`AuthorizedSaga`, `DurableSaga`, and `WorkerProcess`. These delegate to
runtime-kernels, the same crate the LLVM backend links.

Authority configuration and the `now` input come from trusted host startup
and clock code. Verify signatures, issuer/audience and time validity before
authorizing. Grants are authority-bound; caller-asserted `Auth::login` roles
cannot produce them. `AuthorizedSaga` recovers before accepting requests and
authorizes before inserting transaction rows or calling providers.

DurableSaga shares native WAL/FULL SQLite setup and the exclusive instance
lock. Input and protocol version are immutable for a transaction ID. Recovery
retries operations with that ID; **providers must implement durable
idempotency**. It is not exactly-once delivery to arbitrary external APIs.
The acceptance provider stores its receipt and business effect atomically,
and real process-kill tests cover the effect-before-log windows.

WorkerProcess uses an explicit executable and owns its Unix process group.
Stop/Drop terminate the group and reap its leader; OS resources are released,
but killed workers do not run Rust destructors. This supplies process
separation, not a permissions sandbox. Descendants must remain in the group.
Non-Unix launch is unsupported. Existing closure sandbox fixtures retain their
fixture status and cannot earn process-isolation evidence.

These runtime tests do not automatically upgrade a source certificate. The
application's use of the boundaries and its provider assumptions still need
to be included in stronger evidence.
