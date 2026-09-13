//! LLVM codegen — docs/goal.md row 5 ("native, hardware-speed codegen"),
//! started here for the first time. Emits **textual LLVM IR** and shells
//! out to the system `clang` to assemble/link a real native binary,
//! rather than binding to the LLVM C API from Rust (`inkwell`/`llvm-sys`)
//! — this environment has LLVM 22, recent enough that a Rust binding
//! crate's supported-version list might not cover it yet, and textual IR
//! sidesteps that entirely: it's a stable, documented format, and `clang`
//! on this system is *the same* LLVM 22, so there's no version-skew risk
//! between what's emitted and what assembles it. Several real compilers
//! (plenty of small production and hobby ones) use exactly this strategy.
//!
//! **Scoped to what's honestly supported, not silently narrowed.**
//! `check_supported`'s own `unsupported(...)` call sites are the
//! authoritative list of what's rejected — treat that function itself as
//! the source of truth, not this comment, which has drifted out of sync
//! with what actually compiles more than once already (see
//! `docs/LANGUAGE.md` §10's "Updated 22 Aug 2026" note for a concrete
//! example: this file's own earlier doc comment claimed `box`/`&`/`*`
//! had no codegen, when `nir_alloc`/`nir_free` (driven by
//! `ownership.rs`'s `FreeMap`) already compiled real heap alloc/free by
//! the time that claim was read again). As of this writing: every
//! scalar integer type, signed (`i8`/`i16`/`i32`/`i64`) and unsigned
//! (`u8`/`u16`/`u32`/`u64`/`usize` — `widen_to_i64`'s doc comment on why
//! unsigned needed only one small, contained change, not a parallel
//! signed/unsigned codegen path), plus `bool`, `unit`, `f64`, `str`,
//! `box`/`&`/`*`, `tcp`/`tcp_listener`, `sha256_hex`/
//! `constant_time_str_eq` (`STR_CRYPTO_BUILTINS`), and `Vector`/`Matrix`
//! (plus most of the dense-linalg/geometry/Kalman builtin surface) all
//! compile — including `print` on every one of those scalar shapes
//! (`Codegen::call`'s `Ty::Bool`/`Ty::Unit` arms handle the two that
//! used to be rejected) — and, as of Phase 4a, `struct`/`enum`/`match`
//! over **non-affine** payloads (construction, `expr.field` access, and
//! `match`'s enum-variant + literal-pattern arms). Rejected: an
//! **affine-containing** `struct`/`enum`/`match` (a `box`/`&`/`tcp`/`file`/
//! `db`/`mq` field/payload, transitively — Phase 4b; needs
//! `ownership.rs`'s `FreeMap` generalized beyond `Ty::Box`-only
//! `still_owned_boxes` plus a new `at_match_arm_end` entry), `thread`/
//! `chan`/`sandbox`/`file`/`json`/`db`/`mq`/`transact`/every Row 12
//! identity builtin, `fn(..)->..`/`acquire`/`requires(...)`, and `print`
//! on a whole `Vector`/`Matrix` argument.
//!
//! **Tier 1 vs Tier 2 finally means something.** `refine.rs` and
//! `smt.rs` both proved things and both said, explicitly, "not wired to
//! elide the runtime check — there's no backend to spend the payoff on
//! yet." There is now. A `let`/assignment whose span is in the passed-in
//! `SmtReport::proven_in_range` gets no runtime bounds check emitted at
//! all (Tier 1, silent, exactly as docs/goal.md §4 describes); one that
//! isn't gets an explicit compare-and-trap sequence in the compiled
//! binary (Tier 2, a real cost, visible in the generated IR). Same
//! distinction for division and `proven_nonzero_divisor`. This is the
//! first place in the whole codebase where a static proof actually
//! changes what runs, not just what's reported.
//!
//! **Codegen strategy: alloca everywhere, correctness over cleverness.**
//! Every parameter and every `let` gets its own stack slot
//! (`alloca`/`store`/`load`), the same strategy `clang -O0` itself uses
//! and every "toy compiler to LLVM" tutorial teaches — it's simple to
//! get right, and LLVM's own optimizer (not run here; nothing asks for
//! `-O2`) would promote these to registers anyway if it were. Allocas
//! are emitted at the point of each `let`, not hoisted to the entry
//! block: Nirdosha's scoping rules mean a name is only ever referenced
//! somewhere its `let` already dominates (you can't read a variable
//! before its declaration or from a sibling branch), so this is valid
//! LLVM IR without the hoisting pass a stricter backend might do. `&&`/
//! `||` are lowered to real conditional branches, not eager bitwise
//! `and`/`or` — short-circuit evaluation is a tested behavior
//! (`tests/basic.rs`'s short-circuit tests), and this backend has to
//! preserve it, not just the interpreter.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use crate::ast::*;
use crate::ownership::{self, FreeMap};
use crate::smt::SmtReport;
use crate::token::Span;
use crate::typeck::{is_optional_verified_identity, is_verified_identity};

// Per-concern split of what used to be one 13.7k-line file -- see
// rfcs/0018-composable-verified-codegen-rules.md's motivation and the
// accompanying refactor plan. Pure file reorganization: every item below
// still lives on the single `Codegen` type via a separate `impl
// Codegen<'_>` block per file, and every moved item is `pub(super)` so
// this module and its siblings can still reach it -- no behavior change.
mod call_dispatch;
mod checks;
mod control_flow;
mod domain;
mod guards;
#[cfg(test)]
mod internal_tests;
mod layout;
mod serve_codec;
mod support;
mod type_infer;

pub use checks::{check_supported, check_supported_with_plugins};
use layout::*;

#[derive(Debug, Clone, PartialEq)]
pub struct CodegenError {
    pub message: String,
}

impl std::fmt::Display for CodegenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

fn unsupported<T>(msg: impl Into<String>) -> Result<T, CodegenError> {
    Err(CodegenError { message: msg.into() })
}

/// `db_execute`/`db_query`'s trailing `?`-placeholder bind values —
/// `emit_db_binds` builds a `[N x NIR_BIND_VALUE_LLTY]` array of these,
/// GEP'd into by field index like any other named-struct-shaped value in
/// this file (`agg_byte_size_operand`'s own "trust the target's natural
/// layout, don't hand-replicate it" stance applies here too: a plain,
/// non-packed anonymous struct type follows the same C-layout rules
/// Rust's `#[repr(C)]` does for `runtime-kernels/src/lib.rs`'s
/// `NirBindValue` — same field order, same primitive types, both sides
/// agree without either one computing offsets by hand). Field order:
/// `tag`(i32) / `i`(i64) / `f`(double) / `s_ptr`(ptr) / `s_len`(i64).
const NIR_BIND_VALUE_LLTY: &str = "{ i32, i64, double, ptr, i64 }";

/// `transact { commit/compensate }`'s bounded retry-with-backoff
/// (`Codegen::emit_call_with_retry`) — a fixed compile-time attempt
/// count and a doubling backoff (`nir_sleep_ms`) between attempts, e.g.
/// 100ms/200ms/400ms for the two retries after the first attempt.
/// Deliberately small and fixed rather than configurable: this phase's
/// durability guarantee doesn't depend on how many live retries happen
/// before a `commit`/`compensate` is left `*_pending` for replay to
/// finish later — retrying live is purely a latency optimization for
/// the common transient-failure case, not the actual safety mechanism.
const TRANSACT_RETRY_MAX_ATTEMPTS: i64 = 3;
const TRANSACT_RETRY_BASE_BACKOFF_MS: i64 = 100;

/// One binding's declared type plus the LLVM register holding a
/// *pointer* to its stack slot (an `alloca` result) — reads go through a
/// `load`, writes through a `store`, exactly like `clang -O0`'s output.
#[derive(Clone)]
struct Scopes(Vec<HashMap<String, (Ty, String)>>);

impl Scopes {
    fn new() -> Self {
        Scopes(vec![HashMap::new()])
    }
    fn push(&mut self) {
        self.0.push(HashMap::new());
    }
    fn pop(&mut self) {
        self.0.pop();
    }
    fn define(&mut self, name: &str, ty: Ty, ptr_reg: String) {
        self.0.last_mut().unwrap().insert(name.to_string(), (ty, ptr_reg));
    }
    fn get(&self, name: &str) -> Option<(Ty, String)> {
        self.0.iter().rev().find_map(|s| s.get(name)).cloned()
    }
}

/// A function's declared signature — codegen's own copy, built once up
/// front (mirroring `typeck::FnSig`, which is private to that module and
/// not reusable here). `call()` needs this for two things LLVM requires
/// to get exactly right at every call site: the call instruction's
/// return type must match the callee's `define` exactly, and every
/// argument's type annotation must match the corresponding declared
/// parameter type exactly — guessing either from the argument
/// *expression's* own shape (an earlier draft's approach) is wrong
/// whenever a literal argument's "natural" type doesn't match a narrower
/// declared parameter (see the "found by testing" note in the module
/// doc / docs/PHASE0.md's write-up of this milestone).
struct FnSig {
    params: Vec<Ty>,
    ret: Ty,
    /// `FnDecl::requires`'s own copy — `None` for a native plugin (never
    /// gated) and for every ordinary `.nir` fn. `Some(req)` is what
    /// `Codegen::emit_acquire` checks a `proof` against, and what makes
    /// `name` callable only as `acquire name(proof)` rather than
    /// directly (`typeck.rs`'s `PrivilegedFnNotAcquired`, already
    /// enforced before codegen runs — this field exists purely so
    /// `emit_acquire` can look the requirement back up by name, since
    /// `Codegen` doesn't otherwise keep a reference to the whole
    /// `Program`).
    requires: Option<Requirement>,
}

struct Codegen<'a> {
    out: String,
    /// Every `alloca` this function emits, collected here instead of
    /// written inline to `self.out` at the point of use, then spliced
    /// into `self.out` right after `entry:` once `function()` finishes
    /// generating the body. Reset per-function. Necessary because an
    /// aggregate (`Vector`/`Matrix`) alloca's address is always taken
    /// (passed to `memcpy`/GEP/a `call`), so LLVM's `mem2reg` can never
    /// promote it away — an alloca emitted inline inside a loop body
    /// would allocate fresh, unreclaimed stack space on every iteration
    /// instead of the one real stack slot a loop actually needs. Scalar
    /// allocas are hoisted the same way for uniformity and safety (it
    /// costs nothing — `mem2reg` eliminates them regardless of which
    /// block they start in), not just the aggregate ones that need it.
    entry_allocas: String,
    /// Every `str` literal's backing global constant, collected here for
    /// the same structural reason `entry_allocas` exists: a global
    /// definition (`@.str.N = ...`) is only valid LLVM IR at module
    /// scope, never written mid-function the way `self.out` is being
    /// built — a literal can appear anywhere inside any function body.
    /// Module-scoped, not per-function (unlike `entry_allocas`): never
    /// reset, appended to `self.out` once at the very end of
    /// `emit_llvm_ir`. LLVM doesn't care about textual definition order
    /// for a global referenced by name, so appending at the end rather
    /// than the true point of first use is fine.
    string_globals: String,
    /// Every `spawn`'s own generated trampoline function (`spawn_thread`'s
    /// doc comment) — same structural reason `string_globals` exists: a
    /// top-level `define` is only valid at module scope, never written
    /// mid-function the way `self.out` is being built, but a `spawn` can
    /// appear anywhere inside any function body. Built by temporarily
    /// swapping it into `self.out` (so every ordinary instruction-emitting
    /// helper — `widen_to_i64`, `narrow_from_i64`, `llvm_ty`, ... — just
    /// works unmodified), then swapping back; appended to `self.out` once
    /// at the very end, alongside `string_globals`.
    trampolines: String,
    /// One entry per `transact` call site actually compiled — `(site_id,
    /// trampoline_function_name)`, consumed once by `emit_c_main` to
    /// emit `nir_transact_register_replay_site` calls in generated
    /// `main`'s own prologue. Empty for a program that never uses
    /// `transact` at all, so `nir_transact_log_init`/`nir_transact_replay_all`
    /// are only emitted (and only ever open/touch a durability log file)
    /// when the program actually needs them — the same zero-cost-when-
    /// unused posture every other optional kernel subsystem here already
    /// has.
    transact_sites: Vec<(i64, String)>,
    tmp: usize,
    label: usize,
    smt_report: &'a SmtReport,
    /// Where to insert `nir_free` for still-owned `box`-typed bindings —
    /// computed once, up front, by `ownership.rs`'s own move-tracking
    /// pass (see `FreeMap`'s doc) rather than a second, codegen-side
    /// liveness analysis. Consulted at every scope-closing point
    /// (`Stmt::Return`, a loop body's end-of-iteration, an `if`/`audited`
    /// block's own close, and a function's implicit fall-off-the-end).
    free_map: FreeMap,
    sigs: HashMap<String, FnSig>,
    /// The function currently being generated code for — `Stmt::Return`
    /// needs its declared return type to guard/narrow against, and
    /// there's no other way to reach it from inside `stmt()` without
    /// threading it through every call.
    current_fn_ret: Ty,
    /// This function's own name — `free_map.at_fn_end` is keyed by it,
    /// consulted only at the implicit fall-off-the-end return point.
    current_fn_name: String,
    /// `Some("%sret.ret")` while generating a function whose return type
    /// is `Ty::is_aggregate()` — the implicit out-pointer parameter
    /// `Stmt::Return` memcpys its result into, instead of emitting a
    /// `ret <ty> <val>`. `None` for every scalar-returning function
    /// (including `unit`), which still just `ret`s normally.
    current_fn_sret: Option<String>,
    /// `Some(spec)` while generating a function that declared `nfr(...)`
    /// — `Stmt::Return`'s own arms (and `function()`'s implicit fall-
    /// off-the-end path) consult `error_rate_max.is_some()` to decide
    /// whether a `Result`-tagged return value's `Err`-ness needs
    /// computing at all. `None` (the common case) means every return
    /// site's `nir_nfr_call_end` (if any — see the next two fields) just
    /// passes a literal `0`.
    current_fn_nfr: Option<NfrSpec>,
    /// The two per-invocation SSA registers `nir_nfr_call_begin`
    /// produced at function entry — `Some((id, start))` exactly when
    /// `current_fn_nfr` is `Some`, threaded separately (not recomputed
    /// from `current_fn_nfr`) since they're register *names*, not
    /// values, valid only within this one function's own IR.
    current_fn_nfr_regs: Option<(String, String)>,
    /// The name of this function's own first `RoleView`-typed parameter,
    /// if it has one — `emit_field_masking`'s only source of "does the
    /// caller have proof of the role a returned field's `requires(role:
    /// ...)` demands." `None` means every role-masked field this
    /// function returns is unconditionally masked (fail-closed: no
    /// proof present is treated the same as proof of the wrong role,
    /// never as "trust it anyway").
    current_fn_role_view_param: Option<String>,
    /// Same as `current_fn_role_view_param`, for `ClaimView` and
    /// `requires(claim: ...)`.
    current_fn_claim_view_param: Option<String>,
    /// Once a block's been given a terminator (`br`/`ret`), any further
    /// statements in the same source block are unreachable — this stops
    /// codegen from emitting a second terminator into an already-closed
    /// block, which would be invalid IR.
    terminated: bool,
    /// Inside a `Stmt::Audited` body — `guard_in_range` and the
    /// division-by-zero trap both check this first and skip emitting
    /// their guard entirely when it's set (docs/goal.md §4's Tier-3 escape
    /// hatch). A plain `bool`, not a depth counter: nested `audited`
    /// blocks don't need their own count, only "is at least one
    /// enclosing scope audited" — restored to its prior value (not
    /// unconditionally cleared) on exit so a nested `audited` inside a
    /// non-audited function correctly re-enables guards afterward, and
    /// one written (redundantly) inside an already-audited block doesn't
    /// prematurely turn guards back on when *it* exits.
    audited: bool,
    /// The program's struct/enum declaration table — built once at the
    /// start of `emit_llvm_ir` and consulted by every Row 11 codegen path
    /// (`llvm_ty`'s struct/enum arms, `construct`, `match_expr`, field
    /// access) to resolve a `Ty::Named`'s fields/payloads/variants and to
    /// re-check affinity (the free `llvm_ty`/`check_supported` pre-pass
    /// builds its own throwaway copy, since it runs before this `Codegen`
    /// exists). Borrows from `program` for the same lifetime `smt_report`
    /// does — see `emit_llvm_ir`'s `'a` unification.
    registry: TypeRegistry<'a>,
    /// Every distinct concrete `struct`/`enum` instantiation already
    /// emitted as a real LLVM named-type declaration (`%Point = type
    /// {...}`), keyed by mangled name — so `declare_named_type` declares
    /// each one exactly once even when many call sites construct the
    /// same instantiation. Built up over the whole `emit_llvm_ir` run,
    /// then the collected declarations are prepended to the module top
    /// (before any `define` that references one) at the very end.
    declared_named_types: HashSet<String>,
    /// The `%Name = type { ... }` text itself, accumulated in dependency
    /// order (`declare_named_type` recurses into a struct's named-typed
    /// fields first so a `%Outer = type { %Point }` always follows its
    /// `%Point = type { ... }`), then spliced into `self.out` at position
    /// 0 once `emit_llvm_ir` finishes. Module-scoped, never reset — same
    /// structural shape as `string_globals` (appended once at the end),
    /// just prepended instead because a named type must textually precede
    /// any `define` that mentions it.
    named_type_decls: String,
    /// `program.workflows` — `emit_workflow_start`/`emit_workflow_advance`'s
    /// own doc comments have the full reasoning: `workflow_lower.rs`
    /// desugars every `workflow` block into ordinary `FnDecl`s whose body
    /// is a single call to `__workflow_start`/`__workflow_advance`/etc.
    /// with the workflow's own name baked in as a compile-time `str`
    /// literal — never a runtime value — so codegen can resolve which
    /// `WorkflowDecl` a given call site means at compile time and emit
    /// bespoke, inlined control flow per workflow, the same way
    /// `emit_check_role`/`emit_transact` hand-build IR for their own
    /// specific builtins rather than a generic runtime dispatch table.
    workflows: &'a [WorkflowDecl],
}

