# Nirdosha v2 — the comment-encoded declarative layer (the Option C migration)

**Status:** proposal accepted in review; recorded on the deferred-work
register (nirdosha#65) as the resolution of the syntax fork.
**Affects:** the proprietary `.nir` tier primarily; the Rust dialect
scanners additively; certificates, trust chain, roles, NFR — **zero
change** (audited in §7).
**Empirical basis:** every rustc/cargo claim in this document was
verified on the current toolchain before writing (see §2.3).

---

## 0. Executive summary

A `.nir` file becomes **valid Rust** — compiled and run unchanged by
plain `cargo`, extension intact — while the entire declarative moat
layer (workflows, screens, dashboards, `transact` claims, contracts)
rides `///` doc comments: the one channel plain rustc accepts, rustdoc
JSON carries to any agent or auditor, and both Nirdosha compilers parse.

```
                          ┌─ plain cargo ──── rustc builds & runs it (comments inert)
.nir v2 (= valid Rust ────┼─ cargo nirdosha ─ verifies contracts AND declarations,
   + comment layer)       │                    refuses drift, mints the certificate
                          └─ nirdosha ─────── proprietary: verifies + generates
                                                (emit-ui reads the comment layer)
```

**One file, three readers. No grammar fork (Option A), no
attribute-macro soup (Option B — impossible anyway: rustc rejects
unknown attributes; doc comments are the only inert channel). The
native `.nir` syntax stays, as authoring sugar that desugars to v2.**

The core split, and the whole design in two lines:

> **Declarations are data — data rides comments.**
> **Execution is code — code stays Rust, with its claims in comments.**

### Why `cargo nirdosha` exists at all (asked once, answered here forever)

Short answer: **it is the free tier.** Without it there are two readers
— plain cargo and the paid compiler — which is proprietary-only, the
exact model the decision record rejects. Each reader earns its place
by answering a different question:

| Reader | Answers | Tier |
|---|---|---|
| plain `cargo` | "will it build & run?" | floor — everyone, no vendor |
| `cargo nirdosha` | "are its claims **true**?" | free hook — verification, certificates, gates |
| proprietary `nirdosha` | "will it **generate & prove**?" | paid moat — UI/CRUD/workflow codegen, Z3, signed plugins |

Five reasons the middle reader must exist as its own tool, recorded so
the question never re-opens unfounded:

1. **The funnel needs something free to land on.** A `.rs` file with
   inert comments is just Rust — it hooks nobody. "Same source, two
   compilers, lying is a build error, here's the certificate" hooks
   everyone — and it only works if verification installs without the
   paid compiler. No free verification → no funnel → proprietary-only.
2. **We commoditize our own complement before someone else does**
   (falsifier #3 on the register). If `cargo nirdosha` doesn't become
   the free verification standard over Rust, Vera or rustc-native
   effects will. Owning the free layer is what lets the paid layer
   sit *above* a standard we control.
3. **Certificates need a free minter to become an ecosystem
   artifact.** Third parties — a downstream crate consumer, an agent,
   an auditor — must verify contract claims with no vendor
   relationship, or `nirdosha.certificate/v1` stays a vendor feature
   instead of becoming a standard.
4. **A whole class of customer never buys — and spreads the standard
   anyway.** A Rust shop that wants `effects(pure)` checking, role
   typestate, and SLA bench gates in CI gets permanent free value;
   some fraction converts, all of them spread the certificate format.
   And the free tier is deliberately *light* — scanner + delegator +
   optional nightly driver — vs the heavy proprietary toolchain, so
   the free experience is a 30-second install with zero new
   toolchain.
5. **The free mechanics are cargo-native.** Refuse-or-delegate in
   front of cargo, `verify --workspace`/`bench` as free CI gates, the
   Stage-2 MIR driver on `RUSTC_WORKSPACE_WRAPPER` — none of it fits
   a standalone proprietary compiler naturally.

One-liner for the whole architecture: **cargo runs it, `cargo nirdosha`
believes it, `nirdosha` builds the rest of the product around it.**

---

## 1. The problem (why A and B both miss)

- **Option A — two grammars forever.** `.nir` keeps its own frontend
  (box/chan/workflow/screen as keywords); the dialect keeps Rust
  syntax. Two parsers, two typecheckers, two of everything to maintain;
  zero ecosystem or LLM-prior pull for the `.nir` surface.
- **Option B — converge via macros.** Executable constructs can become
  macros/prelude calls, but *declarative* constructs cannot become
  attributes: rustc rejects unknown attributes (`cannot find attribute
  "workflow"`), so plain-cargo compatibility dies. The only
  rustc-inert channel for declarations is comments — specifically
  **doc comments**, because plain `//` comments vanish after lexing,
  while `///` desugars to `#[doc = "..."]` attributes that survive to
  HIR, to rustdoc, and to the Stage-2 rustc driver.
- **Option C — the move.** Split the language by *nature*, not by
  tier: everything that is a declaration becomes `nirdosha:`-prefixed
  JSON in doc comments (generalizing the already-shipped
  `nirdosha:contract` encoding); everything that executes becomes
  ordinary Rust, with a comment carrying its *claim* so a Nirdosha
  compiler can check declared-vs-actual.

---

## 2. The idea, precisely

### 2.1 The channel

| channel | plain rustc | rustdoc JSON | Stage-2 driver | Nirdosha scanners |
|---|---|---|---|---|
| `//` comment | ignored | **lost** | lost | lost |
| `///` doc comment | accepted (inert) | **carried** | read via `hir_attrs`/`doc_str` | read |
| unknown attribute | **rejected** | — | — | — |
| `#[contract]` macro | expands (works) | n/a | n/a | n/a |

Conclusion: **the declarative layer must ride `///` doc comments.**
That is not a concession — it is the same channel contracts already
use, and it means the whole moat layer is machine-extractable from
compiled documentation.

### 2.2 The encoding

```
/// nirdosha:contract  { "effects": ["pure"], "requires": {...}, "nfr": {...} }   // shipped
/// nirdosha:workflow  { "name": "Approval", "data": {...}, "states": {...} }
/// nirdosha:screen    { "for": "Case", "title": "Cases", "fields": {...} }
/// nirdosha:dashboard { "tiles": [...], "charts": [...] }
/// nirdosha:transact  { "verify": "check_amount", "compensate": "refund_amount" }
```

Same rules the contract encoding already enforces: JSON objects,
`deny_unknown_fields` (a typo'd key is a hard error, not a silent
ignore), one declaration decorating the Rust item it is about
(`screen` on the struct, `transact` on the function, `workflow` on the
data struct), consecutive doc lines joined before parsing.

### 2.3 Verified, not assumed (the three facts this design stands on)

1. **Plain cargo builds `.nir` files, extension intact** —
   `[[bin]] path = "src/main.nir"` and `[lib] path = "src/lib.nir"`
   both build and run under stock cargo (tested: `cargo run` prints
   from a `.nir` file; exit 0).
2. **The comment layer flows into rustdoc JSON** — a
   `/// nirdosha:workflow {...}` on a Rust item appears verbatim in
   `cargo doc -Z unstable-options --output-format json` output
   (tested). Agents and auditors extract the moat layer from *compiled
   docs*, exactly as they already can for contracts.
3. **rustc rejects the native `.nir` grammar** —
   `error: expected identifier, found "06_shared_lib.nir"` (tested)
   — which is precisely why v2 must exist at all, and why the
   declarative constructs could never have been plain Rust.

---

## 3. Construct-by-construct migration map

| `.nir` today | v2 (= valid Rust + comment layer) | Channel | Who checks it |
|---|---|---|---|
| `struct Case { id: i64, title: str }` | `pub struct Case { id: i64, title: String }` | Rust | rustc |
| `str` / `unit` / `print(..)` | `String`/`&str` / `()` / `println!` | Rust | rustc |
| contracts: `requires(role:)`, effects, nfr | `#[contract(..)]` **or** `/// nirdosha:contract {json}` | both (shipped) | macro + scanners + driver |
| **logging-policy guard** | `#[contract(logging(domain = "..", country = "..", entity = ".."))]` **or** `/// nirdosha:contract {"logging":{"domain":"..","country":"..","entity":".."}}` | both | macro resolves policy register, runtime guard scrubs fields |
| `requires(role)` gating, `acquire f(proof)` | `RoleProof<R>` typestate + `Auth::prove` (shipped) | Rust types | **rustc itself** — uncallable without proof, under any compiler |
| CRUD conventions `list_/create_/update_/delete_<S>`, `stat_/chart_<name>` | the same names on real Rust fns | Rust | emit-ui infers from names — **zero encoding needed** |
| `workflow Approval { data{..} state .. }` | `/// nirdosha:workflow {json}` on the real `ApprovalData` struct; `start_approval`/`advance_approval` are real Rust fns | comment | Nirdosha cross-references states↔events↔fns |
| `screen Case { field title {..} }` | `/// nirdosha:screen {json}` on `struct Case` | comment | Nirdosha: fields exist on the struct |
| `dashboard { tile .. -> stat_x }` | `/// nirdosha:dashboard {json}` | comment | Nirdosha: tiles/charts reference real fns |
| `transact { network/verify/commit/compensate }` | real Rust body + `/// nirdosha:transact {"verify":..,"compensate":..}` | **both** | Nirdosha checks body matches claim (declared-vs-actual) |
| `box i64` / `froze i64` / `chan i64` / `spawn f(..)` / `sandbox` | `Box`, `Frozen<T>`, `Chan<T>`, `spawn(..)`, `sandbox(..)` from the `nirdosha-rt` prelude (register entry #11) | Rust + needles | needles classify effects; concurrency surface denied in std |
| `db`/`mq`/`file`/`https_get`/`oidc_validate_token`/`check_role` | prelude fns in `nirdosha-rt` (Row-12 identity, entry #5) | Rust + needles | std alternatives denied; effects declared |
| `use "06_shared_lib.nir"` | `mod shared; use shared::..` | Rust | rustc |

**Worked example** (from `examples/syntax/06_identity_and_declarative_ui.nir`):

```rust
/// nirdosha:workflow {
///   "name": "Approval",
///   "data": { "struct": "ApprovalData", "fields": { "amount": "i64" } },
///   "states": {
///     "Pending":  { "transitions": { "Approve": "Approved", "Reject": "Rejected" } },
///     "Approved": { "terminal": true, "on_entry": "log_decision" },
///     "Rejected": { "terminal": true, "on_entry": "log_decision" }
///   }
/// }
pub struct ApprovalData { pub amount: i64 }

pub fn start_approval(data: ApprovalData) -> Result<i64, WorkflowActionError> { /* real Rust */ }
pub fn advance_approval(id: i64, ev: ApprovalEvent) -> Result<bool, WorkflowActionError> { /* real Rust */ }

/// nirdosha:screen {
///   "for": "Case", "title": "Cases",
///   "fields": { "title": { "label": "Case Title" } }
/// }
pub struct Case { pub id: i64, pub title: String, pub status: String }

/// nirdosha:transact { "verify": "check_amount", "compensate": "refund_amount" }
pub fn charge(amount: i64) -> bool {
    let resp = call_api(txn_id, amount);
    if check_amount(resp) { commit_amount(amount); true } else { refund_amount(amount); false }
}
```

Note what the encoding does **to** `transact`: it can no longer be
magic syntax that hides the saga — the body is visible Rust and the
comment is a *claim*. A Nirdosha compiler refusing `charge` because
its body never calls `refund_amount` on the failure path is the same
declared-vs-actual product as `effects(pure)` refusing a lying body.

---

## 4. The comment grammar (Phase-0 spec sketch)

- **Prefixes (registry):** `nirdosha:contract | workflow | screen |
  dashboard | transact`. The registry is extensible — signed plugins
  (#13) may later declare their own kinds (`nirdosha:plugin:<name>:…`),
  which is how a domain plugin's declarations ride the same channel.
- **Payload:** one JSON object per declaration; unknown keys are hard
  errors (`deny_unknown_fields`, exactly like contracts today).
- **Placement:** the doc comment decorates the item it declares about;
  multi-declaration doc comments allowed (contract + transact on one
  fn).
- **Cross-reference rules** — the layer's "typecheck", run by the
  Nirdosha scanners against the syn-parsed Rust AST:
  1. `workflow`: every state name in `transitions` exists; `on_entry`
     names a visible fn; `data.struct` names the decorated struct and
     its fields match.
  2. `screen`: `for` names the decorated struct; every field in
     `fields` exists on it; zero-payload enums render as dropdowns
     (same rule emit-ui uses today).
  3. `dashboard`: every tile/chart names an existing fn with the
     shape emit-ui expects (`stat_* -> i64`, `chart_* -> json`).
  4. `transact`: `verify`/`compensate` name existing fns; the scanner
     checks the decorated fn's body actually calls them on the
     paths the claim implies.
  5. Drift is a **refusal** (exit 1, no delegation), same as a lying
     contract.

---

## 5. Migration phases

| Phase | Where | Deliverable | Exit criterion |
|---|---|---|---|
| **0 — spec freeze** | this doc | the grammar in §4 + JSON schemas | reviewed, on the register |
| **1 — dialect scanner** | `cargo-nirdosha` (dialect repo) | parse the four new kinds, cross-reference, embed results in the certificate's verification payload, refuse drift | fixtures derived from 06 verify clean; a drifted fixture is refused with exact lines; certificates unchanged in schema |
| **2 — emit-ui reads the comment layer** | proprietary repo | `ui_gen` derives the same `Screen`/`FieldSpec`/`Action` manifest from comment layer + syn AST | **golden test:** 06-as-v2 produces the identical manifest that native 06 produces today |
| **3 — transpiler + corpus** | proprietary repo | native `.nir` → v2 converter; migrate `examples/syntax/*`, `enterprise_app`, trade-finance examples | every migrated example passes the golden-equivalence test; native grammar still green (sugar retained) |
| **4 — proprietary consumes v2 natively** | proprietary repo | `nirdosha build/serve/emit-ui` accept v2 files as first-class | the three-reader demo (below) runs end-to-end |

The three-reader demo (Phase 4's exit, and the visceral one for the
funnel): one file — `cargo run` works; `cargo nirdosha verify` mints a
certificate binding its declarations; `nirdosha emit-ui` produces the
styled frontend from its comments. Same source, three compilers, every
difference is the product.

---

## 6. Impact on the UI app (`emit-ui`)

**Changes — the source of truth only.** Today `ui_gen` walks the typed
AST of native `.nir` (structs + CRUD naming + Row-12 identity types).
Under v2 it reads the comment layer plus the syn-parsed Rust AST. The
derivation rules — struct→table, `create_<S>(x: S)`→form,
zero-payload enum→dropdown, `stat_*`→tile, `chart_*`→chart,
`Screen`/`FieldSpec`/`Action` — are **the same rules over a different
source**.

**Unchanged:** the JSON manifest, the generic JS renderer baked into
the template, the HTML template, theming, `${API_BASE}/<fn_name>` fetch
conventions, the plugin/widget vocabulary.

**Upgrades:**
1. **Login stops being a stub.** `emit-ui` v1's disclosed
   client-side `localStorage` identity is replaced by the dialect's
   type-level `RoleProof` + the Row-12 `oidc_validate_token`/`check_role`
   prelude — role gating becomes *checked code*, not client trust.
2. **The declarative layer becomes auditable**: a screen declaration is
   a claim in a certificate, cross-referenced against the struct it
   names — a renamed field that orphans a screen is a build refusal
   under any Nirdosha compiler, and the certificate says so.
3. **Agents can see the whole app in rustdoc**: the UI layer ships in
   compiled docs alongside contracts.

---

## 7. Impact on certificates, trust, and the rest — the "minimal or no
changes" audit

The point of this section: **v2 changes what Nirdosha *reads*, not what
Nirdosha *attests*.** Everything downstream of the reading keys on
artifacts that do not change shape.

| Subsystem | Change | Why |
|---|---|---|
| `nirdosha.certificate/v1` | **none** | Declarations are just more claims inside the existing `verification` payload — the schema already embeds arbitrary tier-specific content. Sources, binding, determinism, `signature` reservation: untouched. |
| `cargo nirdosha verify --audit` | **none** | Binding covers the claims wherever they live; tamper-checking reads source bytes + payload — both unchanged in nature. |
| Signed-plugin trust chain (#13) | **none** | A signature covers the *binding*; it is indifferent to which comment kinds fed the payload. Plugin-declared comment kinds reuse the same channel (§4). |
| `#[contract]` macro + doc encoding | **none** | v2 *is* this pattern, generalized. The dual form (attribute for authoring, comment for portability) extends naturally. |
| Stage-2 rustc driver | **none required** | It already reads doc attrs at HIR (`hir_attrs`/`doc_str`). Optional 2.5: cross-check `transact` claims against MIR bodies — additive, same lattice. |
| Roles / `RoleProof` / `Auth` | **none** | Already type-level; v2's `requires` rides the shipped mechanism. Under plain rustc the gating holds *by types* — the strongest guarantee in the stack needs no Nirdosha tooling at all. |
| NFR / APM kernel | **none** | `nfr()` stays macro-form-only (the recorded rule); flight recorder, bench gates, escalation kernel untouched. |
| Effects needles | **additive** | Prelude entries classified (`Chan::send/recv` = block, `sandbox` = process, prelude file/db = io); plus the recorded "~30-API concurrency/leak denial set" over std — the same price recorded for convergence on the register, unchanged by C. |
| Rustdoc/agent extraction | **strengthens** | The entire declarative layer is machine-readable from compiled docs (verified in §2.3) — same path contracts ride. |
| Workspace gate / strict mode | **none** | In-dialect scoping rules don't see the comment layer at all. |
| `runtime-kernels` | **none (Phase 2/4 touch codegen, not the kernel)** | Thread pool, APM atomics, platform services are consumed identically. |
| Certificate determinism | **none** | The comment layer is source text — hashed with the sources it lives in; no ambient state enters. |

**The one honest conditional, unchanged:** under *plain cargo*, the
declarative layer is inert — nothing checks comment/code drift, the
workflow never typechecks, no UI is generated. That is the design, not
a gap: "plain cargo gives you Rust's guarantees; Nirdosha compilers
give you Nirdosha's." The certificate is what attests which compiler
checked what, and it does so in a format that does not change.

---

## 8. Guarantee matrix under v2

| Guarantee | plain `cargo` | `cargo nirdosha` | proprietary `nirdosha` |
|---|---|---|---|
| Memory safety, no-GC, no data races | ✅ (rustc's own) | ✅ | ✅ |
| Role gating uncallable without proof | ✅ (types) | ✅ | ✅ |
| Contracts (effects/roles/nfr) verified | comments inert | ✅ refusal + certificate | ✅ |
| Declarative layer cross-referenced | inert | ✅ (Phase 1) | ✅ |
| UI generated from declarations | no | no | ✅ (Phase 2/4) |
| Certificate, audit, signatures | n/a | ✅ (shipped / #13) | ✅ same schema |
| Deadlock/leak/pool guarantees | Rust baseline | + needles (recorded price) | full (grammar-grade via prelude enforcement) |

---

## 9. Risks & mitigations

1. **Comment/code drift under plain cargo** — inherent. Mitigation:
   `cargo nirdosha` refuses with exact lines; the workspace gate makes
   it a CI matter; certificates attest.
2. **JSON-in-comments verbosity** vs native syntax. Mitigation: native
   grammar stays as authoring sugar; the converter emits v2; the same
   dual-form that already exists for contracts.
3. **Scanner cross-referencing is new checker work** (§4). Mitigation:
   Phase 1 scopes it to exactly what 06 exercises; extend per-example,
   never speculatively.
4. **Extension mechanics** — bins verified; libs verified; anything
   exotic falls back to a one-line `.rs` re-export shim. Not a design
   risk, recorded as a fact.
5. **Two sources of truth during migration** (native grammar + v2).
   Mitigation: golden-equivalence tests in Phases 2–3 make divergence
   a build failure; native grammar is never removed, only desugared.

---

## 10. What we are explicitly NOT doing

- Not deleting or freezing the native `.nir` grammar — it remains the
  authoring sugar and desugars to v2.
- Not putting executable code in comments — `transact` bodies are real
  Rust; comments carry claims about code, never code.
- Not claiming any plain-cargo guarantee over the declarative layer.
- Not changing the certificate schema, the trust chain, the
  signed-plugin plan, the roles model, or the NFR kernel.

## 11. Open questions

1. `serve` (API generation) under v2 — expected to mirror `emit-ui`
   (Phase 2 rules), needs its own golden tests.
2. Plugin-declared comment kinds (`nirdosha:plugin:<name>:…`) —
   registry design lands with #13, not before.
3. Should the Phase-3 converter also emit `#[contract]` attribute
   forms where the proprietary compiler prefers them? (Dual-form
   question, cosmetic.)
4. Workflow *runtime* placement: does `start_approval`'s durable
  backing (the `workflow_lower` kernel) need a prelude type, or do the
   desugared Rust fns call the kernel directly? Phase 4 question.