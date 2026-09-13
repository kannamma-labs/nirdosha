use super::*;
use super::layout::*;

/// Builtins with codegen support as of Phase 4 of the Vector/Matrix
/// codegen plan — every one of these has loop trip counts that depend
/// only on compile-time-known shape (never on runtime data values), so
/// each fully unrolls into straight-line IR (module doc / design decision
/// 3). `det`/`inv`/`solve`/`rank`/`kf_update_state`/`kf_update_cov` have
/// genuine data-dependent control flow (partial-pivot search) and are
/// deliberately excluded — they land via a linked runtime call in a later
/// phase, not unrolled IR. `rand_seed`/`rand_f64`/`rand_gaussian` are also
/// excluded (unrelated to Vector/Matrix; no RNG state exists in generated
/// code yet).
pub(super) const PHASE4_BUILTINS: &[&str] = &[
    "transpose",
    "dot",
    "cross",
    "zeros",
    "ones",
    "identity",
    "sum",
    "len",
    "norm",
    "norm1",
    "norm_inf",
    "frobenius_norm",
    "trace",
    "is_symmetric",
    "is_diag",
    "is_square",
    "distance",
    "bearing",
    "lla_to_ecef",
    "ecef_to_lla",
    "ecef_to_enu",
    "enu_to_ecef",
    "kf_predict_state",
    "kf_predict_cov",
];


/// Phase 5's builtins — genuine data-dependent control flow (partial-pivot
/// row selection), so these go through a linked native `call` into
/// `runtime-kernels/src/lib.rs`'s staticlib (see `call_builtin_scalar`/
/// `call_builtin_agg`'s dispatch and `build()`'s embedded-lib linking)
/// rather than unrolled IR the way every `PHASE4_BUILTINS` name is.
pub(super) const PHASE5_BUILTINS: &[&str] = &["det", "inv", "solve", "rank", "kf_update_state", "kf_update_cov"];


/// `sha256_hex`/`constant_time_str_eq` — also linked native calls into
/// `runtime-kernels/src/lib.rs` (a from-scratch SHA-256, since that file has no
/// access to the `sha2` crate `interpreter.rs` uses — its own module doc
/// explains why), but kept as their own list rather than folded into
/// `PHASE5_BUILTINS`: that list's whole documented reason is "genuine
/// data-dependent control flow in dense linear algebra," which doesn't
/// describe these two at all (bit-manipulation over a runtime-length
/// byte buffer, not a matrix). Handled directly in `Codegen::call`
/// (like `print`), not through `call_builtin_scalar`/`call_builtin_agg`
/// — neither fits: `sha256_hex` returns `str` (not `Ty::is_aggregate()`,
/// so not `call_builtin_agg`'s convention; not a plain numeric scalar
/// either, so not `call_builtin_scalar`'s).
pub(super) const STR_CRYPTO_BUILTINS: &[&str] = &["sha256_hex", "constant_time_str_eq"];


/// `str_slice`/`str_index_of` — the minimal string-parsing surface a
/// compiled HTTP `serve` needs to hand-parse a request line (`BUILTIN_NAMES`'
/// own doc comment has the full scope). `str_slice` is pure pointer
/// arithmetic on the existing `{ptr, i64}` `str` representation, no kernel
/// call; `str_index_of` is the one real byte-scan, linked to
/// `nir_str_index_of`. Own list, not folded into `STR_CRYPTO_BUILTINS`, for
/// the same "describes something different" reason that one isn't folded
/// into `RAND_BUILTINS`.
pub(super) const STR_BUILTINS: &[&str] = &["str_slice", "str_index_of"];


/// `rand_seed`/`rand_f64`/`rand_gaussian` — a process-wide SplitMix64/
/// Box-Muller stream in `runtime-kernels/src/lib.rs` (its own module doc on why
/// this needed real RNG *state* in generated code, the one thing that
/// was actually missing before — the algorithm itself is a small, pure
/// function, same class as `sha256_hex`). Its own list, not folded into
/// `STR_CRYPTO_BUILTINS`, for the same "describes something different"
/// reason that one isn't folded into `PHASE5_BUILTINS`.
pub(super) const RAND_BUILTINS: &[&str] = &["rand_seed", "rand_f64", "rand_gaussian"];


/// `sleep_ms(ms)` — `docs/ROADMAP.md`'s own B9 ("small, currently
/// omitted... found this session, not previously tracked anywhere"). A
/// real wall-clock sleep, `nir_sleep_ms` (`runtime-kernels/src/lib.rs`).
pub(super) const SLEEP_BUILTINS: &[&str] = &["sleep_ms"];


