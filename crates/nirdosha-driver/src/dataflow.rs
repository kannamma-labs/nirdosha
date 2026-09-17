//! `resource(kind = "..")` (issue #69): a real `rustc_mir_dataflow`
//! forward "may still hold an acquired, unreleased resource" analysis
//! over pre-optimization MIR.
//!
//! Unlike `numeric.rs`'s hand-rolled path-sensitive walker -- which
//! explicitly refuses to enter a loop, because free-form symbolic
//! arithmetic has no sound widening -- this domain is a small monotone
//! lattice (a bitset union), exactly the shape `rustc_mir_dataflow`'s
//! generic fixpoint engine is built for. Loops are fine here.
//!
//! Two passes over the same `Analysis`, per the framework's own
//! documented usage: `ResourceAnalysis` computes the fixpoint (pure
//! state, no diagnostics -- a block's `apply_*` effects can run more
//! than once during convergence, so emitting a diagnostic there could
//! double-report or reflect a not-yet-stable state); `ResourceViolations`
//! then walks the *stable* results exactly once via `visit_results` and
//! reports real violations against that final state.
use nirdosha_contract_core::model::Contract;
use rustc_index::bit_set::DenseBitSet;
use rustc_middle::mir::{
    self, BasicBlock, Body, CallReturnPlaces, Local, Location, Statement, StatementKind,
    Terminator, TerminatorKind,
};
use rustc_middle::ty::{TyCtxt, TyKind};
use rustc_mir_dataflow::{Analysis, ResultsVisitor, visit_results};
use rustc_span::def_id::{LOCAL_CRATE, LocalDefId};

const ACQUIRE_PATH: &str = "nirdosha_rt::resource::acquire";
const RELEASE_PATH: &str = "nirdosha_rt::resource::release";

pub struct Violation {
    pub kind: &'static str,
    pub location: String,
}

pub enum Report {
    /// Not claimed by this fn's contract -- nothing to check.
    NotClaimed,
    /// Checked; every acquire matched a release on every reachable path.
    Clean,
    Violations(Vec<Violation>),
    /// Could not run the analysis at all -- treated as a hard error by
    /// the caller, never as a silent pass: a `resource(..)` claim this
    /// driver cannot check is a lie by omission, same discipline
    /// `numeric::Report::contract_error` uses for a malformed predicate.
    Unsupported(&'static str),
}

/// The resolved callee path of a `Call` terminator's `func` operand, the
/// same `TyKind::FnDef` resolution `main.rs`'s `visit_callee` already
/// uses to recognize calls by name rather than by string-matching
/// source. Strips a local crate's own name prefix off a local `DefId`'s
/// path, the same normalization `main.rs`'s `Effects::name` already
/// applies -- so `acquire`/`release` resolve identically whether
/// `nirdosha_rt` is the real external crate (an already-bare path) or,
/// as in this driver's own tests, a same-crate `mod nirdosha_rt` shim.
fn callee_path<'tcx>(tcx: TyCtxt<'tcx>, func: &mir::Operand<'tcx>) -> Option<String> {
    let mir::Operand::Constant(c) = func else {
        return None;
    };
    let TyKind::FnDef(did, _) = c.const_.ty().kind() else {
        return None;
    };
    let did = *did;
    let path = tcx.def_path_str(did);
    if did.is_local() {
        let local_crate = tcx.crate_name(LOCAL_CRATE).to_string();
        Some(
            path.strip_prefix(&format!("{local_crate}::"))
                .map(str::to_string)
                .unwrap_or(path),
        )
    } else {
        Some(path)
    }
}

pub fn analyze(tcx: TyCtxt<'_>, did: LocalDefId, contract: Option<&Contract>) -> Report {
    if contract.and_then(|c| c.resource.as_ref()).is_none() {
        return Report::NotClaimed;
    }
    if tcx.is_coroutine(did.to_def_id()) {
        return Report::Unsupported("coroutine");
    }
    let mir = tcx.mir_drops_elaborated_and_const_checked(did);
    if mir.is_stolen() {
        return Report::Unsupported("pre-optimization MIR already consumed");
    }
    let body = mir.borrow();
    let analysis = ResourceAnalysis { tcx, body: &body };
    let results = analysis.iterate_to_fixpoint(tcx, &body, None);
    let mut visitor = ResourceViolations {
        tcx,
        violations: Vec::new(),
    };
    visit_results(
        &body,
        mir::traversal::reachable(&body).map(|(bb, _)| bb),
        &results,
        &mut visitor,
    );
    if visitor.violations.is_empty() {
        Report::Clean
    } else {
        Report::Violations(visitor.violations)
    }
}

struct ResourceAnalysis<'a, 'tcx> {
    tcx: TyCtxt<'tcx>,
    body: &'a Body<'tcx>,
}

impl<'tcx> Analysis<'tcx> for ResourceAnalysis<'_, 'tcx> {
    type Domain = DenseBitSet<Local>;
    const NAME: &'static str = "nirdosha_resource";

    fn bottom_value(&self, body: &Body<'tcx>) -> Self::Domain {
        DenseBitSet::new_empty(body.local_decls.len())
    }

