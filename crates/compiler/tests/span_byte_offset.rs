//! `Span.byte` sanity check: does the lexer's byte offset actually
//! locate the same text `line`/`col` describe? This is the load-bearing
//! property `nirdosha fix`'s byte-offset `FixPatch`es depend on -- a
//! wrong byte offset would silently corrupt whatever source range a
//! patch splices into.

use nirdosha::token::Lexer;

#[test]
fn byte_offset_slices_out_the_exact_token_text() {
    let src = "fn add(a: i64, b: i64) -> i64 {\n    return a + b\n}\n";
    let toks = Lexer::new(src).tokenize().expect("lex should succeed");

    // `fn`
    assert_eq!(toks[0].span.byte, 0);
    assert_eq!(&src[toks[0].span.byte..toks[0].span.byte + 2], "fn");

    // `add`
    assert_eq!(&src[toks[1].span.byte..toks[1].span.byte + 3], "add");

    // `return`, on line 2 -- byte offset must land past the newline,
    // not be confused with `col` (which resets per line, `byte` never
    // does).
    let return_tok = toks.iter().find(|t| matches!(t.tok, nirdosha::token::Tok::Return)).expect("return token");
    assert_eq!(&src[return_tok.span.byte..return_tok.span.byte + 6], "return");
    assert_eq!(return_tok.span.line, 2);
}

#[test]
fn byte_offset_advances_past_multi_byte_utf8_correctly() {
    // A non-ASCII string literal before an identifier -- `byte` must
    // count real UTF-8 bytes (the emoji is 4 bytes), not chars, or the
    // next token's slice would land mid-codepoint.
    let src = "let x: str = \"caf\u{00e9}\"\nlet y = z\n";
    let toks = Lexer::new(src).tokenize().expect("lex should succeed");
    let y_tok = toks
        .iter()
        .find(|t| matches!(&t.tok, nirdosha::token::Tok::Ident(n) if n == "y"))
        .expect("ident `y`");
    assert_eq!(&src[y_tok.span.byte..y_tok.span.byte + 1], "y");
}