/// `dec_from_i64`/`dec_to_str`/`dec_round`/`dec_scale`/`dec_from_str` —
/// linked calls into `runtime-kernels/src/lib.rs`'s `rust_decimal`-
/// backed kernels (rfcs/0005-plugin-boundary-safety-and-performance.md's
/// own build-architecture-change finding: `dec128` was interpreter-only
/// specifically because the old bare-`rustc` kernel build had no way to
/// reach `rust_decimal` at all). Own list, not folded into
/// `STR_CRYPTO_BUILTINS`/`RAND_BUILTINS`, for the same "describes
/// something different" reason those two aren't folded into each
/// other.
///
/// `dec_from_str`'s `.nir`-visible return type is `Result(dec128, str)`
/// — it reuses `check_role`'s own established convention
/// (`emit_result_merge`, see that fn's doc comment) via `emit_dec_from_str`
/// (`Codegen::call_ptr`'s dispatch), the same generic tag-then-payload
/// machinery every `db`/`json` builtin already uses. `nir_dec128_from_str`
/// now takes a real `out_err: *mut NirStrOut` alongside its `ok_ptr`
/// bool, populated with a real message on a malformed string —
/// `construct_variant`'s generic payload-placement machinery
/// (`conservative_word_count`'s `Ty::Dec128 => 2` arm) already handled
/// the payload-placement half of this.
pub(super) const DEC128_BUILTINS: &[&str] = &["dec_from_i64", "dec_to_str", "dec_round", "dec_scale", "dec_from_str"];


/// `check_role` — the first compiled builtin to actually construct a
/// real `Result(_, _)` value as its return (`DEC128_BUILTINS`'s own
/// "not yet included: `dec_from_str`" doc comment names exactly this
/// gap; this is that convention, established for real). Originally
/// scoped narrower than the interpreter's own `check_role` (a plain
/// comma-separated role list, not real JSON, since no JSON parser was
/// linked into `runtime-kernels`) — 2026-09: with `serde_json` now
/// linked in for `oidc_validate_token` below, `nir_check_role`'s own
/// body was upgraded to real JSON-array parsing too (falling back to
/// the original comma-separated matching for a `VerifiedIdentity` built
/// directly in `.nir` source, not through `oidc_validate_token` — see
/// its own doc comment). No `codegen.rs` change was needed for that
/// upgrade, only `runtime-kernels`.
///
/// `oidc_validate_token`/`extract_claim`/`identity_expired` (2026-09):
/// real JWT/JWKS signature verification (`nir_oidc_validate_token`,
/// `jsonwebtoken`-backed, same `kty`-locks-`alg` guard
/// `crates/presence-gateway/src/jwt.rs` already uses) plus real JSON
/// claim extraction (`nir_extract_claim`). `identity_expired` needs no
/// kernel at all — `VerifiedIdentity.expires_at` is a plain `i64`
/// field, so it's `now > identity.expires_at` via GEP+load+`icmp`,
/// inline in `Codegen::call` (see its own dispatch arm). Authentication
/// (verifying the identity claims themselves came from a real signed
/// token) is now real, closing the gap this comment used to name as
/// separate from `check_role`'s own *authorization* pipeline.
/// `check_role_path`/`extract_claim_path` and the rest of Row 12
/// (sessions/refresh/revocation/`validate_api_key`) remain real,
/// narrower follow-up work, not attempted here.
pub(super) const IDENTITY_BUILTINS: &[&str] = &[
    "check_role",
    "oidc_validate_token",
    "mock_issue_token",
    "extract_claim",
    "identity_expired",
    "check_role_path",
    "extract_claim_path",
    "check_revocation",
    "create_application_session",
    "session_cookie",
    "verify_session",
    "new_refresh_token",
    "exchange_refresh_token",
    "validate_api_key",
];


/// `db_connect`/`db_query`/`db_execute` — real SQLite connectivity
/// (`Ty::Db`'s own doc comment, `nir_db_*`, `runtime-kernels/src/lib.rs`'s
/// "db kernels" section). Layer 1 only: SQLite via `rusqlite`'s
/// `bundled` feature; a Postgres connection string (`postgres://`/
/// `postgresql://`) is a real, deferred follow-up (dynamic TLS linking),
/// not silently dropped.
pub(super) const DB_BUILTINS: &[&str] = &["db_connect", "db_query", "db_execute"];


