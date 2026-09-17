//! # nirdosha-driver — Stage 2 of the Nirdosha compiler
//!
//! A rustc driver (the Clippy architecture) that runs the *full* rustc
//! pipeline and then verifies Nirdosha contract claims over **MIR**:
//!
//! - **Interprocedural purity.** `effects(pure)` is checked against the
//!   whole call graph of the local crate, transitively. Stage 1 could
//!   only see direct calls inside one body; Stage 2 follows every edge
//!   and names the full chain when a lie is found.
//! - **Real name resolution.** `std::fs::read_to_string` is recognized
//!   because it *is* `std::fs::read_to_string` (a resolved `DefId`),
//!   not because its text looks like a path. Stage 1's over-approximate
//!   string matching is retired for crates compiled through this
//!   driver.
//! - **Conservative local subset.** Expanded HIR and pre-optimization MIR
//!   reject unsafe/static boundaries, reference writes, destructors and
//!   unresolved calls. External calls require summaries; no crate is trusted
//!   wholesale. Scalar arithmetic and local recursion remain supported.
//! - **Numeric assertions.** Shared Z3 integer semantics prove overflow,
//!   nonzero divisors and array bounds over normal MIR paths. An explicit
//!   interval backend is available without Z3. Bound certificates record
//!   per-assertion evidence; proven guards are not removed from MIR yet.
//! - **No totality claim.** Arithmetic overflow guards and recursion are
//!   permitted; termination and panic freedom are not established here.
//!
//! Runs two ways:
//!
//! - as `RUSTC_WORKSPACE_WRAPPER` (set by `cargo nirdosha build --deep`),
//!   where non-dialect crates (no `nirdosha:contract` anywhere) are a
//!   verbatim pass-through — zero behavior change for the rest of the
//!   workspace;
//! - directly: `nirdosha-driver <rustc flags>` (development).
//!
//! Lying is reported as a real rustc error, spans included — the
//! compiler error *is* the punchline. Requires nightly with the
//! `rustc-dev` component (the driver links `rustc_private`).

#![feature(rustc_private)]

extern crate rustc_ast;
extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_index;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_mir_dataflow;
extern crate rustc_span;

use std::collections::{HashMap, HashSet};

mod dataflow;
mod numeric;
mod proof_certificate;
mod std_effects;
use std::process::ExitCode;

use nirdosha_contract_core as cc;

use rustc_driver::{Callbacks, Compilation};
use rustc_hir::intravisit::{self, Visitor};
use rustc_interface::interface::Compiler;
use rustc_middle::mir::{Body, Operand, ProjectionElem, StatementKind, TerminatorKind};
use rustc_middle::ty::{TyCtxt, TyKind};
use rustc_span::def_id::{DefId, LOCAL_CRATE, LocalDefId};

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().collect();
    // RUSTC_WORKSPACE_WRAPPER protocol: cargo invokes the wrapper as
    // `nirdosha-driver <path-to-rustc> <rustc args...>`. cargo-nirdosha
    // marks its delegation with NIRDOSHA_DRIVER; drop the rustc path
    // so run_compiler sees a plain rustc argv.
    if std::env::var_os("NIRDOSHA_DRIVER").is_some()
        && args.get(1).is_some_and(|a| !a.starts_with('-'))
    {
        if let Err(message) = check_toolchain_version(&args[1]) {
            eprintln!("nirdosha-driver: {message}");
            return ExitCode::FAILURE;
        }
        args.remove(1);
    }
    let mut callbacks = NirdoshaCallbacks { certificate: None };
    let status = rustc_driver::catch_with_exit_code(|| {
        rustc_driver::run_compiler(&args, &mut callbacks);
    });
    if status == ExitCode::SUCCESS {
        if let Some(pending) = callbacks.certificate {
            if let Err(error) = pending.write() {
                eprintln!("nirdosha: cannot write MIR certificate: {error}");
                return ExitCode::FAILURE;
            }
        }
    }
    status
}

