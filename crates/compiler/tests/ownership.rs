//! Tests for `ownership.rs` — docs/goal.md row 1's static move-checker. Each
//! test name states the property being proved; see `ownership.rs`'s module
//! doc for why the branch-merge and loop-double-pass cases specifically
//! need their own coverage (both were real bugs caught while writing this
//! module, not hypothetical edge cases added for completeness).

use nirdosha::ownership::{check_ownership, OwnershipErrorKind};
use nirdosha::parser::Parser;
use nirdosha::token::Lexer;
use nirdosha::typeck::{typecheck, TypeErrorKind};

fn parse_ok(src: &str) -> nirdosha::ast::Program {
    let toks = Lexer::new(src).tokenize().expect("lex should succeed");
    let program = Parser::new(toks).parse_program().expect("parse should succeed");
    typecheck(&program).expect("should typecheck cleanly before ownership-checking it");
    program
}

fn first_ownership_error(src: &str) -> OwnershipErrorKind {
    let program = parse_ok(src);
    match check_ownership(&program) {
        Ok(()) => panic!("expected an ownership error, but none was found"),
        Err(errors) => errors.into_iter().next().unwrap().kind,
    }
}

/// For cases that are rejected by `typeck.rs` itself (before ownership
/// checking would even run), not by `ownership.rs` — doesn't go through
/// `parse_ok`, since that asserts a clean typecheck.
fn first_type_error(src: &str) -> TypeErrorKind {
    let toks = Lexer::new(src).tokenize().expect("lex should succeed");
    let program = Parser::new(toks).parse_program().expect("parse should succeed");
    match typecheck(&program) {
        Ok(()) => panic!("expected a type error, but the program type-checked cleanly"),
        Err(errors) => errors.into_iter().next().unwrap().kind,
    }
}

// ---- shared borrows (`&`) ----------------------------------------------

#[test]
fn moving_affine_content_out_through_a_shared_reference_is_rejected() {
    // The one real rule this increment needs: `*r` for `r: &box T` is a
    // type error, not an ownership error -- you can't move out of a
    // shared reference at all, regardless of move-state, the same rule
    // real Rust enforces (`*r` for `r: &Box<T>` needs `T: Copy` or an
    // explicit `.clone()`).
    let kind = first_type_error(
        r#"
        fn main() -> i64 {
            let b: box i64 = box 5
            let r: &box i64 = &b
            let stolen: box i64 = *r
            return *stolen
        }
    "#,
    );
    assert_eq!(kind, TypeErrorKind::CannotMoveOutOfReference { content: nirdosha::ast::Ty::Box(Box::new(nirdosha::ast::Ty::I64)) });
}

// ---- borrow liveness (v1, lexical) — ownership.rs's own "Borrow
// liveness" doc section explains the real use-after-free this closes
// and exactly what it does/doesn't cover -----------------------------

#[test]
fn moving_a_let_bound_borrows_referent_is_rejected() {
    // The real bug this closes: `r` still holds `b`'s own address after
    // `consume(b)` frees it (`codegen.rs` hands out the same address
    // twice) -- accepting this would compile a genuine, live
    // use-after-free, not a theoretical one.
    let kind = first_ownership_error(
        r#"
        fn consume(b: box i64) -> i64 { return *b }
        fn main() -> i64 {
            let b: box i64 = box 7
            let r: &box i64 = &b
            let used: i64 = consume(b)
            return used
        }
    "#,
    );
    assert_eq!(kind, OwnershipErrorKind::MoveWhileBorrowed { name: "b".to_string(), borrower: "r".to_string() });
}

#[test]
fn moving_after_the_borrows_own_scope_ends_is_allowed() {
    // `r` is declared inside the `if` block, so its own lexical scope
    // (and, in v1's model, the borrow's entire tracked lifetime) ends
    // when that block closes -- `consume(b)` after the `if` sees no
    // live borrow of `b` at all and must be allowed. This is the
    // regression case for the "lexical, not a true liveness analysis"
    // scope-cut `ownership.rs`'s own doc comment discloses: a real
    // NLL-style checker would additionally accept `r` used *inside* the
    // block after `b` is later moved outside it (still impossible
    // here) -- but must never *reject* this strictly simpler case.
    // `hold` merely takes the reference and never reads through it --
    // `&box T` is borrow-and-pass-around-only in this language today,
    // not read-through (`ownership.rs`'s own module doc, "Known
    // limitation: no place-expression semantics"), so `*r` for `r:
    // &box i64` is a *type* error regardless of this feature, unrelated
    // to what this test is actually proving.
    let program = parse_ok(
        r#"
        fn hold(r: &box i64) -> bool { return true }
        fn consume(b: box i64) -> i64 { return *b }
        fn main() -> i64 {
            let b: box i64 = box 7
            let cond: bool = true
            if cond {
                let r: &box i64 = &b
                let held: bool = hold(r)
            }
            let used: i64 = consume(b)
            return used
        }
    "#,
    );
    check_ownership(&program).expect("the borrow's own scope already ended before the move");
}