pub fn emit_llvm_ir<'a>(program: &'a Program, smt_report: &'a SmtReport) -> Result<String, CodegenError> {
    emit_llvm_ir_impl(program, smt_report, &[], &HashSet::new(), None)
}

/// Reviving compiled `nirdosha serve` (`rfcs/0010-landing-and-serve-
/// exposure.md`) — `port` to bind, and `ui_html` the caller already
/// generated at compile time (`ui_gen::generate`, `main.rs::cmd_build`)
/// to bake into the binary, since a compiled process has no `Program`
/// AST left at runtime to generate it from. Native plugins aren't
/// supported together with `--serve` yet (`emit_c_main_serve`'s own
/// doc comment has the full disclosure), so this always passes an
/// empty plugin roster to `emit_llvm_ir_impl`, unlike
/// `emit_llvm_ir_with_native_plugins`.
pub struct ServeCodegenOptions {
    pub port: u16,
    pub ui_html: Vec<u8>,
    /// RFC 0016's FAPI wiring: `true` only when a pack governing the
    /// project's graph (`hi_plugin.rs`'s
    /// `wiring_requires_sender_constrained_tokens`) declares the
    /// `sender_constrained_tokens` wiring requirement -- set by
    /// `main.rs::cmd_build`, never inferred here, never on by default.
    /// Threaded straight through to `compiled_serve::nir_compiled_serve_run`'s
    /// own `require_dpop` parameter; see `ServeConfig::require_sender_constrained_tokens`'s
    /// own doc comment for what it actually turns on at runtime.
    pub require_sender_constrained_tokens: bool,
}

pub fn emit_llvm_ir_for_serve<'a>(program: &'a Program, smt_report: &'a SmtReport, serve: &ServeCodegenOptions) -> Result<String, CodegenError> {
    emit_llvm_ir_impl(program, smt_report, &[], &HashSet::new(), Some(serve))
}

/// rfcs/0005-plugin-boundary-safety-and-performance.md §3: the compiled-
/// path counterpart to `run_with_plugins` (`lib.rs`) — a project's own
/// entrypoint, not the bare `nirdosha build`/`emit-llvm` CLI (which,
/// like the interpreted path before Track G's own auto-discovery lands,
/// has no way to *find* a plugin crate on its own), calls this instead
/// of plain `emit_llvm_ir` once it has both a compiled-native-capable
/// plugin roster (`native_plugins`) and, for honesty, the names of any
/// *other* plugin the program's typecheck pass also saw but that has no
/// native form (`reject_plugin_names` — still cleanly rejected by
/// `check_supported_with_plugins`, exactly as before this existed,
/// rather than silently accepted or hitting an untested "unknown
/// function" path).
pub fn emit_llvm_ir_with_native_plugins<'a>(
    program: &'a Program,
    smt_report: &'a SmtReport,
    native_plugins: &[crate::plugin::NativePluginBuiltin],
    reject_plugin_names: &HashSet<String>,
) -> Result<String, CodegenError> {
    for np in native_plugins {
        if let Err(msg) = np.validate() {
            return unsupported(msg);
        }
    }
    if let Err(msg) = crate::plugin::validate_plugin_roster(native_plugins) {
        return unsupported(msg);
    }
    emit_llvm_ir_impl(program, smt_report, native_plugins, reject_plugin_names, None)
}

