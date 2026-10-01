# RFC 0020: Compile-time policy compliance and encryption requirements for the v2 Rust dialect

> **Status: `crud`/`policy!` (the forbidden-operation guarantee),
> `categorical_actions!` (per-value role gating), and the
> `nirdosha_rt::web` HTTP layer (with OpenAPI baked in) are all built
> and proven** — see "Categorical field actions, the HTTP layer, and
> OpenAPI" below for the newest piece. Encryption (at-rest/in-transit)
> is still just a declared, unenforced field on `Policy` — no crypto or
> TLS exists in `nirdosha-rt` yet; see Open Questions. An earlier draft
> of this RFC proposed a doc-comment/JSON scanning mechanism for all of
> this; that approach was rejected before any of it was built — see
> Rejected Alternatives.

## Motivation

The v2 corpus (`examples/nirdosha-v2-corpus`) already has a working,
compiler-enforced role-gating mechanism for the Rust dialect —
`#[nirdosha_rt::contract(requires(role = "..."))]` injects an
unforgeable `RoleProof<R>` parameter, mintable only by
`Auth::prove::<R>()` (`crates/nirdosha-rt/src/role.rs`). That work
replaced the corpus's original, forgeable `RoleProof`/`check_role`/
`acquire` (a public-field struct and a role check that never verified
*which* role a proof was for) with this real, type-checked guarantee.
Field-level rules (masking, validation) already have a working, if
manual, pattern too: `Option<&RoleProof<R>>` + `mask_unless`
(`50_field_masking_and_check_role.nir`), and ordinary hand-written
`validate_*` functions.

What's missing is a guarantee for domain/compliance constraints that
aren't about any one field or function — "this domain never allows
deletion," "this data must be encrypted at rest and in transit." Today
that's convention: nothing stops a `delete_product` function from
existing in a codebase that isn't supposed to allow deletion at all.

This RFC adds that guarantee the same way every other guarantee in
this dialect works: real `rustc`, not a separate scanning tool. See
`docs/nirdosha-rt-dialect.md`'s "no bespoke parser, ever" rule, added
alongside this RFC.

This is adjacent to but distinct from RFC 0016 (sealed domain packs,
cryptographically attested invariants for the `hi`/native-`.nil`
generate pipeline): RFC 0016 solves "domain knowledge must survive an
LLM re-generating the program," a different pipeline with a
cryptographic sealing model. This RFC solves a narrower problem for the
Rust dialect specifically — "a function that performs a forbidden
operation for its domain must fail to compile" — with no sealing, no
attestation, no external tool.

## Design

### `nirdosha_rt::policy!` — declares a compliance policy as a real type

Same convention as the existing `roles!` macro
(`crates/nirdosha-rt/src/lib.rs`): a marker type plus a trait impl,
invoked once at crate root.

```rust
nirdosha_rt::policy! {
    FinancialUs = "financial_us" {
        forbids: [delete],
        encryption: [at_rest, in_transit],
    }
}
```

expands to:

```rust
pub mod nirdosha_policies {
    pub struct FinancialUs;
    impl ::nirdosha_rt::Policy for FinancialUs {
        const NAME: &'static str = "financial_us";
        const FORBIDDEN_OPS: &'static [&'static str] = &["delete"];
        const ENCRYPTED_CONCERNS: &'static [&'static str] = &["at_rest", "in_transit"];
    }
}
```

`Policy` (`crates/nirdosha-rt/src/policy.rs`) is a plain trait with two
`&'static [&'static str]` associated consts — no unstable features
anywhere in this design.

### `crud(op = "..", policy = "..")` — a new `#[contract(..)]` clause

Extends the existing contract grammar
(`crates/nirdosha-contract-core/src/parse.rs`) rather than inventing a
new macro or doc-comment channel — the same macro users already write
for `requires`/`nfr`:

```rust
#[nirdosha_rt::contract(requires(role = "admin"), crud(op = "delete", policy = "financial_us"))]
fn delete_product(id: i64) -> Result<(), &'static str> { .. }
```