#[test]
fn a_borrow_taken_only_as_a_bare_call_argument_does_not_block_a_later_move() {
    // v1 only tracks a borrow created by a `let` initializer
    // (`Stmt::Let`'s own handler) -- `hold(&b)` here is a temporary,
    // never bound to a name, so it creates nothing for `is_borrowed` to
    // find. This is the same "real, disclosed limitation, not a false
    // positive" this module's own doc comment names -- must keep
    // passing cleanly, exactly as it did before this feature existed.
    let program = parse_ok(
        r#"
        fn hold(r: &box i64) -> bool { return true }
        fn consume(b: box i64) -> i64 { return *b }
        fn main() -> i64 {
            let b: box i64 = box 7
            let x: bool = hold(&b)
            let used: i64 = consume(b)
            return used
        }
    "#,
    );
    check_ownership(&program).expect("a bare call-argument borrow must never block a later move");
}

// ---- `froze` — RFC 0006 Pillar 1's `Froze<T>` -------------------------

#[test]
fn moving_affine_content_out_through_a_froze_handle_is_rejected() {
    // `Ty::Froze` gets exactly `Ty::Ref`'s own `CannotMoveOutOfReference`
    // rule, for the identical reason: a `froze` handle isn't affine (it
    // may have other live copies), so extracting affine content out of
    // it *by value* would silently duplicate ownership.
    let kind = first_type_error(
        r#"
        fn main() -> i64 {
            let f: froze box i64 = froze box 5
            let stolen: box i64 = *f
            return *stolen
        }
    "#,
    );
    assert_eq!(kind, TypeErrorKind::CannotMoveOutOfReference { content: nirdosha::ast::Ty::Box(Box::new(nirdosha::ast::Ty::I64)) });
}

#[test]
fn a_froze_handle_is_freely_copyable_not_affine() {
    // Unlike `box` (see `use_after_move_via_function_call_is_rejected`
    // just below), passing the *same* `froze` binding to two separate
    // calls is not a move at all — `Ty::Froze` is deliberately not
    // affine (Pillar 1's whole point: shared, immutable, any number of
    // holders). This must ownership-check cleanly.
    let program = parse_ok(
        r#"
        fn read_it(f: froze i64) -> i64 {
            return *f
        }
        fn main() -> i64 {
            let f: froze i64 = froze 21
            let a: i64 = read_it(f)
            let b: i64 = read_it(f)
            return a + b
        }
    "#,
    );
    check_ownership(&program).expect("a froze handle used twice must not be an ownership error");
}

#[test]
fn borrowing_a_non_identifier_is_a_parse_error() {
    let toks = Lexer::new("fn main() { let x: i64 = &(1 + 1) }").tokenize().unwrap();
    let result = Parser::new(toks).parse_program();
    assert!(result.is_err(), "borrowing a non-identifier expression must be rejected");
}

#[test]
fn reference_to_reference_is_rejected_even_with_a_space() {
    // docs/GRAMMAR.md documents two independent, stacked limitations here:
    // `&&x` can't even be *written* (lexes as one AndAnd token), but
    // writing it with a space instead (`& &x`) genuinely does lex as two
    // separate `&` tokens -- and is *still* rejected, by the separate
    // Ident-only restriction on `&`'s operand (`&x`'s own operand here is
    // `Expr::Ref(...)`, not a bare identifier). This test is the second
    // limitation, isolated from the lexer one.
    let toks = Lexer::new("fn main() { let n: i64 = 5\nlet r: &i64 = & &n }").tokenize().unwrap();
    let result = Parser::new(toks).parse_program();
    assert!(result.is_err(), "`& &x` must be rejected even though it lexes as two Amp tokens");
}

// ---- the basics: box, deref, move -----------------------------------