fn emit_llvm_ir_impl<'a>(
    program: &'a Program,
    smt_report: &'a SmtReport,
    native_plugins: &[crate::plugin::NativePluginBuiltin],
    reject_plugin_names: &HashSet<String>,
    serve: Option<&ServeCodegenOptions>,
) -> Result<String, CodegenError> {
    check_supported_with_plugins(program, reject_plugin_names)?;
    let registry = TypeRegistry::build(program);
    let mut sigs: HashMap<String, FnSig> = program
        .fns
        .iter()
        .map(|f| {
            (f.name.clone(), FnSig { params: f.params.iter().map(|p| p.ty.clone()).collect(), ret: f.ret.clone(), requires: f.requires.clone() })
        })
        .collect();
    // A native plugin's signature slots into the exact same table a
    // user `fn`'s does — `Codegen::call`'s existing generic fallback
    // (the `self.sigs.get(name)` path every ordinary function call
    // already goes through) needs zero changes to reach it; only the
    // `declare` line below (in place of a real `define`) and the linked
    // staticlib (`build_with_native_plugins`) are new.
    for np in native_plugins {
        sigs.insert(np.name.clone(), FnSig { params: np.params.clone(), ret: np.ret.clone(), requires: None });
    }
    // Trusts the program already passed `ownership::check_ownership` (the
    // caller's job, same as `typecheck_and_own`'s existing precedent) —
    // this recomputes the same move-tracking traversal for its own
    // FreeMap side data, not to re-validate.
    let free_map = ownership::compute_free_map(program);
    let mut cg =
        Codegen {
            out: String::new(),
            entry_allocas: String::new(),
            string_globals: String::new(),
            trampolines: String::new(),
            transact_sites: Vec::new(),
            tmp: 0,
            label: 0,
            smt_report,
            free_map,
            sigs,
            current_fn_ret: Ty::Unit,
            current_fn_name: String::new(),
            current_fn_sret: None,
            current_fn_nfr: None,
            current_fn_nfr_regs: None,
            current_fn_role_view_param: None,
            current_fn_claim_view_param: None,
            terminated: false,
            audited: false,
            registry,
            declared_named_types: HashSet::new(),
            named_type_decls: String::new(),
            workflows: &program.workflows,
        };

    writeln!(cg.out, "declare i32 @printf(ptr, ...)").unwrap();
    writeln!(cg.out, "declare void @abort() noreturn").unwrap();
    // Every aggregate (`Vector`/`Matrix`) copy — function-prologue
    // copy-in, a `let`/assignment's value copy, a matrix literal's
    // per-row copy — goes through this one intrinsic rather than a
    // hand-unrolled load/store loop; `clang -O2` lowers a small
    // constant-size `memcpy` to inline loads/stores itself, so this
    // costs nothing extra at the optimized output (module doc's own
    // "correctness over cleverness" call, extended to aggregates).
    writeln!(
        cg.out,
        "declare void @llvm.memcpy.p0.p0.i64(ptr noalias writeonly, ptr noalias readonly, i64, i1 immarg)"
    )
    .unwrap();
    // Phase 4's geometry/norm builtins (`lla_to_ecef`, `bearing`, `norm`,
    // ...) need real transcendental functions this backend has no plain
    // instruction for. `sqrt`/`sin`/`cos`/`fabs`/`maxnum` are standard
    // LLVM intrinsics (recognized by name, no special attributes needed
    // on the `declare` itself); `atan2` has no LLVM intrinsic form, so
    // it's declared as the plain libm C function instead — `build()`
    // links `-lm` for it (see that function's own note).
    writeln!(cg.out, "declare double @llvm.sqrt.f64(double)").unwrap();
    writeln!(cg.out, "declare double @llvm.sin.f64(double)").unwrap();
    writeln!(cg.out, "declare double @llvm.cos.f64(double)").unwrap();
    writeln!(cg.out, "declare double @llvm.fabs.f64(double)").unwrap();
    writeln!(cg.out, "declare double @llvm.maxnum.f64(double, double)").unwrap();
    writeln!(cg.out, "declare double @atan2(double, double)").unwrap();
    // Phase 5's data-dependent-control-flow builtins (`det`/`inv`/
    // `solve`/`rank`/`kf_update_state`/`kf_update_cov`) — linked native
    // calls into `runtime-kernels/src/lib.rs`'s staticlib (`build()` writes it
    // out and links it alongside this `.ll`) rather than hand-emitted
    // branchy IR for partial-pivot selection. `i32` return, not `i1`:
    // Rust's `extern "C" fn -> bool` ABI representation isn't guaranteed
    // the way a plain `i32` 0/nonzero convention is.
    writeln!(cg.out, "declare double @nir_det(ptr, i64)").unwrap();
    writeln!(cg.out, "declare i32 @nir_inv(ptr, i64, ptr)").unwrap();
    writeln!(cg.out, "declare i32 @nir_solve(ptr, i64, ptr, ptr)").unwrap();
    writeln!(cg.out, "declare i64 @nir_rank(ptr, i64, i64)").unwrap();
    writeln!(cg.out, "declare i32 @nir_kf_update_state(ptr, ptr, ptr, ptr, ptr, i64, i64, ptr)").unwrap();
    writeln!(cg.out, "declare i32 @nir_kf_update_cov(ptr, ptr, ptr, ptr, ptr, i64, i64, ptr)").unwrap();
    // `str`'s one non-trivial operation (`==`/`!=`) — a length check plus
    // a byte compare, same "reuse proven Rust code via a linked call"
    // choice as the Phase 5 builtins above, not hand-emitted IR.
    writeln!(cg.out, "declare i32 @nir_str_eq(ptr, i64, ptr, i64)").unwrap();
    // `sha256_hex`/`constant_time_str_eq` — `STR_CRYPTO_BUILTINS`'
    // doc comment.
    writeln!(cg.out, "declare void @nir_sha256_hex(ptr, i64, ptr, i64, ptr)").unwrap();
    writeln!(cg.out, "declare i32 @nir_constant_time_str_eq(ptr, i64, ptr, i64)").unwrap();
    // `str_index_of` — `STR_BUILTINS`' doc comment. `str_slice`/`len(str)`
    // need no declare here: both are pure pointer arithmetic in generated
    // IR, no linked kernel call.
    writeln!(cg.out, "declare i64 @nir_str_index_of(ptr, i64, ptr, i64)").unwrap();
    // `rand_seed`/`rand_f64`/`rand_gaussian` — `RAND_BUILTINS`' doc comment.
    writeln!(cg.out, "declare void @nir_rand_seed(i64)").unwrap();
    writeln!(cg.out, "declare double @nir_rand_f64()").unwrap();
    writeln!(cg.out, "declare double @nir_rand_gaussian(double, double)").unwrap();
    // Phase B1's `tcp`/`tcp_listener` kernels — real socket syscalls,
    // wrapped in Rust inside the linked staticlib rather than hand-
    // emitted raw syscall IR, same "reuse proven Rust code via a linked
    // call" choice as every other runtime kernel here. Handles are plain
    // `i64` fds (`llvm_ty`'s note on `Ty::Tcp`/`Ty::TcpListener`).
    writeln!(cg.out, "declare i64 @nir_tcp_connect(ptr, i64, i64)").unwrap();
    writeln!(cg.out, "declare i64 @nir_tcp_listen(i64)").unwrap();
    writeln!(cg.out, "declare i64 @nir_tcp_accept(i64)").unwrap();
    writeln!(cg.out, "declare i64 @nir_tcp_send(i64, ptr, i64)").unwrap();
    writeln!(cg.out, "declare i64 @nir_tcp_recv(i64, ptr, i64)").unwrap();
    writeln!(cg.out, "declare i32 @nir_tcp_stop(i64)").unwrap();
    // `file`'s kernels — `open`/`send`/`recv`/`stop`, same shape as the
    // `tcp` kernels just above (`runtime-kernels/src/lib.rs`'s own "file" section).
    writeln!(cg.out, "declare i64 @nir_file_open(ptr, i64, ptr, i64)").unwrap();
    writeln!(cg.out, "declare i64 @nir_file_write(i64, ptr, i64)").unwrap();
    writeln!(cg.out, "declare i64 @nir_file_read(i64, ptr, i64)").unwrap();
    writeln!(cg.out, "declare i32 @nir_file_stop(i64)").unwrap();
    // `dec128`'s kernels (`runtime-kernels/src/lib.rs`'s "dec128 kernels"
    // section) — `{i64, i64}` by value everywhere, matching `llvm_ty`'s
    // own `Ty::Dec128` arm exactly.
    writeln!(cg.out, "declare {{i64, i64}} @nir_dec128_from_i64(i64, i32)").unwrap();
    writeln!(cg.out, "declare i64 @nir_dec128_to_str({{i64, i64}}, ptr, i64)").unwrap();
    writeln!(cg.out, "declare {{i64, i64}} @nir_dec128_add({{i64, i64}}, {{i64, i64}})").unwrap();
    writeln!(cg.out, "declare {{i64, i64}} @nir_dec128_sub({{i64, i64}}, {{i64, i64}})").unwrap();
    writeln!(cg.out, "declare {{i64, i64}} @nir_dec128_mul({{i64, i64}}, {{i64, i64}})").unwrap();
    writeln!(cg.out, "declare {{i64, i64}} @nir_dec128_div({{i64, i64}}, {{i64, i64}})").unwrap();
    writeln!(cg.out, "declare i32 @nir_dec128_cmp({{i64, i64}}, {{i64, i64}})").unwrap();
    writeln!(cg.out, "declare {{i64, i64}} @nir_dec128_round({{i64, i64}}, i32)").unwrap();
    writeln!(cg.out, "declare i64 @nir_dec128_scale({{i64, i64}})").unwrap();
    writeln!(cg.out, "declare {{i64, i64}} @nir_dec128_from_str(ptr, i64, ptr, ptr)").unwrap();
    // `box`'s heap allocator — see `Expr::Box`'s doc comment for why
    // `nir_free` isn't called anywhere yet (this phase deliberately
    // leaks; a later phase wires the calls once ownership.rs's move data
    // is threaded into codegen). Declared now regardless, same as every
    // other runtime kernel here, so that later phase is a pure `call`-
    // site change, not a declare-list change too.
    writeln!(cg.out, "declare ptr @nir_alloc(i64)").unwrap();
    writeln!(cg.out, "declare void @nir_free(ptr)").unwrap();
    // `chan`/`spawn`/`join`'s kernels (`runtime-kernels/src/lib.rs`'s
    // "chan/spawn/join kernels" section) — every channel/thread payload
    // crosses this boundary as one `i64` word (`Ty::Channel`/`Ty::Thread`'s
    // own `llvm_ty` note); `nir_thread_spawn`'s first argument is a bare
    // function pointer (this file's generated trampolines, `spawn_thread`'s
    // own doc comment) — LLVM's opaque `ptr` covers that with no separate
    // function-pointer type needed.
    writeln!(cg.out, "declare i64 @nir_chan_new()").unwrap();
    writeln!(cg.out, "declare i64 @nir_chan_send(i64, i64)").unwrap();
    writeln!(cg.out, "declare i64 @nir_chan_recv(i64)").unwrap();
    writeln!(cg.out, "declare i64 @nir_thread_spawn(ptr, ptr)").unwrap();
    writeln!(cg.out, "declare i64 @nir_thread_join(i64)").unwrap();
    // `nfr(...)`'s kernels (`runtime-kernels/src/lib.rs`'s "nfr kernels"
    // section) — `nir_nfr_register` runs once per tracked function, in
    // `emit_c_main`'s own prologue (`declare_nfr_globals`/the
    // registration loop there); `nir_nfr_call_begin`/`_end` bracket
    // every call to it (`Codegen::function`, `Stmt::Return`'s own arms).
    writeln!(cg.out, "declare i64 @nir_nfr_register(ptr, i64, i64, double, i64, i64)").unwrap();
    writeln!(cg.out, "declare i64 @nir_nfr_call_begin(i64)").unwrap();
    writeln!(cg.out, "declare void @nir_nfr_call_end(i64, i64, i32)").unwrap();
    // `check_role`'s real implementation (`IDENTITY_BUILTINS`'s own doc
    // comment) — `1` if `role` is present in `claims` (real JSON roles
    // array, falling back to a comma-separated list), `0` otherwise.
    writeln!(cg.out, "declare i32 @nir_check_role(ptr, i64, ptr, i64)").unwrap();
    // `oidc_validate_token`/`extract_claim` (`IDENTITY_BUILTINS`'s own
    // doc comment) — real JWT/JWKS signature verification and JSON claim
    // extraction. Every `out_*` param is a `ptr` (either `{ptr, i64}`'s
    // own two words, or a plain `i64`) written into directly, matching
    // `emit_oidc_validate_token`'s own field-pointer-as-out-param design.
    writeln!(
        cg.out,
        "declare i32 @nir_oidc_validate_token(ptr, i64, ptr, i64, ptr, i64, ptr, i64, ptr, ptr, ptr, ptr, ptr, ptr, ptr)"
    )
    .unwrap();
    writeln!(cg.out, "declare i32 @nir_extract_claim(ptr, i64, ptr, i64, ptr)").unwrap();
    // `mock_issue_token` (`IDENTITY_BUILTINS`'s own doc comment) — the
    // inverse of `oidc_validate_token`: signs a token instead of
    // verifying one, HS256-only, `nir_mock_issue_token`'s own doc comment
    // has the scope. `out_token`/`out_err` are `{ptr, i64}`-shaped, same
    // as every other `str`-payload out-param above.
    writeln!(
        cg.out,
        "declare i32 @nir_mock_issue_token(ptr, i64, ptr, i64, ptr, i64, i64, i64, ptr, i64, ptr, i64, ptr, ptr)"
    )
    .unwrap();
    // The rest of Row 12 (`docs/nirdosha_row12_functions_identity.md`,
    // `kernel::identity`'s own module doc has the full design): dotted-
    // path claim lookup, sessions, refresh tokens, revocation, API keys.
    writeln!(cg.out, "declare i32 @nir_check_role_path(ptr, i64, ptr, i64, ptr, i64)").unwrap();
    writeln!(cg.out, "declare i32 @nir_extract_claim_path(ptr, i64, ptr, i64, ptr)").unwrap();
    writeln!(cg.out, "declare i32 @nir_check_revocation(ptr, i64)").unwrap();
    writeln!(
        cg.out,
        "declare void @nir_create_application_session(ptr, i64, ptr, i64, ptr, i64, ptr, i64, ptr, ptr, ptr, ptr)"
    )
    .unwrap();
    writeln!(cg.out, "declare void @nir_session_cookie(ptr, i64, i64, i64, ptr)").unwrap();
    // Real server-side session lookup (red-team report A2) —
    // `nir_verify_session`'s own doc comment (`identity.rs`) has the
    // full design; shape mirrors `nir_validate_api_key` just below
    // (one `str` input, the same 6-field `VerifiedIdentity` out-params
    // plus `out_err`), just with a session id instead of an API key.
    writeln!(cg.out, "declare i32 @nir_verify_session(ptr, i64, ptr, ptr, ptr, ptr, ptr, ptr, ptr)").unwrap();
    writeln!(cg.out, "declare void @nir_new_refresh_token(i64, ptr)").unwrap();
    writeln!(
        cg.out,
        "declare i32 @nir_exchange_refresh_token(i64, i64, ptr, i64, ptr, i64, ptr, i64, ptr, i64, ptr, ptr, ptr, ptr, ptr, ptr, ptr)"
    )
    .unwrap();
    writeln!(cg.out, "declare i32 @nir_validate_api_key(ptr, i64, ptr, i64, ptr, ptr, ptr, ptr, ptr, ptr, ptr)").unwrap();
    // `db`/`json` (`DB_BUILTINS`/`JSON_BUILTINS`'s own doc comments) —
    // real SQLite connectivity and JSON navigation. Every bind-value
    // param below is `ptr` to a `[N x NIR_BIND_VALUE_LLTY]` array (or
    // `null` for the zero-bind case).
    writeln!(cg.out, "declare i32 @nir_db_connect(ptr, i64, ptr, ptr)").unwrap();
    writeln!(cg.out, "declare i32 @nir_db_stop(i64)").unwrap();
    writeln!(cg.out, "declare i32 @nir_db_execute(i64, ptr, i64, ptr, i64, ptr, ptr)").unwrap();
    writeln!(cg.out, "declare i32 @nir_db_query(i64, ptr, i64, ptr, i64, ptr, ptr)").unwrap();
    writeln!(cg.out, "declare i32 @nir_json_validate(ptr, i64, ptr)").unwrap();
    writeln!(cg.out, "declare i32 @nir_json_get(ptr, i64, ptr, i64, ptr, ptr)").unwrap();
    writeln!(cg.out, "declare i32 @nir_json_array_get(ptr, i64, i64, ptr, ptr)").unwrap();
    writeln!(cg.out, "declare i32 @nir_json_array_len(ptr, i64, ptr, ptr)").unwrap();
    writeln!(cg.out, "declare i32 @nir_json_get_str(ptr, i64, ptr, i64, ptr, ptr)").unwrap();
    writeln!(cg.out, "declare i32 @nir_json_get_i64(ptr, i64, ptr, i64, ptr, ptr)").unwrap();
    writeln!(cg.out, "declare i32 @nir_json_get_f64(ptr, i64, ptr, i64, ptr, ptr)").unwrap();
    writeln!(cg.out, "declare i32 @nir_json_get_bool(ptr, i64, ptr, i64, ptr, ptr)").unwrap();
    writeln!(cg.out, "declare i32 @nir_json_set_str(ptr, i64, ptr, i64, ptr, i64, ptr, ptr)").unwrap();
    // `emit_encode_value_json`/`emit_decode_value_json`'s own runtime
    // half (this file's doc comment above those functions has the full
    // design) — the generic `Ty`<->JSON walk Stage 1 of reviving
    // compiled `serve` needs, none of which the request-shaped
    // `nir_json_get_*`/`_set_str` builtins above cover: encoding/
    // decoding one *bare* scalar (not keyed inside an object) and
    // folding an already-encoded JSON fragment into an object under a
    // given key. The four encoders are infallible (`void`, no `out_err`)
    // — see their own doc comments in `runtime-kernels/src/lib.rs`.
    writeln!(cg.out, "declare void @nir_json_encode_i64(i64, ptr)").unwrap();
    writeln!(cg.out, "declare void @nir_json_encode_f64(double, ptr)").unwrap();
    writeln!(cg.out, "declare void @nir_json_encode_bool(i32, ptr)").unwrap();
    writeln!(cg.out, "declare void @nir_json_encode_str(ptr, i64, ptr)").unwrap();
    writeln!(cg.out, "declare i32 @nir_json_decode_i64(ptr, i64, ptr, ptr)").unwrap();
    writeln!(cg.out, "declare i32 @nir_json_decode_f64(ptr, i64, ptr, ptr)").unwrap();
    writeln!(cg.out, "declare i32 @nir_json_decode_bool(ptr, i64, ptr, ptr)").unwrap();
    writeln!(cg.out, "declare i32 @nir_json_decode_str(ptr, i64, ptr, ptr)").unwrap();
    writeln!(cg.out, "declare i32 @nir_json_set_raw(ptr, i64, ptr, i64, ptr, i64, ptr, ptr)").unwrap();
    // `nirdosha_compiled_serve`'s own C-ABI bridge (`crates/compiled-
    // serve/src/lib.rs::nir_compiled_serve_run`) — only ever actually
    // `call`ed from `emit_c_main_serve`'s generated `main`, itself only
    // emitted when `build_serve` (not plain `build`) is used, but
    // declared unconditionally here anyway, same "an unused `declare`
    // is inert" convention every other kernel declare in this preamble
    // already follows.
    writeln!(cg.out, "declare i32 @nir_compiled_serve_run(ptr, i64, ptr, i64, i64, i64)").unwrap();
    // `env` (`ENV_BUILTINS`'s own doc comment, RFC 0011 §1) — reads a
    // process environment variable. Not resource-gated (no `Domain`, no
    // handle) — same reason `nir_json_get_str` isn't either.
    writeln!(cg.out, "declare i32 @nir_env_get(ptr, i64, ptr, ptr)").unwrap();
    // `transact { ... }` (`docs/TRANSACT.md`, `Codegen::emit_transact`'s
    // own doc comment has the compiled-backend scope). `txn_id`'s real
    // implementation; `sleep_ms`, ordinary compiled `sleep_ms(ms)` (`B9`).
    writeln!(cg.out, "declare void @nir_transact_gen_txn_id(ptr)").unwrap();
    writeln!(cg.out, "declare void @nir_sleep_ms(i64)").unwrap();
    // Durability log + crash replay (`kernel::transact`'s own module doc
    // has the full design) — every `nir_transact_*` here is a no-op on
    // a program that never calls it (the log file is never even opened
    // unless `nir_transact_log_init` itself is emitted, gated on
    // `Codegen::transact_sites` being non-empty).
    writeln!(cg.out, "declare i32 @nir_transact_log_init()").unwrap();
    writeln!(cg.out, "declare i32 @nir_transact_begin(ptr, i64, i64)").unwrap();
    writeln!(cg.out, "declare i32 @nir_transact_mark_commit_pending(ptr, i64, ptr, i64)").unwrap();
    writeln!(cg.out, "declare i32 @nir_transact_mark_compensate_pending(ptr, i64, ptr, i64)").unwrap();
    writeln!(cg.out, "declare i32 @nir_transact_mark_committed(ptr, i64)").unwrap();
    writeln!(cg.out, "declare i32 @nir_transact_mark_compensated(ptr, i64)").unwrap();
    writeln!(cg.out, "declare i64 @nir_transact_decode_args(ptr, i64, ptr, i64)").unwrap();
    writeln!(cg.out, "declare void @nir_transact_register_replay_site(i64, ptr)").unwrap();
    writeln!(cg.out, "declare void @nir_transact_replay_all()").unwrap();
    // `mq`/`http`/`https` (`MQ_BUILTINS`/`HTTP_BUILTINS`'s own doc
    // comments) — real Redis connectivity and HTTP(S) client calls.
    writeln!(cg.out, "declare i32 @nir_mq_connect(ptr, i64, i64, ptr, ptr)").unwrap();
    writeln!(cg.out, "declare i32 @nir_mq_stop(i64)").unwrap();
    writeln!(cg.out, "declare i32 @nir_mq_publish(i64, ptr, i64, ptr, i64, ptr)").unwrap();
    writeln!(cg.out, "declare i32 @nir_mq_consume(i64, ptr, i64, i64, ptr, ptr)").unwrap();
    writeln!(cg.out, "declare i32 @nir_http_get(ptr, i64, i64, ptr, i64, ptr, ptr, ptr)").unwrap();
    writeln!(cg.out, "declare i32 @nir_http_post(ptr, i64, i64, ptr, i64, ptr, i64, ptr, ptr, ptr)").unwrap();
    writeln!(cg.out, "declare i32 @nir_https_get(ptr, i64, i64, ptr, i64, ptr, ptr, ptr)").unwrap();
    writeln!(cg.out, "declare i32 @nir_https_post(ptr, i64, i64, ptr, i64, ptr, i64, ptr, ptr, ptr)").unwrap();
    // rfcs/0011-uniform-service-provider-model.md §1/§2's `call`-shape
    // dispatch entrypoint (`CALL_BUILTINS`'s own doc comment) —
    // `(url_ptr, url_len, path_ptr, path_len, body_ptr, body_len,
    // out_status, out_body, out_err)`, `emit_call_via`'s own shape below
    // mirrors `emit_http_call`'s `nir_http_post`-style call closely.
    writeln!(cg.out, "declare i32 @nir_call_via(ptr, i64, ptr, i64, ptr, i64, ptr, ptr, ptr)").unwrap();
    // `workflow` Layer 1 (`docs/WORKFLOW.md`, `Codegen::emit_workflow_start`'s
    // own doc comment has the full scope) — the in-memory instance
    // table, SLA/escalation detection, and the four notification
    // channels.
    writeln!(cg.out, "declare i32 @nir_workflow_create_instance(ptr, i64, ptr, i64, ptr)").unwrap();
    writeln!(cg.out, "declare i32 @nir_workflow_get_state(ptr, i64, i64, ptr)").unwrap();
    writeln!(cg.out, "declare i32 @nir_workflow_set_state(ptr, i64, i64, ptr, i64)").unwrap();
    writeln!(cg.out, "declare i32 @nir_workflow_list_overdue(ptr, i64, ptr, i64, ptr, ptr)").unwrap();
    writeln!(cg.out, "declare i32 @nir_workflow_send(ptr, i64, i64, ptr, i64, ptr, i64, ptr, i64, ptr)").unwrap();
    writeln!(cg.out, "declare i32 @nir_notify(i64, i64, ptr, i64, ptr, i64, ptr, i64, ptr)").unwrap();
    // The APM kernel's flight recorder (`runtime-kernels/src/kernel/
    // mod.rs`'s own doc comment) — declared unconditionally like every
    // other kernel here, called exactly once by `emit_c_main` on every
    // exit path, never by anything a `.nir` program itself writes (not
    // in `ast::BUILTIN_NAMES` at all — there is no `Expr` that lowers to
    // a `call` to this).
    writeln!(cg.out, "declare void @nir_kernel_flight_recorder_dump()").unwrap();
    // RFC 0011 §3's open domain registry bootstrap — `emit_c_main` emits
    // exactly one call to this, at the very top of `main`, strictly
    // before any user code, so the 7 built-in domains get their
    // historical, stable ids 0-6. This declare, like the one above,
    // carries zero knowledge of the built-in domain set or its order —
    // that's `runtime-kernels`'s own `BUILTIN_DOMAINS` array, the only
    // place that order is written down.
    writeln!(cg.out, "declare void @nir_kernel_register_builtin_domains()").unwrap();
    writeln!(cg.out, "declare void @nir_kernel_register_domain(ptr, i64, ptr, i64, i64)").unwrap();
    // RFC 0011 §2/§4's plugin-provider dispatch table registration —
    // `domain_name` matches the immediately-preceding `nir_kernel_
    // register_domain` call's own `name` argument exactly (the kernel
    // looks up that already-registered `DomainId` by name rather than
    // re-deriving it), `scheme` is the compiler's own already-normalized
    // scheme identifier (`plugin::normalize_scheme`'s output, not the
    // raw declared scheme text). The four trailing `ptr`s are the
    // provider's own function addresses, taken directly from the global
    // symbols this file already `declare`s for every native plugin
    // builtin (just below) — opaque `ptr`s here since their *real* LLVM
    // signatures differ per shape (`_op`'s one `str` arg vs `_request`'s
    // two); `is_call_shape` tells the kernel which one it's holding, so
    // it can transmute it back to the correct `extern "C" fn` type on
    // its own side rather than this call needing two mutually-exclusive
    // parameters.
    writeln!(cg.out, "declare void @nir_kernel_register_plugin_provider(ptr, i64, ptr, i64, ptr, ptr, ptr, ptr, i32)").unwrap();
    // RFC 0011 §5's reaper bootstrap — `emit_c_main` emits exactly one
    // call to this, immediately after the registration calls above,
    // still strictly before any user code runs. Carries no arguments;
    // `kernel::reaper::start` reads its own interval from
    // `NIRDOSHA_KERNEL_REAPER_INTERVAL_SECS` on the kernel side.
    writeln!(cg.out, "declare void @nir_kernel_start_reaper()").unwrap();
    // "%lld\n\0" — 6 bytes (%, l, l, d, \n, \0), not 5; LLVM's array
    // constant size has to match the literal exactly, byte for byte.
    writeln!(cg.out, "@.int_fmt = private unnamed_addr constant [6 x i8] c\"%lld\\0A\\00\"").unwrap();
    // "%f\n\0" — 4 bytes. `%f` (not `%g`/`%e`) is a plain, standard
    // choice for a `double`; it won't byte-for-byte match the
    // interpreter's `render()` (Rust's shortest-round-trip `f64`
    // formatting), a known, honest cosmetic difference between the two
    // execution paths, not a semantic one — both print the same real
    // number.
    writeln!(cg.out, "@.float_fmt = private unnamed_addr constant [4 x i8] c\"%f\\0A\\00\"").unwrap();
    // "%.*s\n\0" — 6 bytes. `%.*s` (precision from an explicit `i32` arg,
    // not `%s`) prints exactly `len` bytes regardless of what follows
    // them in memory — load-bearing, since a `str` value's buffer isn't
    // guaranteed NUL-terminated by this design (see `Ty::Str`'s note in
    // `llvm_ty`), only the format string itself is.
    writeln!(cg.out, "@.str_fmt = private unnamed_addr constant [6 x i8] c\"%.*s\\0A\\00\"").unwrap();
    // "()\n\0" — 4 bytes. No format specifier at all: `unit` carries no
    // runtime data to interpolate, so this is printed as a fixed literal
    // — matching `interpreter.rs`'s `render()`, which prints `"()"` for
    // `Value::Unit`, so interpreted/compiled output agree.
    writeln!(cg.out, "@.unit_fmt = private unnamed_addr constant [4 x i8] c\"()\\0A\\00\"").unwrap();
    // Native plugin builtins (rfcs/0005 §3): one `declare` per symbol,
    // the same "linked native call into a staticlib" shape as
    // `nir_det`/`nir_rank`/etc. above, just for a third-party-supplied
    // symbol/library instead of `runtime-kernels/src/lib.rs`'s own. `validate()`
    // (called by `emit_llvm_ir_with_native_plugins` before this ever
    // runs) already proved every param/ret is a plain scalar `llvm_ty`
    // knows how to render.
    for np in native_plugins {
        let param_lltys: Result<Vec<String>, CodegenError> = np.params.iter().map(|t| llvm_ty(t, &cg.registry)).collect();
        let param_lltys = param_lltys?;
        let ret_llty = llvm_ty(&np.ret, &cg.registry)?;
        writeln!(cg.out, "declare {ret_llty} @{}({})", np.name, param_lltys.join(", ")).unwrap();
    }
    writeln!(cg.out).unwrap();

    // One process-wide storage slot per `nfr(...)`-tracked function,
    // populated once by `emit_c_main`'s own prologue (`nir_nfr_register`)
    // before `nir_main` runs, then read by that same function's own
    // entry prologue (`Codegen::function`) on every call — see this
    // module's "nfr kernels" declares above for the kernel side.
    for f in &program.fns {
        if f.nfr.is_some() {
            writeln!(cg.out, "@nfr_id.{} = global i64 0", f.name).unwrap();
        }
    }

    for f in &program.fns {
        cg.function(f)?;
    }

    match serve {
        Some(opts) => {
            let exposed = crate::typeck::exposed_fn_names(program);
            let mut route_entries = Vec::with_capacity(exposed.len());
            // Sorted, not iteration order over a `HashSet` — an
            // unstable route-table order would make the emitted `.ll`
            // (and so the final binary) non-deterministic run to run
            // for the exact same source, same "deterministic output"
            // bar every other pass in this file already holds itself
            // to (`declare_named_type`'s own dependency-order comment
            // makes the same point for a different reason).
            let mut names: Vec<&String> = exposed.iter().collect();
            names.sort();
            for name in names {
                let f = program.fns.iter().find(|f| &f.name == name).expect("typeck.rs already proved every exposed name resolves to a real fn");
                let wrapper_symbol = cg.emit_serve_route_wrapper(f)?;
                route_entries.push((format!("/api/{name}"), wrapper_symbol));
            }
            cg.emit_c_main_serve(program, &route_entries, &opts.ui_html, opts.port, opts.require_sender_constrained_tokens)?;
        }
        None => cg.emit_c_main(program, native_plugins)?,
    }
    // See `string_globals`'s own doc — every `str` literal's backing
    // global constant, collected during function codegen since it can't
    // be written mid-function-body, appended here once at the end.
    cg.out.push_str(&cg.string_globals);
    // See `trampolines`'s own doc — every `spawn` call site's generated
    // trampoline function, appended here for the same reason.
    cg.out.push_str(&cg.trampolines);
    // See `named_type_decls`'s own doc — every `%Name = type {...}`
    // declaration, collected during function codegen (a named type is
    // only discovered when a concrete instantiation is actually used)
    // and prepended here at the *top* of the module, so each one
    // textually precedes the `define` blocks that reference it (LLVM
    // textual IR requires a named struct type to be declared before any
    // use in a function body). `declare_named_type` already emitted
    // them in dependency order (a struct's named-typed fields before the
    // struct itself), so prepending the whole buffer preserves that
    // order in the final text.
    cg.out.insert_str(0, &cg.named_type_decls);
    Ok(cg.out)
}

