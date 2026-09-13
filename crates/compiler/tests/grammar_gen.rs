//! The mechanically-generated-grammar pipeline, run for real: traces
//! the actual current parser over `examples/**/*.nir` plus every
//! `nirdosha::capabilities` snippet, folds the observed productions,
//! and reports what came out. Run `cargo test --test grammar_gen --
//! --nocapture` to see the generated EBNF/GBNF and the coverage
//! disclosure directly; `nirdosha grammar-export` runs the same
//! pipeline from the CLI and writes it to files.

use nirdosha::grammar_gen::{default_corpus, generate};

#[test]
fn corpus_parses_clean_and_grammar_is_generated() {
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let corpus = default_corpus(&repo_root);
    assert!(corpus.len() > 50, "expected a real corpus (examples/**/*.nir + capabilities), got {} items -- is the repo layout what this test expects?", corpus.len());

    let report = generate(&corpus);

    println!("\n--- covered rules ({}) ---\n{:?}", report.covered.len(), report.covered);
    println!("\n--- uncovered rules ({}) ---\n{:?}", report.uncovered.len(), report.uncovered);
    println!("\n--- EBNF ---\n{}", report.ebnf);
    println!("\n--- GBNF ---\n{}", report.gbnf);

    assert!(
        report.parse_errors.is_empty(),
        "corpus items failed to parse against the current compiler (fix the corpus item or the parser regressed):\n{}",
        report.parse_errors.iter().map(|(l, e)| format!("- {l}: {e}")).collect::<Vec<_>>().join("\n")
    );
}