/// `env` (RFC 0011 §1) — reads a process environment variable via
/// `nir_env_get`. Not resource-gated: no `Domain`, no handle, no pool —
/// unlike `DB_BUILTINS` right above.
pub(super) const ENV_BUILTINS: &[&str] = &["env"];


/// `json_parse`/`json_get`/`json_get_str`/`json_get_i64`/`json_get_f64`/
/// `json_get_bool`/`json_array_get`/`json_array_len`/`json_set_str` —
/// real JSON navigation (`Ty::Json`'s own doc comment). Compiles
/// `Ty::Json` as the raw text itself (same `{ptr, i64}` representation
/// `Ty::Str` already has), each accessor re-parsing via `nir_json_*` —
/// see `Ty::Json`'s `llvm_ty` arm and `nir_db_query`'s doc comment for
/// the full reasoning on this representation choice.
pub(super) const JSON_BUILTINS: &[&str] =
    &["json_parse", "json_get", "json_get_str", "json_get_i64", "json_get_f64", "json_get_bool", "json_array_get", "json_array_len", "json_set_str"];


/// `mq_connect`/`mq_publish`/`mq_consume` — real Redis connectivity
/// (`Ty::Mq`'s own doc comment, `nir_mq_*`, `runtime-kernels/src/lib.rs`'s
/// "mq kernels" section). `mq_connect_via` (the plugin-dispatched
/// external-service-boundary path, `rfcs/0003-plugin-abi-v2.md`) is
/// deliberately not included here — a separate mechanism, owned
/// elsewhere.
pub(super) const MQ_BUILTINS: &[&str] = &["mq_connect", "mq_publish", "mq_consume"];


/// `http_get`/`http_post`/`https_get`/`https_post` — real HTTP(S) client
/// calls (`ast::BUILTIN_NAMES`'s own doc comment has the full, already-
/// locked design). No new `Ty` — `HttpResponse` is a plain prelude
/// struct, and every call here is a one-shot request/response, never a
/// persisted connection handle.
pub(super) const HTTP_BUILTINS: &[&str] = &["http_get", "http_post", "https_get", "https_post"];


/// `call_via` — rfcs/0011-uniform-service-provider-model.md §1/§2's
/// `call`-shape provider dispatch entrypoint. Kept as its own const
/// list, not folded into `HTTP_BUILTINS` above, since it isn't a fixed
/// core-HTTP call the way those four are: `http://`/`https://` sniff
/// straight through to the same core path, but any other scheme
/// resolves at runtime against the plugin-provider table
/// (`kernel::plugin_provider`) — a real, disclosed dispatch difference
/// worth its own named list even though `emit_call_via`'s own codegen
/// shape mirrors `emit_http_call`'s closely.
pub(super) const CALL_BUILTINS: &[&str] = &["call_via"];


/// `workflow` Layer 1 (`docs/WORKFLOW.md`) — every builtin
/// `workflow_lower.rs` desugars a `workflow` block's synthesized
/// functions into, plus `__workflow_overdue` (`docs/ROADMAP.md` A15,
/// added alongside this). `__workflow_pending_for_me`/
/// `__workflow_submitted_by_me`/`__workflow_history` are included here
/// (real, compiled, runtime-`Err`-producing — `Codegen::
/// emit_workflow_unsupported_query`'s own doc comment) rather than
/// rejected at compile time, because `workflow_lower.rs` synthesizes
/// `list_<w>_pending_for_me`/`list_<w>_submitted_by_me`/
/// `get_<w>_history` **unconditionally** for *every* `workflow` block —
/// unlike `__workflow_link_advance` (only synthesized when a `link`-
/// marked transition actually exists), a `check_expr`-time rejection of
/// these three would make *every* workflow fail to compile, not just
/// ones that call them. `__workflow_link_advance` is deliberately
/// **not** included here — magic-link consumption needs real durable
/// storage this Layer 1 doesn't have, and since its own `*_via_link` fn
/// only exists for a workflow that actually declares a `link` transition,
/// rejecting it at compile time (`check_expr`'s pre-pass) narrows
/// correctly, to exactly those programs, instead of blocking everything.
pub(super) const WORKFLOW_BUILTINS: &[&str] = &[
    "__workflow_start",
    "__workflow_advance",
    "__workflow_overdue",
    "__workflow_pending_for_me",
    "__workflow_submitted_by_me",
    "__workflow_history",
];


/// `send_email`/`send_sms`/`send_push`/`notify` (`docs/WORKFLOW.md`) —
/// a real, generic, provider-agnostic authenticated HTTPS POST for the
/// first three; `notify` always takes the documented *offline* path
/// (falls back to `send_email`) since nothing in this round populates a
/// real presence table (`nir_notify`'s own doc comment).
pub(super) const NOTIFY_BUILTINS: &[&str] = &["send_email", "send_sms", "send_push", "notify"];