/// `typeck::bind_type_params`'s exact substitution logic, duplicated here
/// (that function is private to `typeck.rs`) so `Codegen::ctor_ty`/
/// `infer_type_args` can resolve a generic constructor's type parameters
/// from its arguments' own types the same way `typeck.rs`'s own
/// `resolve_type_args` fall-back path does — no inference theory of
/// codegen's own, just the same structural bind `typeck.rs` already
/// proved correct for every generic program.
fn bind_type_params_owned(decl_ty: &Ty, concrete_ty: &Ty, type_params: &[String], subst: &mut HashMap<String, Ty>) {
    match (decl_ty, concrete_ty) {
        (Ty::Named(name, args), _) if args.is_empty() && type_params.iter().any(|p| p == name) => {
            subst.entry(name.clone()).or_insert_with(|| concrete_ty.clone());
        }
        (Ty::Box(a), Ty::Box(b))
        | (Ty::Froze(a), Ty::Froze(b))
        | (Ty::Ref(a), Ty::Ref(b))
        | (Ty::Thread(a), Ty::Thread(b))
        | (Ty::Channel(a), Ty::Channel(b)) => bind_type_params_owned(a, b, type_params, subst),
        (Ty::Vector(a, _), Ty::Vector(b, _)) | (Ty::Matrix(a, _, _), Ty::Matrix(b, _, _)) => {
            bind_type_params_owned(a, b, type_params, subst)
        }
        (Ty::Named(dn, dargs), Ty::Named(cn, cargs)) if dn == cn && dargs.len() == cargs.len() => {
            for (da, ca) in dargs.iter().zip(cargs.iter()) {
                bind_type_params_owned(da, ca, type_params, subst);
            }
        }
        _ => {}
    }
}

