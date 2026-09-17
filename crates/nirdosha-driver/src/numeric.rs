//! Path-sensitive numeric verification of pre-optimization runtime MIR.
//! A check is proven only if every path reaching it proves the condition.
//! Cycles, excessive path expansion, and unknown control flow discard ALL
//! results for the body. No finite unrolling is mistaken for induction.
use nirdosha_smt_core::{Engine, IntType, Number, Op};
use rustc_middle::{
    mir::*,
    ty::{self, Ty, TyCtxt, TyKind},
};
use rustc_span::def_id::LocalDefId;
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Default)]
pub struct Report {
    pub checks: BTreeMap<usize, Check>,
    pub queries: usize,
    pub limitation: Option<&'static str>,
}
pub struct Check {
    pub kind: &'static str,
    pub proven: bool,
    pub location: String,
}
impl Report {
    pub fn proven(&self, block: BasicBlock) -> bool {
        self.limitation.is_none() && self.checks.get(&block.index()).is_some_and(|c| c.proven)
    }
    pub fn json(&self, name: &str) -> Value {
        json!({"function": name, "solver_queries": self.queries, "limitation": self.limitation,
            "checks": self.checks.iter().map(|(bb,c)| json!({"block":bb,"kind":c.kind,"proven":c.proven,"location":c.location})).collect::<Vec<_>>()})
    }
    pub fn proofs(&self, name: &str) -> Vec<Value> {
        self.checks.iter().filter(|(_, c)| c.proven).map(|(bb,c)| json!({
            "function":name,"block":bb,"kind":c.kind,"location":c.location,
            "method":nirdosha_smt_core::BACKEND,"elidable":true,
            "scope":"all normal paths reaching this pre-optimization MIR assertion; prior enabled guards hold"
        })).collect()
    }
}
#[derive(Clone)]
enum Val {
    Scalar(Number),
    Tuple(Vec<Val>),
    Unknown,
}
type State = Vec<Val>;