pub fn check_supported(program: &Program) -> Result<(), CodegenError> {
    check_supported_with_plugins(program, &std::collections::HashSet::new())
}


/// Same as [`check_supported`], plus a set of plugin builtin names
/// (`docs/ROADMAP.md` Track G, G1 / rfcs/0003-plugin-abi-v2.md's plugin
/// gallery) to reject explicitly rather than silently falling through.
/// Before this existed, a plugin builtin's name matched neither
/// `is_builtin` (deliberately excluded from `ast::BUILTIN_NAMES`) nor
/// any rejection arm here, so `check_expr`'s `Expr::Call` case walked
/// straight past it into the argument loop and returned `Ok(())` for
/// the call itself — meaning the moment `typecheck_with_plugins` made
/// a plugin call pass typechecking, it would reach real codegen
/// (`Codegen::call`) with no matching entry in either the builtin or
/// user-fn tables, an untested "unknown function" path rather than a
/// clean, named rejection. `check_supported` itself keeps its exact
/// existing signature (an empty `plugin_names` set) — its one real
/// caller (`emit_llvm_ir`) is unaffected until a future plugin-aware
/// `build`/`emit-llvm` path (not yet wired up — plugins are
/// interpreter-only for the CLI today) calls this sibling instead.
pub fn check_supported_with_plugins(
    program: &Program,
    plugin_names: &std::collections::HashSet<String>,
) -> Result<(), CodegenError> {
    // Real namespacing/`pub`/`use` (`docs/ROADMAP.md` Track F, F2;
    // `docs/NEXT_GEN.md` §F2) isn't ported to the compiled path yet — same
    // incremental-porting pattern Track B already uses for `transact`/
    // `db`/`json`/`mq`/etc. A namespaced (`module Ident { ... }`)
    // declaration's own `name` is deliberately left unmangled (`ast::
    // StructDecl::ns`'s doc comment: every pre-F2 consumer, including
    // this one, reads `.name` directly), so two such declarations in
    // different modules sharing a bare name would silently collide as
    // the exact same unmangled LLVM symbol if this ever reached real
    // codegen — rejected up front, honestly, rather than risking a
    // miscompile. A program with no real (`ns: Some(_)`) declaration is
    // completely unaffected — every existing `.nir` program, and every
    // legacy string-named `module "Display Name" { ... }` block.
    if let Some(f) = program.fns.iter().find(|f| f.ns.is_some()) {
        return unsupported(format!(
            "`{}` is declared inside a real `module {} {{ ... }}` namespace -- modules/`pub`/`use` \
             aren't supported by the compiled path (`nirdosha build`/`emit-llvm`) yet, only the \
             interpreter (`nirdosha <file>`/`serve`)",
            f.name,
            f.ns.as_deref().unwrap_or("")
        ));
    }
    if let Some(s) = program.structs.iter().find(|s| s.ns.is_some()) {
        return unsupported(format!(
            "`{}` is declared inside a real `module {} {{ ... }}` namespace -- modules/`pub`/`use` \
             aren't supported by the compiled path yet, only the interpreter",
            s.name,
            s.ns.as_deref().unwrap_or("")
        ));
    }
    if let Some(e) = program.enums.iter().find(|e| e.ns.is_some()) {
        return unsupported(format!(
            "`{}` is declared inside a real `module {} {{ ... }}` namespace -- modules/`pub`/`use` \
             aren't supported by the compiled path yet, only the interpreter",
            e.name,
            e.ns.as_deref().unwrap_or("")
        ));
    }
    let registry = TypeRegistry::build(program);
    // `Vector`/`Matrix` currently codegen only for scalar element types
    // (f64/i64/...). A non-scalar element (struct/enum/another Vector)
    // used to reach `elem_byte_size` and panic; reject it up front with
    // a named error so the generate repair loop can teach the rule.
    fn is_scalar_element(ty: &crate::ast::Ty) -> bool {
        matches!(ty, crate::ast::Ty::I8 | crate::ast::Ty::U8 | crate::ast::Ty::I16
            | crate::ast::Ty::U16 | crate::ast::Ty::I32 | crate::ast::Ty::U32
            | crate::ast::Ty::I64 | crate::ast::Ty::U64 | crate::ast::Ty::Usize
            | crate::ast::Ty::F64)
    }
    fn check_vec_elem(ty: &crate::ast::Ty) -> Option<String> {
        match ty {
            crate::ast::Ty::Vector(elem, n) => {
                if !is_scalar_element(elem) {
                    return Some(format!("Vector({elem:?}, {n})"));
                }
                check_vec_elem(elem)
            }
            crate::ast::Ty::Matrix(elem, r, c) => {
                if !is_scalar_element(elem) {
                    return Some(format!("Matrix({elem:?}, {r}, {c})"));
                }
                check_vec_elem(elem)
            }
            _ => None,
        }
    }
    for f in &program.fns {
        for p in &f.params {
            if let Some(bad) = check_vec_elem(&p.ty) {
                return unsupported(format!(
                    "`{}` uses {bad} with a non-scalar element type -- Vector/Matrix currently only support scalar elements (i64, f64, etc.); use `json` for a variable-length list of structs/enums",
                    f.name
                ));
            }
        }
        if let Some(bad) = check_vec_elem(&f.ret) {
            return unsupported(format!(
                "`{}` returns {bad} with a non-scalar element type -- Vector/Matrix currently only support scalar elements (i64, f64, etc.); use `json` for a variable-length list of structs/enums",
                f.name
            ));
        }
    }
    // Reject a cyclic struct/enum *declaration* itself, before any
    // function signature or body is even walked — the same "reject,
    // don't leak the backend's own error text" standard every other
    // `check_supported` rejection already holds itself to, rather than
    // letting `struct A { b: B } struct B { a: A }` reach real LLVM
    // codegen and surface a raw `clang`/LLVM "identified structure type
    // is recursive" error. `box`/`&` back-references remain fine (they
    // don't recurse past a pointer field — see `has_cyclic_layout`'s doc
    // comment), so this only fires on a genuinely infinite-size shape.
    for s in &program.structs {
        let mut visiting = vec![s.name.clone()];
        if s.fields.iter().any(|f| has_cyclic_layout(&f.ty, &registry, &mut visiting)) {
            return unsupported(format!(
                "`{}` is a cyclic struct type -- one of its fields eventually contains `{}` \
                 again with no `box`/`&` indirection in between, which has no finite size; wrap \
                 the back-reference in `box {}` (or `&{}`) to break the cycle",
                s.name, s.name, s.name, s.name
            ));
        }
    }
    for e in &program.enums {
        let mut visiting = vec![e.name.clone()];
        if e.variants.iter().any(|v| v.payload.iter().any(|t| has_cyclic_layout(t, &registry, &mut visiting))) {
            return unsupported(format!(
                "`{}` is a cyclic enum type -- one of its variants eventually contains `{}` \
                 again with no `box`/`&` indirection in between, which has no finite size; wrap \
                 the back-reference in `box {}` (or `&{}`) to break the cycle",
                e.name, e.name, e.name, e.name
            ));
        }
    }
    for f in &program.fns {
        for p in &f.params {
            llvm_ty(&p.ty, &registry)?;
        }
        llvm_ty(&f.ret, &registry)?;
        check_stmts(&f.body.stmts, plugin_names, &registry)?;
    }
    Ok(())
}