impl Codegen<'_> {
    fn fresh_reg(&mut self, prefix: &str) -> String {
        self.tmp += 1;
        format!("%{prefix}.{}", self.tmp)
    }
    fn fresh_label(&mut self, prefix: &str) -> String {
        self.label += 1;
        format!("{prefix}.{}", self.label)
    }
    /// A fresh, module-unique global name (`@.prefix.N`) — shares the
    /// `tmp` counter `fresh_reg` uses (no collision risk: `@.` vs. `%`
    /// are disjoint sigils), used for each `str` literal's backing
    /// constant.
    fn fresh_global(&mut self, prefix: &str) -> String {
        self.tmp += 1;
        format!("@.{prefix}.{}", self.tmp)
    }

    /// Every `alloca` in the whole file must go through this, never a
    /// direct `writeln!(self.out, "... = alloca ...")` — see the
    /// `entry_allocas` field doc for why.
    fn emit_alloca(&mut self, dest: &str, ty: &str) {
        writeln!(self.entry_allocas, "  {dest} = alloca {ty}").unwrap();
    }

    /// The `&mut self` wrapper every real-codegen call site uses
    /// instead of the free `llvm_ty` function: resolves `ty` to its
    /// LLVM type string exactly as the free function does, *and* — for
    /// a `Ty::Named` — ensures the real `%Name = type {...}`
    /// declaration has been emitted into `named_type_decls` (memoized
    /// by mangled name) before handing back the name a `define`/GEP/
    /// `alloca` is about to reference. The free function can't do this
    /// half (it has no `&mut self`); the `check_supported` pre-pass
    /// doesn't need this half (it never emits a `define`, only
    /// validates — so it calls the free function directly with its own
    /// throwaway registry). Pure delegation otherwise — no type-
    /// resolution logic of its own.
    fn llvm_ty(&mut self, ty: &Ty) -> Result<String, CodegenError> {
        if let Ty::Named(_, _) = ty {
            self.declare_named_type(ty)?;
        }
        llvm_ty(ty, &self.registry)
    }

    /// Emit the real `%Name = type { ... }` declaration for one concrete
    /// `struct`/`enum` instantiation, exactly once per distinct mangled
    /// name (memoized via `declared_named_types`). Called from `llvm_ty`
    /// above — so any codegen path that resolves a `Ty::Named` to its
    /// LLVM name automatically declares it too, no separate "declare
    /// every struct" pass needed.
    ///
    /// **Struct layout**: `{ f0_llty, f1_llty, ... }` in field order, each
    /// field's LLVM type resolved via the free `llvm_ty` against the
    /// instantiation's own type-argument substitution. A named-typed
    /// field (a nested struct/enum) is declared *first*, recursively, so
    /// the outer `%Outer = type { %Point }` textually follows `%Point` —
    /// LLVM requires a named struct type to be declared before any use,
    /// including as another struct's field type. LLVM's own struct
    /// layout then computes the real per-field alignment/padding; this
    /// codebase never reads field offsets by hand for a struct (always
    /// via `getelementptr %Name, ..., i32 0, i32 <idx>`), so the exact
    /// padding is LLVM's concern, not ours.
    ///
    /// **Enum layout**: `{ i64 tag, [N x i64] payload }` — a hand-rolled
    /// tagged union (LLVM has no native enum/sum-type equivalent).
    /// `tag` is the variant's declaration-order index
    /// (`registry.enum_variants` order, matching `typeck.rs`'s own
    /// exhaustiveness-checking order). `N` is
    /// `1 + max_over_variants(sum_of(conservative_word_count(payload_field)))`,
    /// a real compile-time integer, so the same buffer fits every
    /// variant's payload without a per-variant type. Payload fields are
    /// stored into this raw `[N x i64]` buffer at 8-byte-aligned word
    /// offsets by `construct`/`match_expr` — see those for the GEP
    /// arithmetic; the buffer's element type is always `i64` regardless
    /// of a field's real type, so a `getelementptr i64, ptr %payload,
    /// i64 <word_off>` gives an always-8-byte-aligned address for any
    /// field store/load (every type this language has needs at most
    /// 8-byte alignment).
    ///
    /// A real, pre-existing gap found this session (2026-09), disclosed
    /// rather than fixed here: `match <result_expr> { Ok(j) => Ok(j),
    /// Err(e) => Err(e) }` -- re-wrapping a `Result` value inside a
    /// `match` where an arm's own tail expression directly constructs
    /// `Ok(...)`/`Err(...)` -- hits this function's own `unreachable!`
    /// with `decl_name="Ok"` (or `"Err"`): whatever infers that arm's own
    /// type resolves `Ok`/`Err` as if they named a plain, zero-type-
    /// argument struct/enum, instead of the prelude `Result` enum's own
    /// generic variant constructors. Confirmed independent of which
    /// concrete types are involved. Workaround used throughout
    /// `examples/features/55_nirdosha_ops_console.nir`: route each arm
    /// through a small named helper (`fn wrap_ok(v: T) -> Result(T, E)
    /// { return Ok(v) }`) instead of writing the bare constructor as the
    /// arm's own tail expression.
    fn declare_named_type(&mut self, ty: &Ty) -> Result<(), CodegenError> {
        let Ty::Named(decl_name, args) = ty else {
            return Ok(());
        };
        let mangled = mangle_ty(ty);
        if !self.declared_named_types.insert(mangled.clone()) {
            return Ok(()); // already declared — memoized
        }
        let decl_name = decl_name.as_str();
        if let Some(fields) = self.registry.struct_fields(decl_name) {
            let type_params = self.registry.struct_type_params(decl_name).unwrap_or(&[]);
            let subst = zip_type_params(type_params, args);
            let mut parts: Vec<String> = Vec::with_capacity(fields.len());
            for f in fields {
                let field_ty = substitute_ty(&f.ty, &subst);
                // A named-typed field is a dependency of this struct's
                // own declaration — declare it first so its `%Name =
                // type {...}` textually precedes this one. Recurses
                // through `llvm_ty`'s `Ty::Named` branch (which calls
                // this method), so arbitrarily-deep nesting is ordered
                // correctly in one pass.
                if matches!(field_ty, Ty::Named(_, _)) {
                    self.declare_named_type(&field_ty)?;
                }
                parts.push(llvm_ty(&field_ty, &self.registry)?);
            }
            writeln!(self.named_type_decls, "%{mangled} = type {{ {} }}", parts.join(", ")).unwrap();
            Ok(())
        } else if let Some(variants) = self.registry.enum_variants(decl_name) {
            let type_params = self.registry.enum_type_params(decl_name).unwrap_or(&[]);
            let subst = zip_type_params(type_params, args);
            let max_payload_words: u64 = variants
                .iter()
                .map(|v| {
                    v.payload
                        .iter()
                        .map(|t| conservative_word_count(&substitute_ty(t, &subst), &self.registry))
                        .sum::<u64>()
                })
                .max()
                .unwrap_or(0);
            let n = max_payload_words;
            writeln!(self.named_type_decls, "%{mangled} = type {{ i64, [{n} x i64] }}").unwrap();
            Ok(())
        } else {
            unreachable!("typeck.rs already proved every Ty::Named resolves to a struct or enum: decl_name={decl_name:?} args={args:?} mangled={mangled:?}")
        }
    }

    fn function(&mut self, f: &FnDecl) -> Result<(), CodegenError> {
        self.current_fn_ret = f.ret.clone();
        self.current_fn_name = f.name.clone();
        // Aggregate returns use an sret-style out-pointer, passed as an
        // implicit first argument, rather than an LLVM-level aggregate
        // return value — the first by-pointer ABI convention in this
        // file (module doc / `Codegen::expr_ptr`). The caller allocas
        // its own destination and passes its address; this function's
        // `Stmt::Return` memcpys its computed result into it and `ret
        // void`s, instead of `ret <ty> <val>`.
        let is_agg_ret = f.ret.is_aggregate();
        let ret_llty: String = if is_agg_ret { "void".to_string() } else { self.llvm_ty(&f.ret)? };
        let name = if f.name == "main" { "nir_main" } else { f.name.as_str() };

        let mut params: Vec<String> = Vec::new();
        if is_agg_ret {
            params.push("ptr %sret.ret".to_string());
        }
        for p in &f.params {
            if p.ty.is_aggregate() {
                // Aggregate params are passed as a plain pointer, not by
                // value — see the prologue below for the copy-in that
                // makes this behave like the language's actual value
                // semantics (there's no `m[i,j] = x` lvalue syntax, so a
                // callee can never observably mutate the caller's copy
                // through this pointer; the prologue copy exists so a
                // whole-variable reassignment inside the callee doesn't
                // either).
                params.push(format!("ptr %arg.{}", p.name));
            } else {
                params.push(format!("{} %arg.{}", self.llvm_ty(&p.ty)?, p.name));
            }
        }
        writeln!(self.out, "define {ret_llty} @{name}({}) {{", params.join(", ")).unwrap();
        writeln!(self.out, "entry:").unwrap();
        // Every `alloca` emitted from here until this function's closing
        // brace lands in `self.entry_allocas` instead of `self.out`
        // (see that field's doc) — remember exactly where in `self.out`
        // they belong (right after `entry:`, before anything else this
        // function emits) and splice them in once the body's done.
        let alloca_splice_pos = self.out.len();
        self.entry_allocas.clear();
        self.terminated = false;
        self.current_fn_sret = if is_agg_ret { Some("%sret.ret".to_string()) } else { None };

        let mut scopes = Scopes::new();
        for p in &f.params {
            if p.ty.is_aggregate() {
                let agg_llty = self.llvm_ty(&p.ty)?;
                let local_ptr = format!("%{}.addr", p.name);
                self.emit_alloca(&local_ptr, &agg_llty);
                let bytes = agg_byte_size_operand(&p.ty, &self.registry);
                writeln!(
                    self.out,
                    "  call void @llvm.memcpy.p0.p0.i64(ptr {local_ptr}, ptr %arg.{}, i64 {bytes}, i1 false)",
                    p.name
                )
                .unwrap();
                scopes.define(&p.name, p.ty.clone(), local_ptr);
            } else {
                let ty = self.llvm_ty(&p.ty)?;
                let ptr = format!("%{}.addr", p.name);
                self.emit_alloca(&ptr, &ty);
                writeln!(self.out, "  store {ty} %arg.{}, ptr {ptr}", p.name).unwrap();
                scopes.define(&p.name, p.ty.clone(), ptr);
            }
        }

        // Field masking's only source of "who's calling" — the first
        // `RoleView`/`ClaimView`-typed parameter, if either exists (this
        // function's own doc comment on `current_fn_role_view_param`).
        // A plain linear scan of the signature, not a scope lookup: this
        // runs once per function, at codegen time, over a handful of
        // params, not on any hot path.
        self.current_fn_role_view_param =
            f.params.iter().find(|p| matches!(&p.ty, Ty::Named(n, args) if n == "RoleView" && args.is_empty())).map(|p| p.name.clone());
        self.current_fn_claim_view_param =
            f.params.iter().find(|p| matches!(&p.ty, Ty::Named(n, args) if n == "ClaimView" && args.is_empty())).map(|p| p.name.clone());

        // `nfr(...)` entry instrumentation — see `current_fn_nfr`/
        // `current_fn_nfr_regs`'s own doc comments and `emit_nfr_call_end`
        // for the matching exit side, emitted at every one of this
        // function's own return points.
        self.current_fn_nfr = f.nfr.clone();
        self.current_fn_nfr_regs = if f.nfr.is_some() {
            let id_reg = self.fresh_reg("nfr_id");
            writeln!(self.out, "  {id_reg} = load i64, ptr @nfr_id.{}", f.name).unwrap();
            let start_reg = self.fresh_reg("nfr_start");
            writeln!(self.out, "  {start_reg} = call i64 @nir_nfr_call_begin(i64 {id_reg})").unwrap();
            Some((id_reg, start_reg))
        } else {
            None
        };

        self.stmts(&f.body.stmts, &mut scopes)?;

        // A function whose body definitely returns on every path
        // (typeck.rs already proved this for any non-`unit` return type)
        // never falls off the end reachably — but the *block* still
        // needs a terminator if the very last statement wasn't itself a
        // `return` on this specific path (e.g. a `unit`-returning
        // function that just runs off the end normally).
        if !self.terminated {
            if let Some(names) = self.free_map.at_fn_end.get(&f.name).cloned() {
                self.emit_frees_for_names(&names, &scopes);
            }
            if f.ret == Ty::Unit {
                // Only reachable for `unit` (typeck's definite-return
                // analysis rules out every other return type falling
                // through) — never `Result`-typed, so `was_err` is
                // unconditionally `"0"` here, no tag inspection needed.
                self.emit_nfr_call_end("0");
                writeln!(self.out, "  ret void").unwrap();
            } else {
                // typeck.rs's definite-return analysis already rules
                // this out for any well-typed program; unreachable is
                // the honest LLVM idiom for "provably can't happen".
                writeln!(self.out, "  unreachable").unwrap();
            }
        }
        writeln!(self.out, "}}\n").unwrap();
        // Splice every alloca this function collected into place, right
        // after `entry:` — see `entry_allocas`'s doc and the note at
        // this function's start where `alloca_splice_pos` was captured.
        self.out.insert_str(alloca_splice_pos, &self.entry_allocas);
        Ok(())
    }

    fn stmts(&mut self, stmts: &[Stmt], scopes: &mut Scopes) -> Result<(), CodegenError> {
        for s in stmts {
            if self.terminated {
                break; // dead code after a `return`/branch — not emitted
            }
            self.stmt(s, scopes)?;
        }
        Ok(())
    }

    fn stmt(&mut self, stmt: &Stmt, scopes: &mut Scopes) -> Result<(), CodegenError> {
        match stmt {
            Stmt::Let { name, ty, value, span } => {
                if ty.is_aggregate() {
                    // No integer guard/narrow pipeline applies to an
                    // aggregate — always a fresh destination alloca plus
                    // a whole-value `memcpy` from whatever `expr_ptr`
                    // produced, never a direct alias of the source
                    // pointer: `let w = v` has to give `w` its own
                    // storage, or a later `w = ...` reassignment would
                    // silently mutate `v` too (see `Codegen::expr_ptr`'s
                    // `Expr::Ident` arm, which deliberately returns the
                    // existing pointer with no copy of its own). A
                    // constructor expression here is built with `ty` as its
                    // expected type (`expr_ptr_expected`), so a generic
                    // zero-payload variant like `None` resolves from this
                    // `let`'s own annotation rather than the no-context
                    // `ctor_ty` fallback.
                    let src = self.expr_ptr_expected(value, ty, scopes)?;
                    let agg_llty = self.llvm_ty(ty)?;
                    let dest = self.fresh_reg(&format!("{name}.addr"));
                    self.emit_alloca(&dest, &agg_llty);
                    let bytes = agg_byte_size_operand(ty, &self.registry);
                    writeln!(self.out, "  call void @llvm.memcpy.p0.p0.i64(ptr {dest}, ptr {src}, i64 {bytes}, i1 false)").unwrap();
                    scopes.define(name, ty.clone(), dest);
                    return Ok(());
                }
                let val = self.expr(value, scopes)?; // i64 (or i1 for bool)
                let val = self.guard_in_range(&val, ty, *span)?; // checked at i64 width, before narrowing
                let val = if ty.is_integer() { self.narrow_from_i64(&val, ty)? } else { val };
                let llty = self.llvm_ty(ty)?;
                let ptr = self.fresh_reg(&format!("{name}.addr"));
                self.emit_alloca(&ptr, &llty);
                writeln!(self.out, "  store {llty} {val}, ptr {ptr}").unwrap();
                scopes.define(name, ty.clone(), ptr);
                Ok(())
            }
            Stmt::Return { value, span } => {
                // Captured once, up front — every arm below needs it
                // emitted immediately before its own `ret`, since nothing
                // can follow a block's terminator in valid IR. `ty`
                // still resolves at this point (before any pop), whether
                // or not `value` itself is the box being returned —
                // `ownership.rs` already excluded a directly-returned box
                // from this exact list (see `FreeMap::at_return`'s doc).
                let free_names = self.free_map.at_return.get(span).cloned().unwrap_or_default();
                match value {
                    Some(e) => {
                        let ret_ty = self.current_fn_ret.clone();
                        if ret_ty.is_aggregate() {
                            let src = self.expr_ptr_expected(e, &ret_ty, scopes)?;
                            // Masks in place, before the value is copied
                            // out to the caller's own `sret` slot below —
                            // see `emit_field_masking`'s own doc comment.
                            // Safe to mutate `src` even when it's an
                            // existing local's own storage (e.g. `return
                            // e` for some `let e: Employee = ...`): this
                            // is a `return`, so nothing in this function
                            // reads that binding again either way.
                            self.emit_field_masking(&src, &ret_ty, scopes)?;
                            let sret = self
                                .current_fn_sret
                                .clone()
                                .expect("an aggregate-returning function always sets current_fn_sret");
                            let bytes = agg_byte_size_operand(&ret_ty, &self.registry);
                            writeln!(self.out, "  call void @llvm.memcpy.p0.p0.i64(ptr {sret}, ptr {src}, i64 {bytes}, i1 false)").unwrap();
                            self.emit_frees_for_names(&free_names, scopes);
                            // `nfr(error_rate_max: ...)` needs to know
                            // whether this specific return was `Err` —
                            // `typeck.rs` already proved `ret_ty` is
                            // `Result(_, _)` whenever that field is
                            // declared (`NfrErrorRateNeedsResultReturn`),
                            // so the tag word at offset 0 (`Ok` = 0,
                            // `Err` = 1, `ast::prelude_enums`' own
                            // declaration order) is exactly the flag
                            // needed. Every other `nfr(...)` field (or no
                            // `nfr(...)` at all) skips this entirely — no
                            // reason to inspect a tag nothing will check.
                            let was_err = if self.current_fn_nfr.as_ref().is_some_and(|n| n.error_rate_max.is_some()) {
                                let agg_llty = self.llvm_ty(&ret_ty)?;
                                let tag_ptr = self.fresh_reg("nfr_tag_ptr");
                                writeln!(self.out, "  {tag_ptr} = getelementptr inbounds {agg_llty}, ptr {src}, i32 0, i32 0").unwrap();
                                let tag = self.fresh_reg("nfr_tag");
                                writeln!(self.out, "  {tag} = load i64, ptr {tag_ptr}").unwrap();
                                let is_err = self.fresh_reg("nfr_is_err");
                                writeln!(self.out, "  {is_err} = icmp ne i64 {tag}, 0").unwrap();
                                let is_err_i32 = self.fresh_reg("nfr_is_err_i32");
                                writeln!(self.out, "  {is_err_i32} = zext i1 {is_err} to i32").unwrap();
                                is_err_i32
                            } else {
                                "0".to_string()
                            };
                            self.emit_nfr_call_end(&was_err);
                            writeln!(self.out, "  ret void").unwrap();
                        } else {
                            let val = self.expr(e, scopes)?;
                            // `refine.rs`/`smt.rs` now both record a proof for
                            // `return` sites too (they gained their own
                            // `current_fn_ret` field, the same fix this file
                            // already had) — so this can be genuine Tier 1 in
                            // practice, not just in principle. Still routed
                            // through the same real guard either way, not a
                            // hardcoded always-check special case.
                            let val = self.guard_in_range(&val, &ret_ty, *span)?;
                            let val = if ret_ty.is_integer() { self.narrow_from_i64(&val, &ret_ty)? } else { val };
                            self.emit_frees_for_names(&free_names, scopes);
                            // Never `Result`-typed here — `Result` is
                            // always `is_aggregate()` (routed through the
                            // branch above), so `was_err` is always `"0"`.
                            self.emit_nfr_call_end("0");
                            let ret_llty = self.llvm_ty(&ret_ty)?;
                            if matches!(ret_ty, Ty::Unit) {
                                // `ret void` takes no operand. An explicit
                                // `return <unit-typed call>` (e.g. `return
                                // print("...")` early-exiting a `-> unit` fn)
                                // is typecheck-legal and lands here in the
                                // scalar arm — `llvm_ty(unit)` is "void",
                                // but the call's own value ("0") must not
                                // ride along: `ret void 0` is invalid IR and
                                // clang rejects the whole module (found
                                // 2026-09-11 while compile-testing the
                                // paste-anywhere prompt's rule-20 example
                                // end to end, not by reading).
                                writeln!(self.out, "  ret void").unwrap();
                            } else {
                                writeln!(self.out, "  ret {} {val}", ret_llty).unwrap();
                            }
                        }
                    }
                    None => {
                        self.emit_frees_for_names(&free_names, scopes);
                        self.emit_nfr_call_end("0");
                        writeln!(self.out, "  ret void").unwrap();
                    }
                }
                self.terminated = true;
                Ok(())
            }
            Stmt::While { cond, body, span } => self.while_loop(*span, cond, body, scopes),
            Stmt::Expr(e) => {
                // An aggregate-valued expression statement (e.g. a bare
                // `v = w` reassignment) has no bare SSA value to give
                // `expr()` — same fork as `Stmt::Let`/`Stmt::Return`,
                // based on `local_ty_of`'s (typeck-mirroring) guess at
                // `e`'s type rather than a full inference pass, matching
                // this function's own existing precedent.
                if self.local_ty_of(e, scopes).is_aggregate() {
                    self.expr_ptr(e, scopes)?;
                } else {
                    self.expr(e, scopes)?;
                }
                Ok(())
            }
            Stmt::Audited { body, span, .. } => {
                // Save/restore, not unconditional reset -- see
                // `Codegen::audited`'s doc comment for why (nesting).
                let was_audited = self.audited;
                self.audited = true;
                scopes.push();
                let result = self.stmts(body, scopes);
                // Only when the block falls through normally -- if it
                // already emitted a `ret`/`br` (an inner `return`), the
                // block is terminated and nothing more can follow it in
                // valid IR; `Stmt::Return`'s own free-emission already
                // covered every still-owned box on that path.
                if result.is_ok()
                    && !self.terminated
                    && let Some(names) = self.free_map.at_audited_end.get(span).cloned()
                {
                    self.emit_frees_for_names(&names, scopes);
                }
                scopes.pop();
                self.audited = was_audited;
                result
            }
        }
    }

    /// A compile-time string literal as a real `{ptr, i64}` SSA value —
    /// `emit_check_role`'s own `err_msg_global`/`insertvalue` sequence,
    /// factored out so `check_role_path`/`extract_claim_path` (and any
    /// future caller needing a fixed `Err` message) don't repeat it.
    fn const_str_value(&mut self, global_prefix: &str, s: &str) -> String {
        let global = self.fresh_global(global_prefix);
        writeln!(self.string_globals, "{global} = private unnamed_addr constant [{} x i8] c\"{}\"", s.len(), llvm_escape_bytes(s.as_bytes())).unwrap();
        let partial = self.fresh_reg(&format!("{global_prefix}_partial"));
        writeln!(self.out, "  {partial} = insertvalue {{ptr, i64}} undef, ptr {global}, 0").unwrap();
        let full = self.fresh_reg(&format!("{global_prefix}_full"));
        writeln!(self.out, "  {full} = insertvalue {{ptr, i64}} {partial}, i64 {}, 1", s.len()).unwrap();
        full
    }

    /// `emit_result_merge`'s twin for an aggregate (multi-field struct)
    /// `Ok` payload — `emit_oidc_validate_token`'s own tag/`memcpy`/
    /// branch/merge sequence, factored out so
    /// `exchange_refresh_token`/`validate_api_key` (both reissuing a full
    /// `VerifiedIdentity`, the same shape `oidc_validate_token` already
    /// has) don't repeat it a third and fourth time. `ok_scratch` is a
    /// pointer to an already-fully-written `ok_ty`-typed value (built via
    /// out-params pointing directly at its own fields, same discipline
    /// every caller here already uses); `err_scratch` a pointer to an
    /// already-written `{ptr, i64}` error message.
    fn emit_result_merge_agg(&mut self, result_ty: &Ty, is_ok: &str, ok_ty: &Ty, ok_scratch: &str, err_scratch: &str, label_prefix: &str) -> Result<String, CodegenError> {
        let result_llty = self.llvm_ty(result_ty)?;
        let dest = self.fresh_reg(&format!("{label_prefix}_result_addr"));
        self.emit_alloca(&dest, &result_llty);
        let tag_ptr = self.fresh_reg(&format!("{label_prefix}_tag_ptr"));
        writeln!(self.out, "  {tag_ptr} = getelementptr inbounds {result_llty}, ptr {dest}, i32 0, i32 0").unwrap();
        let payload_ptr = self.fresh_reg(&format!("{label_prefix}_payload_ptr"));
        writeln!(self.out, "  {payload_ptr} = getelementptr inbounds {result_llty}, ptr {dest}, i32 0, i32 1").unwrap();

        let ok_label = self.fresh_label(&format!("{label_prefix}_ok"));
        let err_label = self.fresh_label(&format!("{label_prefix}_err"));
        let merge_label = self.fresh_label(&format!("{label_prefix}_merge"));
        writeln!(self.out, "  br i1 {is_ok}, label %{ok_label}, label %{err_label}").unwrap();

        writeln!(self.out, "{ok_label}:").unwrap();
        writeln!(self.out, "  store i64 0, ptr {tag_ptr}").unwrap();
        let bytes = agg_byte_size_operand(ok_ty, &self.registry);
        writeln!(self.out, "  call void @llvm.memcpy.p0.p0.i64(ptr {payload_ptr}, ptr {ok_scratch}, i64 {bytes}, i1 false)").unwrap();
        writeln!(self.out, "  br label %{merge_label}").unwrap();

        writeln!(self.out, "{err_label}:").unwrap();
        writeln!(self.out, "  store i64 1, ptr {tag_ptr}").unwrap();
        let err_val = self.fresh_reg(&format!("{label_prefix}_err_val"));
        writeln!(self.out, "  {err_val} = load {{ptr, i64}}, ptr {err_scratch}").unwrap();
        writeln!(self.out, "  store {{ptr, i64}} {err_val}, ptr {payload_ptr}").unwrap();
        writeln!(self.out, "  br label %{merge_label}").unwrap();

        writeln!(self.out, "{merge_label}:").unwrap();
        Ok(dest)
    }

    /// Shared tag-then-payload/branch/merge shape for a `Result(_, str)`
    /// builtin whose success/failure was decided by one already-called
    /// linked kernel — every `db`/`json` builtin below uses this
    /// (`emit_check_role`/`emit_oidc_validate_token` predate this helper
    /// and aren't refactored onto it, but follow the identical shape).
    /// `is_ok`/`ok_val`/`err_val` are all already-computed SSA values by
    /// the time this is called — `ok_val` loaded from wherever the
    /// kernel wrote it (harmless to load unconditionally even on the
    /// failure path: an uninitialized-but-never-dereferenced `{ptr,i64}`
    /// bit pattern is not a memory access), `err_val` the same. Every
    /// caller here has exactly one payload value per variant (never more
    /// than one field), so — like `emit_check_role`'s own doc comment
    /// already established for its single-`str`-field case — storing
    /// directly at `payload_ptr` (word offset 0) is exactly equivalent
    /// to `construct_variant`'s general per-field GEP addressing.
    fn emit_result_merge(
        &mut self,
        result_ty: &Ty,
        is_ok: &str,
        ok_llty: &str,
        ok_val: &str,
        err_val: &str,
        label_prefix: &str,
    ) -> Result<String, CodegenError> {
        let result_llty = self.llvm_ty(result_ty)?;
        let dest = self.fresh_reg(&format!("{label_prefix}_result_addr"));
        self.emit_alloca(&dest, &result_llty);
        let tag_ptr = self.fresh_reg(&format!("{label_prefix}_tag_ptr"));
        writeln!(self.out, "  {tag_ptr} = getelementptr inbounds {result_llty}, ptr {dest}, i32 0, i32 0").unwrap();
        let payload_ptr = self.fresh_reg(&format!("{label_prefix}_payload_ptr"));
        writeln!(self.out, "  {payload_ptr} = getelementptr inbounds {result_llty}, ptr {dest}, i32 0, i32 1").unwrap();

        let ok_label = self.fresh_label(&format!("{label_prefix}_ok"));
        let err_label = self.fresh_label(&format!("{label_prefix}_err"));
        let merge_label = self.fresh_label(&format!("{label_prefix}_merge"));
        writeln!(self.out, "  br i1 {is_ok}, label %{ok_label}, label %{err_label}").unwrap();

        writeln!(self.out, "{ok_label}:").unwrap();
        writeln!(self.out, "  store i64 0, ptr {tag_ptr}").unwrap();
        writeln!(self.out, "  store {ok_llty} {ok_val}, ptr {payload_ptr}").unwrap();
        writeln!(self.out, "  br label %{merge_label}").unwrap();

        writeln!(self.out, "{err_label}:").unwrap();
        writeln!(self.out, "  store i64 1, ptr {tag_ptr}").unwrap();
        writeln!(self.out, "  store {{ptr, i64}} {err_val}, ptr {payload_ptr}").unwrap();
        writeln!(self.out, "  br label %{merge_label}").unwrap();

        writeln!(self.out, "{merge_label}:").unwrap();
        Ok(dest)
    }

    fn icmp(&mut self, cond: &str, llty: &str, l: &str, r: &str) -> Result<String, CodegenError> {
        let out = self.fresh_reg("cmp");
        writeln!(self.out, "  {out} = icmp {cond} {llty} {l}, {r}").unwrap();
        Ok(out)
    }

    /// `o{cond}` (ordered) comparisons throughout — `==`/`!=`/`<`/`>`/
    /// `<=`/`>=` on `f64` all mean "and neither operand is NaN," LLVM's
    /// "ordered" family, not "unordered" (`u{cond}`, true if *either*
    /// side is NaN) — the same comparison semantics Rust's own `f64`
    /// `PartialOrd`/`PartialEq` already use, which `interpreter.rs`'s
    /// `eval_binary` inherits for free via native Rust `<`/`==` on `f64`
    /// (no separate NaN-handling decision was made there; this just has
    /// to agree with it).
    fn fcmp(&mut self, cond: &str, l: &str, r: &str) -> Result<String, CodegenError> {
        let out = self.fresh_reg("fcmp");
        writeln!(self.out, "  {out} = fcmp {cond} double {l}, {r}").unwrap();
        Ok(out)
    }

    /// `l * r` at the internal i64/double width -- shared by `agg_mul`'s
    /// three shapes (scale, mat-vec, mat-mat), all of which need the
    /// same scalar multiply repeated many times.
    fn emit_mul(&mut self, l: &str, r: &str, is_float: bool) -> String {
        let out = self.fresh_reg("agg_mul_elem");
        if is_float {
            writeln!(self.out, "  {out} = fmul double {l}, {r}").unwrap();
        } else {
            writeln!(self.out, "  {out} = mul i64 {l}, {r}").unwrap();
        }
        out
    }

    /// `l + r` at the internal i64/double width -- the accumulation half
    /// of `agg_mul`'s mat-vec/mat-mat dot-product chains.
    fn emit_add(&mut self, l: &str, r: &str, is_float: bool) -> String {
        let out = self.fresh_reg("agg_add_elem");
        if is_float {
            writeln!(self.out, "  {out} = fadd double {l}, {r}").unwrap();
        } else {
            writeln!(self.out, "  {out} = add i64 {l}, {r}").unwrap();
        }
        out
    }

    /// `l - r` at `double` width — the geometry/norm builtins' analog of
    /// `emit_add`/`emit_mul` (Phase 4 only ever subtracts `f64`s: no
    /// integer-typed builtin in this phase's scope needs it).
    fn emit_sub(&mut self, l: &str, r: &str) -> String {
        let out = self.fresh_reg("agg_sub_elem");
        writeln!(self.out, "  {out} = fsub double {l}, {r}").unwrap();
        out
    }

    /// LLVM's own hex bit-pattern float literal — the exact format
    /// `Expr::Float`'s own codegen already uses (module doc there), reused
    /// here for every closed-form constant Phase 4's geometry builtins
    /// need (`pi`, WGS84's `a`/`e2`, `360.0`, ...) that has no
    /// corresponding `Expr::Float` AST node to read it from.
    fn float_const(f: f64) -> String {
        format!("0x{:016X}", f.to_bits())
    }

    /// A one-`double`-argument call against a `declare`d LLVM intrinsic or
    /// libm function (`func` already includes its own `@` sigil, e.g.
    /// `"@llvm.sqrt.f64"`) — the geometry/norm builtins' shared workhorse
    /// for everything transcendental this backend doesn't have a plain
    /// instruction for.
    fn emit_call1(&mut self, func: &str, arg: &str) -> String {
        let r = self.fresh_reg("libm");
        writeln!(self.out, "  {r} = call double {func}(double {arg})").unwrap();
        r
    }

    /// The two-argument analog of `emit_call1` — `atan2`/`llvm.maxnum.f64`.
    fn emit_call2(&mut self, func: &str, a: &str, b: &str) -> String {
        let r = self.fresh_reg("libm");
        writeln!(self.out, "  {r} = call double {func}(double {a}, double {b})").unwrap();
        r
    }

    /// The real OS-level entry point — Nirdosha's own `main` was renamed
    /// to `@nir_main` (module doc) to avoid the clash. Exit code
    /// convention: `unit`-returning `main` exits 0; an integer-returning
    /// one truncates/extends its result to `i32`, the same "the returned
    /// value is the program's result" convention `main.rs`'s CLI already
    /// uses for the interpreter.
    fn emit_c_main(&mut self, program: &Program, native_plugins: &[crate::plugin::NativePluginBuiltin]) -> Result<(), CodegenError> {
        let main_fn = program.fns.iter().find(|f| f.name == "main").expect("typeck.rs already required a main");
        if main_fn.ret.is_aggregate() {
            // There's no sensible "exit code" for a raw Vector/Matrix
            // the way there is for an integer/f64 result (every other
            // branch below truncates/converts to `i32`) — and nothing
            // in this phase can print one either (`call()`'s `print`
            // arm rejects an aggregate argument). Fails cleanly here
            // rather than emitting a `call {ret_ty} @nir_main()` against
            // a callee whose real LLVM signature is actually `void`
            // (sret convention) — a genuine invalid-IR mismatch this
            // guard exists specifically to prevent.
            return unsupported(
                "codegen doesn't support `main` returning a Vector/Matrix/struct/enum directly yet — print \
                 its elements/fields instead of returning the aggregate itself",
            );
        }
        writeln!(self.out, "define i32 @main() {{").unwrap();
        writeln!(self.out, "entry:").unwrap();
        // RFC 0011 §3's open domain registry bootstrap — strictly first,
        // before durability/transact replay and `nfr` registration below,
        // so every built-in domain accessor (`domain::db()`, etc.) any of
        // that setup might reach is already resolved to a stable id.
        writeln!(self.out, "  call void @nir_kernel_register_builtin_domains()").unwrap();
        // RFC 0011 §4's per-provider registration: one `nir_kernel_
        // register_domain(...)` call per distinct validated plugin
        // provider in `native_plugins`, in the order given, immediately
        // after the built-in bootstrap call above and still strictly
        // before any user code runs. A provider's identity is
        // self-describing (its own normalized scheme), so — unlike the
        // built-ins' fixed 0-6 order — there's no positional-order
        // drift risk here to worry about; codegen just walks the list
        // it was given.
        let native_plugin_names: std::collections::HashSet<&str> = native_plugins.iter().map(|p| p.name.as_str()).collect();
        for np in native_plugins {
            let Some((shape, scheme)) = crate::plugin::provider_shape_and_scheme(&np.name) else { continue };
            let domain_name = format!("{shape}_provider_{scheme}");
            let env_var = np.env_var.map(str::to_string).unwrap_or_else(|| format!("NIRDOSHA_KERNEL_MAX_{}_{}", shape.to_ascii_uppercase(), scheme.to_ascii_uppercase()));
            let default_max = np.default_max.unwrap_or(10_000);

            let name_global = self.fresh_global("plugin_domain_name");
            writeln!(self.string_globals, "{name_global} = private unnamed_addr constant [{} x i8] c\"{}\"", domain_name.len(), llvm_escape_bytes(domain_name.as_bytes())).unwrap();
            let env_var_global = self.fresh_global("plugin_domain_env_var");
            writeln!(self.string_globals, "{env_var_global} = private unnamed_addr constant [{} x i8] c\"{}\"", env_var.len(), llvm_escape_bytes(env_var.as_bytes())).unwrap();

            writeln!(
                self.out,
                "  call void @nir_kernel_register_domain(ptr {name_global}, i64 {}, ptr {env_var_global}, i64 {}, i64 {default_max})",
                domain_name.len(),
                env_var.len(),
            )
            .unwrap();

            // RFC 0011 §2/§4: the dispatch-table registration, right
            // after this same provider's domain registration above.
            // `validate_plugin_roster` (Phase 3) already proved exactly
            // one of `_op`/`_request` exists for this `_connect` — same
            // "shape isn't a separate declared field, inferred from
            // which sibling is present" rule, re-applied here rather
            // than threaded through as new state.
            let prefix = np.name.trim_end_matches("_connect");
            let op_name = format!("{prefix}_op");
            let request_name = format!("{prefix}_request");
            let is_valid_name = format!("{prefix}_is_valid");
            let close_name = format!("{prefix}_close");
            let (op_or_request_name, is_call_shape) = if native_plugin_names.contains(request_name.as_str()) {
                (request_name.as_str(), 1)
            } else {
                (op_name.as_str(), 0)
            };

            let scheme_global = self.fresh_global("plugin_scheme");
            writeln!(self.string_globals, "{scheme_global} = private unnamed_addr constant [{} x i8] c\"{}\"", scheme.len(), llvm_escape_bytes(scheme.as_bytes())).unwrap();

            writeln!(
                self.out,
                "  call void @nir_kernel_register_plugin_provider(ptr {name_global}, i64 {}, ptr {scheme_global}, i64 {}, ptr @{}, ptr @{op_or_request_name}, ptr @{is_valid_name}, ptr @{close_name}, i32 {is_call_shape})",
                domain_name.len(),
                scheme.len(),
                np.name,
            )
            .unwrap();
        }
        // RFC 0011 §5's reaper — one call, after every domain/plugin-
        // provider registration above (so a pool-backed registry that
        // self-registers with the reaper on first use, e.g.
        // `db.rs`/`http.rs`/`plugin_provider.rs`'s own registries, has
        // whatever it needs already resolved), still strictly before any
        // user code runs.
        writeln!(self.out, "  call void @nir_kernel_start_reaper()").unwrap();
        // Durability log init + crash replay — strictly before any user
        // code (including `nfr` registration, harmless either order, but
        // definitely before `nir_main`) runs, and strictly after every
        // `transact` site's replay trampoline is registered
        // (`nir_transact_register_replay_site`), so `nir_transact_replay_all`
        // never dispatches to a site that isn't registered yet. Entirely
        // absent from the emitted IR for a program with no `transact` at
        // all (`self.transact_sites` empty) — zero cost when unused, same
        // convention every other optional kernel subsystem in this file
        // already follows.
        if !self.transact_sites.is_empty() {
            let log_ok = self.fresh_reg("transact_log_init_ok");
            writeln!(self.out, "  {log_ok} = call i32 @nir_transact_log_init()").unwrap();
            let log_ok_b = self.fresh_reg("transact_log_init_ok_b");
            writeln!(self.out, "  {log_ok_b} = icmp ne i32 {log_ok}, 0").unwrap();
            let log_init_ok_label = self.fresh_label("transact_log_init_ok");
            let log_init_fail_label = self.fresh_label("transact_log_init_fail");
            writeln!(self.out, "  br i1 {log_ok_b}, label %{log_init_ok_label}, label %{log_init_fail_label}").unwrap();
            writeln!(self.out, "{log_init_fail_label}:").unwrap();
            // Fail fast and loud, matching `instance_lock`'s own stated
            // philosophy (its doc comment) and `guard_in_range`'s existing
            // trap convention elsewhere in this file — a program that
            // declares `transact` but can't durably log it must not run
            // silently without the guarantee it was written to rely on.
            // `nir_transact_log_init` itself already printed the real
            // reason (another instance holding the log, or a plain I/O
            // error) to stderr before returning `0`.
            writeln!(self.out, "  call void @abort()").unwrap();
            writeln!(self.out, "  unreachable").unwrap();
            writeln!(self.out, "{log_init_ok_label}:").unwrap();
            for (site_id, tramp_name) in self.transact_sites.clone() {
                writeln!(self.out, "  call void @nir_transact_register_replay_site(i64 {site_id}, ptr {tramp_name})").unwrap();
            }
            writeln!(self.out, "  call void @nir_transact_replay_all()").unwrap();
        }
        // `nfr(...)` registration — once per tracked function, before
        // `nir_main` (the `.nir` program's own `main`) ever runs, so
        // every `nir_nfr_call_begin`/`_end` inside it already has a real
        // id to look up (`@nfr_id.<name>`, declared alongside the
        // per-function global-storage loop in `emit_llvm_ir_impl`).
        for f in &program.fns {
            let Some(nfr) = &f.nfr else { continue };
            let name_bytes = f.name.as_bytes();
            let name_global = self.fresh_global("nfr_name");
            let escaped = llvm_escape_bytes(name_bytes);
            writeln!(self.string_globals, "{name_global} = private unnamed_addr constant [{} x i8] c\"{escaped}\"", name_bytes.len())
                .unwrap();
            let latency = nfr.latency_ms.unwrap_or(-1);
            let error_rate = llvm_f64_literal(nfr.error_rate_max.unwrap_or(-1.0));
            let throughput = nfr.throughput_min_per_sec.unwrap_or(-1);
            let concurrency = nfr.concurrency_max.unwrap_or(-1);
            let id_reg = self.fresh_reg("nfr_registered_id");
            writeln!(
                self.out,
                "  {id_reg} = call i64 @nir_nfr_register(ptr {name_global}, i64 {}, i64 {latency}, double {error_rate}, i64 {throughput}, i64 {concurrency})",
                name_bytes.len()
            )
            .unwrap();
            writeln!(self.out, "  store i64 {id_reg}, ptr @nfr_id.{}", f.name).unwrap();
        }
        if main_fn.ret == Ty::Unit {
            writeln!(self.out, "  call void @nir_main()").unwrap();
            writeln!(self.out, "  call void @nir_kernel_flight_recorder_dump()").unwrap();
            writeln!(self.out, "  ret i32 0").unwrap();
        } else if main_fn.ret == Ty::Str {
            // Same "no sensible exit code" reasoning as the aggregate
            // case above — `str` is a `{ptr, i64}` struct value, not
            // something `sext`/`trunc`/`fptosi` (the only conversions
            // the generic fallback below knows) can turn into an `i32`.
            // Resolved the same way `Ty::Unit` above is: there's no exit
            // code to compute either way, so print the value (exactly
            // `Codegen::call`'s own `Ty::Str` print-arm sequence — same
            // `%.*s` format, same explicit length rather than a NUL scan,
            // `Ty::Str`'s own note in `llvm_ty` on why) and exit 0. This
            // is precisely the workaround the old rejection message told
            // callers to do by hand (`print(...)` then return `unit`);
            // automating it here removes the need for that workaround.
            let r = self.fresh_reg("main_str_result");
            writeln!(self.out, "  {r} = call {{ptr, i64}} @nir_main()").unwrap();
            let ptr_reg = self.fresh_reg("main_str_ptr");
            writeln!(self.out, "  {ptr_reg} = extractvalue {{ptr, i64}} {r}, 0").unwrap();
            let len_reg = self.fresh_reg("main_str_len");
            writeln!(self.out, "  {len_reg} = extractvalue {{ptr, i64}} {r}, 1").unwrap();
            let len_i32 = self.fresh_reg("main_str_len_i32");
            writeln!(self.out, "  {len_i32} = trunc i64 {len_reg} to i32").unwrap();
            writeln!(self.out, "  call i32 (ptr, ...) @printf(ptr @.str_fmt, i32 {len_i32}, ptr {ptr_reg})").unwrap();
            writeln!(self.out, "  call void @nir_kernel_flight_recorder_dump()").unwrap();
            writeln!(self.out, "  ret i32 0").unwrap();
        } else {
            let llty = self.llvm_ty(&main_fn.ret)?;
            let r = self.fresh_reg("main_result");
            writeln!(self.out, "  {r} = call {llty} @nir_main()").unwrap();
            let r32 = match llty.as_str() {
                "i64" => {
                    let t = self.fresh_reg("exit_code");
                    writeln!(self.out, "  {t} = trunc i64 {r} to i32").unwrap();
                    t
                }
                "i32" => r,
                // `sext`/`trunc` are integer-only instructions -- `f64`
                // needs its own conversion (`fptosi`, truncating toward
                // zero, the same as Rust's `as i32` would). A real,
                // previously-latent bug (this arm used to be the
                // catch-all `_ => sext`, which is simply invalid LLVM IR
                // for a `double`) -- found and fixed while adding `f64`
                // support, the same way this file's other bugs were:
                // by actually compiling a program that hit it.
                "double" => {
                    let t = self.fresh_reg("exit_code");
                    writeln!(self.out, "  {t} = fptosi double {r} to i32").unwrap();
                    t
                }
                // `i8`/`i16`-width results (`i8`/`i16`, and their
                // unsigned counterparts `u8`/`u16`, which map to the
                // exact same LLVM widths — `llty` alone can't tell them
                // apart, hence checking `main_fn.ret.is_unsigned()`
                // directly) need widening up to `i32`; `zext` for
                // unsigned, `sext` for signed, same distinction
                // `widen_to_i64` makes and for the same reason.
                _ => {
                    let t = self.fresh_reg("exit_code");
                    let op = if main_fn.ret.is_unsigned() { "zext" } else { "sext" };
                    writeln!(self.out, "  {t} = {op} {llty} {r} to i32").unwrap();
                    t
                }
            };
            writeln!(self.out, "  call void @nir_kernel_flight_recorder_dump()").unwrap();
            writeln!(self.out, "  ret i32 {r32}").unwrap();
        }
        writeln!(self.out, "}}").unwrap();
        Ok(())
    }

    /// The compiled `--serve` binary's real entry point — reviving
    /// compiled `nirdosha serve` (`rfcs/0010-landing-and-serve-exposure.md`,
    /// Stage 3/4 of that revival). Runs the *same* domain/reaper/
    /// transact-replay/`nfr` bootstrap `emit_c_main` itself runs, but
    /// instead of ever calling the program's own `nir_main()`, builds
    /// the real `&[CRoute]` dispatch table (`route_entries`'s own
    /// `(http_path, wrapper_symbol)` pairs — the caller resolves the
    /// exposure set and calls `emit_serve_route_wrapper` per entry
    /// *before* this runs, so this function only assembles what
    /// already exists) and hands it, plus the compile-time-baked UI
    /// HTML (`ui_gen::generate`'s output — a compiled binary has no
    /// `Program` AST left at runtime to generate it from, unlike the
    /// deleted interpreted `serve.rs`), to
    /// `compiled_serve::nir_compiled_serve_run` — this crate's own
    /// C-ABI bridge into the real HTTP engine (`crates/compiled-serve`).
    /// Never returns under normal operation.
    ///
    /// **Native plugins are not supported in `--serve` mode yet** — a
    /// real, disclosed narrowing, not an oversight: `emit_c_main`'s own
    /// per-provider domain/dispatch-table registration is simply
    /// omitted here, since `build_serve` (`codegen.rs`'s own public
    /// entry point for this mode) has no native-plugin roster to give
    /// it in the first place.
    fn emit_c_main_serve(&mut self, program: &Program, route_entries: &[(String, String)], ui_html: &[u8], port: u16, require_sender_constrained_tokens: bool) -> Result<(), CodegenError> {
        writeln!(self.out, "define i32 @main() {{").unwrap();
        writeln!(self.out, "entry:").unwrap();

        writeln!(self.out, "  call void @nir_kernel_register_builtin_domains()").unwrap();
        writeln!(self.out, "  call void @nir_kernel_start_reaper()").unwrap();

        // Durability log init + crash replay — identical ordering and
        // reasoning to `emit_c_main`'s own copy of this block (see its
        // comment): strictly before any route can possibly run, strictly
        // after every `transact` site's replay trampoline is registered.
        if !self.transact_sites.is_empty() {
            let log_ok = self.fresh_reg("transact_log_init_ok");
            writeln!(self.out, "  {log_ok} = call i32 @nir_transact_log_init()").unwrap();
            let log_ok_b = self.fresh_reg("transact_log_init_ok_b");
            writeln!(self.out, "  {log_ok_b} = icmp ne i32 {log_ok}, 0").unwrap();
            let log_init_ok_label = self.fresh_label("transact_log_init_ok");
            let log_init_fail_label = self.fresh_label("transact_log_init_fail");
            writeln!(self.out, "  br i1 {log_ok_b}, label %{log_init_ok_label}, label %{log_init_fail_label}").unwrap();
            writeln!(self.out, "{log_init_fail_label}:").unwrap();
            writeln!(self.out, "  call void @abort()").unwrap();
            writeln!(self.out, "  unreachable").unwrap();
            writeln!(self.out, "{log_init_ok_label}:").unwrap();
            for (site_id, tramp_name) in self.transact_sites.clone() {
                writeln!(self.out, "  call void @nir_transact_register_replay_site(i64 {site_id}, ptr {tramp_name})").unwrap();
            }
            writeln!(self.out, "  call void @nir_transact_replay_all()").unwrap();
        }

        // `nfr(...)` registration — before the listener ever starts
        // accepting, same as `emit_c_main`'s copy, so every exposed
        // route's own `nir_nfr_call_begin`/`_end` (inside its real
        // compiled body) already has a real id to look up.
        for f in &program.fns {
            let Some(nfr) = &f.nfr else { continue };
            let name_bytes = f.name.as_bytes();
            let name_global = self.fresh_global("nfr_name");
            let escaped = llvm_escape_bytes(name_bytes);
            writeln!(self.string_globals, "{name_global} = private unnamed_addr constant [{} x i8] c\"{escaped}\"", name_bytes.len()).unwrap();
            let latency = nfr.latency_ms.unwrap_or(-1);
            let error_rate = llvm_f64_literal(nfr.error_rate_max.unwrap_or(-1.0));
            let throughput = nfr.throughput_min_per_sec.unwrap_or(-1);
            let concurrency = nfr.concurrency_max.unwrap_or(-1);
            let id_reg = self.fresh_reg("nfr_registered_id");
            writeln!(
                self.out,
                "  {id_reg} = call i64 @nir_nfr_register(ptr {name_global}, i64 {}, i64 {latency}, double {error_rate}, i64 {throughput}, i64 {concurrency})",
                name_bytes.len()
            )
            .unwrap();
            writeln!(self.out, "  store i64 {id_reg}, ptr @nfr_id.{}", f.name).unwrap();
        }

        // ---- the dispatch table: one `CRoute { path_ptr, path_len,
        // handler }` per exposed fn (`compiled_serve::CRoute`'s own
        // exact `#[repr(C)]` field order/types) ----
        let mut route_elems = Vec::with_capacity(route_entries.len());
        for (path, wrapper_symbol) in route_entries {
            let path_global = self.fresh_global("serve_route_path");
            writeln!(self.string_globals, "{path_global} = private unnamed_addr constant [{} x i8] c\"{}\"", path.len(), llvm_escape_bytes(path.as_bytes())).unwrap();
            route_elems.push(format!("{{ ptr, i64, ptr }} {{ ptr {path_global}, i64 {}, ptr @{wrapper_symbol} }}", path.len()));
        }
        let routes_global = self.fresh_global("serve_routes");
        writeln!(
            self.string_globals,
            "{routes_global} = private unnamed_addr constant [{} x {{ ptr, i64, ptr }}] [{}]",
            route_entries.len(),
            route_elems.join(", ")
        )
        .unwrap();

        // The UI HTML — baked in as a plain byte-string global, empty
        // meaning "no UI" (`compiled_serve::ServeConfig::ui_html`'s own
        // "GET / 404s" contract for that case).
        let (ui_ptr_operand, ui_len) = if ui_html.is_empty() {
            ("null".to_string(), 0usize)
        } else {
            let ui_global = self.fresh_global("serve_ui_html");
            writeln!(self.string_globals, "{ui_global} = private unnamed_addr constant [{} x i8] c\"{}\"", ui_html.len(), llvm_escape_bytes(ui_html)).unwrap();
            (ui_global, ui_html.len())
        };

        let code = self.fresh_reg("serve_run_code");
        let require_dpop = if require_sender_constrained_tokens { 1 } else { 0 };
        writeln!(
            self.out,
            "  {code} = call i32 @nir_compiled_serve_run(ptr {routes_global}, i64 {}, ptr {ui_ptr_operand}, i64 {ui_len}, i64 {port}, i64 {require_dpop})",
            route_entries.len()
        )
        .unwrap();
        writeln!(self.out, "  ret i32 {code}").unwrap();
        writeln!(self.out, "}}").unwrap();
        Ok(())
    }
}

