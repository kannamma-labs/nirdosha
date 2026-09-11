//! Regression tests for parse diagnostics' found-token rendering:
//! every "found ..." message must name the *source text* the
//! programmer actually typed, and reserved words must say they are
//! reserved. Root cause this locks in (a real `hi` :generate give-up,
//! see `token.rs`'s `Display for Tok` doc comment): a struct field
//! named `state` used to answer `expected identifier, found State` --
//! `Tok`'s derived-Debug variant name -- which the LLM self-repair
//! loop in `hi_llm.rs` feeds straight back to the model. The model
//! can't act on a Rust enum name it never saw, so the identical
//! failure burned every retry. These tests pin the *readable* form
//! so the old one can't quietly come back with a new token variant.

use nirdosha::parser::Parser;
use nirdosha::token::Lexer;

fn parse_error(src: &str) -> nirdosha::parser::ParseError {
    let toks = Lexer::new(src).tokenize().expect("lex should succeed");
    Parser::new(toks).parse_program().expect_err("this source must fail to parse")
}

/// The exact failure from the give-up run: `state` (a real reserved
/// keyword -- it names a `workflow` state-machine state) as a struct
/// field name. The diagnostic must name the word and flag it as
/// reserved, or a repairing agent just sees an internal enum name.
#[test]
fn reserved_keyword_as_field_name_names_the_word_itself() {
    let err = parse_error(
        "struct Player {\n    state: str,\n    score: i64,\n}\nfn main() { }\n",
    );
    assert!(
        err.message.contains("expected identifier, found the reserved keyword `state`"),
        "the diagnostic must name the exact source text and flag it reserved, got: {}",
        err.message
    );
    // The span is the repair loop's other half: without the line/col
    // the model can't find *which* occurrence to rename.
    assert_eq!((err.span.line, err.span.col), (2, 5), "span must point at the offending field, got {:?}", err.span);
}

/// Same class, different word: `open` (a `Ty::File`-opening keyword)
/// is ordinary app vocabulary -- exactly the shape a model reaches for
/// as a field name (`open: bool` on an order, a connection, a ticket).
#[test]
fn other_app_vocabulary_keywords_get_the_same_readable_form() {
    let err = parse_error("struct Order {\n    open: bool,\n}\nfn main() { }\n");
    assert!(
        err.message.contains("found the reserved keyword `open`"),
        "got: {}",
        err.message
    );
    let err = parse_error("struct Server {\n    serve: bool,\n}\nfn main() { }\n");
    assert!(
        err.message.contains("found the reserved keyword `serve`"),
        "got: {}",
        err.message
    );
}

/// `let` names collide too, and a keyword is equally unusable as a
/// `fn`'s own name -- one rendering class, several collision sites.
#[test]
fn reserved_keyword_as_let_name_and_fn_name() {
    let err = parse_error("fn f() -> i64 {\n    let state: i64 = 1\n    return state\n}\nfn main() { }\n");
    assert!(
        err.message.contains("expected identifier, found the reserved keyword `state`"),
        "got: {}",
        err.message
    );
    let err = parse_error("fn state() -> i64 { return 1 }\nfn main() { }\n");
    assert!(
        err.message.contains("expected identifier, found the reserved keyword `state`"),
        "got: {}",
        err.message
    );
}

/// Scalar type names are reserved as well (`i64`/`str`/... lex as
/// their own `TypeName` token, so they can never be identifiers), and
/// the rendering must say *why* a plain backticked word would read as
/// a legal identifier to a repairing agent that doesn't know better.
#[test]
fn scalar_type_name_as_identifier_is_flagged_as_reserved() {
    let err = parse_error("struct S {\n    str: i64,\n}\nfn main() { }\n");
    assert!(
        err.message.contains("found the reserved type name `str`"),
        "got: {}",
        err.message
    );
}

/// Non-keyword finds render as their own source text too -- the
/// rule-9 match-arm-block trap (`found LBrace` before) and end-of-file
/// (`found Eof`) are the two an agent meets most after keywords.
#[test]
fn symbols_and_eof_render_as_source_text() {
    let err = parse_error(
        "fn f() -> i64 {\n    let x: i64 = match ok(3) {\n        Ok(v) => { let a = v a },\n        Err(e) => -1,\n    }\n    return x\n}\nfn main() { }\n",
    );
    assert!(
        err.message.contains("expected an expression, found `{`"),
        "got: {}",
        err.message
    );
    let err = parse_error("struct S {\n");
    assert!(
        err.message.contains("expected identifier, found end of file"),
        "got: {}",
        err.message
    );
}