pub(super) fn check_stmts(stmts: &[Stmt], plugin_names: &std::collections::HashSet<String>, registry: &TypeRegistry) -> Result<(), CodegenError> {
    for s in stmts {
        check_stmt(s, plugin_names, registry)?;
    }
    Ok(())
}


pub(super) fn check_stmt(s: &Stmt, plugin_names: &std::collections::HashSet<String>, registry: &TypeRegistry) -> Result<(), CodegenError> {
    match s {
        Stmt::Let { ty, value, .. } => {
            llvm_ty(ty, registry)?;
            check_expr(value, plugin_names, registry)
        }
        Stmt::Return { value: Some(e), .. } => check_expr(e, plugin_names, registry),
        Stmt::Return { value: None, .. } => Ok(()),
        Stmt::While { cond, body, .. } => {
            check_expr(cond, plugin_names, registry)?;
            check_stmts(&body.stmts, plugin_names, registry)
        }
        Stmt::Expr(e) => check_expr(e, plugin_names, registry),
        // `audited` only suppresses guard *emission* (`Codegen::audited`,
        // checked inside `guard_in_range`/the division trap) -- every
        // statement inside still has to be otherwise codegen-supported,
        // so this walks in exactly like `While`'s body.
        Stmt::Audited { body, .. } => check_stmts(body, plugin_names, registry),
    }
}


