//! Multi-file `use` resolution (`docs/ROADMAP.md` Track F, F2 piece 3;
//! `docs/NEXT_GEN.md` §F2). Every other pass in this compiler (`typeck`,
//! `interpreter`, `ui_gen`, `codegen`) still only ever sees one flat
//! `Program`, exactly as before F2 — this module is what produces that
//! one `Program` from however many files a `use "path.nir"` graph
//! actually spans, before any of them run.
//!
//! Deliberately separate from `parser::parse_program` (one file/token-
//! stream only) and from `lib.rs`'s `run*`/`run_diagnostic*` family
//! (which take a bare `src: &str` with no file path to resolve a
//! relative import against, and are unchanged by F2 for exactly that
//! reason — a program with no `use` directives behaves identically
//! either way, since `Program.imports` is simply empty). Only
//! `main.rs`'s CLI, which always has a real file path on disk, and
//! `lib.rs::run_program_with_tracer_transact_and_workflow_log` (which
//! takes an already-loaded `Program`), go through this.

use crate::ast::{FnDecl, Program};
use crate::{parser, token};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Reads `entry_path`, parses it, and recursively resolves every
/// leading `use "..."` it declares. Returns the fully-merged `Program`
/// plus the *entry file's own* source text (an imported file's source
/// is never returned — nothing downstream needs it: neither `TypeError`
/// nor `ParseError` ever re-quotes a source snippet, only a `line:col`,
/// which is already meaningful relative to whichever file actually
/// produced it — see `resolve_imports`'s doc comment for why that's
/// enough for correct diagnostics with no multi-file source map).
/// Which phase a [`LoadDiagnostic`] came from -- drives both its
/// `Display` formatting (matching each phase's pre-existing prose
/// exactly, so [`load_program`]'s `String` wrapper stays byte-for-byte
/// unchanged) and whether a real source `Span` exists at all: `Io` and
/// `Import` failures have no in-source position (a missing file, an
/// import cycle), so their `span` stays the `0,0,0` placeholder that
/// was already implicit before this type existed.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum LoadStage {
    Io,
    Lex,
    Parse,
    Import,
}

/// Structured sibling of the plain `String` every load failure used to
/// flatten into immediately -- both `LexError`/`ParseError`'s real
/// `Span` used to be thrown away the moment `parse_one` formatted them
/// into a message (`mcp_tools::run_verify_pipeline`'s `VerifyDiagnostic`
/// hardcoded `line: 0, col: 0` for every load-stage error as a direct
/// result: there was nowhere else for the real position to go). Kept
/// alongside the original `String`-returning functions, not instead of
/// them -- [`load_program`] is a thin wrapper over
/// [`load_program_diag`] whose `Display` impl reproduces the exact same
/// text every existing caller already depends on.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LoadDiagnostic {
    pub stage: LoadStage,
    pub path: String,
    pub message: String,
    pub span: token::Span,
}

impl std::fmt::Display for LoadDiagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.stage {
            LoadStage::Io => write!(f, "error reading {}: {}", self.path, self.message),
            LoadStage::Lex => {
                write!(f, "lex error in {} at {}:{}: {}", self.path, self.span.line, self.span.col, self.message)
            }
            LoadStage::Parse => {
                write!(f, "parse error in {} at {}:{}: {}", self.path, self.span.line, self.span.col, self.message)
            }
            // Import-resolution failures (missing file, cycle, an
            // imported file's own load error) never carried a
            // `{path} at {line}:{col}:` prefix even before this type
            // existed -- `resolve_imports` builds its own fully-worded
            // `String` message per failure kind, so this just passes it
            // through unchanged.
            LoadStage::Import => write!(f, "{}", self.message),
        }
    }
}

/// See `entry_path`'s own doc comment on [`load_program`] -- identical
/// behavior, `Err` carries a [`LoadDiagnostic`] instead of a pre-
/// flattened `String`.
pub fn load_program_diag(entry_path: &str) -> Result<(Program, String), LoadDiagnostic> {
    let placeholder_span = token::Span { line: 0, col: 0, byte: 0 };
    let src = std::fs::read_to_string(entry_path).map_err(|e| LoadDiagnostic {
        stage: LoadStage::Io,
        path: entry_path.to_string(),
        message: e.to_string(),
        span: placeholder_span,
    })?;
    let mut program = parse_one_diag(&src, entry_path)?;
    let mut visited = HashSet::new();
    if let Ok(canon) = std::fs::canonicalize(entry_path) {
        visited.insert(canon);
    }
    resolve_imports(&mut program, entry_path, &mut visited).map_err(|message| LoadDiagnostic {
        stage: LoadStage::Import,
        path: entry_path.to_string(),
        message,
        span: placeholder_span,
    })?;
    Ok((program, src))
}

pub fn load_program(entry_path: &str) -> Result<(Program, String), String> {
    load_program_diag(entry_path).map_err(|e| e.to_string())
}

fn parse_one_diag(src: &str, path: &str) -> Result<Program, LoadDiagnostic> {
    let toks = token::Lexer::new(src).tokenize().map_err(|e| LoadDiagnostic {
        stage: LoadStage::Lex,
        path: path.to_string(),
        message: e.message,
        span: e.span,
    })?;
    parser::Parser::new(toks).parse_program().map_err(|e| LoadDiagnostic {
        stage: LoadStage::Parse,
        path: path.to_string(),
        message: e.message,
        span: e.span,
    })
}

