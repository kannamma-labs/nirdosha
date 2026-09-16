# RFC 0020: Declarative entity/CRUD, policy compliance, and encryption annotations for the v2 Rust dialect

## Motivation

The v2 corpus (`examples/nirdosha-v2-corpus`) already has a working,
compiler-enforced role-gating mechanism for the Rust dialect —
`#[nirdosha_rt::contract(requires(role = "..."))]` injects an
unforgeable `RoleProof<R>` parameter, mintable only by
`Auth::prove::<R>()` (`crates/nirdosha-rt/src/role.rs`). That work
replaced the corpus's original, forgeable `RoleProof`/`check_role`/
`acquire` (a public-field struct and a role check that never verified
*which* role a proof was for) with this real, type-checked guarantee.

That mechanism is function-scoped: one function, one role. Two gaps
surfaced while designing a CRUD REST API for a `Product` entity on top
of it:

1. **Field-level rules repeat by hand.** `50_field_masking_and_check_role.nir`
   already shows the pattern for masking one field
   (`Option<&RoleProof<Admin>>` + `mask_unless`), but nothing ties a
   struct's fields to their access/validation rules declaratively —
   every consumer re-derives and re-writes the same checks. The corpus
   already has the *vocabulary* for this — `nirdosha:field` doc
   comments exist in `50_field_masking_and_check_role.nir` and
   `nirdosha:screen`'s embedded field metadata (patterns, min values)
   exists in `39_screen_ui.nir` — but **nothing parses either one**.
   `crates/cargo-nirdosha/src/lib.rs`'s `walk_items` only visits
   `Item::Fn`/`Item::Impl`/`Item::Trait`/`Item::Mod`; it never visits
   `Item::Struct` or field-level attributes at all.
2. **Compliance and data-protection constraints have no home.** Real
   CRUD entities often carry domain constraints that aren't about any
   one field or function — "this domain never allows deletion,"
   "these records must be retained N days," "this data must be
   encrypted at rest and in transit." There is no mechanism today to
   declare such a constraint once and have it apply — and be checked —
   across every entity that claims it.

This RFC proposes closing both gaps with three new doc-comment
channels (`nirdosha:entity`, `nirdosha:field`, `nirdosha:policy`),
consumed by a new codegen/verification step, still targeting the same
"plain Rust today, checked by tooling on top" posture the rest of the
v2 dialect already has.

This is adjacent to but distinct from RFC 0016 (sealed domain packs,
cryptographically attested invariants for the `hi`/native-`.nil`
generate pipeline): RFC 0016 solves "domain knowledge must survive an
LLM re-generating the program," a different pipeline with a
cryptographic sealing model. This RFC solves "domain/compliance
constraints for a hand-written v2 Rust-dialect entity should be
declared once, referenced by name, and checked at build time" — no
sealing, no attestation, plain `cargo-nirdosha` scanning, same trust
model the rest of Stage 1 already uses.

## Design

### Grammar

Extends the existing `nirdosha:<channel> {json}` doc-comment
convention (`crates/nirdosha-contract-core/src/docparse.rs`'s
`DOC_PREFIX` pattern, one prefix per channel):

```ebnf
NirdoshaComment ::= "nirdosha:" Channel WS JsonObject
Channel         ::= "entity" | "field" | "policy"
                   | "contract" | "validate" | "screen" | "schema" | ... (unchanged)

Attachment ::= NirdoshaComment ItemStruct        (* nirdosha:entity, nirdosha:policy *)
             | NirdoshaComment StructField       (* nirdosha:field *)
```

`nirdosha:policy` attaches to a zero-sized marker struct (a name with
no runtime representation, matching how `nirdosha:role_mapping` in
`46_db_schema_and_role_mapping_conventions.nir` already attaches
metadata to a plain struct). `nirdosha:entity` attaches to the entity
struct itself; `nirdosha:field` attaches to one of its fields — both
already legal Rust doc-comment attachment points, no new syntax.