/// Full pipeline from a well-typed, ownership-checked `Program` to a real
/// native executable at `output_path`: emit LLVM IR, write it to a temp
/// `.ll` file, invoke the system `clang` to assemble and link it. Returns
/// `clang`'s stderr on failure — a `CodegenError`-shaped failure (an
/// unsupported construct) is reported before `clang` ever runs, so the
/// two failure modes stay distinguishable to a caller.
/// The IR itself is unoptimized either way (module doc: "correctness over
/// cleverness," alloca everywhere) — `OptLevel` controls only whether
/// `clang` is asked to optimize *after* that, the same as it would for C
/// source. `O2` is the default `build()` uses because docs/goal.md row 5 is
/// about hardware speed, not about this backend's own IR being clever;
/// `O0` stays available for debugging a miscompile without an optimizer
/// in the way, and — not incidentally — running the exact same IR
/// through both levels is a real stress test: LLVM treats `unreachable`
/// (this backend emits it for provably-dead code, e.g. a definitely-
/// returning function's fallthrough) as a hard guarantee and optimizes
/// aggressively around it, so a subtly wrong `unreachable` that `-O0`
/// happens not to disturb is exactly the kind of bug `-O2` would expose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptLevel {
    O0,
    O2,
}

impl OptLevel {
    fn clang_flag(self) -> &'static str {
        match self {
            OptLevel::O0 => "-O0",
            OptLevel::O2 => "-O2",
        }
    }
}

