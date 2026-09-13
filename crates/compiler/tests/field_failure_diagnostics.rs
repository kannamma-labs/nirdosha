//! Regression tests for the "teach the fix at the source" pass: five
//! diagnostics an LLM importing a foreign-language idiom is likely to
//! hit (`let _ = expr`/`mut`, field/index assignment, a top-level
//! `for`, a JS/TS-flavored scalar type name, a `box` use-after-move),
//! each of which used to be a bare, unhelpful message and now names
//! the actual mistake and the fix. Same discipline
//! `reserved_word_diagnostics.rs` already pins for the reserved-word
//! case; these are the same class of regression lock, one per new
//! message.

use nirdosha::ownership::{check_ownership, OwnershipErrorKind};
use nirdosha::parser::{ParseError, Parser};
use nirdosha::token::Lexer;
use nirdosha::typeck::{typecheck, TypeErrorKind};

fn parse_error(src: &str) -> ParseError {
    let toks = Lexer::new(src).tokenize().expect("lex should succeed");
    Parser::new(toks).parse_program().expect_err("this source must fail to parse")
}

fn first_type_error(src: &str) -> TypeErrorKind {
    let toks = Lexer::new(src).tokenize().expect("lex should succeed");
    let program = Parser::new(toks).parse_program().expect("parse should succeed");
    match typecheck(&program) {
        Ok(()) => panic!("expected a type error, but the program type-checked cleanly"),
        Err(errors) => errors.into_iter().next().unwrap().kind,
    }
}

fn first_ownership_error(src: &str) -> OwnershipErrorKind {
    let toks = Lexer::new(src).tokenize().expect("lex should succeed");
    let program = Parser::new(toks).parse_program().expect("parse should succeed");
    typecheck(&program).expect("should typecheck cleanly before ownership-checking it");
    match check_ownership(&program) {
        Ok(()) => panic!("expected an ownership error, but none was found"),
        Err(errors) => errors.into_iter().next().unwrap().kind,
    }
}

// ---- `let` without a type / the Rust `let _ = expr` discard idiom -----

#[test]
fn let_discard_names_the_missing_type_and_the_missing_discard_form() {
    // The exact field failure this pass started from: `hi generate`
    // wrote Rust's `let _ = credit_cents(...)` idiom, which used to
    // fail with the bare, unhelpful `expected \`:\`, found \`=\``.
    let err = parse_error("fn credit_cents(a: i64, b: i64) -> i64 { return a + b }\nfn main() -> i64 {\n    let _ = credit_cents(1, 2)\n    return 0\n}\n");
    assert!(
        err.message.contains("`let` always needs an explicit type") && err.message.contains("no `let _ = expr` discard form"),
        "got: {}",
        err.message
    );
}

#[test]
fn let_mut_names_the_missing_mut_qualifier() {
    let err = parse_error("fn main() -> i64 {\n    let mut x: i64 = 0\n    return x\n}\n");
    assert!(err.message.contains("Nirdosha has no `mut` qualifier"), "got: {}", err.message);
}

#[test]
fn ordinary_let_with_a_type_still_parses() {
    let toks = Lexer::new("fn main() -> i64 {\n    let x: i64 = 5\n    return x\n}\n").tokenize().unwrap();
    Parser::new(toks).parse_program().expect("a normal, well-formed `let` must be unaffected");
}

// ---- field/index assignment --------------------------------------------

#[test]
fn field_assignment_names_the_rebuild_fix() {
    let err = parse_error(
        "struct Point { x: i64, y: i64 }\nfn main() -> i64 {\n    let p: Point = Point(1, 2)\n    p.x = 5\n    return p.x\n}\n",
    );
    assert!(
        err.message.contains("can't be assigned into") && err.message.contains("rebuild the whole value"),
        "got: {}",
        err.message
    );
}

// ---- foreign top-level keywords ----------------------------------------

#[test]
fn top_level_for_names_while_as_the_replacement() {
    let err = parse_error("for i in 0 {\n    print(i)\n}\n");
    assert!(err.message.contains("Nirdosha has no `for`") && err.message.contains("loop with `while`"), "got: {}", err.message);
}

#[test]
fn top_level_class_is_named_too() {
    let err = parse_error("class Foo {\n}\n");
    assert!(err.message.contains("Nirdosha has no `class`"), "got: {}", err.message);
}

// ---- foreign scalar type names ------------------------------------------

#[test]
fn js_style_number_type_suggests_i64() {
    let kind = first_type_error("fn main() -> number {\n    return 0\n}\n");
    match kind {
        TypeErrorKind::UnknownType(n) => assert_eq!(n, "number"),
        other => panic!("expected UnknownType, got {other:?}"),
    }
}

#[test]
fn js_style_number_type_message_suggests_i64() {
    let toks = Lexer::new("fn main() -> number {\n    return 0\n}\n").tokenize().unwrap();
    let program = Parser::new(toks).parse_program().unwrap();
    let err = typecheck(&program).expect_err("`number` must not typecheck").into_iter().next().unwrap();
    assert!(err.to_string().contains("did you mean `i64`?"), "got: {err}");
}

#[test]
fn genuinely_unknown_type_gets_no_wrong_guess() {
    let toks = Lexer::new("fn main() -> TotallyMadeUpType {\n    return 0\n}\n").tokenize().unwrap();
    let program = Parser::new(toks).parse_program().unwrap();
    let err = typecheck(&program).expect_err("an unknown type must still fail").into_iter().next().unwrap();
    assert!(!err.to_string().contains("did you mean"), "a name with no known foreign-type match must not get a guess, got: {err}");
}

// ---- `box` use-after-move -------------------------------------------------

#[test]
fn use_after_move_message_names_the_borrow_fix() {
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
    let err = nirdosha::ownership::OwnershipError { kind, span: nirdosha::token::Span { line: 1, col: 1, byte: 0 } };
    assert!(
        err.to_string().contains("affine") && err.to_string().contains("Borrow with `&b`"),
        "got: {err}"
    );
}