    fn initialize_start_block(&self, _body: &Body<'tcx>, _state: &mut Self::Domain) {}

    fn apply_primary_statement_effect(
        &self,
        state: &mut Self::Domain,
        statement: &Statement<'tcx>,
        _location: Location,
    ) {
        // A plain `let y = x;`/move keeps a tracked local's bit alive
        // under its new name, so rebinding between acquire and release
        // doesn't read as a leak -- drop elaboration routinely inserts
        // exactly this shape (`_2 = move _1; release(move _2)`) even for
        // a direct `release(r)` call, so this isn't an edge case. A
        // `Move` also clears the source: it's really gone, and leaving
        // its bit set would still look "acquired" at whatever later
        // point (e.g. this fn's own exit) that dead local's storage is
        // inspected. `Copy` leaves the source as still holding it. Any
        // other assignment to a local overwrites whatever it used to
        // hold.
        let StatementKind::Assign(assign) = &statement.kind else {
            return;
        };
        let Some(dest) = assign.0.as_local() else {
            return;
        };
        match &assign.1 {
            mir::Rvalue::Use(mir::Operand::Move(src), _) => {
                if let Some(src) = src.as_local() {
                    if state.contains(src) {
                        state.insert(dest);
                    } else {
                        state.remove(dest);
                    }
                    state.remove(src);
                    return;
                }
            }
            mir::Rvalue::Use(mir::Operand::Copy(src), _) => {
                if let Some(src) = src.as_local() {
                    if state.contains(src) {
                        state.insert(dest);
                    } else {
                        state.remove(dest);
                    }
                    return;
                }
            }
            _ => {}
        }
        state.remove(dest);
    }

    fn apply_primary_terminator_effect(
        &self,
        state: &mut Self::Domain,
        terminator: &Terminator<'tcx>,
        _location: Location,
    ) {
        let TerminatorKind::Call { func, args, .. } = &terminator.kind else {
            return;
        };
        if callee_path(self.tcx, func).as_deref() != Some(RELEASE_PATH) {
            return;
        }
        let Some(arg) = args.first() else { return };
        if let mir::Operand::Move(place) | mir::Operand::Copy(place) = &arg.node
            && let Some(local) = place.as_local()
        {
            state.remove(local);
        }
    }

    fn apply_call_return_effect(
        &self,
        state: &mut Self::Domain,
        block: BasicBlock,
        return_places: CallReturnPlaces<'_, 'tcx>,
    ) {
        let TerminatorKind::Call { func, .. } = &self.body[block].terminator().kind else {
            return;
        };
        if callee_path(self.tcx, func).as_deref() != Some(ACQUIRE_PATH) {
            return;
        }
        return_places.for_each(|place| {
            if let Some(local) = place.as_local() {
                state.insert(local);
            }
        });
    }
}

struct ResourceViolations<'tcx> {
    tcx: TyCtxt<'tcx>,
    violations: Vec<Violation>,
}

impl<'tcx> ResourceViolations<'tcx> {
    fn location_of(&self, span: rustc_span::Span) -> String {
        self.tcx.sess.source_map().span_to_diagnostic_string(span)
    }

    fn location(&self, terminator: &Terminator<'tcx>) -> String {
        self.location_of(terminator.source_info.span)
    }
}

impl<'a, 'tcx> ResultsVisitor<'tcx, ResourceAnalysis<'a, 'tcx>> for ResourceViolations<'tcx> {
    /// Fires with the state as it was *entering* this statement, before
    /// this analysis's own primary effect for it (the propagation logic
    /// in `apply_primary_statement_effect`) has run. Drop elaboration
    /// routinely lowers `r = acquire(..)` into a fresh-temp call
    /// followed by a plain move-assignment into `r` (see
    /// `apply_primary_statement_effect`'s own doc comment) -- so a
    /// reassignment that clobbers a still-held resource, not just a
    /// second direct `acquire()` call, is exactly where a double-acquire
    /// actually shows up in this IR.
    fn visit_after_early_statement_effect(
        &mut self,
        state: &DenseBitSet<Local>,
        statement: &Statement<'tcx>,
        _location: Location,
    ) {
        let StatementKind::Assign(assign) = &statement.kind else {
            return;
        };
        let Some(dest) = assign.0.as_local() else {
            return;
        };
        if state.contains(dest) {
            self.violations.push(Violation {
                kind: "double_acquire",
                location: self.location_of(statement.source_info.span),
            });
        }
    }

    /// Fires with the state as it was *entering* this terminator, before
    /// this analysis's own primary/call-return effects for it have run
    /// (this analysis's `apply_early_terminator_effect` is the default
    /// no-op) -- exactly the precondition an acquire/release call site
    /// needs checked against.
    fn visit_after_early_terminator_effect(
        &mut self,
        state: &DenseBitSet<Local>,
        terminator: &Terminator<'tcx>,
        _location: Location,
    ) {
        let TerminatorKind::Call {
            func,
            args,
            destination,
            ..
        } = &terminator.kind
        else {
            return;
        };
        match callee_path(self.tcx, func).as_deref() {
            Some(ACQUIRE_PATH) => {
                if let Some(local) = destination.as_local()
                    && state.contains(local)
                {
                    self.violations.push(Violation {
                        kind: "double_acquire",
                        location: self.location(terminator),
                    });
                }
            }
            Some(RELEASE_PATH) => {
                let Some(arg) = args.first() else { return };
                if let mir::Operand::Move(place) | mir::Operand::Copy(place) = &arg.node
                    && let Some(local) = place.as_local()
                    && !state.contains(local)
                {
                    self.violations.push(Violation {
                        kind: "release_without_acquire",
                        location: self.location(terminator),
                    });
                }
            }
            _ => {}
        }
    }

    /// Fires with the state as of just after a terminator's own primary
    /// effect (for `Return`, no call-return effect exists, so this is
    /// simply the final state on this path) -- any bit still set here is
    /// a resource acquired somewhere upstream and never released before
    /// this fn hands control back to its caller.
    fn visit_after_primary_terminator_effect(
        &mut self,
        state: &DenseBitSet<Local>,
        terminator: &Terminator<'tcx>,
        _location: Location,
    ) {
        if !matches!(terminator.kind, TerminatorKind::Return) || state.is_empty() {
            return;
        }
        self.violations.push(Violation {
            kind: "leaked",
            location: self.location(terminator),
        });
    }
}
