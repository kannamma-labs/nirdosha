# Structural index — large `crates/compiler/src/` files

Line numbers are approximate, taken from the file state as of the date
in each file's header — they will drift as files are edited. If a
number looks wrong, `grep -n '<the name>' crates/compiler/src/<file>.rs` to
find the current location; the names in this index are the durable
part.

**2026-09-07 regeneration note.** This index was badly stale (last
dated 2026-08-23): it listed `interpreter.rs` and `serve.rs` as
current, large files with live line-by-line structure, but both were
deleted entirely in the interpreter-removal pass (`05a747c`) — their
sections are gone below, not just renumbered. Every remaining file
grew substantially in the two weeks since (codegen.rs alone gained
~1800 lines) and every section below was re-verified against the
current source rather than hand-patched. `runtime_kernels.rs` is also
gone from this list, but for a different reason: it moved out of
`crates/compiler/src/` entirely, into its own `crates/runtime-kernels/`
crate (`docs/adr/0003-runtime-kernels-cargo-dependency.md`) — out of
this index's own stated scope, not deleted. Newly added sections below
(`contract_check.rs`, `ownership.rs`, `smt.rs`, `refine.rs`, `main.rs`,
`token.rs`, `effects.rs`) either grew past this index's original
threshold or simply weren't covered before.

## codegen.rs (6886 lines, as of 2026-09-07)