/// The `det`/`inv`/`solve`/`rank`/`kf_update_state`/`kf_update_cov`/
/// `tcp`/`file`/`dec128` kernels, built once at `nirdosha`'s own build
/// time by `build.rs` (`cargo rustc` against `../runtime-kernels`, its
/// own real Cargo package — see that crate's and `build.rs`'s own doc
/// comments) and embedded here — `build()` writes this out alongside
/// the generated `.ll` file and links it in, so every native binary
/// `nirdosha build` produces carries its own copy, with no runtime
/// dependency on this compiler's installation.
static RUNTIME_KERNELS_LIB: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/libnirdosha_runtime.a"));

/// `crates/compiled-serve`, built once at `nirdosha`'s own build time
/// by `build.rs::build_compiled_serve` — the same "embed a staticlib,
/// link it only when the feature it backs is actually used" shape
/// `RUNTIME_KERNELS_LIB` already established, just conditional
/// (`build_impl` only writes/links this when `serve.is_some()`) rather
/// than unconditional, since most `nirdosha build` invocations never
/// use `--serve` at all.
static COMPILED_SERVE_LIB: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/libnirdosha_compiled_serve.a"));

/// The OS-level system libraries `RUNTIME_KERNELS_LIB`'s own code (now
/// including the `nir_tcp_*` kernels' `std::net` calls) needs at final
/// link time — captured by `build.rs` via `rustc --print=native-static-
/// libs` at the same moment it builds that staticlib, since `rustc`
/// itself doesn't drive this crate's final link (see `build.rs`'s doc
/// comment for the real Windows failure this fixes: `ws2_32.lib` wasn't
/// being linked, so `nir_tcp_connect`/etc. were unresolved externals).
/// Whitespace-separated, already in whatever form the platform's own
/// linker expects (`-lfoo` on Unix, `foo.lib` on Windows-MSVC) — passed
/// through to `clang` as separate arguments unchanged, not parsed
/// further.
#[allow(dead_code)] // only read under `#[cfg(windows)]` below; Unix has its own `-lm` arm
static NATIVE_STATIC_LIBS: &str = include_str!(concat!(env!("OUT_DIR"), "/native_static_libs.txt"));