/// Issue #65 item 9 / #72's fallback safety net: this binary links
/// `rustc_private`, so it is ABI-compatible with only the exact nightly
/// it was built against (`NIRDOSHA_RUSTC_VERSION`, embedded by
/// `build.rs`'s own `rustc --version` at build time — see that file).
/// Under `RUSTC_WORKSPACE_WRAPPER`, cargo hands this fn the *active*
/// toolchain's own resolved `rustc` path as `args[1]`; if that
/// toolchain doesn't match, letting `run_compiler` load against it
/// in-process is a symbol/ABI mismatch — undefined behavior, not a
/// clean error — so this checks first and fails closed with an
/// actionable message instead.
fn check_toolchain_version(rustc_path: &str) -> Result<(), String> {
    let built_against = env!("NIRDOSHA_RUSTC_VERSION");
    let output = std::process::Command::new(rustc_path)
        .arg("--version")
        .output()
        .map_err(|e| {
            format!("cannot run `{rustc_path} --version` to check the active toolchain: {e}")
        })?;
    let active = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if active != built_against {
        return Err(format!(
            "toolchain mismatch: this binary was built against `{built_against}`, but the \
             active toolchain (`{rustc_path}`) reports `{active}`. rustc_private has no \
             stable ABI across builds -- rebuild nirdosha-driver against this exact \
             toolchain before using --deep with it."
        ));
    }
    Ok(())
}

struct NirdoshaCallbacks {
    certificate: Option<proof_certificate::Pending>,
}

impl Callbacks for NirdoshaCallbacks {
    fn after_analysis<'tcx>(&mut self, _compiler: &Compiler, tcx: TyCtxt<'tcx>) -> Compilation {
        analyze(tcx, &mut self.certificate)
    }
}

// ---------------------------------------------------------------------------
// Claims: parse `nirdosha:contract` doc attributes off every MIR body.
// At HIR time the #[contract] attribute macro has already expanded into
// exactly this doc form — so both authoring surfaces converge here.
// ---------------------------------------------------------------------------

