//! Path-sensitive numeric verification of pre-optimization runtime MIR.
//! A check is proven only if every path reaching it proves the condition.
//! Cycles, excessive path expansion, and unknown control flow discard ALL
//! results for the body. No finite unrolling is mistaken for induction.
//!
//! `requires(expr)`/`ensures(expr)` (issue #68) ride the same VC IR: a
//! `requires` predicate is assumed once, right after the entry state is
//! built, exactly like an already-passed `Assert` guard; an `ensures`
//! predicate is checked at every `Return` the path walk actually
//! reaches, exactly like an `Assert` condition -- one more root over the
//! shared `Engine`, not a second prover.
use nirdosha_contract_core::model::Contract;
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
    /// A `requires`/`ensures` predicate that doesn't parse, uses
    /// unsupported syntax, or names something that isn't `result` or one
    /// of this function's own parameters. Unlike `limitation` (a sound
    /// "can't prove it" outcome), this is a hard error: the contract
    /// itself is malformed, not merely unprovable.
    pub contract_error: Option<String>,
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

pub fn analyze(tcx: TyCtxt<'_>, did: LocalDefId, contract: Option<&Contract>) -> Report {
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
    let ensures = match contract.and_then(|c| c.ensures.as_ref()) {
        Some(ensures) => match syn::parse_str::<syn::Expr>(&ensures.expr) {
            Ok(expr) => Some(expr),
            Err(e) => {
                return Report {
                    contract_error: Some(format!("ensures(..): {e}")),
                    ..Report::default()
                };
            }
        },
        None => None,
    };
    let mut walk = Walk {
        tcx,
        body: &body,
        env: ty::TypingEnv::post_analysis(tcx, did),
        engine,
        report: Report::default(),
        steps: 0,
        ensures,
    };
    let state: State = body.local_decls.iter().map(|d| walk.fresh(d.ty)).collect();
    if let Some(requires) = contract.and_then(|c| c.requires.as_ref()).and_then(|r| r.expr.as_deref()) {
        match syn::parse_str::<syn::Expr>(requires) {
            Ok(expr) => match walk.eval_predicate(&expr, &state, IntType::BOOL) {
                Ok(n) => {
                    let p = walk.engine.test(&n, true);
                    walk.engine.assume(&p);
                }
                Err(e) => {
                    walk.report.contract_error = Some(format!("requires(..): {e}"));
                    return walk.report;
                }
            },
            Err(e) => {
                walk.report.contract_error = Some(format!("requires(..): {e}"));
                return walk.report;
            }
        }
    }
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
    /// This function's own `ensures(..)` predicate, pre-parsed once;
    /// checked at every `Return` terminator the path walk reaches.
    ensures: Option<syn::Expr>,
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
    /// Resolve a `requires`/`ensures` identifier against this function's
    /// own MIR locals. `result` is the return place; every other name
    /// must be a source-level parameter name (via debug info) -- an
    /// unresolved identifier is refused, never silently left unproven.
    fn contract_local(&self, name: &str) -> Option<Local> {
        if name == "result" {
            return Some(RETURN_PLACE);
        }
        self.body.var_debug_info.iter().find_map(|info| {
            if info.name.as_str() != name {
                return None;
            }
            match &info.value {
                VarDebugInfoContents::Place(place)
                    if place.projection.is_empty()
                        && place.local.index() >= 1
                        && place.local.index() <= self.body.arg_count =>
                {
                    Some(place.local)
                }
                _ => None,
            }
        })
    }

    /// Evaluate the restricted `predicate::check_predicate_shape` subset
    /// against `state`. `hint` types a bare integer literal when neither
    /// operand names a local with a real MIR type (e.g. `1 + 1`); an
    /// identifier or the other side of a binary op always overrides it,
    /// so a mismatched hint can only affect a literal-only sub-expression.
    fn eval_predicate(&self, expr: &syn::Expr, state: &State, hint: IntType) -> Result<Number, String> {
        match expr {
            syn::Expr::Paren(e) => self.eval_predicate(&e.expr, state, hint),
            syn::Expr::Group(e) => self.eval_predicate(&e.expr, state, hint),
            syn::Expr::Path(p) => {
                let name = p
                    .path
                    .get_ident()
                    .ok_or_else(|| "expected a plain identifier".to_string())?
                    .to_string();
                let local = self.contract_local(&name).ok_or_else(|| {
                    format!("`{name}` is not `result` or one of this function's own parameters")
                })?;
                match self.place(Place::from(local), state) {
                    Val::Scalar(n) => Ok(n),
                    _ => Err(format!("`{name}` is not an integer or boolean value")),
                }
            }
            syn::Expr::Lit(l) => match &l.lit {
                syn::Lit::Int(i) => {
                    let bits: i128 = i.base10_parse().map_err(|e| e.to_string())?;
                    Ok(self.engine.constant(bits as u128, hint))
                }
                syn::Lit::Bool(b) => Ok(self.engine.constant(b.value as u128, IntType::BOOL)),
                _ => Err("requires/ensures literals must be integers or booleans".into()),
            },
            syn::Expr::Unary(u) => {
                let inner_hint = if matches!(u.op, syn::UnOp::Not(_)) {
                    IntType::BOOL
                } else {
                    hint
                };
                let n = self.eval_predicate(&u.expr, state, inner_hint)?;
                match u.op {
                    syn::UnOp::Neg(_) => Ok(self.engine.negate(&n)),
                    syn::UnOp::Not(_) => Ok(self.engine.invert(&n)),
                    _ => Err("unsupported unary operator in requires/ensures".into()),
                }
            }
            syn::Expr::Binary(b) => {
                let op = match b.op {
                    syn::BinOp::Add(_) => Op::Add,
                    syn::BinOp::Sub(_) => Op::Sub,
                    syn::BinOp::Mul(_) => Op::Mul,
                    syn::BinOp::Div(_) => Op::Div,
                    syn::BinOp::Rem(_) => Op::Rem,
                    syn::BinOp::Eq(_) => Op::Eq,
                    syn::BinOp::Ne(_) => Op::Ne,
                    syn::BinOp::Lt(_) => Op::Lt,
                    syn::BinOp::Le(_) => Op::Le,
                    syn::BinOp::Gt(_) => Op::Gt,
                    syn::BinOp::Ge(_) => Op::Ge,
                    syn::BinOp::And(_) => Op::And,
                    syn::BinOp::Or(_) => Op::Or,
                    _ => return Err("unsupported operator in requires/ensures".into()),
                };
                let bool_op = matches!(op, Op::And | Op::Or);
                let left_hint = if bool_op { IntType::BOOL } else { hint };
                let left = self.eval_predicate(&b.left, state, left_hint)?;
                let right_hint = if bool_op { IntType::BOOL } else { left.ty };
                let right = self.eval_predicate(&b.right, state, right_hint)?;
                Ok(self.engine.binary(op, &left, &right).0)
            }
            _ => Err(
                "unsupported expression in requires/ensures — only identifiers, integer/bool \
                 literals, arithmetic, comparisons, &&, ||, unary -/!, and parens are supported"
                    .into(),
            ),
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
            TerminatorKind::Return => {
                if let Some(expr) = self.ensures.clone() {
                    let proven = match self.eval_predicate(&expr, &state, IntType::BOOL) {
                        Ok(n) => {
                            let p = self.engine.test(&n, true);
                            self.engine.prove(&p)
                        }
                        Err(e) => {
                            self.report
                                .contract_error
                                .get_or_insert(format!("ensures(..): {e}"));
                            false
                        }
                    };
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
                            kind: "ensures",
                            proven,
                            location,
                        });
                }
            }
            TerminatorKind::Unreachable
            | TerminatorKind::UnwindResume
            | TerminatorKind::UnwindTerminate(_)
            | TerminatorKind::TailCall { .. } => {}
            _ => self.report.limitation = Some("unsupported control flow"),
        }
        self.engine.pop();
        path.pop();
    }
}