### JSON schemas

**`nirdosha:policy`** — a named, reusable compliance profile:

```json
{
  "name": "string, required — referenced by entities as \"policy\": \"<name>\"",
  "forbids": ["create" | "read" | "update" | "delete"],
  "retention_days": "integer, optional",
  "encryption": {
    "at_rest": "required" | "optional" | "none",
    "in_transit": "required" | "optional" | "none"
  },
  "audit": { "on": ["create" | "read" | "update" | "delete"] }
}
```

**`nirdosha:entity`** — attached to a struct:

```json
{
  "table": "string",
  "policy": "string, optional — a nirdosha:policy name",
  "crud": {
    "create": {"requires": {"role": "string"}} | {"public": true},
    "read":   {"requires": {"role": "string"}} | {"public": true},
    "update": {"requires": {"role": "string"}} | {"public": true},
    "delete": {"requires": {"role": "string"}} | {"public": true}
  }
}
```

An operation absent from `crud` is not exposed at all — there is no
"private by omission but still generated" state.

**`nirdosha:field`** — attached to a struct field:

```json
{
  "requires": {"role": "string"},
  "validate": {"pattern": "regex", "min_len": int, "max_len": int, "min": number, "max": number},
  "encrypt": {"at_rest": true}
}
```

### Worked example

```rust
/// nirdosha:policy {"name":"financial_us","forbids":["delete"],"retention_days":2555,"encryption":{"at_rest":"required","in_transit":"required"},"audit":{"on":["create","update"]}}
struct FinancialUsPolicy;

/// nirdosha:entity {"table":"products","policy":"financial_us","crud":{"create":{"requires":{"role":"admin"}},"read":{"public":true},"update":{"requires":{"role":"admin"}}}}
struct Product {
    id: i64,
    /// nirdosha:field {"validate":{"pattern":"^[A-Za-z0-9 ]+$","min_len":1}}
    name: String,
    /// nirdosha:field {"validate":{"min":0}}
    price_cents: i64,
    /// nirdosha:field {"requires":{"role":"admin"},"validate":{"min":0},"encrypt":{"at_rest":true}}
    cost_cents: i64,
    /// nirdosha:field {"validate":{"min":0}}
    stock: i64,
}
```

`Product.crud` has no `"delete"` key — because `financial_us` forbids
it. That omission is what the static rule below turns from convention
into a build-time guarantee.

### Static semantics (the actual new checked behavior)

Implemented as an extension to `cargo-nirdosha`'s existing scan
(`crates/cargo-nirdosha/src/lib.rs`'s `verify_file`/`check_fn`
pattern, generalized to also `walk` `Item::Struct` and its fields —
the gap named in Motivation §1):

1. **Policy-forbidden operation is a build error.** If
   `entity.policy` names policy `P` and `entity.crud` contains an
   operation in `P.forbids`, `cargo-nirdosha verify` refuses the
   build — the same "lying is a build error" posture already applied
   to `effects(pure)` claims.
2. **Required at-rest encryption without a per-field opt-in is a
   build error.** If `P.encryption.at_rest == "required"`, every field
   of an entity referencing `P` must carry `field.encrypt.at_rest:
   true`, or the build is refused.
3. **`nirdosha:field`/`nirdosha:entity` parsing itself is new.**
   Neither channel has a parser anywhere today
   (`nirdosha-contract-core` only parses `nirdosha:contract`) — this
   RFC's Phase 1 (see below) is specifically adding
   `docparse`/`model` support for both, mirroring `Contract`'s
   `#[serde(deny_unknown_fields)]` posture so a typo'd key is a build
   error, not a silent no-op.

### Codegen: generate readable source, not hidden macro output

Given `entity.crud`/`field.validate`/`field.requires`, a generator
produces:

| Annotation | Generated |
|---|---|
| `crud.create.requires` | `#[nirdosha_rt::contract(requires(role = "admin"))] fn create_product(..)` |
| `crud.read.public` | `fn list_products(admin_proof: Option<&RoleProof<Admin>>) -> ..` |
| `field.requires` | `mask_unless(admin_proof, field)` around that field at every read path |
| `field.validate.*` | one `fn validate_<field>(..) -> Result<(), &'static str>` per field, matching the hand-written pattern already used in `55_product_crud_api.nir`'s design |
| the struct itself | the HTTP route table (`GET/POST/PUT/DELETE /api/<table>[/:id]`) |

This deliberately follows `crates/compiler/src/crud_gen.rs`'s existing
precedent (JSON entity plan → generated, readable `.nil` *source
file*) rather than a derive-macro that hides generated logic in
compiled output invisible to `git diff`. The rest of this dialect's
enforcement (the `#[contract(...)]` macro) only ever *wraps* a
hand-written body; it never invents one. A struct-to-CRUD generator is
qualitatively different — it synthesizes real logic (storage, JSON
encoding, routing) — so keeping the output as inspectable, editable
source is the more conservative choice, consistent with the rest of
this codebase's bias toward auditable generated artifacts.

`audit.on` routes through the audit-log mechanism the corpus already
has and exercises — `ledger.nil`'s `record_entry`/`AuditEntry`, used
today by `enterprise_app.nil` — not a new logging primitive.

### Encryption — what's real and what's new infrastructure

Neither knob is a decoration; each requires a specific, concrete
mechanism, stated plainly so `"encrypt": true` never means "does
nothing":

- **At rest**: `nirdosha-rt` has no crypto today. Realizing this means
  encrypting a flagged field's bytes before they reach storage and
  decrypting only on an authorized read — composing with the existing
  `Option<&RoleProof<R>>` gate (no proof → don't decrypt, don't just
  zero the field). Uses `aes-gcm` (audited, not hand-rolled). Key
  sourcing for the fixture tier mirrors `lib.nil`'s own honest
  fixture-vs-real split (a fixed local key, explicitly documented as
  non-production, the same posture `mock_issue_token` already has).
- **In transit**: `nirdosha_rt::prelude::{listen, accept, Tcp}`
  (`crates/nirdosha-rt/src/prelude.rs:1043-1110`) are raw
  `std::net::TcpListener`/`TcpStream` — no TLS anywhere. Realizing
  `"in_transit": "required"` means wrapping every accepted connection
  in a `rustls` handshake before any `recv`/`send` — new capability,
  not a flag on existing code, and it changes every `serve` example's
  listener, not just new ones.

### Sequencing

1. `nirdosha:entity`/`nirdosha:field`/`nirdosha:policy` parsing +
   the policy-vs-entity conflict check (§ Static semantics 1–2) — pure
   logic, no new dependency, reuses `cargo-nirdosha`'s existing scan
   architecture.
2. `audit.on` → `ledger.nil` wiring — existing mechanism, new caller.
3. At-rest encryption via `aes-gcm` — new dependency, real key-handling
   design needed even for the fixture tier.
4. In-transit TLS via `rustls` — largest, affects every existing
   `serve` example's listener, not just new ones.

Each phase is independently mergeable and each is a strict superset of
guarantee over the previous one — nothing in phase *n* needs phase
*n+1* to be useful on its own.

## Effect on the permission model

This does not introduce a new permission primitive — it's a
code-generation and static-checking layer entirely on top of the
existing one. `crud.*.requires` compiles down to exactly the same
`#[nirdosha_rt::contract(requires(role = "..."))]` /
`Auth::prove::<R>()` / `RoleProof<R>` mechanism already shipped and
verified (`crates/nirdosha-rt/src/role.rs`,
`crates/nirdosha-macros/src/lib.rs`) — the same mechanism this RFC's
Motivation describes replacing the old forgeable `acquire`/`check_role`
with. There is no analogue to native-`.nil`'s `acquire`, `screen`'s
view/edit gates, or `serve.rs`'s route-exposure model here — those are
the *other* compiler's grammar (RFC 0010 covers that surface); this
RFC is scoped entirely to the doc-comment/attribute Rust dialect the
v2 corpus and `cargo-nirdosha`/`nirdosha-driver` already target.

