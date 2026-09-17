//! Integer semantics shared by the native compiler and rustc MIR frontend.
//! The default backend uses Z3; `--no-default-features` substitutes conservative
//! intervals (no path narrowing or relational reasoning). Unknown is never proof.
#[cfg(feature = "smt")]
mod smt;
#[cfg(feature = "smt")]
pub use smt::*;
#[cfg(not(feature = "smt"))]
mod interval;
#[cfg(not(feature = "smt"))]
pub use interval::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntType {
    pub bits: u32,
    pub signed: bool,
}
impl IntType {
    pub const BOOL: Self = Self {
        bits: 1,
        signed: false,
    };
    pub fn new(bits: u32, signed: bool) -> Self {
        assert!((1..=128).contains(&bits));
        Self { bits, signed }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Xor,
    Shl,
    Shr,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signed_division_and_remainder_follow_rust() {
        let e = Engine::new();
        let ty = IntType::new(64, true);
        for (a, b) in [(-7i64, 2i64), (7, -2), (-7, -2), (7, 2), (-1, 2), (0, -2)] {
            e.push();
            let left = e.constant(a as u128, ty);
            let right = e.constant(b as u128, ty);
            for (op, result) in [(Op::Div, a / b), (Op::Rem, a % b)] {
                let value = e.binary(op, &left, &right).0;
                assert!(
                    e.prove(&e.equal(&value, &e.constant(result as u128, ty))),
                    "{a} {op:?} {b}"
                );
            }
            e.pop();
        }
    }
    #[test]
    fn overflow_and_wrapping_are_distinct() {
        let e = Engine::new();
        let ty = IntType::new(8, false);
        let (value, overflow) = e.binary(Op::Add, &e.constant(255, ty), &e.constant(1, ty));
        assert!(e.prove(&e.equal(&value, &e.constant(0, ty))));
        assert!(e.prove(&e.test(&overflow, true)));
    }
    #[test]
    fn casts_truncate_and_sign_extend() {
        let e = Engine::new();
        let signed = e.constant(255, IntType::new(8, true));
        let extended = e.cast(&signed, IntType::new(64, true));
        assert!(e.prove(&e.equal(&extended, &e.constant((-1i64) as u128, extended.ty))));
        let truncated = e.cast(
            &e.constant(256, IntType::new(32, false)),
            IntType::new(8, false),
        );
        assert!(e.prove(&e.equal(&truncated, &e.constant(0, truncated.ty))));
    }
    #[test]
    fn solver_invocation_distinguishes_relational_reasoning_from_intervals() {
        let e = Engine::new();
        let ty = IntType::new(32, false);
        let x = e.fresh(ty);
        let y = e.fresh(ty);
        let relation = e.binary(Op::Gt, &x, &y).0;
        e.assume(&e.test(&relation, true));
        let (difference, overflow) = e.binary(Op::Sub, &x, &y);
        let nonzero = e.not(&e.equal(&difference, &e.constant(0, ty)));
        assert_eq!(e.prove(&nonzero), cfg!(feature = "smt"));
        assert_eq!(e.prove(&e.test(&overflow, false)), cfg!(feature = "smt"));
        assert_eq!(e.queries(), if cfg!(feature = "smt") { 2 } else { 0 });
    }
    #[test]
    #[cfg(feature = "smt")]
    fn full_width_u128_and_i128_bounds() {
        let e = Engine::new();
        for ty in [IntType::new(128, false), IntType::new(128, true)] {
            let max = if ty.signed {
                i128::MAX as u128
            } else {
                u128::MAX
            };
            let (value, overflow) = e.binary(Op::Add, &e.constant(max, ty), &e.constant(1, ty));
            assert!(e.prove(&e.test(&overflow, true)));
            assert!(e.prove(&e.equal(
                &value,
                &e.constant(if ty.signed { i128::MIN as u128 } else { 0 }, ty)
            )));
        }
    }
}
