//! `sequence(before = "..", after = "..")` (issue #76): a real
//! `rustc_mir_dataflow` forward "has `before` definitely already run
//! on every path reaching here" analysis over pre-optimization MIR —
//! the ordering half of #76's "signed domain pack invariants" gap,
//! built on the exact same framework and `Analysis` shape #69's
//! `resource(kind = "..")` check (`dataflow.rs`) already proved out
//! for this rustc version.
//!
//! **The lattice, and why it's sound as a "must" analysis even though
//! `rustc_mir_dataflow`'s generic join is a union (an "may" framework
//! by default):** track the *negated* fact, "`before` has **not** yet
//! run," as a single tracked bit. It starts set at function entry
//! (nothing has run yet), is cleared the moment `before` returns
//! successfully, and — because `DenseBitSet::join` is a union — a
//! merge point keeps the bit set if it is set on *any* incoming edge.
//! That is exactly "we cannot yet guarantee `before` ran on every
//! path," the real must-property this check needs; the union-based
//! framework computes it for free once the fact being unioned is
//! phrased as a negative.
//!
//! **Disclosed scope** (see `nirdosha_contract_core::model::Sequence`'s
//! own doc comment for the full statement): direct-call-only, no
//! interprocedural call-graph stitching (same as `resource(..)`'s own
//! precedent), and subject-blind — it does not distinguish which
//! value/account a call operates on.

use nirdosha_contract_core::model::Contract;
use rustc_index::bit_set::DenseBitSet;
use rustc_middle::mir::{self, BasicBlock, Body, CallReturnPlaces, Local, Location, Statement, Terminator, TerminatorKind};
use rustc_middle::ty::{TyCtxt, TyKind};
use rustc_mir_dataflow::{Analysis, ResultsVisitor, visit_results};
use rustc_span::def_id::LocalDefId;

pub struct Violation {
    pub location: String,
}

pub enum Report {
    /// Not claimed by this fn's contract — nothing to check.
    NotClaimed,
    /// Checked; every call to `after` was guaranteed to be preceded by
    /// a call to `before` on every reachable path.
    Clean,
    Violations(Vec<Violation>),
    /// Could not run the analysis at all — a `sequence(..)` claim this
    /// driver cannot check is a lie by omission, same discipline
    /// `dataflow::Report::Unsupported`/`numeric::Report::contract_error`
    /// already use.
    Unsupported(&'static str),
}

/// `true` if `func` resolves to a fn/method whose own last path
/// segment is `name` — plain-name matching, not a full qualified path:
/// a pack declares `sequence(before = "debit", after = "credit")` with
/// bare names, the same convention `nirdosha-contract-core::pack`'s
/// `mandatory_fns`/`protected_structs` already use.
fn callee_matches<'tcx>(tcx: TyCtxt<'tcx>, func: &mir::Operand<'tcx>, name: &str) -> bool {
    let mir::Operand::Constant(c) = func else {
        return false;
    };
    let TyKind::FnDef(did, _) = c.const_.ty().kind() else {
        return false;
    };
    tcx.def_path_str(*did).rsplit("::").next() == Some(name)
}

pub fn analyze(tcx: TyCtxt<'_>, did: LocalDefId, contract: Option<&Contract>) -> Report {
    let Some(sequence) = contract.and_then(|c| c.sequence.as_ref()) else {
        return Report::NotClaimed;
    };
    if tcx.is_coroutine(did.to_def_id()) {
        return Report::Unsupported("coroutine");
    }
    let mir = tcx.mir_drops_elaborated_and_const_checked(did);
    if mir.is_stolen() {
        return Report::Unsupported("pre-optimization MIR already consumed");
    }
    let body = mir.borrow();
    let analysis = SequenceAnalysis {
        tcx,
        body: &body,
        before: sequence.before.clone(),
    };
    let results = analysis.iterate_to_fixpoint(tcx, &body, None);
    let mut visitor = SequenceViolations {
        tcx,
        after: sequence.after.clone(),
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

/// The single tracked fact's bit index — this analysis tracks exactly
/// one boolean, not a per-local set the way `dataflow.rs`'s
/// `ResourceAnalysis` must (there, *which* local holds the resource is
/// the whole point; here, only "has `before` run yet" matters). Reuses
/// `RETURN_PLACE` (`_0`, always present in any MIR body) purely as a
/// convenient, always-valid bit index into a `DenseBitSet<Local>` —
/// `Local` (unlike a bare `usize`) already implements the
/// `DebugWithContext` the dataflow framework's fixpoint debug-logging
/// requires, the same domain type `dataflow.rs` uses. This never reads
/// or writes the actual return place; the index is just borrowed.
const NOT_YET_BEFORE: Local = mir::RETURN_PLACE;

struct SequenceAnalysis<'a, 'tcx> {
    tcx: TyCtxt<'tcx>,
    body: &'a Body<'tcx>,
    before: String,
}

impl<'tcx> Analysis<'tcx> for SequenceAnalysis<'_, 'tcx> {
    type Domain = DenseBitSet<Local>;
    const NAME: &'static str = "nirdosha_sequence";

    fn bottom_value(&self, body: &Body<'tcx>) -> Self::Domain {
        DenseBitSet::new_empty(body.local_decls.len())
    }

    fn initialize_start_block(&self, _body: &Body<'tcx>, state: &mut Self::Domain) {
        // At fn entry, `before` has definitely not run yet on any path.
        state.insert(NOT_YET_BEFORE);
    }

    fn apply_primary_statement_effect(
        &self,
        _state: &mut Self::Domain,
        _statement: &Statement<'tcx>,
        _location: Location,
    ) {
        // No statement carries this fact — only a call to `before`
        // (handled below) ever changes it.
    }

    fn apply_call_return_effect(
        &self,
        state: &mut Self::Domain,
        block: BasicBlock,
        _return_places: CallReturnPlaces<'_, 'tcx>,
    ) {
        let TerminatorKind::Call { func, .. } = &self.body[block].terminator().kind else {
            return;
        };
        if callee_matches(self.tcx, func, &self.before) {
            // `before` returned normally on this path — it will never
            // again be "not yet" from here on, unless a later branch
            // re-merges with one that never called it (handled by the
            // union join at that merge point, not here).
            state.remove(NOT_YET_BEFORE);
        }
    }
}

struct SequenceViolations<'tcx> {
    tcx: TyCtxt<'tcx>,
    after: String,
    violations: Vec<Violation>,
}

impl<'a, 'tcx> ResultsVisitor<'tcx, SequenceAnalysis<'a, 'tcx>> for SequenceViolations<'tcx> {
    /// Fires with the state as it was *entering* this terminator,
    /// before this analysis's own call-return effect for it (if any)
    /// has run — exactly the precondition a call to `after` needs
    /// checked against (same timing `dataflow.rs`'s own
    /// `RELEASE_PATH` check uses).
    fn visit_after_early_terminator_effect(
        &mut self,
        state: &DenseBitSet<Local>,
        terminator: &Terminator<'tcx>,
        _location: Location,
    ) {
        let TerminatorKind::Call { func, .. } = &terminator.kind else {
            return;
        };
        if callee_matches(self.tcx, func, &self.after) && state.contains(NOT_YET_BEFORE) {
            self.violations.push(Violation {
                location: self.tcx.sess.source_map().span_to_diagnostic_string(terminator.source_info.span),
            });
        }
    }
}
