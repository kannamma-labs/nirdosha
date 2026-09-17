//! Issue #68: `#[nirdosha_rt::contract(requires(..)/ensures(..))]` (the
//! attribute-macro authoring form, not just the doc-comment form
//! `nirdosha-driver`'s own tests exercise) expands to ordinary, runnable
//! Rust under a stock toolchain -- the predicate rides along as a
//! doc-encoded string, plus a dead sibling fn that makes plain rustc
//! type-check it against this fn's real parameter/return types right
//! now, at macro-expansion time (an undeclared identifier or a
//! non-boolean expression is a real compile error under *both*
//! compilers, exactly like every other contract clause).

#[nirdosha_rt::contract(requires(y != 0))]
fn safe_div(x: i64, y: i64) -> i64 {
    x / y
}

#[nirdosha_rt::contract(ensures(result >= 0))]
fn magnitude(x: i32) -> i32 {
    if x < 0 { -x } else { x }
}

#[nirdosha_rt::contract(requires(y > 0), ensures(result >= x))]
fn add_positive(x: i64, y: i64) -> i64 {
    x + y
}

#[test]
fn requires_and_ensures_expand_to_ordinary_callable_rust() {
    assert_eq!(safe_div(10, 2), 5);
    assert_eq!(magnitude(-3), 3);
    assert_eq!(magnitude(3), 3);
    assert_eq!(add_positive(2, 3), 5);
}