#[test]
fn use_after_move_via_let_is_rejected() {
    let kind = first_ownership_error(
        r#"
        fn main() -> i64 {
            let b: box i64 = box 7
            let c: box i64 = b
            return *b
        }
    "#,
    );
    assert_eq!(kind, OwnershipErrorKind::UseAfterMove { name: "b".to_string() });
}

#[test]
fn use_after_move_via_function_call_is_rejected() {
    // Passing an affine value to a function by name moves it, same as a
    // `let` — this test is the "call argument" moving-position, distinct
    // from the "let initializer" one above.
    let kind = first_ownership_error(
        r#"
        fn consume(b: box i64) -> i64 {
            return *b
        }
        fn main() -> i64 {
            let b: box i64 = box 7
            let first: i64 = consume(b)
            let second: i64 = consume(b)
            return first + second
        }
    "#,
    );
    assert_eq!(kind, OwnershipErrorKind::UseAfterMove { name: "b".to_string() });
}

// ---- nested boxes: a real gap found while testing, fixed and pinned ---

#[test]
fn dereferencing_a_nested_box_twice_is_use_after_move() {
    // `*bb` for `bb: box box i64` hands out the *inner* `box i64` by
    // value — itself affine — so unlike `*b` for a scalar box, this has
    // to consume `bb`. A first draft of `ownership.rs` exempted every
    // deref unconditionally and would have accepted this; see the
    // `Expr::Deref` arm of `touch_expr` for the fix.
    let kind = first_ownership_error(
        r#"
        fn main() -> i64 {
            let bb: box box i64 = box box 5
            let a: box i64 = *bb
            let b: box i64 = *bb
            return *a + *b
        }
    "#,
    );
    assert_eq!(kind, OwnershipErrorKind::UseAfterMove { name: "bb".to_string() });
}

// ---- branch-merge: docs/goal.md-relevant regression coverage --------------

#[test]
fn moving_in_only_one_if_branch_still_poisons_later_use() {
    // `b` is moved only inside the `then` branch. The checker can't know
    // at compile time whether `cond` is true or false, so it has to
    // assume the worse case for anything after the `if` — this is what
    // `merge_moved` exists for.
    let kind = first_ownership_error(
        r#"
        fn sink(b: box i64) -> i64 { return *b }
        fn main() -> i64 {
            let b: box i64 = box 1
            let cond: bool = true
            if cond {
                let used: i64 = sink(b)
            }
            return *b
        }
    "#,
    );
    assert_eq!(kind, OwnershipErrorKind::UseAfterMove { name: "b".to_string() });
}

// ---- the loop double-pass: catches a second-iteration-only move -------

#[test]
fn moving_a_pre_loop_variable_inside_the_body_is_rejected() {
    // `b` is declared before the loop and moved on the *first* iteration
    // of the body. A checker that only examined the body once, from the
    // pre-loop state, would accept this (iteration 1 looks fine in
    // isolation) — it's only wrong because the body would run again. This
    // is exactly the bug `ownership.rs`'s module doc describes catching
    // during development; this test is what would have caught it.
    let kind = first_ownership_error(
        r#"
        fn sink(b: box i64) -> i64 { return *b }
        fn main() -> i64 {
            let b: box i64 = box 1
            let n: i64 = 0
            while n < 3 {
                let used: i64 = sink(b)
                n = n + 1
            }
            return n
        }
    "#,
    );
    assert_eq!(kind, OwnershipErrorKind::UseAfterMove { name: "b".to_string() });
}

#[test]
fn all_examples_pass_ownership_checking() {
    // The three Phase 0 examples don't use `box` at all, so for those this
    // is really a "the checker doesn't false-positive on ordinary scalar
    // code" test; `ownership.nir` is the one that actually exercises moves.
    for src in [
        include_str!("fixtures/hello.nir"),
        include_str!("fixtures/factorial.nir"),
        include_str!("fixtures/loop.nir"),
        include_str!("fixtures/ownership.nir"),
        include_str!("fixtures/borrow.nir"),
        include_str!("fixtures/threads.nir"),
        include_str!("fixtures/channels.nir"),
        include_str!("fixtures/sandbox.nir"),
        include_str!("fixtures/sandbox_channels.nir"),
        include_str!("fixtures/strings.nir"),
        include_str!("fixtures/tcp_client.nir"),
    ] {
        let program = parse_ok(src);
        assert_eq!(check_ownership(&program), Ok(()));
    }
}