pub(super) fn check_expr(e: &Expr, plugin_names: &std::collections::HashSet<String>, registry: &TypeRegistry) -> Result<(), CodegenError> {
    match e {
        Expr::Int(_, _) | Expr::Bool(_, _) | Expr::Ident(_, _) | Expr::Float(_, _) | Expr::Str(_, _) => Ok(()),
        Expr::Unary(_, inner, _) => check_expr(inner, plugin_names, registry),
        Expr::Binary(_, l, r, _) => {
            check_expr(l, plugin_names, registry)?;
            check_expr(r, plugin_names, registry)
        }
        Expr::Call(name, args, _) => {
            // A struct/variant constructor call is syntactically just
            // `Expr::Call` (no dedicated AST node) — nothing special to
            // check here beyond its arguments, same as any other call;
            // `Codegen::construct`/`expr_ptr`'s real handling is where
            // the actual construction codegen lives.
            if name == "print" {
                // No syntactic rejection needed here any more: `print`
                // now handles every scalar shape (`Codegen::call`'s
                // `Ty::Bool`/`Ty::Unit` arms) — the one real remaining
                // rejection, a `Vector`/`Matrix` argument, needs real
                // type info this purely-syntactic pre-pass doesn't have,
                // so it's caught later in `Codegen::call` itself (the
                // existing `arg_ty.is_aggregate()` check there), same as
                // before. Each argument still gets walked for its own
                // recursive validity by the shared loop below.
            } else if name == "__workflow_link_advance" {
                // `WORKFLOW_BUILTINS`'s own doc comment: magic-link
                // consumption needs real durable storage (a
                // `workflow_log.rs`-shaped table keyed by instance id,
                // surviving process restarts) that this Layer 1 compiled
                // backend doesn't have — `WorkflowInstance`
                // (`runtime-kernels/src/lib.rs`) is a plain in-process
                // `HashMap`, gone the moment the binary exits. Rejected
                // here (rather than at runtime, like
                // `__workflow_pending_for_me`/`__workflow_submitted_by_me`/
                // `__workflow_history`) because a `*_via_link` fn only
                // exists for a workflow that actually declares a `link`
                // transition, so this narrows correctly to exactly those
                // programs — same "specific reason, not a generic
                // fallthrough" treatment `network_retry`/`network_timeout`
                // already get in `emit_transact`.
                return unsupported(format!(
                    "codegen doesn't support `{name}` yet — magic-link advance needs real \
                     durable, restart-surviving storage that this compiled backend's Layer 1 \
                     workflow runtime doesn't have (an in-process table only, see \
                     `runtime-kernels`'s `WorkflowInstance`); use `advance_<workflow>`/state \
                     transitions instead"
                ));
            } else if is_builtin(name)
                && !PHASE4_BUILTINS.contains(&name.as_str())
                && !PHASE5_BUILTINS.contains(&name.as_str())
                && !STR_CRYPTO_BUILTINS.contains(&name.as_str())
                && !STR_BUILTINS.contains(&name.as_str())
                && !RAND_BUILTINS.contains(&name.as_str())
                && !DEC128_BUILTINS.contains(&name.as_str())
                && !IDENTITY_BUILTINS.contains(&name.as_str())
                && !DB_BUILTINS.contains(&name.as_str())
                && !ENV_BUILTINS.contains(&name.as_str())
                && !JSON_BUILTINS.contains(&name.as_str())
                && !SLEEP_BUILTINS.contains(&name.as_str())
                && !MQ_BUILTINS.contains(&name.as_str())
                && !HTTP_BUILTINS.contains(&name.as_str())
                && !CALL_BUILTINS.contains(&name.as_str())
                && !WORKFLOW_BUILTINS.contains(&name.as_str())
                && !NOTIFY_BUILTINS.contains(&name.as_str())
            {
                // Every builtin not in `PHASE4_BUILTINS` (unrolled IR),
                // `PHASE5_BUILTINS`/`STR_CRYPTO_BUILTINS`/`STR_BUILTINS`
                // (linked runtime call), `RAND_BUILTINS` (linked call
                // into a process-wide RNG stream), or `DB_BUILTINS`/
                // `JSON_BUILTINS` (linked SQLite/JSON kernel calls) is
                // rejected here with a specific reason rather than
                // falling through to `check_expr`'s per-argument walk,
                // which would report a less specific one.
                return unsupported(format!(
                    "codegen doesn't support `{name}` yet — this builtin is interpreter-only \
                     for now (numeric codegen lands in a later phase)"
                ));
            } else if plugin_names.contains(name) {
                // rfcs/0003-plugin-abi-v2.md: a plugin builtin's `call`
                // is an opaque `Arc<dyn Fn>` with no stable calling
                // convention into generated LLVM IR — interpreter-only,
                // permanently, not "not yet" the way a real numeric
                // builtin above might be. Named and rejected explicitly
                // here rather than falling through to `Codegen::call`'s
                // user-fn lookup, which has no entry for a plugin name
                // (plugins are never part of `program.fns`) and would
                // hit an untested "unknown function" path instead of a
                // clean, actionable error.
                return unsupported(format!(
                    "codegen doesn't support plugin builtin `{name}` yet — plugin calls are \
                     interpreter-only; `nirdosha build`/`emit-llvm` can't link an opaque Rust \
                     closure into generated native code without a stable C-ABI plugin-calling \
                     convention, which doesn't exist yet"
                ));
            }
            for a in args {
                check_expr(a, plugin_names, registry)?;
            }
            Ok(())
        }
        Expr::If { cond, then_block, else_block, .. } => {
            check_expr(cond, plugin_names, registry)?;
            check_stmts(&then_block.stmts, plugin_names, registry)?;
            match else_block.as_deref() {
                Some(ElseBranch::Block(b)) => check_stmts(&b.stmts, plugin_names, registry),
                Some(ElseBranch::If(e2)) => check_expr(e2, plugin_names, registry),
                None => Ok(()),
            }
        }
        Expr::Assign(_, rhs, _) => check_expr(rhs, plugin_names, registry),
        // Row 11: both now recurse structurally, same "walk, don't
        // reject" treatment every other now-supported construct gets —
        // real type-directed validation (the affine check) happens where
        // `Codegen`'s own methods can see a base/scrutinee's actual
        // resolved type (`Codegen::field_access`/`Codegen::match_expr`),
        // not in this purely-syntactic pre-pass.
        Expr::FieldAccess(base, _, _) => check_expr(base, plugin_names, registry),
        Expr::Match { scrutinee, arms, .. } => {
            check_expr(scrutinee, plugin_names, registry)?;
            for arm in arms {
                check_expr(&arm.body, plugin_names, registry)?;
            }
            Ok(())
        }
        // `box`/`*`/`&` land as of this phase — see `llvm_ty`'s
        // `Ty::Box`/`Ty::Ref` arm and `Codegen::expr`'s real `Expr::Box`/
        // `Expr::Deref`/`Expr::Ref` arms for the actual codegen. This
        // structural pre-pass has no type info (that's `local_ty_of`'s
        // job, at real IR-gen time), so it just recurses into whatever's
        // inside — same "walk, don't reject" treatment every other
        // already-supported unary-ish construct gets.
        Expr::Box(inner, _) | Expr::Froze(inner, _) | Expr::Deref(inner, _) | Expr::Ref(inner, _) => {
            check_expr(inner, plugin_names, registry)
        }
        // `spawn`/`join` land as of this phase (`runtime-kernels`'
        // `nir_thread_spawn`/`nir_thread_join`) — this structural pre-pass
        // has no type info (that's `Codegen::expr`'s job, at real IR-gen
        // time, where a spawned function's own signature is checked for
        // word-sized args/return), so it just recurses, same "walk,
        // don't reject" treatment every other now-supported construct
        // gets.
        Expr::Spawn(_, args, _) => {
            for a in args {
                check_expr(a, plugin_names, registry)?;
            }
            Ok(())
        }
        Expr::Join(inner, _) => check_expr(inner, plugin_names, registry),
        // Compiled for real, 2026-09 (`Codegen::emit_acquire`) — this
        // structural pre-pass has no type info (same reasoning
        // `Expr::Spawn`'s own arm above already gives), so it just
        // recurses into `proof`.
        Expr::Acquire(_, proof, _) => check_expr(proof, plugin_names, registry),
        // `chan` construction itself needs no type info at all (every
        // `Ty::Channel` value is the same `i64` handle regardless of its
        // payload type — `llvm_ty`'s own `Ty::Channel` arm) — real per-
        // payload-type validation happens where `send`/`recv` can see the
        // channel's actual type via `local_ty_of`, same "type-oblivious
        // pre-pass, real check happens at IR-gen time" precedent
        // `print`'s aggregate rejection already established (module doc).
        Expr::Chan(_) => Ok(()),
        Expr::Send(chan, value, _) => {
            check_expr(chan, plugin_names, registry)?;
            check_expr(value, plugin_names, registry)
        }
        Expr::Recv(chan, _) => check_expr(chan, plugin_names, registry),
        // Same reasoning as `send`/`recv` above: `stop` is one AST node
        // (`Expr::StopSandbox`) shared by `sandbox`/`tcp`/`tcp_listener`,
        // dispatched on the operand's type — `sandbox` itself stays
        // rejected (unsupported below), `stop` recurses structurally so
        // `Codegen::expr` can accept it for a `Ty::Tcp`/`Ty::TcpListener`
        // operand and reject it for `Ty::Sandbox` with real type info.
        Expr::SpawnSandbox(_, _, _) => {
            unsupported("codegen doesn't support `sandbox` yet — interpreter-only for now")
        }
        Expr::StopSandbox(inner, _) => check_expr(inner, plugin_names, registry),
        // `open(path, mode)` compiles now (`nir_file_open`,
        // `runtime-kernels/src/lib.rs`) — recurse into both operands the same as
        // `Expr::Connect` below.
        Expr::Open(path, mode, _) => {
            check_expr(path, plugin_names, registry)?;
            check_expr(mode, plugin_names, registry)
        }
        Expr::Connect(host, port, _) => {
            check_expr(host, plugin_names, registry)?;
            check_expr(port, plugin_names, registry)
        }
        Expr::Listen(port, _) => check_expr(port, plugin_names, registry),
        Expr::Accept(listener, _) => check_expr(listener, plugin_names, registry),
        // Vector/Matrix indexing lands as of this phase — the base and
        // every index expression are walked the same way `ArrayLit`'s
        // elements are, so a still-unsupported construct nested inside
        // either (e.g. a `.*` index expression) keeps its own specific
        // rejection reason instead of being silently accepted because
        // `Expr::Index` itself is now fine.
        Expr::Index(base, indices, _) => {
            check_expr(base, plugin_names, registry)?;
            for idx in indices {
                check_expr(idx, plugin_names, registry)?;
            }
            Ok(())
        }
        // Vector/Matrix literals land as of this phase — walk each
        // element the same way `Expr::Call`'s arguments are walked, so a
        // still-unsupported construct nested inside a literal (e.g. a
        // `.*` element expression) is still caught with its own specific
        // reason rather than silently accepted because the outer
        // `ArrayLit` shape itself is now fine.
        Expr::ArrayLit(elements, _) => {
            for e in elements {
                check_expr(e, plugin_names, registry)?;
            }
            Ok(())
        }
        // `transact { ... }` — compiled 2026-09 (`Codegen::emit_transact`'s
        // own doc comment has the full scope: Layer 1 control flow only,
        // no retry/timeout/durability/replay). `network_retry`/
        // `network_timeout` are the one part of the construct genuinely
        // rejected here, not just deferred — a compiled trap is an
        // unrecoverable `abort()`, and `network`'s own declared return
        // type is restricted to a bare scalar (`Ty::is_transact_scalar`,
        // never `Result(_, _)`), so there is no non-trapping failure
        // signal for a retry loop to react to at all. Every slot's own
        // arguments are still walked structurally (same as `Expr::Call`'s
        // own args), so a still-unsupported construct nested inside one
        // is caught with its own specific reason.
        Expr::Transact { precheck, network, network_retry, network_timeout, verify, commit, compensate, log, .. } => {
            if network_retry.is_some() || network_timeout.is_some() {
                return unsupported(
                    "codegen doesn't support `network`'s `retry`/`timeout` modifiers yet — the \
                     now-deleted interpreter's retry-on-trap semantics relied on catching an \
                     internal RuntimeError before it ever unwound; a compiled trap calls abort() \
                     directly (this language's trap model everywhere else), which is unrecoverable, \
                     so there is no non-trapping way to detect a failed `network` call to retry \
                     (`network`'s own declared return type is restricted to a bare scalar by \
                     Ty::is_transact_scalar, never Result(_, _), so there's no Err to react to \
                     either) — omit `retry`/`timeout` on `network` for now"
                        .to_string(),
                );
            }
            if let Some(p) = precheck {
                for a in &p.args {
                    check_expr(a, plugin_names, registry)?;
                }
            }
            for a in &network.args {
                check_expr(a, plugin_names, registry)?;
            }
            for a in &verify.args {
                check_expr(a, plugin_names, registry)?;
            }
            for a in &commit.args {
                check_expr(a, plugin_names, registry)?;
            }
            if let Some(c) = compensate {
                for a in &c.args {
                    check_expr(a, plugin_names, registry)?;
                }
            }
            if let Some(l) = log {
                for a in &l.args {
                    check_expr(a, plugin_names, registry)?;
                }
            }
            Ok(())
        }
    }
}