// `(name.lib, bytes)` for every `NATIVE_STATIC_LIBS` token `build.rs`
// found as a real file under its own private build (a crate-private
// import lib like `windows.0.52.0.lib`, not a genuine system-provided
// one) — see `build.rs`'s doc comment on `extra_native_libs.rs` for why
// these need to be linked by embedded-and-rewritten path instead of a
// bare `-lname` the way `kernel32.lib`/`advapi32.lib`/etc. are below.
// Defines `EXTRA_NATIVE_LIBS: &[(&str, &[u8])]`, `#[allow(dead_code)]`d
// from inside the generated snippet itself (only read under
// `#[cfg(windows)]` below).
include!(concat!(env!("OUT_DIR"), "/extra_native_libs.rs"));

pub fn build(
    program: &Program,
    smt_report: &SmtReport,
    output_path: &std::path::Path,
    opt: OptLevel,
) -> Result<(), String> {
    build_impl(program, smt_report, output_path, opt, &[], &HashSet::new(), None)
}

/// The compiled-path counterpart to `build()`, for a project entrypoint
/// that has a native-callable plugin roster ready to link
/// (rfcs/0005-plugin-boundary-safety-and-performance.md §3) — see
/// `emit_llvm_ir_with_native_plugins`'s doc comment for the same
/// "project's own entrypoint, not the bare CLI" scoping.
pub fn build_with_native_plugins(
    program: &Program,
    smt_report: &SmtReport,
    output_path: &std::path::Path,
    opt: OptLevel,
    native_plugins: &[crate::plugin::NativePluginBuiltin],
    reject_plugin_names: &HashSet<String>,
) -> Result<(), String> {
    build_impl(program, smt_report, output_path, opt, native_plugins, reject_plugin_names, None)
}

/// `nirdosha build --serve` (reviving compiled `nirdosha serve`,
/// `rfcs/0010-landing-and-serve-exposure.md`) — same pipeline as
/// `build()`, except the generated binary's real `main` dispatches
/// HTTP requests to the program's exposure set instead of ever calling
/// its own `nir_main()` (`emit_llvm_ir_for_serve`/`emit_c_main_serve`),
/// and the final link additionally embeds `nirdosha-compiled-serve`'s
/// own staticlib (`build.rs::build_compiled_serve`). Native plugins
/// aren't supported together with `--serve` yet — always `&[]`/empty,
/// unlike `build_with_native_plugins`.
pub fn build_serve(program: &Program, smt_report: &SmtReport, output_path: &std::path::Path, opt: OptLevel, serve: &ServeCodegenOptions) -> Result<(), String> {
    build_impl(program, smt_report, output_path, opt, &[], &HashSet::new(), Some(serve))
}

fn build_impl(
    program: &Program,
    smt_report: &SmtReport,
    output_path: &std::path::Path,
    opt: OptLevel,
    native_plugins: &[crate::plugin::NativePluginBuiltin],
    reject_plugin_names: &HashSet<String>,
    serve: Option<&ServeCodegenOptions>,
) -> Result<(), String> {
    let ir = match serve {
        Some(opts) => emit_llvm_ir_for_serve(program, smt_report, opts).map_err(|e| e.to_string())?,
        None => emit_llvm_ir_with_native_plugins(program, smt_report, native_plugins, reject_plugin_names).map_err(|e| e.to_string())?,
    };

    // `process::id()` alone is **not** unique enough: it's identical
    // across every thread inside one process, so two concurrent `build`
    // calls in the same process (e.g. two tests running in parallel,
    // which is `cargo test`'s default) would race on the same temp file
    // — one call's IR silently overwriting or getting deleted out from
    // under the other. Found exactly this way: `cargo test`'s default
    // parallelism turned three independently-correct compiles into three
    // empty-stdout failures, not a hypothetical worry. A process-wide
    // atomic counter, combined with the pid, makes each call's filename
    // genuinely unique regardless of how many `build`s run concurrently.
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut ll_path = std::env::temp_dir();
    ll_path.push(format!("nirdosha_{}_{n}.ll", std::process::id()));
    std::fs::write(&ll_path, &ir).map_err(|e| format!("writing {}: {e}", ll_path.display()))?;

    let mut runtime_lib_path = std::env::temp_dir();
    runtime_lib_path.push(format!("nirdosha_runtime_{}_{n}.a", std::process::id()));
    std::fs::write(&runtime_lib_path, RUNTIME_KERNELS_LIB)
        .map_err(|e| format!("writing {}: {e}", runtime_lib_path.display()))?;

    // Each native plugin's own precompiled staticlib, written to a
    // uniquely-named temp file (same collision-avoidance reasoning as
    // `runtime_lib_path` above) and linked alongside `RUNTIME_KERNELS_LIB`
    // — the exact same mechanism, generalized to a third-party-supplied
    // library instead of only this compiler's own.
    let mut native_plugin_lib_paths = Vec::with_capacity(native_plugins.len());
    for (i, np) in native_plugins.iter().enumerate() {
        let mut p = std::env::temp_dir();
        p.push(format!("nirdosha_plugin_{}_{n}_{i}.a", std::process::id()));
        std::fs::write(&p, np.static_lib).map_err(|e| format!("writing {}: {e}", p.display()))?;
        native_plugin_lib_paths.push(p);
    }

    // Only written/linked for a `--serve` build — `runtime_lib_path`
    // first, so the (rare, harmless — see `COMPILED_SERVE_LIB`'s own
    // doc comment) symbol overlap between the two staticlibs
    // (`compiled-serve` bundles its own copy of `runtime-kernels`
    // transitively) resolves from the plain kernels archive first,
    // same "first satisfied wins" archive-linking behavior every other
    // duplicate-capable link in this function already relies on.
    let compiled_serve_lib_path = if serve.is_some() {
        let mut p = std::env::temp_dir();
        p.push(format!("nirdosha_compiled_serve_{}_{n}.a", std::process::id()));
        std::fs::write(&p, COMPILED_SERVE_LIB).map_err(|e| format!("writing {}: {e}", p.display()))?;
        Some(p)
    } else {
        None
    };

    let mut clang_cmd = std::process::Command::new("clang");
    clang_cmd.arg(&ll_path).arg(&runtime_lib_path).arg(opt.clang_flag());
    if let Some(p) = &compiled_serve_lib_path {
        clang_cmd.arg(p);
    }
    for p in &native_plugin_lib_paths {
        clang_cmd.arg(p);
    }
    // Phase 4's `declare double @atan2(double, double)` (geometry
    // builtins) has no LLVM intrinsic form, unlike `sqrt`/`sin`/`cos` —
    // it's the plain libm function, so it needs to actually be linked.
    // Harmless to pass unconditionally on Unix even for a program that
    // never calls it. Windows has no separate `libm` to link against —
    // math functions live in the C runtime clang already links by
    // default — and passing `-lm` there makes the MSVC linker fail
    // outright looking for a nonexistent `m.lib`, so this flag is
    // Unix-only.
    #[cfg(unix)]
    clang_cmd.arg("-lm");
    // Found by a real macOS CI failure, not anticipated in advance:
    // `native-tls`'s macOS backend (`security-framework`, pulled in
    // for `nir_https_get`/`nir_https_post` — `Ty::Db`'s `native-tls`
    // dependency comment) links against `Security.framework`/
    // `CoreFoundation.framework` (`AuthorizationCreate`/`CFArrayCreate`/
    // etc.) — real macOS system frameworks clang does **not** auto-link
    // when the input is a bare `.ll`/staticlib pair rather than actual
    // Objective-C/C source (unlike compiling a `.m` file, where the
    // default SDK sysroot linking pulls these in implicitly). Every
    // other affine-handle kernel in `RUNTIME_KERNELS_LIB` links fine
    // without them, so this stayed invisible until a real compiled
    // binary using the TLS path was actually linked on a real macOS
    // runner — the same "found by testing, not review" discipline this
    // file's own `-lm`/`NATIVE_STATIC_LIBS` comments already document
    // for their own platforms. Harmless to pass unconditionally even
    // for a program that never calls `https_get`/`https_post` (same
    // reasoning as `-lm` above) — the linker only pulls in what's
    // actually referenced.
    //
    // `SystemConfiguration.framework` is the same story, found the same
    // way, one real macOS CI failure later: `tokio-postgres` (`Ty::Db`'s
    // Postgres backend, `nir_db_connect` et al.) depends on `whoami` for
    // its default-username resolution, and `whoami`'s macOS backend calls
    // `SCDynamicStoreCopyComputerName` — a `SystemConfiguration.framework`
    // symbol, not `Security`/`CoreFoundation`. rustc's release build
    // merges `runtime-kernels` and its dependency graph into very few
    // codegen units, so this reference rides along in the same object
    // file as ordinary, always-linked runtime kernels (it surfaced on
    // trivial programs with no `db` usage at all, not just ones that
    // touch Postgres) — same "the linker only pulls in what's actually
    // referenced [into that object file]" mechanics as the other two
    // frameworks above, just a different object file.
    #[cfg(target_os = "macos")]
    clang_cmd
        .arg("-framework")
        .arg("Security")
        .arg("-framework")
        .arg("CoreFoundation")
        .arg("-framework")
        .arg("SystemConfiguration");
    // Windows has no equivalent hand-picked single flag — `std::net`
    // (the `nir_tcp_*` kernels) needs `ws2_32.lib`, and other stdlib
    // pieces need their own system libs beside it, so the captured,
    // rustc-verified list is used instead of guessing which ones.
    // `NATIVE_STATIC_LIBS`'s doc comment on its declaration above has the
    // real failure this fixes.
    //
    // Can't pass rustc's tokens straight through as positional args:
    // found on real Windows CI, a real second failure past the first —
    // `clang: error: no such file or directory: 'kernel32.lib'`. Clang
    // preflight-checks any *positional* (non-flag) argument as a literal
    // path relative to the current directory, even though a plain
    // `foo.lib` token is exactly what MSVC's own linker resolves via its
    // library search path, never by looking in the cwd. `-lfoo` (Clang's
    // ordinary, cross-target library flag) skips that preflight check
    // entirely and does reach the linker's search path — so each
    // `foo.lib` token here is stripped to `foo` and passed as `-lfoo`
    // instead. A non-`.lib` token (e.g. `/defaultlib:...`, on a
    // dynamic-CRT build) is forwarded verbatim via `-Xlinker` instead,
    // the same reason `-l` works for the others.
    //
    // `build.rs`'s own `+crt-static` flag (its doc comment on the
    // `RUSTFLAGS` it sets has the full story: `bundled` SQLite's C code
    // and this compiler's own generated `declare`s for libc functions
    // structurally expect different CRT linkage models otherwise) means
    // this list shouldn't even contain a `msvcrt`-style dynamic-CRT
    // `/defaultlib:` token on a correctly-configured Windows build
    // anymore — three real, wrong `-Xlinker`/`NODEFAULTLIB` guesses were
    // tried and disproven in this exact spot before finding that real
    // root cause, each fixing one symbol set by excluding a library the
    // *other* half of the link needed. Nothing platform-specific is
    // hand-picked here anymore; every token is just forwarded as
    // `rustc` itself reports it.
    // A handful of tokens above aren't genuine system-provided libs at
    // all — a crate-private import lib like `windows.0.52.0.lib` (the
    // `windows`/`windows-sys` family, at least) ships inside that crate's
    // own build output, nowhere on the linker's default search path.
    // `build.rs` already found and embedded any such file (see
    // `extra_native_libs.rs`'s doc comment); write each one back out to a
    // real temp path and link it *by path* (`runtime_lib_path`'s own
    // pattern) instead of `-lname` — found on real Windows CI as `LNK1181:
    // cannot open input file 'windows.0.52.0.lib'`, the exact "no such
    // file" failure a bare `-l` produces when the named file isn't on any
    // search path clang/the linker already knows about.
    #[cfg(windows)]
    let mut extra_lib_paths = Vec::new();
    #[cfg(windows)]
    for token in NATIVE_STATIC_LIBS.split_whitespace() {
        match token.strip_suffix(".lib") {
            Some(name) => {
                if let Some((_, bytes)) = EXTRA_NATIVE_LIBS.iter().find(|(n, _)| *n == token) {
                    let mut p = std::env::temp_dir();
                    p.push(format!("nirdosha_extralib_{}_{n}_{token}", std::process::id()));
                    std::fs::write(&p, bytes)
                        .map_err(|e| format!("writing {}: {e}", p.display()))?;
                    clang_cmd.arg(&p);
                    extra_lib_paths.push(p);
                } else {
                    clang_cmd.arg(format!("-l{name}"));
                }
            }
            None => {
                clang_cmd.arg("-Xlinker").arg(token);
            }
        }
    }
    // No `/NODEFAULTLIB:<X>` here, after three real wrong turns in this
    // exact spot each fixing one missing-symbol set by excluding a
    // library the *other* half of the link needed (`msvcrt` dropped
    // outright, `/NODEFAULTLIB:MSVCRT`, `/NODEFAULTLIB:LIBCMT` — this
    // last one traded `bundled` SQLite's unresolved externals for a
    // freshly-unresolved `printf` from `codegen.rs`'s own generated
    // `nir_main`). The actual root cause: `codegen.rs`'s plain
    // (non-`dllimport`) `declare`s for libc functions structurally
    // expect the *static* CRT, while `cc`-crate-compiled C code
    // (`bundled` SQLite) defaults to the *dynamic* one — no
    // `-Xlinker` flag on this side of the link can reconcile two
    // objects built expecting different CRT models. `build.rs` fixes it
    // at the source instead: `-C target-feature=+crt-static` on the
    // nested `cargo rustc` build makes both `rustc`'s own generated code
    // and (via the `cc` crate's own `CARGO_CFG_TARGET_FEATURE` check,
    // switching `cl.exe` from `/MD` to `/MT`) SQLite's compiled object
    // agree on the static CRT throughout — see that flag's own doc
    // comment for the full story.
    let result = clang_cmd.arg("-o").arg(output_path).output();
    let _ = std::fs::remove_file(&ll_path); // best-effort cleanup either way
    let _ = std::fs::remove_file(&runtime_lib_path);
    if let Some(p) = &compiled_serve_lib_path {
        let _ = std::fs::remove_file(p);
    }
    for p in &native_plugin_lib_paths {
        let _ = std::fs::remove_file(p);
    }
    #[cfg(windows)]
    for p in &extra_lib_paths {
        let _ = std::fs::remove_file(p);
    }

    match result {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => Err(format!(
            "clang failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )),
        Err(e) => Err(format!("could not run `clang`: {e} (is it installed and on PATH?)")),
    }
}

