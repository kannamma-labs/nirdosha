use crate::{IntType, Op};
use std::str::FromStr;
use z3::{
    Params, SatResult, Solver,
    ast::{Ast, Bool, Int},
};

pub const BACKEND: &str = "z3";
#[derive(Clone)]
pub struct Number {
    pub ty: IntType,
    term: Int,
}
#[derive(Clone)]
pub struct Predicate(Bool);
pub struct Engine {
    solver: Solver,
    queries: std::cell::Cell<usize>,
}

/// Truncation toward zero, extracted unchanged from compiler/src/smt.rs.
/// The caller must establish/guard b != 0 before interpreting the result.
/// At zero this relation is inconsistent: it describes normal continuation
/// only, and must never be introduced before querying the division guard.
pub fn div_rem(solver: &Solver, a: &Int, b: &Int) -> (Int, Int) {
    let q = Int::fresh_const("div_q");
    let rem = Int::fresh_const("div_r");
    solver.assert(a.eq(b * &q + &rem));
    let zero = Int::from_i64(0);
    let abs_b = b.lt(&zero).ite(&(-b), b);
    solver.assert(rem.lt(&abs_b));
    solver.assert(rem.gt(&(-abs_b)));
    solver.assert(rem.eq(&zero) | a.lt(&zero).eq(&rem.lt(&zero)));
    (q, rem)
}

fn int(value: u128) -> Int {
    Int::from_str(&value.to_string()).unwrap()
}
fn bounds(ty: IntType) -> (Int, Int) {
    if ty.signed {
        let half = int(1u128 << (ty.bits - 1));
        (-&half, &half - 1)
    } else {
        (
            Int::from_i64(0),
            if ty.bits == 128 {
                int(u128::MAX)
            } else {
                int((1u128 << ty.bits) - 1)
            },
        )
    }
}
fn wrap(term: &Int, ty: IntType) -> Int {
    // Euclidean modulo is intentional here: two's-complement truncation.
    // Avoid int->BV->int for arithmetic, which is much harder for the solver.
    let modulus = if ty.bits == 128 {
        int(u128::MAX) + 1
    } else {
        int(1u128 << ty.bits)
    };
    let (lo, _) = bounds(ty);
    (term - &lo).modulo(&modulus) + lo
}
impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}
impl Engine {
    pub fn new() -> Self {
        let solver = Solver::new();
        let mut params = Params::new();
        params.set_u32("timeout", 200);
        params.set_u32("rlimit", 100_000);
        solver.set_params(&params);
        Self {
            solver,
            queries: std::cell::Cell::new(0),
        }
    }
    pub fn push(&self) {
        self.solver.push();
    }
    pub fn pop(&self) {
        self.solver.pop(1);
    }
    pub fn assume(&self, p: &Predicate) {
        self.solver.assert(&p.0);
    }
    pub fn queries(&self) -> usize {
        self.queries.get()
    }
    pub fn prove(&self, p: &Predicate) -> bool {
        self.queries.set(self.queries.get() + 1);
        self.push();
        self.solver.assert(p.0.not());
        let result = self.solver.check() == SatResult::Unsat;
        self.pop();
        result
    }
    pub fn fresh(&self, ty: IntType) -> Number {
        let term = Int::fresh_const("mir");
        let (lo, hi) = bounds(ty);
        self.solver.assert(term.ge(lo));
        self.solver.assert(term.le(hi));
        Number { ty, term }
    }
    pub fn constant(&self, bits: u128, ty: IntType) -> Number {
        Number {
            ty,
            term: wrap(&int(bits), ty).simplify(),
        }
    }
    pub fn cast(&self, n: &Number, ty: IntType) -> Number {
        Number {
            ty,
            term: wrap(&n.term, ty),
        }
    }
    pub fn test(&self, n: &Number, expected: bool) -> Predicate {
        Predicate(n.term.eq(if expected { 1 } else { 0 }))
    }
    pub fn equal(&self, a: &Number, b: &Number) -> Predicate {
        Predicate(a.term.eq(&b.term))
    }
    pub fn not(&self, p: &Predicate) -> Predicate {
        Predicate(p.0.not())
    }
    fn boolean(&self, p: Bool) -> Number {
        Number {
            ty: IntType::BOOL,
            term: p.ite(&Int::from_i64(1), &Int::from_i64(0)),
        }
    }
    pub fn negate(&self, n: &Number) -> Number {
        Number {
            ty: n.ty,
            term: wrap(&(-&n.term), n.ty),
        }
    }
    pub fn invert(&self, n: &Number) -> Number {
        Number {
            ty: n.ty,
            term: n.term.to_ast(n.ty.bits).bvnot().to_int(n.ty.signed),
        }
    }
    /// Returns the wrapped result and a flag for add/sub/mul overflow.
    pub fn binary(&self, op: Op, a: &Number, b: &Number) -> (Number, Number) {
        let l = &a.term;
        let r = &b.term;
        let raw = match op {
            Op::Add => l + r,
            Op::Sub => l - r,
            Op::Mul => l * r,
            Op::Div => div_rem(&self.solver, l, r).0,
            Op::Rem => div_rem(&self.solver, l, r).1,
            Op::Eq | Op::Ne | Op::Lt | Op::Le | Op::Gt | Op::Ge => {
                let p = match op {
                    Op::Eq => l.eq(r),
                    Op::Ne => l.ne(r),
                    Op::Lt => l.lt(r),
                    Op::Le => l.le(r),
                    Op::Gt => l.gt(r),
                    _ => l.ge(r),
                };
                return (self.boolean(p), self.constant(0, IntType::BOOL));
            }
            Op::And | Op::Or | Op::Xor | Op::Shl | Op::Shr => {
                let lb = l.to_ast(a.ty.bits);
                let rb = if matches!(op, Op::Shl | Op::Shr) {
                    r.modulo(a.ty.bits).to_ast(a.ty.bits)
                } else {
                    r.to_ast(a.ty.bits)
                };
                let v = match op {
                    Op::And => lb.bvand(&rb),
                    Op::Or => lb.bvor(&rb),
                    Op::Xor => lb.bvxor(&rb),
                    Op::Shl => lb.bvshl(&rb),
                    _ if a.ty.signed => lb.bvashr(&rb),
                    _ => lb.bvlshr(&rb),
                };
                return (
                    Number {
                        ty: a.ty,
                        term: v.to_int(a.ty.signed),
                    },
                    self.constant(0, IntType::BOOL),
                );
            }
        };
        let (lo, hi) = bounds(a.ty);
        let overflow = self.boolean(raw.lt(lo) | raw.gt(hi));
        (
            Number {
                ty: a.ty,
                term: wrap(&raw, a.ty),
            },
            overflow,
        )
    }
}