- `1` — module doc: LLVM codegen strategy overview (docs/goal.md row 5, "native, hardware-speed codegen"); its own "what's supported" summary is disclosed as lossy — `check_supported`'s `unsupported(...)` call sites are the actual source of truth, not this comment (caught drifting once already re: `box`/`&`/`*`)
- `79` `struct CodegenError` — the one error type every codegen fallibility collapses into
- `89` `fn unsupported<T>` — the "reject, don't mis-compile" helper every not-yet-compiled construct calls
- `267`/`289` `fn affine_codegen_supported`/`affine_codegen_supported_visiting` — whether an affine-containing `Ty::Named` (Phase 4b) can compile
- `334` `fn llvm_ty` — maps a `Ty` to its LLVM type string (free-function form, used pre-`Codegen` construction)
- `515` `fn mangle_ty` — generic instantiation name mangling (`%Result$i64$str`-style)
- `570` `fn conservative_word_count` — over-allocated word count for a tagged-union enum payload
- `606`/`625`/`662`/`679`/`699`/`744` `fn agg_elem_and_len`/`elem_byte_size`/`ty_byte_size`/`agg_layout`/`agg_byte_size_operand`/`has_cyclic_layout` — `Vector`/`Matrix`/struct layout-size helpers feeding heap-alloc sizing
- `779` `pub fn check_supported` — thin wrapper over `check_supported_with_plugins` with an empty plugin set; the real gate: walks the whole program, rejects everything `codegen.rs` doesn't yet compile with a named reason (grep this for the ground-truth "what's compiled vs interpreter-only" list, don't trust docs)
- `799` `pub fn check_supported_with_plugins` — same gate, plus a plugin-builtin-name set to reject explicitly (rather than falling through to an untested "unknown function" path) once a plugin call passes `typecheck_with_plugins` — rfcs/0003's plugin gallery, `docs/ROADMAP.md` Track G/G1
- `883`/`890`/`911` `fn check_stmts`/`check_stmt`/`check_expr` — `check_supported`'s recursive walk
- `1108` `struct Scopes` / `impl Scopes` — codegen's own variable-name→(Ty, LLVM-pointer) environment
- `1139` `struct FnSig`
- `1154` `struct Codegen<'a>` — the main codegen driver: LLVM context/module/builder-equivalent state, register/label/global counters
- `1284` `pub fn emit_llvm_ir` — top-level entry point: program → full LLVM IR text
- `1300` `pub fn emit_llvm_ir_with_native_plugins` — the compiled-path counterpart for a project entrypoint with a native-callable plugin roster ready to link (rfcs/0005 §3), not the bare CLI (which can't discover a plugin crate on its own yet); validates each `NativePluginBuiltin` before delegating to `emit_llvm_ir_impl`
- `1314` `fn emit_llvm_ir_impl` — the shared body both `emit_llvm_ir` and `emit_llvm_ir_with_native_plugins` funnel through
- `1568` `fn bind_type_params_owned` — resolves a generic decl's type params against a concrete instantiation
- `1590` `impl Codegen<'_>` — the big impl block (through ~6710), methods below are inside it
- `1591`/`1595`/`1603` `fn fresh_reg`/`fresh_label`/`fresh_global` — the SSA-register/label/global-name counters
- `1611` `fn emit_alloca`
- `1627` `fn llvm_ty` (method) — pure delegation to the free function, but through `&mut self`
- `1668` `fn declare_named_type` — emits a `struct`/`enum`'s LLVM named type declaration (struct: real fields; enum: hand-rolled `{i64 tag, [N x i64] payload}`)
- `1731` `fn ctor_ty` — resolves a struct/enum constructor call's target type
- `1754` `fn infer_type_args`
- `1775` `fn field_index_and_ty`
- `1807`/`1836`/`1869` `fn construct`/`construct_struct`/`construct_variant` — lowering `Expr::Call` as a struct/enum constructor
- `1914` `fn store_value_into` / `1954` `fn expr_ptr_expected`
- `1968` `fn function` — lowers one `FnDecl` to an LLVM function definition
- `2096`/`2106` `fn stmts`/`fn stmt` — statement-sequence and single-statement lowering
- `2274` `fn local_ty_of` — recovers an already-typechecked expr's `Ty` without a full inference pass (trusts typeck already ran)
- `2434`/`2452` `fn array_lit_ty`/`mul_result_ty`
- `2473` `fn builtin_result_ty` — a builtin call's result type, needed before its args are known (for aggregate dest allocation)
- `2551` `fn widen_to_i64` — the one real signed/unsigned instruction choice (`zext` vs `sext`) this backend needs
- `2572`/`2589`/`2612`/`2642` `fn narrow_from_i64`/`to_i64_word`/`from_i64_word`/`is_word_sized` — the word-sized-value marshaling `spawn`/`chan` need to move an arbitrary scalar through one `i64`-wide kernel slot
- `2656` `fn spawn_thread` — `spawn name(args)`'s real codegen (`runtime-kernels/src/lib.rs`'s chan/spawn/join kernels); every param and the return type must be word-sized, checked here (no signature info in `check_supported`'s structural pre-pass)
- `2728` `fn emit_spawn_trampoline`
- `2796` `fn guard_in_range` — Tier-1/2 bounds check emission, elided where SMT already proved it safe
- `2830` `fn emit_affine_free` — real `nir_free` call driven by `ownership.rs`'s `FreeMap`
- `2951` `fn emit_frees_for_names`
- `2974` `fn emit_nfr_call_end` — emits `nir_nfr_call_end` at a return point; a no-op unless the current fn declared `nfr(...)` (NFR annotations, 2026-08 work)
- `2989` `fn emit_field_masking` — masks every `requires(...)`-annotated field of a struct value in place before it's returned to a caller without the matching proof; a no-op (no branch emitted) for any type with no masked field
- `3023` `fn emit_requirement_check` — whether the current fn's own role/claim-view *parameter* satisfies a `Requirement`, fail-closed (`"false"`) if it has no such parameter
- `3042` `fn emit_str_field_eq_check` — the shared core both `emit_requirement_check` and `emit_acquire` reduce to: GEP a field, load its `str`, compare via `nir_str_eq`
- `3082` `fn emit_zero_value` — the masked-out value `emit_field_masking` stores
- `3114` `fn guard_index_in_bounds` — `Vector`/`Matrix` dynamic-index bounds check
- `3141` `fn while_loop`
- `3180` `fn expr` — **the** scalar-value lowering entry point: `Expr` → one LLVM register/literal operand
- `3703` `fn call` — scalar-returning call dispatch (builtins + user fns)
- `4002` `fn call_args`
- `4065` `fn call_indirect` — `fn(..)->..`-typed value calls; spells out the full `<ret>(<params>)` function type at the call site since the callee isn't a named `@fn`
- `4109` `fn call_builtin_scalar` — the huge per-builtin-name match for scalar-shaped builtins
- `4316` `fn bearing_deg`
- `4355` `fn call_builtin_agg` — same, for aggregate-shaped (`Vector`/`Matrix`-returning) builtins
- `4575`–`4768` geometry/linalg helper cluster: `lla_to_ecef_vals`, `ecef_to_lla_vals`, `enu_rotation_vals`, `mat_mul_ptr_vals`, `mat_mul_a_bt_vals`, `mat_vec_mul_ptr_vals`, `vec_add_vals`, `load_all_f64_vals`, `store_all_f64_vals` — the fully-unrolled-at-compile-time linalg codegen
- `4786` `fn expr_ptr` — **the** aggregate-value lowering entry point: `Expr` → pointer to a stack-allocated value (the `struct`/`enum`/`Vector`/`Matrix` counterpart to `expr`)
- `4897` `fn array_lit`
- `4934` `fn call_ptr` — aggregate-returning call dispatch (mirrors `call`)
- `4973` `fn emit_check_role` — `check_role(identity, "role")`'s codegen: reads `identity.claims_json` (`nir_check_role`, `runtime-kernels/src/lib.rs`) and hand-builds a `Result<RoleView, str>` (same tag-then-payload shape `construct_variant` uses generically, written directly here since no `Expr` exists to recurse on)
- `5062` `fn emit_acquire` — `acquire`'s codegen: `Ok(RoleView(name))`/`Err(reason)` gated on `emit_str_field_eq_check` against an arbitrary `proof` expression's own `role`/`value` field
- `5132` `fn binary` — the main `BinOp` lowering entry point, dispatches to the specialized helpers below for non-scalar cases
- `5289`/`5320`/`5347`/`5373` `fn guard_nonzero_divisor`/`guard_call_ok`/`guard_io_ok`/`guard_recv_ok` — the shared Tier-1/2 trap idiom (icmp → branch → flight-recorder dump + `abort()` + `unreachable`) for division, a fallible native-kernel call (`nir_inv`/`nir_solve`/singular-matrix), a negative `tcp` I/O result, and `recv`'s `<= 0` (peer-closed-is-also-an-error) case respectively
- `5399`/`5408`/`5422`/`5434`/`5444`/`5455` low-level emit helpers: `str_parts`, `icmp`, `fcmp`, `agg_elem_ptr`, `agg_load_elem`, `agg_store_elem`
- `5465`/`5477`/`5490`/`5501`/`5510`/`5517` `fn emit_mul`/`emit_add`/`emit_sub`/`float_const`/`emit_call1`/`emit_call2` — scalar-instruction emit helpers
- `5529` `fn emit_deep_eq` — recursive structural equality for any `Ty` (mirrors the interpreter-era `Value::PartialEq`: content equality for `box`/`&`, field-by-field for structs, tag-then-payload for enums, elementwise for `Vector`/`Matrix`)
- `5740` `fn agg_eq` — `struct`/`enum`/`Vector`/`Matrix` structural `==`/`!=`, built on `emit_deep_eq`
- `5758` `fn str_eq` — `str` `==`/`!=` via a native-runtime `nir_str_eq` call, not hand-emitted IR
- `5783`/`5796`/`5850`/`5923` `fn agg_binary`/`agg_elementwise`/`agg_mul`/`agg_scale` — `Vector`/`Matrix` `+`/`-`/`.*`/`./`/`*` lowering, fully unrolled
- `5937` `fn short_circuit` — `&&`/`||` as real branches, not eager and/or
- `5976` `fn block_trailing_ty`
- `5983` `fn if_expr` — `if`/`else` as a value-producing expression (the block-value-then-merge pattern `match_expr` reuses)
- `6155` `fn match_expr` — top-level `match` lowering, delegates to `match_enum`/`match_literal`
- `6216` `fn match_enum` — real LLVM `switch` on the tag word, one case per declaration-order variant
- `6341` `fn match_literal` — `str`/`i64`/`bool` literal-pattern match; `str` as a sequential `nir_str_eq`-then-branch chain
- `6510` `fn block_side_effects` — runs a block for side effects only (unit-valued `if`/`match` arms)
- `6533` `fn block_value_to_slot`
- `6578` `fn emit_c_main` — synthesizes the real C-ABI `main` wrapping the program's own `main` (return-value-as-exit-code / str-prints-and-exits convention)
- `6715` `pub enum OptLevel` — `-O0`/`-O2` (the whole point of the doc comment above it: `-O2`'s aggressive optimization is what actually stresses a subtly-wrong `unreachable`)
- `6737` `static RUNTIME_KERNELS_LIB` — the statically-embedded `runtime_kernels.rs` static-lib bytes (`include_bytes!`), so a compiled binary has no runtime dependency on this compiler's own installation
- `6751` `static NATIVE_STATIC_LIBS` — OS-level system libs `RUNTIME_KERNELS_LIB`'s code needs at final link time (e.g. `ws2_32.lib` for `nir_tcp_*` on Windows), captured by `build.rs` via `rustc --print=native-static-libs`; read only under `#[cfg(windows)]`
- `6753` `pub fn build` — the actual `clang`/linker invocation producing a native binary from emitted IR
- `6767` `pub fn build_with_native_plugins` — `build`'s counterpart for a project entrypoint with a native-callable plugin roster (rfcs/0005 §3)
- `6778` `fn build_impl` — the shared body both `build` and `build_with_native_plugins` funnel through

## typeck.rs (5151 lines, as of 2026-09-07)

- `61` `pub enum TypeErrorKind` — every structured type error shape (serializes for `--format=json`); every new static check this project adds gets a variant here, e.g. `StrInFnSignature`
- `527` `pub struct TypeError` / `532` `impl Display` — wraps a `TypeErrorKind` with its `Span`, and formats it human-readably
- `1014` `enum MatchWant<'t>` — the three-way "how is this `match`'s value used" distinction (bare statement / no-expected-type infer / a real expected `&Ty`) a plain `Option<&Ty>` would wrongly conflate — see `check_match`
- `1020`/`1039` `struct FnSig`/`Scopes` — signature table and lexical-scope variable-type stack
- `1059` `pub struct Checker<'a>` — the whole typecheck pass's state (registry, signatures, accumulated errors)
- `1115` `pub fn typecheck` — top-level entry point; requires a zero-arg `fn main()`, the entrypoint every `build`/`emit-llvm` caller is about to execute
- `1130` `pub fn typecheck_optional_main` — same, but tolerates no `main` at all — a generated nirdosha-lane program (N `module { }` blocks of `fn`/`screen`, no `main`) is exactly this shape by design
- `1139` `pub fn typecheck_with_native_plugins` — plus a native (compiled-path) plugin's declared signatures (`plugin::NativePluginBuiltin`, rfcs/0008) — registers name/params/ret for arity/type checking only, since a native plugin has no effects field
- `1157` `pub fn typecheck_optional_main_with_ui_components` — plus `ui_plugin::NativeUiComponent`-contributed `layout` widget kinds (rfcs/0009 Phase B); a component is only referenceable inside `layout { ... }`, never callable from `.nir` code like `plugins` above
- `1177`/`1184` `pub struct TypeWarning` / `pub enum TypeWarningKind` — non-fatal findings kept out of `TypeError` on purpose: adding a new fatal condition to `typecheck`'s `Result` would be a behavior change beyond what a warning should cost
- `1237` `pub fn ungated_fn_warnings` — every `fn` reachable with no auth token and no `requires(public)` opt-out (`docs/API_TRUST_MODEL.md` T1b), surfaced at `serve`/`emit-ui` time instead of only discovered in a security review
- `1246`/`1258`/`1266` `fn is_reachable_with_no_token`/`is_verified_identity`/`is_optional_verified_identity` — `ungated_fn_warnings`' own helpers
- `1276` `pub fn workflow_owner_warnings` — every declared `workflow`'s non-terminal `state` with no `owner` entry; terminal states are exempt since nothing is ever *decided* there
- `1314` `pub fn collect_role_claim_strings` — every `role(...)`/`claim(...)` string literal a program references, for the demo identity picker; `role(...)` is any-of so every name is collected, not just the first
- `1371` `fn typecheck_impl` — the real pass every `pub fn typecheck*` above thin-wraps, parameterized over `require_main`/native-plugin signatures/plugin effects/`extra_widget_kinds`. `impl<'a> Checker<'a>` (`1597`–`4973`) holds every method below unless noted otherwise
- `1598` `fn error` — push one `TypeErrorKind` at a `Span`, the one place every check funnels through
- `1610` `fn check_fn_ref` — a screen/dashboard `->`-target slot (`list`/`create`/`update`/`delete`, an action's or dashboard tile's target) must be a bare `Expr::Ident` naming a real user fn; builtins excluded
- `1628` `fn check_visibility_expr` — `view: role("admin")` / `edit: claim(...)`, same shape `requires(...)` accepts but arriving as an ordinary `Expr`
- `1652`/`1688`/`1716` `fn check_pattern_expr`/`check_format_expr`/`check_min_max_expr` — `field <name> { pattern/format/min/max: ... }` shape + field-type-applicability checks, incl. compiling `pattern`'s regex via the `regex` crate at typeck time
- `1749` `fn check_field_render_expr` — a field's `render: "..."` value (e.g. `searchable_select`); its companion `source` key is checked separately since it's a sibling entry
- `1803` `fn check_searchable_select_source_expr` — `source` naming either a declared struct (paginated `/_nirdosha/table/<snake>` route) or a declared fn (unpaginated `callFn` fallback) — the one shape `check_fn_ref`'s "must be a fn" rule doesn't fit
- `1823`/`1892`/`1900` `fn check_screen`/`check_screen_layout`/`check_layout_node` — Row 12 UI-DSL shape checks; `check_layout_node` is this DSL family's first genuinely recursive node, widened by `extra_widget_kinds` (rfcs/0009 Phase B) past the closed `divider`/`card`/`timeline` set
- `1999` `fn check_validate` — a `validate { ... }` block's predicates, typed as real `bool` expressions regardless of the target fn's own shape (a struct param, a `db`-touching body, a loop all still get real static errors here, even though none are statically *proven* by the Z3 pass)
- `2042` `fn check_action_show_result` — `show_result: true` forces the action's already-resolved target fn to return `Result(json, _)`
- `2071` `fn check_dashboard` — Row 12 `dashboard` shape checks (sibling of `check_screen`)
- `2119` `fn check_encode_entry` — `chart`/`panel` `encode <channel> { field/type/aggregate }` (rfcs/0009 Phase A, shared between `visual` and `panel`); `field` is checked to be a string only, since the backing fn returns opaque `json` with no struct to resolve against
- `2185` `fn check_render_expr` — shared `render: "..."` value-in-allowed-set check, serving `visual`/`dashboard`/`panel` alike; `context` is a pre-formatted description since one check serves three callers
- `2209` `fn check_workspace` — `workspace`/`panel` shape checks (rfcs/0009 follow-up): mirrors `check_screen`, plus one extra check `screen` doesn't need — a panel's `source` must be a one-`id`-in/`Result(json,_)`-out fn (`docs/ROADMAP.md` Track E1)
- `2308` `fn check_metric_ref`
- `2320` `fn check_duplicate_type_params`
- `2340` `fn check_visibility` — `pub`/namespaced-name access rules; a qualified self-reference from within a declaration's own module is always legal even when not exported
- `2359` `fn validate_ty` — well-formedness of a type expression itself (arity, unknown names), recursing through every type-former the same way `ast.rs::Ty::contains_str` does
- `2414` `fn check_fn` — per-function entry point: the str-in-signature scan, then body-checks, then `NotAllPathsReturn`
- `2476` `fn check_workflow_decl` — a `workflow` block's `send_email`/`send_sms`/`notify`/etc. action calls; every argument's type is now checked against the callee's real declared parameter types (arity included), not just "does this fn exist"
- `2562`/`2568`/`2574` `fn check_stmts`/`check_block`/`check_stmt` — statement-level checking (no value context)
- `2615` `fn check_stmt_expr`
- `2641` `fn check` — **the** "does this expr have exactly this type" entry point; every value position goes through here, so "no implicit conversions" is enforced in exactly one place
- `2705` `fn infer` — the no-expected-type counterpart to `check`
- `3062`/`3110` `fn infer_sandbox_spawn`/`infer_spawn`
- `3134`/`3214`/`3235` `fn infer_transact`/`infer_transact_slot`/`infer_transact_slot_durable` — `transact`'s slot-shape rules
- `3270` `fn infer_call` — ordinary function-call type inference (builtins dispatch separately, see `infer_builtin_call`)
- `3403` `fn infer_acquire` — privileged first-class function (`acquire`/`requires`) checking
- `3432`/`3471` `fn infer_struct_construction`/`infer_variant_construction`
- `3542` `fn resolve_type_args` — generic instantiation resolution
- `3586` `fn check_match` — exhaustiveness (every enum variant covered exactly once, no wildcard in v1), using `MatchWant` to distinguish statement/value/expected-type positions
- `3742` `fn check_literal_match` — the `str`/`i64`/`bool` literal-pattern sibling of `check_match`
- `3852` `fn is_builtin_or_plugin`
- `3863` `fn infer_builtin_call` — the huge per-builtin-name signature table (this is where a new builtin's typeck signature gets added)
- `4542`–`4602` builtin-checking helpers: `builtin_arity_hint`, `wrong_arg`, `literal_dimension`, `expect_f64_matrix`/`expect_square_f64_matrix`/`expect_f64_vector`
- `4612` `fn infer_binary` — the `BinOp` type-inference dispatch (`Eq`/`NotEq` permit any matching type generically, including `struct`/`enum`)
- `4699`/`4759` `fn infer_mul`/`infer_hadamard` — linear-algebra `*` and elementwise `.*`/`./`
- `4788` `fn infer_array_lit`
- `4812` `fn check_bool_operand`
- `4824` `fn unify_operands` — the shared "do these two operand types agree" logic every binary operator's inference goes through
- `4895` `fn check_if` — every branch (including a missing `else`, unless `ty` is `unit`) must produce the same type
- `4955` `fn check_block_value` — a block used in value position; closes the `impl<'a> Checker<'a>` block at `4973`
- `4996` `fn bind_type_params` — Row 11 layer 6's structural type-parameter binder, `resolve_type_args`'s fallback path
- `5018`/`5034`/`5054`/`5068` `fn definitely_returns`/`if_definitely_returns`/`is_elementwise_operand`/`is_sandbox_safe` — free-function helpers used across the checker
- `5086`/`5123` `pub struct FragmentEnv` / `pub fn validate_fragment` — the agent-facing "typecheck one expression fragment in a given variable-type context" entry point (docs/goal.md row 9 / `docs/nirdosha-agent-api.md`'s A3 endpoint's underlying capability)

## ast.rs (2624 lines, as of 2026-09-07)

- `25` `pub enum Ty` — every static type this language has; the grammar's `type` production made concrete
- `346` `impl Ty` — methods below (through `632`) are inside it
- `347`/`375` `fn from_name`/`name` — string ↔ `Ty` for primitive type names
- `423`–`502` classification predicates: `is_unsigned`, `is_integer`, `is_numeric`, `is_aggregate` (lives in one SSA register vs. needs a stack slot — `codegen.rs`'s `expr` vs. `expr_ptr` split), `is_transact_scalar` (the 4 types `transact`'s durability log can serialize)
- `515` `fn contains_str` — the "enum favoring" str-ban's recursive scan, walks every type-former the same way `validate_ty`/`substitute_ty` do
- `546` `fn is_affine` — the property that makes ownership meaningful; deliberately blind to `Ty::Named`'s real affinity (see `TypeRegistry::is_affine` below for the struct/enum-aware version)
- `571`/`587` `fn in_range`/`bounds` — an integer type's legal value range, used by both the interval-analysis proof and codegen's guard emission
- `633`/`645` `pub struct Param`/`Field`
- `685` `pub struct StructDecl`
- `732`/`746` `pub struct Variant`/`EnumDecl`
- `784`/`943` `pub fn prelude_enums`/`prelude_structs` — `Option(T)`/`Result(T,E)` and every Row-12 identity struct (`VerifiedIdentity`, `RoleView`, ...), injected into every program at parse time
- `923` `fn zero_payload_enum` — shared constructor `prelude_enums` calls for each of its zero-payload enums
- `1101`/`1115` `pub enum Effect` / `impl` (`name`) — the `effect(...)` annotation vocabulary (`pure`/`io`/`network`/`concurrent`/`rng`)
- `1133` `pub struct TransactSlot`
- `1140` `pub struct FnDecl` — a function declaration's full shape (params, ret, body, `effect(...)`, `requires(...)`)
- `1232`/`1239` `pub struct NfrSpec` / `impl` — a non-functional-requirement annotation (a latency/error-rate budget) on a `fn`; a sustained violation escalates on every trip, no debouncing (disclosed follow-up)
- `1259`/`1271` `pub enum Requirement` / `impl` (`proof_ty`, `describe`) — `requires(role/claim: ...)`'s parsed shape
- `1295` `pub struct Block`
- `1300` `pub enum Stmt`
- `1321`/`1348` `pub enum BinOp`/`UnOp`
- `1354` `pub enum Expr` — every expression form the parser produces
- `1573` `pub struct MatchArm` / `1602` `enum LiteralPattern` / `1610` `enum ElseBranch`
- `1615` `impl Expr` (`span`) — every `Expr` variant's source span, for error reporting
- `1654` `pub struct Program` — the whole-program AST root
- `1724` `pub struct ValidateDecl` — a `validate { ... }` block's predicates, mirroring `contract_check::check_fn_contract`'s own `&[String]` list-of-predicates shape, just fed already-parsed `.nir` `Expr`s instead of extraction-JSON strings
- `1740` `pub struct ImportDecl` — one file's own `import` statement, uninterpreted here — resolving/loading/merging another file's `pub` declarations is `main.rs`'s job
- `1777` `pub fn well_known_format_pattern` — `field <name> { format: "..." }`'s fixed vocabulary (`email`/`phone`/`date`/`url`/`uuid`) → regex, the single source of truth `typeck.rs`/`ui_gen.rs` both consume
- `1794`/`1804` `pub struct FieldOverride`/`ActionDecl`
- `1828`/`1877` `pub enum LayoutNode` / `impl` — the recursive `layout { ... }` widget tree; its `Widget` variant is rfcs/0009 Phase B's one plugin extension point, and this is this DSL family's first genuinely recursive node
- `1901` `pub struct ScreenDecl`
- `1917`/`1932` `pub struct MetricRef`/`DashboardDecl`
- `1946` `pub struct Transition` — a `workflow` state's `on <event> -> <state>`; a `via_link` transition means `workflow_lower.rs` also synthesizes an unauthenticated `<event>_via_link` fn and a per-workflow link-token struct
- `1958` `pub struct StateDecl` — `on_entry`/`on_exit` reuse `TransactSlot` (bare-call-only action slots) rather than a new node type
- `1996` `pub struct WorkflowDecl` — parsed as its own top-level decl rather than parser-time-flattened, since lowering needs the full set of states/transitions gathered first
- `2018`/`2034` `pub struct PanelDecl`/`WorkspaceDecl` — rfcs/0009 follow-up: several unrelated functions' results composed onto one page, all keyed off one `subject` struct's `id`
- `2070` `pub fn scope_key` — `(ns, name)` → the one canonical string both a namespaced declaration and every reference to it resolve through; no "which module am I inside" resolution context is ever needed
- `2077` `pub struct TypeRegistry<'a>` — the struct/enum declaration lookup table every later pass (typeck, ownership, effects, codegen, ui_gen) builds once and queries repeatedly
- `2105`–`2286` `impl<'a> TypeRegistry<'a>` methods: `empty`, `build`, `struct_decl`/`enum_decl`, `struct_fields`/`enum_variants`, `struct_type_params`/`enum_type_params`, `is_struct`/`is_enum`, `find_variant`, `is_affine`/`is_affine_visiting` (the struct/enum-aware version, with a cycle guard, delegates to `Ty::is_affine` for everything else)
- `2287` `pub fn result_of` — `Result(ok, str)` shorthand every builtin signature uses (builtins are exempt from the str-ban)
- `2296` `pub fn workflow_result_of` — same shape, `WorkflowActionError` in place of `str`, for `send_email`/`send_sms`/`send_push`/`notify`/`workflow_lower.rs`-synthesized fns (`docs/WORKFLOW.md`)
- `2300`/`2316` `pub fn zip_type_params`/`substitute_ty` — Row 11 layer 6's generic-substitution mechanism, used identically by typeck/ownership/codegen
- `2352` `pub fn literal_value` — extracts a literal integer from an `Expr`, for `zeros`/`ones`/`identity`'s compile-time-only dimension argument
- `2384` `pub const BUILTIN_NAMES` — the full builtin name list (grep here for "does builtin X exist")
- `2622` `pub fn is_builtin`

## ui_gen.rs (2318 lines, as of 2026-09-07)
- `79` `const PRELUDE_STRUCT_NAMES` — the `ast::prelude_structs()` infrastructure types (`VerifiedIdentity`, `Money`, ...) screens are never derived from
- `83` `struct FieldSpec` — one field of a derived form/table column: control kind, gating (`view_roles`/`view_claim`/`edit_roles`/`edit_claim`), format constraints (`pattern`/`min`/`max`), a `render` display hint, and (2026-08-27+) `select_source`/`search_param`/`select_page_size` for a `searchable_select` field
- `174` `enum SelectSource` — `Table(snake_table_name)` (reuses `/_nirdosha/table/<t>` pagination) or `Fn(name)` (unpaginated) for a `searchable_select` field's dropdown source
- `181` `struct Action` — one CRUD-convention (or declared-custom) fn backing a screen: gating, `effect_badges`, its own `params` as a `FieldSpec` tree, plus `label`/`style`/`confirm`/`show_result` for a declared custom action
- `245`/`267` `struct Metric`/`EncodingChannelSpec` — one dashboard tile/chart; `chart_mark`/`chart_encoding` (rfcs/0009 Phase A) populated only when `render == MetricRender::Chart`
- `281`/`289` `enum MetricRender`/`impl` (`as_str`, `from_kv`) — `visual`'s closed render vocabulary: `BarChart`/`Graph`/`Heatmap`/`Timeline`/`Chart` (`Chart` added by rfcs/0009 Phase A)
- `322` `fn parse_chart_config` — flat `mark`/`encode.<channel>.<key>` kv-entries -> `(mark, encoding)`, already typeck-proven well-formed (rfcs/0009 Phase A)
- `350` `struct Screen` — one derived screen: title, `module` nav grouping, fields/actions, `is_singular`, and (Track F1) an optional declared `layout: Option<LayoutNode>`
- `383`/`406`/`414` `struct Panel`/`enum PanelRender`/`impl` — one `workspace` panel; same `Chart` addition as `MetricRender`, extended to panels
- `445` `struct Workspace` — one declared `workspace` block: subject struct's read-only header fields plus its panels
- `471` `struct WorkflowQueue` — one declared `workflow`'s derived "Workflows" nav entry (pending/submitted-by-me/advance/history fns, `data_fields`, `all_states` for a stepper)
- `506`/`531` `fn to_snake_case`/`to_display_label` — PascalCase struct-name conversions
- `521` `fn find_fn`
- `548` `fn ty_label` — a `readonly` field's honest type label
- `572`/`576` `fn resolve_struct`/`resolve_enum`
- `587` `fn is_date_like_field_name` — naming-convention heuristic for calendar-picker vs. plain text input
- `612`/`616` `fn build_field_root`/`build_field` — **the** field→form-control mapping: `build_field_root` gives every independent top-level field a fresh `visiting` cycle guard (2026-08-27, replacing a flat depth cap — a legitimate `Order -> LineItem -> Product -> Category` schema no longer misrenders as `readonly`); `Option(T)` unwraps, a zero-payload-only enum renders as a dropdown, `struct Text { value: str }` renders as a plain input, a struct reference expands one level deep, everything else falls back to `readonly`
- `758`/`762` `fn fn_requires_login`/`fn_role_gate`
- `770` `fn effect_badges`
- `786`/`815` `fn build_action`/`build_custom_action` — CRUD-convention and declared-`screen`-action derivation
- `834`/`849`/`860`/`873`/`886` `fn kv_str`/`kv_ident`/`kv_num`/`kv_bool`/`kv_gate` — `screen` DSL's `key: value` entry helpers (string/bare-name/numeric/boolean values, `role(...)`/`claim(...)` gate extraction)
- `904` `fn find_screen_decl`
- `910` `fn to_title_case`
- `924`/`931`/`942` `fn is_numeric_scalar`/`is_stat_return_ty`/`is_chart_return_ty` — `stat_`/`chart_` naming-convention dashboard-metric detection
- `954`/`971`/`990` `fn build_metric_from_fn`/`build_metrics`/`apply_declared_metrics` — shared metric-building plumbing; `apply_declared_metrics` layers a declared `dashboard { tile/chart "..." -> fn }` on top of naming-convention inference
- `1000`/`1008` `fn build_stats`/`build_charts` — `build_charts` also folds in declared `visual` items (rfcs/0009 Phase A `chart` render, via `parse_chart_config`)
- `1034`/`1042`/`1063`/`1085`/`1126` `pub struct GatedField` / `fn gates_from_screen_decl` / `pub fn field_gates_for_struct`/`field_gates_for_fn`/`update_gates_for_fn` — field-visibility gate resolution. **Now dead code, zero callers**: these existed for `serve.rs`'s server-side redaction/edit-blocking to consult, but `serve.rs` was deleted entirely (interpreter-removal pass); nothing replaced that consumption (only a doc-comment mention remains, in `ast.rs:1817`) — grep-verified, not carried over from the stale prose still in this file's own doc comments
- `1158`/`1170`/`1176`/`1199` `pub struct ValidatedField` / `fn resolve_pattern`/`validations_from_screen_decl`/`pub fn field_validations_for_fn` — field-format-constraint resolution (`pattern`/`format`/`min`/`max`); same now-orphaned-by-`serve.rs`-deletion situation as `GatedField` above
- `1237` `fn apply_field_overrides`
- `1294` `fn build_screens` — assembles the full per-struct `Screen` list (the main derivation pass)
- `1371`/`1404` `fn build_workflow_queues`/`workflows_json`
- `1432`/`1470` `fn build_workspaces`/`workspaces_json`
- `1516` `fn action_json`
- `1535` `fn layout_json` — `layout { ... }` -> JSON tree; a `Widget` leaf's `entries` carries every kv-entry generically (not just the three std kinds' special-cased `source`/`title`), so a plugin-contributed widget's `render_js` can read its own config keys (rfcs/0009 Phase B)
- `1602` `fn widget_entry_json`
- `1612` `fn field_json`
- `1638` `fn identity_catalog_json`
- `1657` `fn mode_badge_html`
- `1667` `fn metrics_json`
- `1692` `fn manifest_json`
- `1760`/`1782`/`1795` `pub fn generate`/`generate_with_ui_components`/`fn generate_impl` — top-level entry points: program → complete self-contained HTML string; `generate_with_ui_components` (rfcs/0009 Phase B) additionally splices linked `ui_plugin::NativeUiComponent`s' JS in and registers them into `WIDGET_RENDERERS`
- `1842` `fn ui_components_script` — rfcs/0009 Phase B: splices each linked component's `render_js` plus a `WIDGET_RENDERERS[name] = render_fn` registration; empty (byte-identical `const WIDGET_RENDERERS = {};`) for `generate`'s own `components: &[]`
- `1861`/`1876` `fn logo_data_uri`/`favicon_data_uri` — brand PNGs embedded via `include_bytes!` as base64 `data:` URIs
- `1897`–`1986` `pub struct Theme` / `ThemeFonts`/`ThemeRadius`/`ThemeDensity`/`ThemeMotion`/`ThemeLayout`/`ThemeTypeScale` — 1:1 mirror of protobox's `resolve_design_tokens()` JSON shape (docs/LANGUAGE.md §11b)
- `1987` `fn theme_value_is_safe`
- `1997`–`2021` `const RAMP_STEPS` / `struct RampRoleStep` + the `PRIMARY`/`ON_PRIMARY`/`SURFACE`/etc. consts — the semantic-role → ramp-step mapping deriving `--md-*` from the raw `brand`/`neutral` ramps
- `2023`/`2162`/`2189` `fn theme_override_css`/`theme_html_class`/`theme_bootstrap_script` — the three `__NIRDOSHA_*__` placeholders `generate_impl` splices in
- `2204` `const TEMPLATE` — `include_str!("ui_gen_template.html")`

## parser.rs (2258 lines, as of 2026-09-07)
- `13` `pub struct ParseError`
- `18` `pub struct Parser` — hand-written recursive-descent, strictly one token of lookahead (`peek2` at `103` is the sole, deliberate exception), no backtracking (docs/GRAMMAR.md's Row 7 claim); `depth` is the combined `parse_expr`/`parse_unary` nesting counter `MAX_PARSE_DEPTH` guards
- `31` `const MAX_PARSE_DEPTH: usize = 50` — past this many combined `parse_expr`/`parse_unary` nesting levels, fail with a normal `ParseError` instead of a real stack overflow (a red-team finding: ~1000 nested parens SIGABRTs an unguarded parse)
- `55` `const MAX_LAYOUT_DEPTH: usize = 12` — same adversarial-input concern as `MAX_PARSE_DEPTH`, sized for `layout { }`'s much shallower realistic ceiling
- `57` `impl Parser` — the whole parser, methods below are inside it
- `64`/`74` `fn enter_nesting`/`exit_nesting` — shared entry/exit pair enforcing `MAX_PARSE_DEPTH`
- `78`/`82`/`86`/`94` `fn peek`/`span`/`bump`/`expect` — the token-stream primitives everything else is built from
- `103` `fn peek2` — one token past `peek()`; this grammar's only two-token lookahead, used by `parse_layout_container_body` to tell a `key: value` config entry from a nested layout item
- `105`/`124` `fn expect_ident`/`expect_usize_literal`
- `155` `fn parse_qualified_name` — `IDENT ("::" IDENT)*` joined into one canonical dotted-path string (`docs/ROADMAP.md` Track F, F2); every *reference* site (types, `match` variant names, `primary`'s bare idents) goes through this, never a separate `Path` AST node
- `187` `fn expect_type` — `type ::= "&"|"box"|"froze"|"thread"|"chan" type | "sandbox" | Vector(T,N) | Matrix(T,R,C) | handle(Kind) | fn(T,..)->R | i8|...|bool|unit | ident(type,*)`
- `264` `pub fn parse_program` — top-level entry point: leading `use "path.nir"*` imports, then `item*`; calls `workflow_lower::lower` on the assembled `Program` before returning
- `394` `fn parse_validate_decl` — `validate IDENT { kv_entry* }` (reuses `parse_kv_entry` unchanged for `pre`/`post`)
- `417` `fn parse_module_decl` — `module (STRING | namespace_module) { ... }`; dispatches to `parse_namespace_module_decl` on `IDENT`, else the legacy pure-nav-grouping `STRING` form
- `484` `fn parse_namespace_module_decl` — the real namespace form (Track F2): `pub?`-prefixed items get `exported`/`ns` set; `nav: "..."` overrides the display name, single-level only
- `557` `fn parse_kv_entry` — one `key: value` slot shared by `screen`/`field`/`action`/`paginate`/`panel` bodies; value is an ordinary `parse_expr()`
- `575` `fn parse_encode_channel_entries` — `encode <channel> { field/type/aggregate: ... }` inside a `visual`/`panel` body (rfcs/0009 Phase A), flattened into `"encode.<channel>.<key>"`-prefixed `KvEntry`s
- `614` `fn parse_screen_decl` — `screen IDENT { paginate{} | field{} | action{} | layout{} | kv_entry }*`; at most one `layout { }` per screen
- `660` `fn parse_field_override`
- `675` `fn parse_action_decl` — `action STRING -> IDENT { kv_entry* }?`, body optional
- `699` `fn parse_layout_decl` — `layout { layout_node* }`, wrapped in one synthetic root `LayoutNode::Column`
- `737` `fn parse_layout_node` — one `row`/`column`/`grid`/`group STRING?`/`tabs{tab STRING{...}}*`/`field IDENT`/`action STRING`/`IDENT { kv_entry* }` (widget leaf) item; depth-capped at `MAX_LAYOUT_DEPTH`
- `830` `fn parse_layout_container_body` — the shared `{ kv_entry* layout_node* }` body every container uses `peek2` to disambiguate
- `858` `fn parse_dashboard_decl` — `dashboard { (tile|chart) STRING -> IDENT | visual STRING -> IDENT { (kv_entry|encode)* }? }*`; at most one `dashboard { }` per program (enforced in `parse_program`)
- `917` `fn parse_workspace_decl` — `workspace IDENT { panel_decl | kv_entry }*` (Track E1)
- `949` `fn parse_panel_decl` — `panel STRING { action_decl | encode | kv_entry }*`, leading `"panel"` consumed by the caller
- `978` `fn parse_workflow_decl` — `workflow IDENT { data_block? state_decl+ }` (docs/WORKFLOW.md); rejects zero `state`s
- `1013` `fn parse_workflow_data_block` — `data { field ("," field)* ","? }`
- `1047` `fn parse_state_decl` — `state IDENT "terminal"? { on_entry{} | on_exit{} | on ... -> ... | kv_entry }*`
- `1095`/`1109` `fn parse_action_block`/`parse_action_call` — `on_entry`/`on_exit` bodies, reusing `TransactSlot` for a plain `name(args)` call
- `1123` `fn parse_transition` — `on "link"? IDENT -> IDENT`
- `1143` `fn parse_type_param_list` — a `struct`/`enum` declaration's bare-name type-parameter list
- `1165` `fn parse_struct_decl`
- `1199` `fn parse_field_mask_requires` — `field: ty requires(role/claim: ...)` on a struct field (masking gate; deliberately not shared with the fn-level `requires` parser since a field can't be `requires(public)`)
- `1233` `fn parse_enum_decl`
- `1275` `fn parse_fn_decl` — `fn IDENT(params) (-> type)? effect(...)? requires(...)? nfr(...)? block`
- `1313` `fn parse_effect_annotation` — `effect(pure | rng,io,concurrent,network)`; `pure` (the empty set) can't combine with others
- `1373` `fn parse_requires_annotation` — `requires(role: "..." | claim: "...", "..." | public)`
- `1411` `fn parse_nfr_annotation` — `nfr(latency_ms/error_rate_max/throughput_min_per_sec/concurrency_max: ...)`, every field optional, each independently range-checked (e.g. `error_rate_max` in `[0.0, 1.0]`)
- `1483`/`1504` `fn expect_i64_lit`/`expect_f64_lit` — bare (optionally negated) numeric literals for `nfr(...)`'s own slots
- `1529` `fn expect_str_lit`
- `1543` `fn parse_block`
- `1554` `fn parse_stmt` — `let | return | while | audited | expr`
- `1565` `fn parse_audited_stmt` — `audited STRING { stmt* }`
- `1584`/`1595`/`1606` `fn parse_let_stmt`/`parse_return_stmt`/`parse_while_stmt`
- `1615` `pub(crate) fn parse_expr` — expression entry point, feeds `if`/`transact`/`match`/assignment into the nesting-guarded precedence-climbing chain below
- `1638` `fn parse_match_expr` — scrutinee via full `parse_expr`; an arm's pattern is either a qualified enum-variant name (optionally `(bindings)`) or (2026-08-2x) a literal/wildcard pattern (`str`/`int`/`bool`/`_`, `ast::LiteralPattern`), dispatched on the arm's leading token alone
- `1727` `fn parse_transact_expr` — fixed-order `transact { precheck? network (retry N)? (timeout N)? verify commit compensate? log? }` (docs/TRANSACT.md — no permutation parsing)
- `1747`/`1773`/`1802` `fn parse_optional_int_modifier`/`parse_transact_slot`/`parse_optional_transact_slot`
- `1813` `fn parse_assignment` — `ident "=" assignment | logic_or`, right-associative
- `1833` `fn parse_if_expr`
- **Precedence-climbing chain** (each calls the next, lowest to highest precedence): `1853 parse_logic_or` → `1864 parse_logic_and` → `1875 parse_equality` → `1891 parse_comparison` → `1909 parse_additive` → `1925 parse_multiplicative` → `1943 parse_unary`
- `1950` `fn parse_unary_inner` — the largest single production: `!`/`-`/`*`(deref)/`box`/`froze`/`&`/`spawn`/`acquire`/`join`/`chan`/`send`/`recv`/`sandbox`/`stop`/`connect`/`listen`/`accept`/`open`, literals
- `2110` `fn parse_call` — call-args and the chained-call rejection (`f()()` is a parse error)
- `2153` `fn parse_postfix` — `.field` access and `v[i]`/`m[i,j]` indexing (one bracket group each, not chained `[i][j]`), sits between `call` and `primary` — neither can follow a *call*'s own result
- `2181` `fn parse_primary` — literals, qualified idents, parenthesized exprs, and `[e1, e2, ...]` array/vector/matrix literals (`Expr::ArrayLit`, `typeck.rs` decides which)
- `2247` `pub fn parse_standalone_expr` — module-level fn (outside `impl Parser`): parses `src` as one bare expression, not a whole program — `contract_check.rs`'s entry point for turning a Hoare predicate string into a real `Expr` via this grammar's own precedence, requiring the whole string consumed

## contract_check.rs (909 lines, as of 2026-09-07)

- `1` — module doc: Tier-1 contract checking (`docs/API_TRUST_MODEL.md` §7.5, built out for real) — given a real `.nir` fn and a Hoare predicate string (a user story's `pre_logic`/`post_logic`, or a `workflow`'s `routing_fn` contract), either proves it for every admissible input or produces a concrete counterexample; deliberately narrow scope (integer params/return only, no loops/calls/division/interprocedural reasoning) — anything outside that is `Unsupported`, reported honestly rather than silently approximated (approximation is sound for a proof but unsound for a counterexample)
- `30` — doc comment on `extra_bindings`: a story's predicate can name a PRD concept (e.g. `high_value_threshold`) the real fn hardcodes as a literal rather than taking as a parameter; the caller must supply a concrete value per such name, or get `UnboundIdentifier` naming exactly the gap
- `56` `pub enum ContractCheckResult`
- `95` `pub fn check_fn_contract` — top-level entry point, `(program, fn_name, pre_logic, post_logic, extra_bindings)` → `ContractCheckResult`
- `142` `pub fn check_fn_contract_exprs` — same, given already-parsed `Expr`s instead of predicate strings
- `146` `fn check_fn_contract_exprs_with_summaries`
- `178` `struct Summary`
- `188` `pub struct ValidateOutcome` — one `validate { ... }` predicate's proof result
- `222` `pub fn run_program_validates` — every `validate` block in a program, proved
- `267` `pub fn check_program_contracts` — pass/fail gate, collapses every `ValidateOutcome` into one `Result`
- `284` `pub fn check_program_contracts_diagnostics` — same, structured per-violation diagnostics instead of a flat error list
- `298` `fn contract_error_message`
- `328` `pub struct ContractDiagnostic`
- `341` `pub fn unsupported_validate_notes` — every `validate` predicate this pass honestly can't model, surfaced by `main.rs::print_unsupported_validate_notes`
- `354` `fn check_fn_contract_parsed` — the real Z3-backed proof/counterexample search, once both predicate strings are parsed `Expr`s
- `429` `fn assert_bounds`
- `435`/`511`/`515` `fn collect_idents`/`collect_idents_block`/`collect_idents_stmts` — free-identifier discovery driving the `extra_bindings` requirement
- `535`/`537` `struct Scopes`/`impl` — symbolic-`Int`-valued variable environment (mirrors `smt.rs`'s own)
- `552` `struct Eval<'s>` / `599` `enum Flow` / `614` `impl Eval<'_>` — the fn body's own symbolic evaluator (through `~894`), structurally parallel to `smt.rs::Checker`
- `895` `fn is_bool_shaped`

## ownership.rs (855 lines, as of 2026-09-07)

- `1` — module doc: static move-checker (`docs/goal.md` row 1's "no GC, no manual `free()`"), runs after `typeck.rs` over the same AST; now genuinely load-bearing (2026-09 correction to this file's own doc comment — see the code itself) now that `codegen.rs`'s `emit_affine_free` drives real `nir_free` calls off this pass's `FreeMap`, unlike the deleted `interpreter.rs`'s clone-on-read semantics that made a use-after-move merely inert
- `16` — the affine rule: `Ty::is_affine` (currently only `Ty::Box`) using a variable by name (as a `let` initializer, assignment RHS, call arg, `return` value) transfers ownership; any later use on the same path is "use after move"
- `35` — known limitation: no place-expression semantics, so `&box T` is borrow-only, not read-through (`**r` through a reference doesn't work)
- `53` — loops are checked twice: once silently to discover what state one iteration produces, merged with the pre-loop state, then checked again for real from that merged state
- `76` `pub enum OwnershipErrorKind`
- `82`/`87` `pub struct OwnershipError` / `impl Display`
- `104`/`106` `struct OwnScopes`/`impl` — the move-tracking scope stack (name → (type, moved-flag))
- `143` `fn merge_moved` — branch-uniform merge: an affine value moved on only *some* `if`/`match` branches is rejected everywhere after that point
- `169` `pub struct FreeMap` — the ownership pass's real deliverable for codegen: which `nir_free` call to emit, at which AST node's span, for every affine binding
- `219`/`227` `fn still_owned_affine`/`all_still_owned_affine`
- `231` `pub struct Checker<'a>`
- `273` `fn builtin_return_ty` — needs the same builtin-signature shape `typeck.rs::infer_builtin_call` has, but without re-deriving it through real type inference
- `307` `fn run_checker`
- `332` `pub fn check_ownership` — top-level entry point
- `345` `pub fn compute_free_map` — the other top-level entry point, what `codegen.rs` actually calls to know what to free and when

## smt.rs (708 lines, as of 2026-09-07)

- `1` — module doc: SMT-backed refinement checking (`docs/goal.md` §3/§6 Phase 2's real Tier-1 pass), a real Z3 solver (`z3` crate) proving (1) an arithmetic expression's value fits its declared target type, (2) a division's divisor is never zero; supersedes `refine.rs`'s interval analysis as the primary Tier-1 checker, though `refine.rs` stays in the tree undeleted
- `48` — **now accurate as of this pass** (previously claimed "not wired to elide anything, no backend exists yet" — corrected 2026-09-07): `codegen.rs` takes `&SmtReport` directly and skips a `guard_in_range`/`guard_index_in_bounds` trap wherever `SmtReport::proven_in_range`/`proven_index_bounds` already proved it. `refine.rs`'s `RefineReport` has no equivalent consumer.
- `70` `pub struct SmtReport` — the whole program's Z3-discharged proof result set, consumed directly by `codegen.rs`
- `87`/`89` `struct Scopes`/`impl` — symbolic-`Int`-valued variable environment
- `115` `pub fn analyze` — top-level entry point, called from `main.rs::cmd_build`/`cmd_emit_llvm` before codegen
- `134`/`145`/`154`/`165` `fn assert_bounds`/`prove_in_range`/`prove_nonzero`/`prove_index_in_bounds` — the actual theorem-proving calls
- `178` `fn ty_dims`
- `186`/`196` `struct Checker<'s>`/`impl` (through `~602`) — the main symbolic-evaluation walk, mirroring `contract_check.rs::Eval`
- `603` `fn assigned_names` — free-variable/reassignment discovery for loop-invariant handling

## refine.rs (680 lines, as of 2026-09-07)

- `1` — module doc: bounds proving via interval (range) analysis, a documented Z3-independent substitution for `smt.rs`'s SMT approach — same two proof targets (value fits its type, divisor never zero), strictly weaker (no condition-based narrowing, no correlated-variable reasoning)
- `30` — **corrected 2026-09-07**: this module's own doc used to say "not wired to elide anything, there's no codegen yet" — codegen exists now and does elide checks, but from `smt.rs::SmtReport`, not this module's `RefineReport`; nothing in `main.rs`/`codegen.rs` calls `refine::analyze` today, only `tests/refine.rs` does, so the "documented Z3-unavailable fallback" this file describes isn't actually wired as a fallback anywhere in the real pipeline
- `78`/`83` `struct Interval`/`impl` — the abstract-interval arithmetic itself
- `158` `pub struct RefineReport` — Tier-1's proof result set, same shape/role as `smt.rs::SmtReport`, but orphaned from the build pipeline (see above)
- `182` `fn ty_dims`
- `190`/`192` `struct Scopes`/`impl` — interval-valued variable environment
- `236` `pub fn analyze` — top-level entry point (test-only caller today)
- `252`/`262` `struct Refiner`/`impl` (through `~574`) — the main interval-evaluation walk
- `575` `fn assigned_names`

## main.rs (613 lines, as of 2026-09-07)

- `3` `fn main` — CLI entry point: dispatches the first arg to a subcommand (`init`/`build`/`emit-llvm`/`emit-ast`/`emit-ui`/`emit-catalog`/`gen-crud`) or prints usage on no/unknown args. No bare `run`, `serve`, or `--sandbox-worker` remain — all three were removed along with `interpreter.rs`/`serve.rs` (see `typecheck_and_own_optional_main_with_ui_components`'s own doc comment at `87`).
- `32` `fn print_usage`
- `58`/`71` doc comment / `fn typecheck_and_own` — the shared parse→typecheck→ownership-check pipeline for `build`/`emit-llvm`. Returns the entry file's own source alongside the `Program`, a holdover from when the now-removed `cmd_sandbox_worker` needed it (`Interpreter::new`'s `source` arg); both current callers discard it (`let (program, _src) = ...`).
- `75`/`88` doc comment / `fn typecheck_and_own_optional_main_with_ui_components` — same pipeline but tolerant of no `fn main()` (a generated nirdosha-lane program's shape), plus every linked `ui_plugin::NativeUiComponent`'s `name` as a legal `layout` widget kind (rfcs/0009 Phase B). `cmd_emit_ui`'s sole entry point — no plain sibling exists, callers passing `&[]` when nothing was discovered.
- `101` `fn print_ungated_fn_warnings` — surfaces `typeck::ungated_fn_warnings` at `emit-ui` time (`docs/API_TRUST_MODEL.md` T1b)
- `113` `fn typecheck_and_own_impl` — the real shared body both `typecheck_and_own` and the UI-components variant funnel through
- `169` `fn print_unsupported_validate_notes` — surfaces `contract_check::unsupported_validate_notes`
- `176`/`179` doc comment / `fn load_theme` — `--theme <path>` (`emit-ui`) resolution: reads a JSON file matching `ui_gen::Theme`'s shape, layered over the baked-in MD3 tokens
- `188`/`202` doc comment / `fn cmd_init` — `nirdosha init <project-name> [--dest][--no-email][--no-roles][--sms][--push][--force]`: scaffolds a starter `.nir` file, a bundled copy of this executable, a `run.sh`/`run.bat` launcher, and a placeholder `jwks.json`. **Stale scaffolding, a real bug, not just a doc issue**: the generated launcher (`init.rs::render_launcher_unix`/`_windows`) invokes `nirdosha serve {project_name}.nir ...` — `serve` was deleted from this file's own dispatch table along with the interpreter, so every freshly-`init`'d project's `./run.sh`/`run.bat` fails with an unknown-subcommand error today. Not fixed here — there's no compiled-path equivalent of dynamic, auth-gated `serve` to point it at yet.
- `307` `fn cmd_build` — `nirdosha build <file> -o <out> [--opt0]`: full pipeline → `codegen::build` → native binary
- `348` `fn cmd_emit_llvm` — same pipeline, prints IR text instead of linking
- `382` `fn cmd_emit_ast` — parses only (not the full `typecheck_and_own` gate)
- `421`/`424` doc comment / `fn cmd_gen_crud` — `nirdosha gen-crud <plan.json> --db <literal> [-o out.nir]`: generates real, compiling CRUD persistence bodies from a plan (`crud_gen.rs`), replacing protobox's placeholder-only Python stub generation, deterministically, no LLM call
- `480` `fn resolve_ui_component_manifest` — `--manifest-path` if given, else a `Cargo.toml` sitting next to the input `.nir` file, else `None` — feeds `ui_plugin::discover_components` (rfcs/0009's Cargo-metadata auto-discovery)
- `488` `fn cmd_emit_ui` — `nirdosha emit-ui <file.nir> -o out.html [--theme <path>][--manifest-path <Cargo.toml>]`: needs the *typed* program (screen inference reads resolved struct fields/fn signatures)
- `575` `const STD_CATALOG_JSON` — `include_str!` of `catalog/std/0.1.json` (rfcs/0009 Phase 0)
- `577` `fn cmd_emit_catalog` — `nirdosha emit-catalog [-o out.json]`: prints the std catalog (no plugin-merge step yet — see `rfcs/0009`'s Status box)

## token.rs (521 lines, as of 2026-09-07)

- `1` — module doc: tokens and the lexer; every token carries a `Span` for structured error reporting downstream (parser, typeck, codegen — corrected 2026-09-07, previously said "parser, interpreter")
- `11` `pub struct Span`
- `17` `pub enum Tok` — every token kind the lexer produces
- `233` `pub struct Token`
- `238` `const TYPE_NAMES` — the fixed primitive/keyword type-name list `expect_type`/the lexer both key off
- `244` `pub struct LexError`
- `249` `pub struct Lexer<'a>` — the hand-written lexer, methods below are inside it

## effects.rs (512 lines, as of 2026-09-07)

- `1` — module doc: effect inference (`docs/goal.md` rows 4/9, `docs/PROTOLANG_PORT.md`'s "Locked design 1") — computes every fn's real effect set (`ast::Effect`) by walking its body; `typeck::typecheck` is the one that checks a declared `effect(...)` annotation against this pass's result, this module only computes
- `18` — classification table: `pure` (empty set, default), `rng` (`rand_*`), `io` (`print`, file I/O, SQLite), `concurrent` (`spawn`/`join`, `chan`, `sandbox`/`stop`), `network` (`connect`/`listen`/`accept`, `tcp`, `mq_*`, Postgres-flavored `db_connect`)
- `36` — recursion: least-fixpoint iteration over the whole call graph (correct for direct/mutual recursion), monotone lattice of at most 4 tags, so termination is guaranteed
- `57` `pub struct FnEffects`
- `62` `pub fn infer_effects` — top-level entry point
- `76` `pub fn infer_effects_with_plugins` — same, plus a native/UI-plugin's own declared effect set folded in
- `115`/`117` `struct Scopes`/`impl`
- `135`/`141`/`147` `fn walk_stmts`/`walk_block`/`walk_stmt`
- `181` `fn local_ty` — a local binding's already-syntactically-explicit type, looked up not inferred
- `277` `fn db_connect_effect` — `network` vs `io` classification for `db_connect`, based on whether its connection-string argument is (or might be) Postgres
- `290` `fn walk_expr` — the main per-expression effect-accumulation walk
