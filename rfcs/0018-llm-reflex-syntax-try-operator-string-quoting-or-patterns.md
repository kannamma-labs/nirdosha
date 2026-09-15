# RFC 0018: LLM-reflex syntax — a real `?` try-operator, single-quoted strings, and `|` or-patterns in `match`

> Status: proposal / pre-draft.
> Cross-references:
> - `docs/research/2026-09-pending-verification-differentiation-work.md` — Phase 4 item 2 (`hint_cache`), the session this RFC grew out of
> - GitHub issues [#63](https://github.com/kannamma-labs/nirdosha/issues/63) (missing `self_repair_hint` arm for `?`) and [#64](https://github.com/kannamma-labs/nirdosha/issues/64) (the hint cache's cache-key bug for lex/parse errors, now fixed)
> - `docs/LANGUAGE.md` §2 (scalar types), §9 (identity builtins returning `Result`)
> - `crates/compiler/src/ast.rs` (`Ty::Named("Result", ...)`, `prelude_enums`, `MatchArm`)
> - `crates/compiler/src/ownership.rs` (`FreeMap::at_return`)
> - `crates/compiler/src/codegen.rs` (`Stmt::Return`'s aggregate/`sret` path)

## Motivation

A real `:generate` run gave up after 4 self-repair attempts on:

```
let proof_fd: RoleView = check_role(identity, "FinanceDirector")?
```

`check_role` returns `Result(RoleView, str)` (`docs/LANGUAGE.md` §9); Nirdosha has no `?` operator, so this is a lex error (`unexpected character` at the `?`). Investigating *why* the self-repair loop — including its `hint_cache` empirically-gated fallback, built specifically for diagnostic classes with no hand-authored advice yet — never recovered within budget surfaced two things:

1. **#63**: no hand-authored `self_repair_hint` arm existed for this exact mistake, unlike the closely analogous `unexpected character `;`` arm (models reaching for C-family statement separators).
2. **#64** (now fixed): the hint cache's own cache key was broken for *every* lex/parse-stage diagnostic, not just this one — a per-self-repair-attempt-unique scratch file path was riding into the cache key unstripped, so a synthesized hint could never be looked up again after the very attempt that produced it, and the "did this hint actually work" check was comparing two strings that always differ regardless of whether the mistake was fixed.

#63 and #64 together explain why the loop couldn't teach itself out of this one mistake. But they only fix the *symptom* — the model will keep reaching for `?` (and other real syntax from the languages it was trained on) forever, and every one of those mistakes costs a real self-repair round trip even with #64 fixed, because the *first* occurrence of any new mistake still has no hint at all until `synthesize_hints` produces one.

**The other lever, not previously pulled**: some of what models reach for reflexively isn't wrong, it's just *not yet legal syntax here*. If the underlying semantics are sound and genuinely useful (a `Result`-propagation operator is a real, common, good idea — Rust, Swift, and Zig all have one), the cheaper fix than teaching every model forever is to make the reflex correct. This RFC proposes three such additions, of very different sizes and risk, found by asking "what does a model trained on Rust/Python/JS write by reflex that collides with Nirdosha's own grammar":

| Addition | Size | Risk |
|---|---|---|
| `'...'` as an alternate string-literal quote | Lexer-only, ~10 lines | Trivial |
| `\|` or-patterns in `match` (zero-payload variants/literals only) | Parser + typeck + codegen, bounded | Low-medium |
| `?` try-operator on a `Result`-returning call | Lexer + parser + typeck + ownership + codegen | Real, the bulk of this RFC |

**A genuine, material scope correction found while investigating**: this project used to keep a second execution engine (a tree-walking interpreter) that every codegen test's own `..._matches_interpreter` naming refers to, and every new expression form had to stay correct in both. **That interpreter no longer exists** — `main.rs` removed it along with the `run`/`serve`/`--sandbox-worker` commands, and the `_matches_interpreter` test names are now pure historical naming (each one just hardcodes its expected output directly; grep confirms there is no `mod interpreter`/interpreter crate left anywhere in the tree). This roughly halves the real cost of every item below versus what "keep two engines in sync" would have implied — worth stating explicitly since it changes the cost/benefit call for a future decision on scope.

## Design

### 1. `'...'` as an alternate string-literal quote (not a `char` type)

Nirdosha has no `char` type today (`docs/LANGUAGE.md` §2's scalar list: `i64`/`f64`/`str`/`bool`/`unit`/`dec128`) and this RFC does not propose adding one — that would ripple through `effects.rs`/`ownership.rs`/codegen/JSON representation for a type this language has never needed. Instead: `'...'` is purely a second spelling for a string literal, exactly like Python's `'...'`/`"..."` equivalence — `'x'`, `'hello'`, and `'it''s escaped, same rules as \"'` all lex to the identical `Tok::Str`/`Expr::Str` node a double-quoted literal with the same content would produce. No length restriction (a "char" reading was considered and rejected — see below).

**Implementation**: `token.rs`'s existing double-quote string-scanning function gains a second entry point (or a `quote: u8` parameter) triggered by `'` instead of `"`, sharing the exact same escape-sequence handling. Nothing downstream of the lexer changes at all — `Expr::Str` already exists and every pass that walks it is already correct.

### 2. `|` or-patterns in `match` — zero-payload alternatives only

**Real v1 scope cut, stated up front**: this RFC proposes or-patterns only for arms whose every alternative is a **zero-payload enum variant or a literal** (`RoleA | RoleB => ...`, `1 | 2 | 3 => ...`) — never a payload-carrying variant (`Ok(x) | Err(x) => ...`). Rust itself requires every alternative of a binding or-pattern to bind the identical set of names at the identical types; implementing and checking that correctly is a materially bigger typeck feature than this RFC's other two items and isn't the case a real field failure has shown yet. Scoping it out here is the same "real, disclosed cut" discipline every other v1 slice in this codebase already holds itself to (see `ast.rs`'s own `LiteralPattern` doc comment for the precedent: "no wildcard/binding-only catch-all patterns" was an identical kind of cut when literal-match arms were added).

**AST change**: `MatchArm` currently models exactly one pattern per arm (`variant: String` for the enum form, or `pattern: Option<LiteralPattern>` for the literal form). Add:

```rust
pub struct MatchArm {
    /// Non-empty. Length > 1 only for an or-pattern arm (`A | B => ...`);
    /// every other arm has exactly one entry, unchanged from today.
    pub patterns: Vec<ArmPattern>,
    pub bindings: Vec<String>, // unchanged; empty whenever patterns.len() > 1 (checked in typeck)
    pub body: Box<Expr>,
    pub span: Span,
}

pub enum ArmPattern {
    Variant(String),
    Literal(LiteralPattern),
}
```

(`variant`/`pattern` collapse into `ArmPattern`; every existing single-pattern call site becomes `patterns: vec![ArmPattern::Variant(name)]` or `vec![ArmPattern::Literal(lit)]` — a mechanical migration, not a behavior change for any existing `.nir` program.)

**Parser**: after parsing one pattern in an arm's head position, loop while the next token is `Tok::Pipe` (new token, `|` — today a lex error, since only `||` is recognized), parsing one more pattern each time. A pattern with any parenthesized binding list (`Variant(x, y)`) followed by `|` is a **typeck** error (`OrPatternWithBindings`), not a parse error — parsing accepts the shape uniformly, matching this codebase's established "parse normally, then validate what came out" precedent (`Expr::Spawn`/`Expr::SpawnSandbox`'s own doc comment in `parser.rs`).

**Typeck**: `check_match`'s exhaustiveness accounting (today: one variant/literal checked off per arm) checks off every pattern in the arm's `patterns` list instead of just one; a duplicate-arm check applies per individual pattern, not per arm, so `RoleA | RoleA => ...` — same variant twice — is still caught. `OrPatternWithBindings` fires if `patterns.len() > 1` and the (single, shared) `bindings` list is non-empty, or if any `ArmPattern::Variant` in the group actually names a variant with a non-zero payload arity (looked up the same way `check_match` already resolves a variant name against the registry).

**Codegen**: today, one match arm compiles to one tag-equality branch (`icmp eq` against the arm's variant's tag) or one literal-equality branch. An or-pattern arm compiles to the disjunction of those same per-pattern comparisons (`or` of N `icmp eq`s, short-circuited the same way a chain of `||` already lowers) branching to the one shared arm body — the arm body itself needs no new codegen at all, since it's compiled exactly as today once its branch is taken.

### 3. `?` try-operator — the bulk of this RFC

**Syntax**: postfix `?` (new `Tok::Question`, today a lex error), parsed generically in `parser.rs::parse_postfix` at the same precedence tier as `.field`/`[index]` (so `foo()?.field` parses, even though `?` itself is position-restricted below) — producing `Expr::Try(Box<Expr>, Span)`. This mirrors `Expr::Spawn`'s own "parse normally as a generic postfix/call form, restrict in typeck" precedent rather than inventing a parser-level restriction — `parser.rs` doesn't need to know or care where `?` is legal.

**v1-restricted positions, a real and disclosed scope cut** — `Expr::Try` is legal **only** as:
- the entire initializer expression of `let name: T = <expr>?` (`T` is the `Ok` payload type),
- the entire value expression of `return <expr>?`, or
- an entire bare statement, `<expr>?` alone on its own line (`Stmt::Expr(Expr::Try(...))`), when the `Ok` payload type is `unit`.

Anywhere else (`foo(bar()?, 1)`, `1 + bar()?`, inside a `match` scrutinee, etc.) is a new `TypeErrorKind::TryOperatorMisuse` — "the `?` operator can only be used directly as a `let` initializer, a `return` value, or a whole statement in this version." This is not an arbitrary restriction: Nirdosha's `return` is a `Stmt`, not an `Expr` (unlike Rust, which can use `return` from any expression position because it has a bottom/`!` type) — a fully general `?` would need an early-return-*from-inside-an-arbitrary-expression* primitive this language doesn't have and this RFC doesn't propose adding (a real, much bigger change to the type system: a bottom type that unifies with everything). Restricting to these three statement-level positions means `?`'s desugaring only ever needs to synthesize a new statement-level early exit, reusing `Stmt::Return`'s own existing machinery — see Codegen below.

**Other typeck rules**:
- `TryOperatorOutsideResultFn` — `?` used inside a function whose own declared return type is not `Result(_, _)` at all.
- `TryOperatorErrorTypeMismatch` — `expr`'s error type (`Result(_, E)`'s `E`) does not exactly equal the enclosing function's own `Result(_, E')`'s `E'`. **v1 has no error-type conversion** (no Rust-style `From`) — the two must match exactly, another real, disclosed cut; a mismatch names both types in the diagnostic.
- `expr` itself must type to `Result(_, _)` at all (`TryOperatorOnNonResult`) — reuses whatever `TypeErrorKind` variant already fires for "expected `Result(..)`, found `T`" elsewhere if one already exists generically, rather than inventing a fourth kind for the same shape.

**Ownership**: `FreeMap::at_return` (`ownership.rs`) is built today by walking every `Stmt::Return` site and recording which affine (`box`) bindings are still owned and need freeing there. A `?` is a second, synthetic exit point out of the function with exactly the same freeing obligation — the ownership pass needs to also visit every `Expr::Try` site (in one of its three legal positions) and record an `at_return`-shaped entry for it, keyed by the `Try`'s own span rather than a `Return`'s. No new *kind* of ownership analysis — the existing "what's still live at this exit point" computation just needs a second class of exit point fed into it.

**Codegen** (`codegen.rs`), the direct extension of `Stmt::Return`'s existing aggregate/`sret` handling read while scoping this RFC:
1. Evaluate `expr` (the `Result`-typed value) via the existing aggregate-expression path (`expr_ptr_expected`, since `Result` is `is_aggregate()`).
2. Load the tag word (offset 0, `0` = `Ok`, `1` = `Err` — `ast::prelude_enums`' own declaration order, the same layout `Stmt::Return`'s existing NFR `was_err` check already reads).
3. Branch on the tag:
   - **`Ok` arm**: extract the payload (offset past the tag, same `getelementptr` shape the interpreter-free `Expr::Match`/`FieldAccess` codegen already uses for enum payload extraction) and continue as this statement's own value (the `let`'s bound value, the `return`'s returned value, or — for the bare-statement form — nothing).
   - **`Err` arm**: build this *function's own* `Result(_, E)` return value with the same `E` payload, re-run the identical sequence `Stmt::Return`'s own `Err`-carrying-aggregate path already runs verbatim (field masking, `free_map.at_return`-driven frees — now including this `Try` site's own entry from the Ownership step above — the NFR `was_err`/`emit_nfr_call_end` hook, the `sret` memcpy, `ret void`), and mark the block terminated the same way `Stmt::Return` does.
4. `local_ty_of` (used by `Stmt::Expr`'s own aggregate-vs-scalar dispatch and elsewhere) gains an `Expr::Try` arm returning the `Ok` payload type, so the three call sites route to the right one of `expr`/`expr_ptr` automatically, no special-casing needed beyond what already exists for every other expression kind.

**Grammar export** (`crates/compiler/nirdosha.gbnf`, `grammar_export` crate): a mechanically-derived grammar (`nirdosha check`'s own doc comment: "runs the real parser over `examples/**/*.nir` + the capabilities corpus and renders what it actually walked") — once real `.nir` example files use `?`/`'...'`/`|`-patterns, re-running `nirdosha grammar-export` picks the new productions up automatically. At least one example under `examples/` per new construct is needed for this to actually happen, not just implementing the parser.

## Effect on the permission model

None of the three change what `requires(role/claim: ...)`, `acquire`, a `screen`'s view/edit gates, or `serve.rs`'s server-side enforcement can express or must check. `?` is pure sugar for an already-legal `match` + early `return`; `|` or-patterns are pure sugar for N already-legal single-pattern arms sharing a body; `'...'` is pure sugar for an already-legal `"..."` literal. No new runtime capability, gate, or enforcement surface is introduced by any of the three.

## Compatibility

All three are additive: `?`, bare `|`, and `'` are lex errors today (confirmed for `?`/`|` by `token.rs`'s single-character dispatch; `'` isn't matched by any existing lexer arm either), so every valid `.nir` program that compiles today has none of them and is completely unaffected. No existing valid program's meaning changes.

## Rejected alternatives

1. **A fully general `?` usable in any expression position.** Rejected for v1 — needs a bottom/never type in the type system so a `return`-from-inside-an-expression can unify with any expected type, a materially bigger change than this RFC's own restricted-statement-position version, and no real field failure has needed it yet (the actual observed mistake was a direct `let` initializer).
2. **`?` with Rust-style automatic error-type conversion (`From`).** Rejected for v1 for the same reason: real, useful, but a second feature (a trait-like conversion mechanism this language has no precedent for) layered on top of the operator itself. An exact-type-match requirement is honest and simple; loosening it is real, disclosed follow-up work if a real program needs to propagate through a type-changing error boundary.
3. **General or-patterns, including binding ones (`Ok(x) | Err(x) => ...`).** Rejected for v1 — correctly enforcing "every alternative binds the same names at the same types" is a real, separate typeck feature, and the zero-payload-only scope covers the likely reflex case (enum-constant grouping, literal grouping) without it.
4. **A real `char` type instead of `'...'` as string sugar.** Rejected — this language has never needed one (no scalar operation distinguishes a single character from any other string), and adding one ripples through every pass that enumerates scalar types for no expressive gain over what `'...'`-as-string-sugar already delivers.
5. **Do nothing further; rely on #64's fixed hint cache alone.** Considered seriously — the cache now genuinely self-heals for *any* future "unexpected character" mistake once it's hit once. Rejected as the *only* answer because it still costs one real self-repair round trip (and, before that first hit, a possibly-failed generate run) for every distinct reflex mistake, forever, versus a one-time compiler change that removes the mistake's cost permanently for the reflexes a Rust/Python-trained model is near-certain to keep repeating.

## Open questions

1. **Escape sequences inside `'...'`.** Proposed: identical to `"..."` (same escape table, same rules), for zero special-casing. Confirm no existing lexer test assumes `'` can never start a token before landing this.
2. **Should `?` require the immediate `let`/`return`/bare-statement form to be syntactically adjacent, or does `let x: T = (foo())?` (a parenthesized inner expression) also count as "directly"?** Proposed: yes, parens don't disqualify it — `typeck` should look through `Expr::Paren` (if one exists) or accept it structurally regardless of wrapping parens, whichever the existing AST already does for other "must be a bare call" restrictions (`Expr::Spawn`'s own restriction is the precedent to check against).
3. **Order of implementation.** Given the real cost/risk difference in the table above, `'...'` and `|` or-patterns are each independently shippable in isolation (neither depends on `?`); `?` is the one that actually closes #63's root motivation. This doc takes no position on sequencing — a future implementation session's own call.