fn analyze(tcx: TyCtxt<'_>, certificate: &mut Option<proof_certificate::Pending>) -> Compilation {
    let mut claims: Vec<(LocalDefId, cc::model::Contract)> = Vec::new();
    let mut malformed: Vec<(LocalDefId, String)> = Vec::new();

    for did in tcx.mir_keys(()).iter() {
        let hir_id = tcx.local_def_id_to_hir_id(*did);
        for attr in tcx.hir_attrs(hir_id) {
            let Some(doc) = attr.doc_str() else { continue };
            match cc::docparse::parse_doc(&doc.as_str()) {
                Ok(Some(contract)) => {
                    claims.push((*did, contract));
                    break;
                }
                Ok(None) => {}
                Err(msg) => {
                    malformed.push((*did, msg));
                    break;
                }
            }
        }
    }

    // A crate with no Nirdosha contracts is a verbatim pass-through:
    // the driver changes nothing about how dependencies build.
    if claims.is_empty() && malformed.is_empty() {
        return Compilation::Continue;
    }

    let claim_by_did: HashMap<LocalDefId, &cc::model::Contract> =
        claims.iter().map(|(did, c)| (*did, c)).collect();
    let numeric: HashMap<_, _> = tcx
        .mir_keys(())
        .iter()
        .filter(|did| {
            matches!(
                tcx.def_kind(**did),
                rustc_hir::def::DefKind::Fn
                    | rustc_hir::def::DefKind::AssocFn
                    | rustc_hir::def::DefKind::Closure
            )
        })
        .map(|did| (*did, numeric::analyze(tcx, *did, claim_by_did.get(did).copied())))
        .collect();
    let mut errors = false;

    for (did, msg) in malformed {
        tcx.dcx()
            .struct_span_err(
                tcx.def_span(did.to_def_id()),
                format!("malformed nirdosha:contract — {msg}"),
            )
            .emit();
        errors = true;
    }

    for (did, contract) in &claims {
        for issue in contract.validate() {
            tcx.dcx()
                .struct_span_err(
                    tcx.def_span(did.to_def_id()),
                    format!("invalid nirdosha:contract — {issue}"),
                )
                .emit();
            errors = true;
        }
        let report = numeric.get(did);
        if let Some(err) = report.and_then(|r| r.contract_error.as_ref()) {
            tcx.dcx()
                .struct_span_err(
                    tcx.def_span(did.to_def_id()),
                    format!("invalid nirdosha:contract — {err}"),
                )
                .emit();
            errors = true;
        } else if contract.ensures.is_some() {
            let checks: Vec<_> = report
                .into_iter()
                .flat_map(|r| r.checks.values())
                .filter(|c| c.kind == "ensures")
                .collect();
            if checks.is_empty() || !checks.iter().all(|c| c.proven) {
                tcx.dcx()
                    .struct_span_err(
                        tcx.def_span(did.to_def_id()),
                        format!(
                            "fn `{}` claims ensures(..) but Z3 cannot prove it holds on every return path",
                            tcx.def_path_str(did.to_def_id())
                        ),
                    )
                    .emit();
                errors = true;
            }
        }
        if contract.resource.is_some() {
            match dataflow::analyze(tcx, *did, Some(contract)) {
                dataflow::Report::NotClaimed | dataflow::Report::Clean => {}
                dataflow::Report::Unsupported(reason) => {
                    tcx.dcx()
                        .struct_span_err(
                            tcx.def_span(did.to_def_id()),
                            format!(
                                "fn `{}` claims resource(..) but its acquire/release discipline cannot be checked: {reason}",
                                tcx.def_path_str(did.to_def_id())
                            ),
                        )
                        .emit();
                    errors = true;
                }
                dataflow::Report::Violations(violations) => {
                    let mut diag = tcx.dcx().struct_span_err(
                        tcx.def_span(did.to_def_id()),
                        format!(
                            "fn `{}` claims resource(..) but its acquire/release discipline is violated",
                            tcx.def_path_str(did.to_def_id())
                        ),
                    );
                    for v in &violations {
                        let reason = match v.kind {
                            "leaked" => "a resource is acquired but never released on this path",
                            "double_acquire" => "acquire() called while a resource from an earlier acquire() is still held",
                            "release_without_acquire" => "release() called on a value this fn never saw acquire() produce",
                            other => other,
                        };
                        diag.note(format!("{reason} — {}", v.location));
                    }
                    diag.emit();
                    errors = true;
                }
            }
        }
        if contract.claims_pure() {
            // Each root gets its own traversal. Caching an incomplete result
            // while walking a recursive component can hide effects from a
            // later root in the same component.
            let mut engine = Effects::new(tcx, &numeric);
            let impurities = engine.effects_of(*did);
            if !impurities.is_empty() {
                let span = tcx.def_span(did.to_def_id());
                let count = impurities.len();
                let mut diag = tcx.dcx().struct_span_err(
                    span,
                    format!(
                        "fn `{}` claims effects(pure) but the effect lattice says otherwise",
                        engine.name(did.to_def_id())
                    ),
                );
                for impurity in &impurities {
                    diag.note(format!(
                        "{} — chain: {}",
                        impurity.reason,
                        impurity.chain.join(" -> ")
                    ));
                }
                if count > 1 {
                    diag.note(format!("({count} impure operations total)"));
                }
                diag.emit();
                errors = true;
            }
        }
    }

    if errors {
        tcx.dcx().abort_if_errors();
    }
    match proof_certificate::prepare(tcx, &numeric) {
        Ok(pending) => *certificate = Some(pending),
        Err(error) => {
            tcx.dcx()
                .err(format!("cannot prepare MIR certificate: {error}"));
            tcx.dcx().abort_if_errors();
        }
    }
    Compilation::Continue
}

// ---------------------------------------------------------------------------
// Each pure root traverses its own reachable local call graph. No partial
// recursive result is cached across roots; every reachable body is inspected.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct Impurity {
    reason: String,
    /// Full call chain from the claiming fn to the impure leaf.
    chain: Vec<String>,
}

/// `def_path_str` prefixes that are impure with real name resolution.
/// Precise needles, not substring guesses: `std::time::Duration` is
/// pure, `std::time::Instant` is not.
const FOREIGN_NEEDLES: &[(&str, &str)] = &[
    ("std::fs::", "file system access"),
    ("std::net::", "network access"),
    ("std::process::", "process spawn"),
    ("std::thread::", "raw threads (not nirdosha-rt managed)"),
    ("std::env::", "environment access"),
    ("std::io::", "I/O"),
    ("std::time::Instant", "clock access"),
    ("std::time::SystemTime", "wall-clock access"),
    ("core::panicking::", "panicking operation"),
    ("std::panicking::", "panicking operation"),
    ("libc::", "libc FFI"),
    ("rand::", "OS randomness"),
];