Field-level `requires` is new *ergonomics*, not a new *guarantee*: it
generates the same `Option<&RoleProof<R>>` + `mask_unless` pattern
already hand-verified in `50_field_masking_and_check_role.nir` (we
proved by compiler error that a `RoleProof<HrStaff>` cannot typecheck
where `RoleProof<Admin>` is required — that property is unchanged
here, just generated instead of hand-written).

## Compatibility

Purely additive. `nirdosha:entity`/`nirdosha:field`/`nirdosha:policy`
are new channel names in a doc-comment convention that already treats
unrecognized channels as inert (`cc::docparse::parse_doc` returns
`Ok(None)` for any string not starting with its own channel's prefix —
confirmed behavior today for `nirdosha:field`, which already sits
unparsed in `50_field_masking_and_check_role.nir` with zero effect).
No existing `.nir`/`.rs` file's behavior changes until it opts into one
of these three channels. `walk_items` gaining `Item::Struct` traversal
is additive to `cargo-nirdosha`'s scan, not a behavior change for
functions.

## Rejected alternatives

**A derive-macro (`#[derive(NirdoshaEntity)]`) instead of a codegen
tool.** Rejected for now: it hides the generated CRUD/validation logic
in compiled output, unreadable in source and undiffable — the opposite
of this codebase's existing pattern (`crud_gen.rs` generates
inspectable `.nil` files; the `#[contract(...)]` macro's own expansion
is deliberately small). Revisit only if the generated surface stays
small enough that hiding it stops being a real transparency cost.

**Reusing RFC 0016's sealed domain packs for policy compliance.**
Rejected: 0016 solves a different problem (domain knowledge surviving
LLM re-generation, with cryptographic attestation) for a different
pipeline (native `.nil`, the `hi` generate loop). Bringing its sealing
machinery into the Rust-dialect world for a `"delete": "forbidden"`
check would be enormous overkill for what's fundamentally a JSON
cross-reference check `cargo-nirdosha` can already do the same way it
checks `effects(pure)`.

**Making `nirdosha:field`'s `validate` clause feed the existing
`nirdosha:validate` Z3 pre/post-condition prover** (used by
`tenure_bonus_pct` in `50_field_masking_and_check_role.nir`).
Rejected: that mechanism proves a *pure* function's output holds for
every possible input of its declared type — it isn't the right tool
for "reject a client-submitted string that fails a regex," which is
ordinary runtime boundary validation, not a universally-quantified
proof obligation.

## Open questions

- Exact wire format for reporting a field-level or policy-level
  violation in `cargo-nirdosha`'s `Finding`/certificate JSON — does it
  get a new `ContractForm` variant, or ride the existing `Finding`
  shape with a `function: None` and a struct/field descriptor instead?
- Whether `nirdosha:policy`'s `retention_days` should be enforced by
  anything at all in Phase 1, or stay documentation-only (declared,
  scanned for presence, but not yet acted on) until a durable-storage
  story exists for the fixture tier.
- Where the at-rest encryption key comes from once this leaves the
  fixture tier — this RFC deliberately does not propose a KMS story,
  the same way `lib.nir` defers real OIDC/`db`/`mq` to "the proprietary
  tier" rather than inventing a fixture-grade one.
- Whether TLS (Phase 4) should be opt-in per `serve` binary or become
  the default for every `listen()` call once available — defaulting
  it on is a behavior change for every existing `serve` example, which
  needs its own compatibility note when that phase is actually scoped.
