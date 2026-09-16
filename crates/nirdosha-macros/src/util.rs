//! Small parsing helpers shared by this crate's function-like macros
//! (`categorical_actions!`, `dashboard!`, `crud_screens!`) — each one
//! hand-parses a `key: value, ...` block via `syn`, not a doc-comment
//! JSON blob, so a malformed invocation is a real `rustc` parse error.

use syn::parse::ParseStream;
use syn::Ident;

pub fn expect_keyword(input: ParseStream, expected: &str) -> syn::Result<()> {
    let ident: Ident = input.parse()?;
    if ident != expected {
        return Err(syn::Error::new(
            ident.span(),
            format!("expected `{expected}`, found `{ident}`"),
        ));
    }
    Ok(())
}
