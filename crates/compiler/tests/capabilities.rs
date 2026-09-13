//! The "what can this compiler pass" check (see
//! `nirdosha::capabilities`'s own doc comment for why this exists
//! alongside `paste_prompt_recipes.rs` rather than instead of it).
//!
//! Run `cargo test --test capabilities -- --nocapture` for a live
//! markdown table of every checked construct against the compiler you
//! actually built -- useful as a source-of-truth check before trusting
//! `paste-anywhere-prompt.md`'s prose, or after touching the parser/
//! typechecker/codegen. In CI this just fails the moment a construct
//! `nirdosha::capabilities::compiler_capabilities()` claims works stops
//! compiling.

use nirdosha::capabilities::{format_report, run_capability_checks};

#[test]
fn compiler_capability_report() {
    let results = run_capability_checks();
    let report = format_report(&results);
    println!("\n{report}");

    let failed: Vec<&str> = results.iter().filter(|r| !r.passed).map(|r| r.name).collect();
    assert!(failed.is_empty(), "capability regression -- these constructs no longer compile against the current build: {failed:?}\n\n{report}");
}
