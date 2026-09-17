//! # Deprecated
//!
//! This is the native `.nir` compiler. It is **deprecated** in favor
//! of the v2 Rust dialect — `crates/cargo-nirdosha` (the verifier),
//! `crates/nirdosha-rt` (the runtime), and `crates/nirdosha-driver`
//! (the Stage-2 MIR checker). Neither `.github/workflows/release.yml`
//! nor the `ghcr.io/protobox/nirdosha-runtime` Docker image exist any
//! more — this crate is no longer built or published as a product.
//! It remains in the workspace as source, still tested by CI, for
//! reference and its own still-real Z3/contract-check machinery; it
//! has no dependency relationship with the v2 dialect in either
//! direction, and none is planned.

pub mod ast;
pub mod capabilities;
pub mod codegen;
pub mod contract_check;
pub mod crud_gen;
pub mod effects;
pub mod explain;
pub mod extraction_schema;
pub mod grammar_gen;
pub mod grammar_trace;
pub mod guarantee_manifest;
pub mod init;
pub mod loader;
pub mod migrate;
pub mod ownership;
pub mod parser;
pub mod plugin;
pub mod rqlite;
pub mod refine;
pub mod smt;
pub mod token;
pub mod typeck;
pub mod ui_gen;
pub mod ui_plugin;
pub mod verify_pipeline;
pub mod workflow_conformance;
pub mod workflow_lower;

/// Structured, per-stage diagnostics for tooling that wants more than a
/// printable string — today just the typecheck stage, since that's the
/// only stage `typeck::validate_fragment` (single-expression-fragment
/// re-validation, `docs/goal.md` row 9) ever produces. Used to also carry
/// `Lex`/`Parse`/`Runtime` variants for the interpreter's `run_diagnostic`
/// family; both are gone along with the interpreter (see git history if
/// that shape is ever needed again for the compiled path).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "stage", content = "diagnostic")]
pub enum Diagnostic {
    Type(typeck::TypeError),
}

impl Diagnostic {
    pub fn span(&self) -> token::Span {
        match self {
            Diagnostic::Type(e) => e.span,
        }
    }
}
