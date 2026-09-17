//! Deliberately weaker, Z3-independent substitution. Endpoints wider than
//! i128 (u128), wrapping ranges, or unsupported operations become unknown.
use crate::{IntType, Op};
pub const BACKEND: &str = "interval";
#[derive(Clone)]
pub struct Number {
    pub ty: IntType,
    range: Option<(i128, i128)>,
}
#[derive(Clone)]
pub struct Predicate(Option<bool>);
pub struct Engine;
fn bounds(t: IntType) -> Option<(i128, i128)> {
    if t.signed {
        Some(if t.bits == 128 {
            (i128::MIN, i128::MAX)
        } else {
            (-(1i128 << (t.bits - 1)), (1i128 << (t.bits - 1)) - 1)
        })
    } else if t.bits == 128 {
        None
    } else {
        Some((0, ((1u128 << t.bits) - 1) as i128))
    }
}
impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}
impl Engine {
    pub fn new() -> Self {
        Self
    }
    pub fn queries(&self) -> usize {
        0
    }
    pub fn push(&self) {}
    pub fn pop(&self) {}
    pub fn assume(&self, _: &Predicate) {}
    pub fn prove(&self, p: &Predicate) -> bool {
        p.0 == Some(true)
    }
    pub fn fresh(&self, ty: IntType) -> Number {
        Number {
            ty,
            range: bounds(ty),
        }
    }
    pub fn constant(&self, bits: u128, ty: IntType) -> Number {
        let value = if ty.signed {
            Some(if ty.bits == 128 {
                bits as i128
            } else {
                ((bits << (128 - ty.bits)) as i128) >> (128 - ty.bits)
            })
        } else {
            i128::try_from(if ty.bits == 128 {
                bits
            } else {
                bits & ((1u128 << ty.bits) - 1)
            })
            .ok()
        };
        Number {
            ty,
            range: value.map(|v| (v, v)),
        }
    }
    pub fn cast(&self, n: &Number, ty: IntType) -> Number {
        if let Some((lo, hi)) = n.range {
            if lo == hi {
                return self.constant(lo as u128, ty);
            }
            if bounds(ty).is_some_and(|(min, max)| lo >= min && hi <= max) {
                return Number { ty, range: n.range };
            }
        }
        self.fresh(ty)
    }
    pub fn equal(&self, a: &Number, b: &Number) -> Predicate {
        Predicate(match (a.range, b.range) {
            (Some((l, h)), Some((r, s))) if h < r || s < l => Some(false),
            (Some((l, h)), Some((r, s))) if l == h && r == s => Some(l == r),
            _ => None,
        })
    }
    pub fn not(&self, p: &Predicate) -> Predicate {
        Predicate(p.0.map(|v| !v))
    }
    pub fn test(&self, n: &Number, expected: bool) -> Predicate {
        self.equal(n, &self.constant(expected as u128, IntType::BOOL))
    }
    fn boolean(&self, p: Option<bool>) -> Number {
        p.map(|v| self.constant(v as u128, IntType::BOOL))
            .unwrap_or_else(|| self.fresh(IntType::BOOL))
    }
    pub fn negate(&self, n: &Number) -> Number {
        self.binary(Op::Sub, &self.constant(0, n.ty), n).0
    }
    pub fn invert(&self, n: &Number) -> Number {
        if let Some((l, h)) = n.range {
            if l == h {
                return self.constant(!(l as u128), n.ty);
            }
        }
        self.fresh(n.ty)
    }
    pub fn binary(&self, op: Op, a: &Number, b: &Number) -> (Number, Number) {
        let unknown = || (self.fresh(a.ty), self.fresh(IntType::BOOL));
        let (Some((l, h)), Some((r, s))) = (a.range, b.range) else {
            return if matches!(op, Op::Eq | Op::Ne | Op::Lt | Op::Le | Op::Gt | Op::Ge) {
                (self.fresh(IntType::BOOL), self.fresh(IntType::BOOL))
            } else {
                unknown()
            };
        };
        let p = match op {
            Op::Eq => self.equal(a, b).0,
            Op::Ne => self.not(&self.equal(a, b)).0,
            Op::Lt => {
                if h < r {
                    Some(true)
                } else if l >= s {
                    Some(false)
                } else {
                    None
                }
            }
            Op::Le => {
                if h <= r {
                    Some(true)
                } else if l > s {
                    Some(false)
                } else {
                    None
                }
            }
            Op::Gt => {
                if l > s {
                    Some(true)
                } else if h <= r {
                    Some(false)
                } else {
                    None
                }
            }
            Op::Ge => {
                if l >= s {
                    Some(true)
                } else if h < r {
                    Some(false)
                } else {
                    None
                }
            }
            _ => None,
        };
        if matches!(op, Op::Eq | Op::Ne | Op::Lt | Op::Le | Op::Gt | Op::Ge) {
            return (self.boolean(p), self.constant(0, IntType::BOOL));
        }
        let range = match op {
            Op::Add => l.checked_add(r).zip(h.checked_add(s)),
            Op::Sub => l.checked_sub(s).zip(h.checked_sub(r)),
            Op::Mul => [
                l.checked_mul(r),
                l.checked_mul(s),
                h.checked_mul(r),
                h.checked_mul(s),
            ]
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .map(|v| (*v.iter().min().unwrap(), *v.iter().max().unwrap())),
            Op::Div if r == s && r != 0 => l
                .checked_div(r)
                .zip(h.checked_div(r))
                .map(|(x, y)| (x.min(y), x.max(y))),
            Op::Rem if l == h && r == s => l.checked_rem(r).map(|v| (v, v)),
            _ => None,
        };
        let Some((lo, hi)) = range else {
            return unknown();
        };
        let Some((min, max)) = bounds(a.ty) else {
            return unknown();
        };
        let overflow = if lo >= min && hi <= max {
            Some(false)
        } else if hi < min || lo > max {
            Some(true)
        } else {
            None
        };
        let value = if lo == hi {
            self.constant(lo as u128, a.ty)
        } else if overflow == Some(false) {
            Number {
                ty: a.ty,
                range: Some((lo, hi)),
            }
        } else {
            self.fresh(a.ty)
        };
        (value, self.boolean(overflow))
    }
}