struct Effects<'a, 'tcx> {
    numeric: &'a HashMap<LocalDefId, numeric::Report>,
    tcx: TyCtxt<'tcx>,
    visited: HashSet<LocalDefId>,
}

impl<'a, 'tcx> Effects<'a, 'tcx> {
    fn new(tcx: TyCtxt<'tcx>, numeric: &'a HashMap<LocalDefId, numeric::Report>) -> Self {
        Self {
            tcx,
            numeric,
            visited: HashSet::new(),
        }
    }

    /// Readable name: locals lose the crate prefix if one is printed
    /// (1.100's `def_path_str` omits it for the local crate), foreigns
    /// keep everything.
    fn name(&self, did: DefId) -> String {
        let path = self.tcx.def_path_str(did);
        if did.is_local() {
            let local_crate = self.tcx.crate_name(LOCAL_CRATE).to_string();
            match path.strip_prefix(&format!("{local_crate}::")) {
                Some(stripped) => stripped.to_string(),
                None => path,
            }
        } else {
            path
        }
    }

    fn effects_of(&mut self, did: LocalDefId) -> Vec<Impurity> {
        if !self.visited.insert(did) {
            // Every reachable body is inspected once per claiming root.
            return Vec::new();
        }

        let mut impurities = Vec::new();
        let my_name = self.name(did.to_def_id());
        if self.tcx.trait_of_assoc(did.to_def_id()).is_some() {
            return vec![Impurity {
                reason: "unresolved trait dispatch has no complete target set".into(),
                chain: vec![my_name],
            }];
        }
        if !self.tcx.is_mir_available(did.to_def_id()) {
            return vec![Impurity {
                reason: "unresolved local call target has no inspectable MIR".into(),
                chain: vec![my_name],
            }];
        }
        if self.tcx.is_coroutine(did.to_def_id()) {
            return vec![Impurity {
                reason: "coroutine effects are unsupported".into(),
                chain: vec![my_name],
            }];
        }
        // Inspect runtime MIR before inlining and dead-code optimization.
        // Otherwise an optimization can erase a forbidden boundary, or turn
        // a callback into apparently harmless arithmetic before we inspect it.
        let body = self
            .tcx
            .mir_drops_elaborated_and_const_checked(did)
            .borrow();
        let hir_id = self.tcx.local_def_id_to_hir_id(did);
        if let Some(sig) = self.tcx.hir_fn_sig_by_hir_id(hir_id) {
            if sig.header.is_unsafe() || sig.header.is_async() {
                impurities.push(Impurity {
                    reason: "unsafe or async function is outside the pure subset".into(),
                    chain: vec![my_name.clone()],
                });
            }
        }
        if let Some(hir_body) = self.tcx.hir_maybe_body_owned_by(did) {
            let mut restrictions = BodyRestrictions {
                tcx: self.tcx,
                reasons: Vec::new(),
            };
            restrictions.visit_body(hir_body);
            for reason in restrictions.reasons {
                impurities.push(Impurity {
                    reason: reason.into(),
                    chain: vec![my_name.clone()],
                });
            }
        }

        for (bb, block) in body.basic_blocks.iter_enumerated() {
            for statement in &block.statements {
                if let StatementKind::Assign(assignment) = &statement.kind {
                    if assignment
                        .0
                        .projection
                        .iter()
                        .any(|p| matches!(p, ProjectionElem::Deref))
                    {
                        impurities.push(Impurity {
                            reason:
                                "write through a reference or pointer is outside the pure subset"
                                    .into(),
                            chain: vec![my_name.clone()],
                        });
                    }
                }
            }
            let Some(terminator) = &block.terminator else {
                continue;
            };
            match &terminator.kind {
                TerminatorKind::Call { func, args, .. }
                | TerminatorKind::TailCall { func, args, .. } => {
                    self.visit_callee(func, args, &body, &my_name, &mut impurities);
                }
                TerminatorKind::Assert { msg, .. } => {
                    if self
                        .numeric
                        .get(&did)
                        .is_some_and(|report| report.proven(bb))
                    {
                        continue;
                    }
                    if let Some(reason) = assert_reason(msg) {
                        impurities.push(Impurity {
                            reason,
                            chain: vec![my_name.clone()],
                        });
                    }
                }
                TerminatorKind::InlineAsm { .. } => {
                    impurities.push(Impurity {
                        reason: "inline assembly — the dialect forbids it".into(),
                        chain: vec![my_name.clone()],
                    });
                }
                TerminatorKind::Drop { place, .. } => {
                    // Issue #78: MIR inserts a `Drop` terminator for any
                    // type needing drop glue, custom `impl Drop` or not
                    // (a `Vec`'s buffer, a plain struct's fields, ...).
                    // Only an *explicit* `impl Drop` can hide a real
                    // effect — `AdtDef::destructor` returns `None` for
                    // ordinary derived/recursive field drops, which are
                    // pure by construction (nothing a summary needs to
                    // vouch for).
                    let ty = place.ty(&body.local_decls, self.tcx).ty;
                    if let Some(adt) = ty.ty_adt_def() {
                        if let Some(destructor) = adt.destructor(self.tcx) {
                            if destructor.did.is_local() {
                                // A local type's `Drop::drop` gets the
                                // same real analysis a local function
                                // call already gets.
                                let callee = destructor.did.expect_local();
                                let mut sub = self.effects_of(callee);
                                for imp in &mut sub {
                                    imp.chain.insert(0, my_name.clone());
                                }
                                impurities.extend(sub);
                            } else {
                                let type_path = self.tcx.def_path_str(adt.did());
                                if !std_effects::classify_destructor(&type_path) {
                                    impurities.push(Impurity {
                                        reason: "destructor effects lack a verified summary"
                                            .into(),
                                        chain: vec![my_name.clone(), type_path],
                                    });
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // Dedup: a callee reached by two paths is one fact.
        let mut seen = HashSet::new();
        impurities.retain(|imp| seen.insert((imp.reason.clone(), imp.chain.clone())));
        impurities
    }

    fn visit_callee(
        &mut self,
        func: &Operand<'tcx>,
        args: &[rustc_span::Spanned<Operand<'tcx>>],
        body: &Body<'tcx>,
        my_name: &str,
        out: &mut Vec<Impurity>,
    ) {
        let Operand::Constant(const_operand) = func else {
            out.push(Impurity {
                reason: "dynamically dispatched call — purity is provable only for \
                         statically resolved calls"
                    .into(),
                chain: vec![my_name.to_string(), "<dynamic call>".into()],
            });
            return;
        };
        let ty = const_operand.const_.ty();
        let TyKind::FnDef(did, _) = ty.kind() else {
            out.push(Impurity {
                reason: "unresolved call target is outside the pure subset".into(),
                chain: vec![my_name.to_string(), "<unresolved call>".into()],
            });
            return;
        };
        let did = *did;
        if did.is_local() {
            let callee = did.expect_local();
            let mut sub = self.effects_of(callee);
            for imp in &mut sub {
                imp.chain.insert(0, my_name.to_string());
            }
            out.extend(sub);
            return;
        }
        // The curated std/core/alloc/nirdosha_rt effect-summary table
        // (issue #71) — consulted before the deny-list/blanket
        // rejection below, the same "resolved DefId, not string-matched
        // source" principle `foreign_reason` already uses, applied to a
        // table that grants trust instead of denying it.
        match std_effects::classify(&self.tcx.def_path_str(did)) {
            Some(std_effects::Effect::Pure) => return,
            Some(std_effects::Effect::HigherOrderPure) => {
                // Trusted for the call itself, but a closure/fn-item
                // argument's own body can still hide an effect (e.g. a
                // `println!` inside `.map(|x| { .. })`) — check every
                // argument that resolves to one, the same recursive
                // local-effects machinery a direct call already gets.
                for arg in args {
                    self.visit_callable_argument(&arg.node, body, my_name, out);
                }
                return;
            }
            None => {}
        }
        if let Some(reason) = self.foreign_reason(did) {
            out.push(Impurity {
                reason: reason.to_string(),
                chain: vec![my_name.to_string(), self.name(did)],
            });
        }
    }

    /// A closure literal or fn-item value passed as an argument to a
    /// trusted higher-order std call (`Iterator::map`, `Option::
    /// and_then`, ...) — resolve its own `DefId` from its MIR type and
    /// check its body exactly as if it had been called directly.
    fn visit_callable_argument(
        &mut self,
        operand: &Operand<'tcx>,
        body: &Body<'tcx>,
        my_name: &str,
        out: &mut Vec<Impurity>,
    ) {
        let Some(place) = operand.place() else {
            return;
        };
        let ty = place.ty(&body.local_decls, self.tcx).ty;
        let (TyKind::Closure(did, _) | TyKind::FnDef(did, _)) = ty.kind() else {
            return;
        };
        let did = *did;
        if did.is_local() {
            let callee = did.expect_local();
            let mut sub = self.effects_of(callee);
            for imp in &mut sub {
                imp.chain.insert(0, my_name.to_string());
            }
            out.extend(sub);
        } else if let Some(reason) = self.foreign_reason(did) {
            out.push(Impurity {
                reason: reason.to_string(),
                chain: vec![my_name.to_string(), self.name(did)],
            });
        }
    }

    fn foreign_reason(&self, did: DefId) -> Option<&'static str> {
        let path = self.tcx.def_path_str(did);
        for (needle, why) in FOREIGN_NEEDLES {
            if path.starts_with(needle) {
                return Some(why);
            }
        }
        Some(
            "external call lacks a verified effect summary (including std, core, alloc and nirdosha_rt)",
        )
    }
}

/// Expanded HIR closes unsafe/static boundaries before MIR optimization.
/// Nested bodies are checked if reachable through the MIR call graph.
struct BodyRestrictions<'tcx> {
    tcx: TyCtxt<'tcx>,
    reasons: Vec<&'static str>,
}

impl<'tcx> Visitor<'tcx> for BodyRestrictions<'tcx> {
    fn visit_block(&mut self, block: &'tcx rustc_hir::Block<'tcx>) {
        if matches!(block.rules, rustc_hir::BlockCheckMode::UnsafeBlock(_)) {
            self.reasons.push("unsafe block is outside the pure subset");
        }
        intravisit::walk_block(self, block);
    }

    fn visit_expr(&mut self, expr: &'tcx rustc_hir::Expr<'tcx>) {
        if let rustc_hir::ExprKind::Path(ref path) = expr.kind {
            let res = self
                .tcx
                .typeck(expr.hir_id.owner.def_id)
                .qpath_res(path, expr.hir_id);
            if matches!(
                res,
                rustc_hir::def::Res::Def(rustc_hir::def::DefKind::Static { .. }, _)
            ) {
                self.reasons.push("static state is outside the pure subset");
            }
        }
        intravisit::walk_expr(self, expr);
    }
}

/// Totality semantics: a pure fn must not panic *except* through the
/// dialect's documented arithmetic-overflow guard.
fn assert_reason(msg: &rustc_middle::mir::AssertMessage<'_>) -> Option<String> {
    use rustc_middle::mir::AssertKind;
    Some(
        match msg {
            // Checked arithmetic is the dialect's documented guard: allowed.
            AssertKind::Overflow(..) | AssertKind::OverflowNeg(_) => return None,
            AssertKind::BoundsCheck { .. } => "can panic (bounds-checked indexing)",
            AssertKind::DivisionByZero(_) => "can panic (division by zero)",
            AssertKind::RemainderByZero(_) => "can panic (remainder by zero)",
            AssertKind::MisalignedPointerDereference { .. } => {
                "can panic (misaligned pointer dereference)"
            }
            AssertKind::NullPointerDereference => "can panic (null pointer dereference)",
            AssertKind::NullReferenceConstructed => "can panic (null reference constructed)",
            AssertKind::InvalidEnumConstruction(_) => "can panic (invalid enum construction)",
            // Coroutine machinery is not a claim violation.
            AssertKind::ResumedAfterReturn(_)
            | AssertKind::ResumedAfterPanic(_)
            | AssertKind::ResumedAfterDrop(_) => return None,
        }
        .to_string(),
    )
}