`op` must be one of `create`/`read`/`update`/`delete`
(`Contract::validate()`'s `KNOWN_CRUD_OPS`); `policy` resolves to a
type the same way `requires(role = "..")` resolves a role name —
reusing `nirdosha-contract-core::role::role_ident`'s existing
snake_case→CamelCase conversion, unchanged, for a second purpose.

### Codegen: a real `const` assertion, evaluated by `rustc`

`crates/nirdosha-macros/src/lib.rs`'s `expand()` emits, alongside the
existing role/NFR codegen, a sibling item:

```rust
const _: () = assert!(
    !::nirdosha_rt::crud_forbidden::<crate::nirdosha_policies::FinancialUs>("delete"),
    "policy `financial_us` forbids the `delete` operation, but `delete_product` declares crud(op = \"delete\", policy = \"financial_us\")"
);
```

`crud_forbidden` (`crates/nirdosha-rt/src/policy.rs`) is a plain
generic `const fn` — not a trait method, so it needs no unstable
const-trait feature — that does a byte-for-byte string search over
`P::FORBIDDEN_OPS` at compile time. `assert!`/`panic!` have been
const-evaluable since Rust 1.57; `rustc` itself refuses the build if
the assertion fails. Proven end to end:

```
$ cargo build --bin v2_55_policy_forbidden_ops   # with a scratch delete_product added
error[E0080]: evaluation panicked: policy `financial_us` forbids the `delete`
operation, but `delete_product` declares crud(op = "delete", policy = "financial_us")
```

`E0080` — a real `rustc` const-evaluation error, not a `cargo-nirdosha`
finding. No separate tool needs to run for this guarantee to hold;
plain `cargo build` already enforces it.

### Worked example

`examples/nirdosha-v2-corpus/src/55_policy_forbidden_operations.nir`:
`financial_us` forbids `delete`; `create_product`/`update_product`
declare `crud(op = "create"/"update", policy = "financial_us")` (both
allowed, both compile normally); there is deliberately no
`delete_product` — the file's own comment shows the exact scratch edit
that fails to compile if you add one.

## Categorical field actions, the HTTP layer, and OpenAPI

A categorical field (e.g. `is_approved: bool`, or an N-variant `enum`
status) generalizes the same idea: each value is a distinct
"transition" that should be independently gated, not one field with
one generic `update`. Two roads exist for connecting a value to a role,
and only one of them may ever decide access:

- **Road 1 (authoritative).** The `#[contract(requires(role = ..))]`-
  gated function generated per value — the exact `RoleProof<R>`
  mechanism proven unforgeable everywhere else in this dialect. This is
  the *only* thing that decides access.
- **Road 2 (derived, non-authoritative).** A `role_for_<field>`
  projection, useful for audit logs, UI hints, and API documentation —
  computed from the *same* declaration as Road 1, never an independent
  input. If the two could ever disagree, Road 1 wins by definition; the
  actual design goal is that they structurally *cannot* disagree,
  because there is only one source.

### `nirdosha_rt::categorical_actions!`

`crates/nirdosha-macros/src/categorical.rs`, re-exported from
`nirdosha_rt`:

```rust
nirdosha_rt::categorical_actions! {
    entity: Product,
    store: product_store,
    field: is_approved: bool,
    actions {
        true  => approve_product    requires role "approver",
        false => disapprove_product requires role "compliance_officer",
    }
}
```

expands to Road 1 (one independently role-gated function per value,
looking the entity up in `store()`, setting the field, and returning a
clone) plus Road 2:

```rust
fn role_for_is_approved(value: bool) -> &'static str {
    match value {
        true => "approver",
        false => "compliance_officer",
    }
}
```

**The coverage guarantee is `rustc`'s, not a scanner's.** Because
`role_for_is_approved` is a genuine `match` over the field's real type,
omitting a value is `error[E0004]: non-exhaustive patterns` — proven by
temporarily dropping the `false` arm from
`crates/nirdosha-rt/tests/categorical.rs`'s invocation and rebuilding.
For an N-variant `enum` field, this scales without change: N arms, N
gated functions, one exhaustive match.

Scope: the macro only handles values that are simultaneously valid
match patterns and value expressions with identical tokens (bool
literals, unit `enum` variants) — deliberately not a general ORM or
state-machine generator.

### `nirdosha_rt::web` — an HTTP layer with OpenAPI on by default

`crates/nirdosha-rt/src/web.rs`. `Router::dispatch` answers
`GET /openapi.json` itself, before consulting the route table — every
Nirdosha web app gets a real OpenAPI 3.0.3 document with no opt-in
step, generated fresh per request from whatever routes are actually
registered (so it can't go stale).

Gated registration (`post_gated::<R>`, `get_gated`, `put_gated`,
`delete_gated`) is where Road 1/Road 2 meet for the HTTP surface: the
handler receives a real `&RoleProof<R>` (Road 1 — the router calls
`Auth::prove::<R>()` and returns 403 on `Err` before the handler ever
runs), and the OpenAPI document's `security` entry for that route is
`R::NAME` — read from the exact same type parameter, not a separately
maintained string:

```rust
Router::new(authenticate)
    .post_gated::<Admin>("/api/products", "Create a product", |req, _params, proof| {
        create_product(proof, ..)  // proof is real, minted by Auth::prove::<Admin>()
    })
```

### Worked example: `56_product_crud_api.nir`

Combines everything: per-field validators, `cost_cents` masking via
`Option<&RoleProof<Admin>>`, `is_approved`'s two transitions via
`categorical_actions!`, and real HTTP via `Router`. Verified two ways:

1. **By hand**, live: `cargo run --bin v2_56_product_crud_api`, then
   `curl`'d — anonymous list masks `cost_cents` to `0`; anonymous
   create is `403`; an admin-token create returns `201` with the real
   cost; an admin (not an approver) trying `/approve` is `403`; the
   approver succeeds; the compliance officer's `/disapprove` is a
   *different* role than approve's, on the same field.
2. **In the golden suite**, `tests/outputs.rs`'s
   `product_crud_api_serves_real_requests` — real `TcpStream` requests
   against the actual spawned binary (same convention as
   `compiled_serve_serves_real_requests`), not a Rust-level function
   call, so the assertion is "the wire protocol behaves correctly,"
   not just "the Rust typechecks."

### Property-based tests

`crates/nirdosha-rt/src/web.rs`'s `tests::properties` and
`crates/nirdosha-rt/tests/categorical.rs`'s `properties` module
(`proptest`), fuzzed rather than example-based:

- path-template matching round-trips for arbitrary segments and
  extracts the exact concrete value at a `{param}` position;
- a mismatched literal segment never matches;
- an unregistered path is always `404` regardless of method;
- **the core property this whole RFC exists for**: gated dispatch
  succeeds if and only if the session's role set actually contains the
  required role, fuzzed over arbitrary role-set contents — not just the
  one hand-picked example in each unit test;
- the categorical macro's two generated functions require *exactly*
  their own declared role, fuzzed over all four combinations of
  holding/not-holding each of the two roles.

## Effect on the permission model

Additive, not a new permission primitive. `crud(..)` composes with
`requires(role = "..")` on the same `#[contract(..)]` invocation — the
two checks are independent (one type-level via `RoleProof<R>`, one
const-eval-level via `crud_forbidden`) and don't interact. There is no
analogue to native-`.nil`'s `acquire`, `screen`'s view/edit gates, or
`serve.rs`'s route-exposure model here — those are the *other*
compiler's grammar (RFC 0010 covers that surface); this RFC is scoped
entirely to the doc-comment/attribute Rust dialect
`cargo-nirdosha`/`nirdosha-driver` already target.

## Compatibility

Purely additive. `crud` is a new optional field on `Contract`
(`#[serde(default, skip_serializing_if = "Option::is_none")]`) and a
new optional clause in the attribute parser — a function with no
`crud(..)` clause is completely unaffected, and every existing contract
in the corpus (verified: `cargo-nirdosha verify` reports 0 violations
across all 69 contracts, up from 67, after this change) continues to
parse and validate exactly as before.

## Rejected alternatives

**A `nirdosha:entity`/`nirdosha:field`/`nirdosha:policy` doc-comment
JSON schema, checked by a `cargo-nirdosha` scanner extension.** This
was this RFC's original design, discarded before any of it was built.
The problem: doc comments are inert to `rustc` — a
`nirdosha:policy {"forbids":["delete"]}` comment is only ever checked
by whoever remembers to run `cargo nirdosha verify`; plain `cargo
build` (which is the entire selling point of this dialect — "it
compiles under stock cargo, checked or not") would happily build a
`delete_product` the policy forbids. That's a real regression from
this dialect's own standing rule (`docs/nirdosha-rt-dialect.md`): every
existing guarantee (`requires(role)`, `nfr(..)`) is enforced by
`rustc` itself, not by an opt-in tool. Reusing/extending the existing
`#[contract(crud(..))]` attribute macro instead means the guarantee
holds under `cargo build` alone, with zero new tooling.

**A derive-macro (`#[derive(NirdoshaEntity)]`) that also generates the
CRUD function bodies, not just checks them.** Considered as part of the
same discarded design (see above) for a fuller "declare an entity once,
get five REST endpoints." Rejected for the same reason plus one more:
it would hide real business logic (storage access, JSON encoding,
routing) inside compiled macro output, invisible to `git diff` — the
opposite of how every other generator in this codebase behaves
(`crud_gen.rs` for the *other* `.nil` compiler emits inspectable source
files; the `#[contract(...)]` macro here only ever *wraps* a
hand-written body, never invents one).

**Feeding field-level range/pattern rules into the existing
`nirdosha:validate` Z3 pre/post-condition prover** (used by
`tenure_bonus_pct` in `50_field_masking_and_check_role.nir`).
Rejected: that mechanism proves a *pure* function's output holds for
every possible input of its declared type — it isn't the right tool
for "reject a client-submitted string that fails a regex," which is
ordinary runtime boundary validation, not a universally-quantified
proof obligation. Field validation stays hand-written `validate_*`
functions, same as designed before this RFC existed.

## Open questions

- `Policy::ENCRYPTED_CONCERNS` is declared and const-checkable
  (`requires_encryption::<P>("at_rest")` exists and is tested) but
  nothing calls it yet — there is no at-rest crypto or in-transit TLS
  anywhere in `nirdosha-rt`. Wiring a real check (e.g. a `crud(..)`
  sibling clause requiring a field to actually be passed through an
  encryption call) is blocked on that infrastructure existing first,
  not on this mechanism — the const-assert *technique* generalizes
  cleanly once there's a real encrypted-field marker to check against.
- Whether `retention_days` belongs on `Policy` as a documented-only
  const (no enforcement possible without a durable-storage story for
  the fixture tier) or should wait until one exists.
- Whether a `crud(..)`-annotated function should also be required to
  declare which *fields* it touches, so a future encryption check could
  cross-reference "this create touches `cost_cents`, which the policy
  requires encrypted" — deliberately left open; the current mechanism
  only reasons about the operation name, not the data it touches.
