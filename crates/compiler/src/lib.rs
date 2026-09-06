pub mod ast;
pub mod codegen;
pub mod contract_check;
pub mod crud_gen;
pub mod effects;
pub mod extraction_schema;
pub mod init;
pub mod instance_lock;
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