pub fn analyze(tcx: TyCtxt<'_>, did: LocalDefId) -> Report {
    if tcx.is_coroutine(did.to_def_id()) {
        return Report {
            limitation: Some("coroutine"),
            ..Report::default()
        };
    }
    let mir = tcx.mir_drops_elaborated_and_const_checked(did);
    if mir.is_stolen() {
        return Report {
            limitation: Some("pre-optimization MIR already consumed"),
            ..Report::default()
        };
    }
    let body = mir.borrow();
    let engine = Engine::new();
    let mut walk = Walk {
        tcx,
        body: &body,
        env: ty::TypingEnv::post_analysis(tcx, did),
        engine,
        report: Report::default(),
        steps: 0,
    };
    let state = body.local_decls.iter().map(|d| walk.fresh(d.ty)).collect();
    walk.block(START_BLOCK, state, &mut Vec::new());
    walk.report.queries = walk.engine.queries();
    if walk.report.limitation.is_some() {
        for c in walk.report.checks.values_mut() {
            c.proven = false;
        }
    }
    walk.report
}
struct Walk<'a, 'tcx> {
    tcx: TyCtxt<'tcx>,
    body: &'a Body<'tcx>,
    env: ty::TypingEnv<'tcx>,
    engine: Engine,
    report: Report,
    steps: usize,
}
impl<'tcx> Walk<'_, 'tcx> {
    fn int_type(&self, ty: Ty<'tcx>) -> Option<IntType> {
        let pointer_bits = self.tcx.data_layout.pointer_size().bits() as u32;
        match ty.kind() {
            TyKind::Bool => Some(IntType::BOOL),
            TyKind::Int(t) => Some(IntType::new(
                t.bit_width().map(|v| v as u32).unwrap_or(pointer_bits),
                true,
            )),
            TyKind::Uint(t) => Some(IntType::new(
                t.bit_width().map(|v| v as u32).unwrap_or(pointer_bits),
                false,
            )),
            _ => None,
        }
    }
    fn fresh(&self, ty: Ty<'tcx>) -> Val {
        self.int_type(ty)
            .map(|t| Val::Scalar(self.engine.fresh(t)))
            .unwrap_or(Val::Unknown)
    }
    fn havoc(&self, state: &mut State) {
        for (v, d) in state.iter_mut().zip(self.body.local_decls.iter()) {
            *v = self.fresh(d.ty);
        }
    }
    fn place(&self, p: Place<'tcx>, state: &State) -> Val {
        let mut v = state[p.local.index()].clone();
        for proj in p.projection {
            v = match (proj, v) {
                (ProjectionElem::Field(i, _), Val::Tuple(fields)) => {
                    fields.get(i.index()).cloned().unwrap_or(Val::Unknown)
                }
                _ => Val::Unknown,
            };
        }
        if matches!(v, Val::Unknown) {
            self.fresh(p.ty(&self.body.local_decls, self.tcx).ty)
        } else {
            v
        }
    }
    fn operand(&self, op: &Operand<'tcx>, state: &State) -> Val {
        match op {
            Operand::Copy(p) | Operand::Move(p) => self.place(*p, state),
            Operand::Constant(c) => match (
                self.int_type(c.const_.ty()),
                c.const_.try_eval_bits(self.tcx, self.env),
            ) {
                (Some(t), Some(bits)) => Val::Scalar(self.engine.constant(bits, t)),
                _ => self.fresh(c.const_.ty()),
            },
            _ => Val::Unknown,
        }
    }
    fn rvalue(&self, rv: &Rvalue<'tcx>, state: &State) -> Val {
        match rv {
            Rvalue::Use(op, _) => self.operand(op, state),
            Rvalue::Cast(CastKind::IntToInt, op, ty) => {
                match (self.operand(op, state), self.int_type(*ty)) {
                    (Val::Scalar(n), Some(t)) => Val::Scalar(self.engine.cast(&n, t)),
                    _ => Val::Unknown,
                }
            }
            Rvalue::UnaryOp(op, operand) => match self.operand(operand, state) {
                Val::Scalar(n) => match op {
                    UnOp::Neg => Val::Scalar(self.engine.negate(&n)),
                    UnOp::Not => Val::Scalar(self.engine.invert(&n)),
                    _ => Val::Unknown,
                },
                _ => Val::Unknown,
            },
            Rvalue::BinaryOp(op, args) => {
                let (Val::Scalar(a), Val::Scalar(b)) =
                    (self.operand(&args.0, state), self.operand(&args.1, state))
                else {
                    return Val::Unknown;
                };
                let kind = match op {
                    BinOp::Add | BinOp::AddWithOverflow => Op::Add,
                    BinOp::Sub | BinOp::SubWithOverflow => Op::Sub,
                    BinOp::Mul | BinOp::MulWithOverflow => Op::Mul,
                    BinOp::Div => Op::Div,
                    BinOp::Rem => Op::Rem,
                    BinOp::Eq => Op::Eq,
                    BinOp::Ne => Op::Ne,
                    BinOp::Lt => Op::Lt,
                    BinOp::Le => Op::Le,
                    BinOp::Gt => Op::Gt,
                    BinOp::Ge => Op::Ge,
                    BinOp::BitAnd => Op::And,
                    BinOp::BitOr => Op::Or,
                    BinOp::BitXor => Op::Xor,
                    BinOp::Shl => Op::Shl,
                    BinOp::Shr => Op::Shr,
                    _ => return Val::Unknown,
                };
                let (n, overflow) = self.engine.binary(kind, &a, &b);
                if matches!(
                    op,
                    BinOp::AddWithOverflow | BinOp::SubWithOverflow | BinOp::MulWithOverflow
                ) {
                    Val::Tuple(vec![Val::Scalar(n), Val::Scalar(overflow)])
                } else {
                    Val::Scalar(n)
                }
            }
            _ => Val::Unknown,
        }
    }
    fn block(&mut self, bb: BasicBlock, mut state: State, path: &mut Vec<BasicBlock>) {
        if self.report.limitation.is_some() {
            return;
        }
        self.steps += 1;
        if self.steps > 2048 || path.len() >= 256 {
            self.report.limitation = Some("path exploration budget exceeded");
            return;
        }
        if path.contains(&bb) {
            self.report.limitation = Some("loop requires an invariant");
            return;
        }
        if self.body.basic_blocks[bb].is_cleanup {
            return;
        }
        path.push(bb);
        self.engine.push();
        for stmt in &self.body.basic_blocks[bb].statements {
            match &stmt.kind {
                StatementKind::Assign(a) => {
                    if a.0.projection.is_empty() {
                        let value = self.rvalue(&a.1, &state);
                        state[a.0.local.index()] = if matches!(value, Val::Unknown) {
                            self.fresh(self.body.local_decls[a.0.local].ty)
                        } else {
                            value
                        };
                    } else if a
                        .0
                        .projection
                        .iter()
                        .any(|p| matches!(p, ProjectionElem::Deref))
                    {
                        self.havoc(&mut state);
                    } else {
                        state[a.0.local.index()] = self.fresh(self.body.local_decls[a.0.local].ty);
                    }
                }
                StatementKind::StorageLive(l) | StatementKind::StorageDead(l) => {
                    state[l.index()] = self.fresh(self.body.local_decls[*l].ty)
                }
                StatementKind::Nop
                | StatementKind::FakeRead(_)
                | StatementKind::PlaceMention(_)
                | StatementKind::AscribeUserType(..)
                | StatementKind::Coverage(_)
                | StatementKind::ConstEvalCounter
                | StatementKind::BackwardIncompatibleDropHint { .. } => {}
                _ => self.havoc(&mut state),
            }
        }
        let term = self.body.basic_blocks[bb].terminator();
        match &term.kind {
            TerminatorKind::Goto { target } => self.block(*target, state, path),
            TerminatorKind::SwitchInt { discr, targets } => {
                let value = self.operand(discr, &state);
                for (bits, target) in targets.iter() {
                    self.engine.push();
                    if let Val::Scalar(n) = &value {
                        self.engine
                            .assume(&self.engine.equal(n, &self.engine.constant(bits, n.ty)));
                    }
                    self.block(target, state.clone(), path);
                    self.engine.pop();
                }
                self.engine.push();
                if let Val::Scalar(n) = &value {
                    for (bits, _) in targets.iter() {
                        self.engine.assume(
                            &self
                                .engine
                                .not(&self.engine.equal(n, &self.engine.constant(bits, n.ty))),
                        );
                    }
                }
                self.block(targets.otherwise(), state, path);
                self.engine.pop();
            }
            TerminatorKind::Assert {
                cond,
                expected,
                msg,
                target,
                ..
            } => {
                let predicate = match self.operand(cond, &state) {
                    Val::Scalar(n) => Some(self.engine.test(&n, *expected)),
                    _ => None,
                };
                let kind = match **msg {
                    AssertKind::Overflow(..) => Some("overflow"),
                    AssertKind::OverflowNeg(_) => Some("overflow_neg"),
                    AssertKind::BoundsCheck { .. } => Some("bounds"),
                    AssertKind::DivisionByZero(_) => Some("division_by_zero"),
                    AssertKind::RemainderByZero(_) => Some("remainder_by_zero"),
                    _ => None,
                };
                if let Some(kind) = kind {
                    let proven = predicate.as_ref().is_some_and(|p| self.engine.prove(p));
                    let location = self
                        .tcx
                        .sess
                        .source_map()
                        .span_to_diagnostic_string(term.source_info.span);
                    self.report
                        .checks
                        .entry(bb.index())
                        .and_modify(|c| c.proven &= proven)
                        .or_insert(Check {
                            kind,
                            proven,
                            location,
                        });
                }
                // MIR specifies these asserts as goto when checks are disabled.
                let disabled = !self.tcx.sess.overflow_checks()
                    && matches!(
                        **msg,
                        AssertKind::OverflowNeg(_)
                            | AssertKind::Overflow(
                                BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Shl | BinOp::Shr,
                                ..
                            )
                    );
                if !disabled {
                    if let Some(p) = predicate {
                        self.engine.assume(&p);
                    }
                }
                self.block(*target, state, path);
            }
            TerminatorKind::Call { target, .. } => {
                if let Some(target) = target {
                    self.havoc(&mut state);
                    self.block(*target, state, path);
                }
            }
            TerminatorKind::Return
            | TerminatorKind::Unreachable
            | TerminatorKind::UnwindResume
            | TerminatorKind::UnwindTerminate(_)
            | TerminatorKind::TailCall { .. } => {}
            _ => self.report.limitation = Some("unsupported control flow"),
        }
        self.engine.pop();
        path.pop();
    }
}