fn parse_one(src: &str, path: &str) -> Result<Program, String> {
    parse_one_diag(src, path).map_err(|e| e.to_string())
}

/// Each `use "path.nir"` in `program` (already parsed, not yet
/// resolved) is loaded relative to `importing_path`'s own directory,
/// recursively resolves *its own* imports the same way, then
/// **typechecks standalone** — own source, own diagnostics, exactly as
/// if it were the entry file itself — before anything is merged. An
/// imported file must be independently valid on its own, full stop.
/// This is also what keeps every error's `line:col` correct with no
/// multi-file source map anywhere in the compiler: a `TypeError`/
/// `ParseError` raised while typechecking file X always already
/// carries X's own `line:col` (`typeck.rs`'s `Span`s are computed
/// purely from X's own token stream) — this fn only ever adds an "in
/// <path>:" prefix for context, never tries to re-render a source
/// excerpt (nothing in this compiler's error `Display` impls do that
/// at all, checked directly — `TypeError`/`ParseError` both print
/// exactly `"{line}:{col}: {message}"`, no source line quoted).
///
/// Only `pub`, real-namespace (`ns: Some(_)`) declarations are merged
/// into `program`'s own flat `fns`/`structs`/`enums` — a non-namespaced
/// (top-level) declaration in an imported file is never visible to the
/// importer, deliberately: only explicitly-exported, explicitly-
/// namespaced items ever cross a file boundary, so merging can never
/// reopen the flat-namespace collision risk `ns` exists to close (see
/// `ast::scope_key`'s doc comment).
fn resolve_imports(program: &mut Program, importing_path: &str, visited: &mut HashSet<PathBuf>) -> Result<(), String> {
    let base_dir = Path::new(importing_path).parent().unwrap_or_else(|| Path::new("."));
    let imports = std::mem::take(&mut program.imports);
    for imp in &imports {
        let target = base_dir.join(&imp.path);
        let canon = std::fs::canonicalize(&target)
            .map_err(|e| format!("error resolving `use \"{}\"` from {importing_path}: {e}", imp.path))?;
        if !visited.insert(canon.clone()) {
            return Err(format!("import cycle detected: `use \"{}\"` from {importing_path}", imp.path));
        }
        let target_str = target.to_string_lossy().to_string();
        let target_src = std::fs::read_to_string(&target).map_err(|e| format!("error reading {target_str}: {e}"))?;
        let mut imported = parse_one(&target_src, &target_str)?;
        // Nested imports resolve relative to *that* file's own
        // directory, recursively, before it's typechecked standalone —
        // same rule as the entry file's own imports, one shared
        // `visited` set threading cycle detection through the whole
        // graph regardless of depth.
        resolve_imports(&mut imported, &target_str, visited)?;
        if let Err(errors) = crate::typeck::typecheck_optional_main(&imported) {
            let joined = errors.iter().map(|e| format!("type error: {e}")).collect::<Vec<_>>().join("\n");
            return Err(format!("in {target_str}:\n{joined}"));
        }
        merge_exported(program, imported, &target_str)?;
    }
    Ok(())
}

/// Pulls every `pub`, real-namespace fn/struct/enum out of `imported`
/// into `program`. A namespace id (`ns`) already present in `program`
/// (from its own source, or from an earlier merged import) colliding
/// with one `imported` also declares is a real `Err`, not a silent
/// overwrite — two different files each declaring `module Audit { }`
/// is exactly as ambiguous as one file declaring it twice, and
/// `typeck.rs`'s own registration pass (keyed by `ast::scope_key`)
/// would only catch this *after* the silent overwrite already lost
/// one side's declarations, so it's caught explicitly, here, first.
fn merge_exported(program: &mut Program, imported: Program, from_path: &str) -> Result<(), String> {
    let mut existing_ns: HashSet<String> = HashSet::new();
    existing_ns.extend(program.fns.iter().filter_map(fn_ns));
    existing_ns.extend(program.structs.iter().filter_map(|s| s.ns.clone()));
    existing_ns.extend(program.enums.iter().filter_map(|e| e.ns.clone()));

    let mut incoming_ns: HashSet<String> = HashSet::new();
    incoming_ns.extend(imported.fns.iter().filter(|f| f.exported).filter_map(fn_ns));
    incoming_ns.extend(imported.structs.iter().filter(|s| s.exported).filter_map(|s| s.ns.clone()));
    incoming_ns.extend(imported.enums.iter().filter(|e| e.exported).filter_map(|e| e.ns.clone()));

    if let Some(dup) = incoming_ns.intersection(&existing_ns).next() {
        return Err(format!(
            "module `{dup}` (imported from {from_path}) collides with an already-declared module of the same name"
        ));
    }

    program.fns.extend(imported.fns.into_iter().filter(|f| f.ns.is_some() && f.exported));
    program.structs.extend(imported.structs.into_iter().filter(|s| s.ns.is_some() && s.exported));
    program.enums.extend(imported.enums.into_iter().filter(|e| e.ns.is_some() && e.exported));
    Ok(())
}

fn fn_ns(f: &FnDecl) -> Option<String> {
    f.ns.clone()
}
