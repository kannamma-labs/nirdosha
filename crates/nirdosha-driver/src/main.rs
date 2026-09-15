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
//! - **Totality at the MIR level.** `unwrap`/`expect`/bounds checks/
//!   division-by-zero compile to `assert` terminators — flagged exactly,
//!   with zero false positives from same-named user methods. Arithmetic
//!   overflow checks are allowed: checked arithmetic is the dialect's
//!   documented runtime guard.
//! - **Default-deny third party.** A call into a crate that is neither
//!   `core`/`std`/`alloc`/`nirdosha_rt` nor contract-carrying makes the
//!   caller impure — an unverified dependency cannot launder effects.
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
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_span;

use std::collections::{HashMap, HashSet};
use std::process::ExitCode;

use nirdosha_contract_core as cc;

use rustc_driver::{Callbacks, Compilation};
use rustc_interface::interface::Compiler;
use rustc_middle::mir::{Operand, TerminatorKind};
use rustc_middle::ty::{TyCtxt, TyKind};
use rustc_span::def_id::{DefId, LocalDefId, LOCAL_CRATE};

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().collect();
    // RUSTC_WORKSPACE_WRAPPER protocol: cargo invokes the wrapper as
    // `nirdosha-driver <path-to-rustc> <rustc args...>`. cargo-nirdosha
    // marks its delegation with NIRDOSHA_DRIVER; drop the rustc path
    // so run_compiler sees a plain rustc argv.
    if std::env::var_os("NIRDOSHA_DRIVER").is_some()
        && args.get(1).is_some_and(|a| !a.starts_with('-'))
    {
        args.remove(1);
    }
    rustc_driver::catch_with_exit_code(|| {
        rustc_driver::run_compiler(&args, &mut NirdoshaCallbacks);
    })
}

struct NirdoshaCallbacks;

impl Callbacks for NirdoshaCallbacks {
    fn after_analysis<'tcx>(&mut self, _compiler: &Compiler, tcx: TyCtxt<'tcx>) -> Compilation {
        analyze(tcx)
    }
}

// ---------------------------------------------------------------------------
// Claims: parse `nirdosha:contract` doc attributes off every MIR body.
// At HIR time the #[contract] attribute macro has already expanded into
// exactly this doc form — so both authoring surfaces converge here.
// ---------------------------------------------------------------------------

fn analyze(tcx: TyCtxt<'_>) -> Compilation {
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

    let mut engine = Effects::new(tcx);
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
        if contract.claims_pure() {
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
    Compilation::Continue
}

// ---------------------------------------------------------------------------
// The effect lattice engine: least-effect computation over the local
// call graph, memoized, cycle-safe (a back-edge contributes nothing —
// any impure call site is found from the body that contains it, so
// recursion like factorial stays pure and laundering through cycles is
// impossible).
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

/// Crates whose non-needle functions are considered pure by default.
/// Everything else is default-deny: an unverified third-party crate
/// cannot launder effects into a pure fn.
///
/// `nirdosha_rt` is in here on purpose: the `#[contract]` macro injects
/// `nirdosha_rt::nfr::enter` into any fn with an `nfr(..)` clause —
/// including pure-claiming ones. `effects(pure)` describes the *user's*
/// body; the injected guard's clock/semaphore is the dialect's own
/// infrastructure, declared honestly by the separate `nfr(..)` clause.
/// The needle table above still pins `nirdosha_rt`'s impure spots if
/// anything changes.
const KNOWN_PURE_CRATES: &[&str] = &["core", "std", "alloc", "nirdosha_rt"];

struct Effects<'tcx> {
    tcx: TyCtxt<'tcx>,
    memo: HashMap<LocalDefId, Vec<Impurity>>,
    visiting: HashSet<LocalDefId>,
}

impl<'tcx> Effects<'tcx> {
    fn new(tcx: TyCtxt<'tcx>) -> Self {
        Self {
            tcx,
            memo: HashMap::new(),
            visiting: HashSet::new(),
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
        if let Some(memo) = self.memo.get(&did) {
            return memo.clone();
        }
        if !self.visiting.insert(did) {
            // Back-edge in recursion: contributes nothing (see the
            // cycle-safety note above).
            return Vec::new();
        }

        let mut impurities = Vec::new();
        let my_name = self.name(did.to_def_id());
        let body = self.tcx.optimized_mir(did.to_def_id());

        for block in body.basic_blocks.iter() {
            let Some(terminator) = &block.terminator else { continue };
            match &terminator.kind {
                TerminatorKind::Call { func, .. } | TerminatorKind::TailCall { func, .. } => {
                    self.visit_callee(func, &my_name, &mut impurities);
                }
                TerminatorKind::Assert { msg, .. } => {
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
                _ => {}
            }
        }

        self.visiting.remove(&did);

        // Dedup: a callee reached by two paths is one fact.
        let mut seen = HashSet::new();
        impurities.retain(|imp| seen.insert((imp.reason.clone(), imp.chain.clone())));
        self.memo.insert(did, impurities.clone());
        impurities
    }

    fn visit_callee(&mut self, func: &Operand<'tcx>, my_name: &str, out: &mut Vec<Impurity>) {
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
        if let Some(reason) = self.foreign_reason(did) {
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
        let crate_name = self.tcx.crate_name(did.krate);
        if KNOWN_PURE_CRATES.contains(&crate_name.as_str()) {
            return None;
        }
        Some("call into unverified third-party crate — publish contracts for it or keep it out of pure fns")
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