//! Verification for **v2** `.nir` source: valid Rust plus the
//! `nirdosha:*` doc-comment declarative layer (`docs/
//! nirdosha-v2-comment-layer.md`), as opposed to `parser.rs`'s native
//! grammar, which this module never touches.
//!
//! There is no proprietary codegen for v2 yet (Phase 4 of the
//! migration doc — "proprietary consumes v2 natively" — is still
//! open), so the only truthful way to check a v2 candidate is the same
//! two readers the v2 corpus itself is held to:
//!
//! 1. **plain cargo** — does it build at all? Answered by actually
//!    invoking `cargo build` against a small persistent scratch
//!    package (`nirdosha-rt` as its only dependency), the same way
//!    `examples/nirdosha-v2-corpus` is built.
//! 2. **`cargo nirdosha`** — are its `nirdosha:contract` claims
//!    internally consistent (`effects`/`requires`/`nfr` shape, dialect
//!    restrictions)? Answered in-process via `cargo_nirdosha::
//!    verify_sources`, the exact library call `cargo nirdosha verify`
//!    itself runs — no subprocess, no re-implementation.
//!
//! `nirdosha:validate`/`workflow`/`screen`/... cross-referencing has no
//! checker yet anywhere in this codebase (the v2 corpus's own README
//! calls them "inert" — Phase 1 scanner teeth, not shipped) — this
//! module reports what's real today, not what the design doc plans.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The outcome of running a v2 candidate through both readers.
#[derive(Debug, Clone, serde::Serialize)]
pub struct V2Verdict {
    /// Whether `cargo build` accepted the candidate as-is.
    pub builds: bool,
    /// `rustc`'s own diagnostics, verbatim, when `builds` is false.
    pub build_diagnostic: Option<String>,
    /// How many `nirdosha:contract` doc comments were found (the only
    /// comment kind `cargo-nirdosha` actually models today).
    pub contracts_found: usize,
    /// `cargo-nirdosha`'s findings at `Error` severity — a malformed
    /// contract, an unknown field, a dialect restriction (`unsafe`,
    /// raw threads) — formatted `file:line message`.
    pub violations: Vec<String>,
}

impl V2Verdict {
    pub fn passed(&self) -> bool {
        self.builds && self.violations.is_empty()
    }

    /// A short, generic outcome string -- the same "the verdict a
    /// verify/fix/certify carries" signal `mcp_tools::McpCallLog`
    /// already extracts from every native verdict shape (`/verdict`),
    /// mirrored here so a v2 call logs just as legibly.
    pub fn verdict(&self) -> &'static str {
        if !self.builds {
            "build_failed"
        } else if !self.violations.is_empty() {
            "violations_found"
        } else {
            "clean"
        }
    }
}

/// The workspace root: three levels up from this crate's manifest dir
/// (`crates/compiler` -> `crates` -> repo root), computed at compile
/// time the same way other `include_str!`-adjacent paths in this crate
/// are anchored.
fn nirdosha_rt_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/compiler has a parent")
        .join("nirdosha-rt")
}

/// A dedicated scratch package outside the real workspace tree (own
/// `[workspace]` table, own target dir) — never a member of the root
/// workspace, so building it can never perturb (or be perturbed by)
/// the real build. Persistent across calls (not recreated per verify)
/// so `cargo`'s own incremental cache makes the second call onward
/// fast; only the candidate source and, in the rare case a call
/// arrives with a stale nirdosha-rt path baked in, the Cargo.toml are
/// rewritten.
fn scratch_dir() -> PathBuf {
    std::env::temp_dir().join("nirdosha-hi-v2-scratch")
}

fn ensure_scratch_package() -> Result<PathBuf, String> {
    let dir = scratch_dir();
    let src = dir.join("src");
    std::fs::create_dir_all(&src).map_err(|e| format!("failed to create v2 scratch package: {e}"))?;

    let manifest = format!(
        "[workspace]\n\n\
         [package]\n\
         name = \"hi-v2-scratch\"\n\
         version = \"0.0.0\"\n\
         edition = \"2024\"\n\
         publish = false\n\n\
         [dependencies]\n\
         nirdosha-rt = {{ path = {:?} }}\n\
         serde = {{ version = \"1\", features = [\"derive\"] }}\n\
         serde_json = \"1\"\n\n\
         [[bin]]\n\
         name = \"candidate\"\n\
         path = \"src/candidate.nir\"\n",
        nirdosha_rt_path().to_str().expect("nirdosha-rt path is valid UTF-8")
    );
    let manifest_path = dir.join("Cargo.toml");
    // Only rewrite when the content actually changed, so an unrelated
    // Cargo.toml touch never invalidates the incremental build cache.
    if std::fs::read_to_string(&manifest_path).ok().as_deref() != Some(manifest.as_str()) {
        std::fs::write(&manifest_path, &manifest).map_err(|e| format!("failed to write v2 scratch Cargo.toml: {e}"))?;
    }
    let candidate = src.join("candidate.nir");
    if !candidate.exists() {
        std::fs::write(&candidate, "fn main() {}\n").map_err(|e| format!("failed to seed v2 scratch candidate: {e}"))?;
    }
    Ok(dir)
}

/// Every call reuses one candidate path; concurrent callers (an MCP
/// server handling more than one request) are serialized by this lock
/// rather than each getting their own scratch package, trading a small
/// amount of contention for a warm, shared incremental cache.
static SCRATCH_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Verify a v2 (Rust + comment-layer) source string against both
/// readers. `source` should be a complete, runnable program (a `fn
/// main`), the same convention native `.nir` candidates already follow.
pub fn verify_v2_source(source: &str) -> Result<V2Verdict, String> {
    let _guard = SCRATCH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = ensure_scratch_package()?;
    let candidate_path = dir.join("src/candidate.nir");
    std::fs::write(&candidate_path, source).map_err(|e| format!("failed to stage v2 candidate: {e}"))?;

    let output = Command::new("cargo")
        .args(["build", "--quiet"])
        .current_dir(&dir)
        .output()
        .map_err(|e| format!("failed to invoke cargo on the v2 candidate: {e}"))?;
    let builds = output.status.success();
    let build_diagnostic = if builds { None } else { Some(String::from_utf8_lossy(&output.stderr).into_owned()) };

    let summary = cargo_nirdosha::verify_sources("candidate", &dir, &[candidate_path], false);
    let violations = summary
        .violations()
        .into_iter()
        .map(|f| format!("{}:{} {}", f.file.display(), f.line, f.message))
        .collect();

    Ok(V2Verdict { builds, build_diagnostic, contracts_found: summary.contracts.len(), violations })
}

/// A curated structural summary of a v2 candidate — the v2 analogue of
/// `mcp_tools::describe_program`, but over `syn`'s AST (a v2 file is
/// plain Rust) instead of this crate's own native `ast::Program`.
/// Parses even if the candidate doesn't build (a syntactically valid
/// but not-yet-typechecking draft is still legitimate to inspect, the
/// same posture the native `describe` tool takes).
pub fn describe_v2_source(source: &str) -> Result<serde_json::Value, String> {
    let file = syn::parse_file(source).map_err(|e| format!("file does not parse as Rust: {e}"))?;

    let mut functions = Vec::new();
    let mut structs = Vec::new();
    let mut enums = Vec::new();
    let mut declarations = Vec::new();

    for item in &file.items {
        match item {
            syn::Item::Fn(f) => {
                functions.push(serde_json::json!({
                    "name": f.sig.ident.to_string(),
                    "params": f.sig.inputs.len(),
                }));
                declarations.extend(nirdosha_doc_comments(&f.attrs, &f.sig.ident.to_string()));
            }
            syn::Item::Struct(s) => {
                let fields: Vec<String> = s.fields.iter().filter_map(|f| f.ident.as_ref().map(|i| i.to_string())).collect();
                structs.push(serde_json::json!({ "name": s.ident.to_string(), "fields": fields }));
                declarations.extend(nirdosha_doc_comments(&s.attrs, &s.ident.to_string()));
            }
            syn::Item::Enum(e) => {
                let variants: Vec<String> = e.variants.iter().map(|v| v.ident.to_string()).collect();
                enums.push(serde_json::json!({ "name": e.ident.to_string(), "variants": variants }));
                declarations.extend(nirdosha_doc_comments(&e.attrs, &e.ident.to_string()));
            }
            _ => {}
        }
    }

    Ok(serde_json::json!({
        "functions": functions,
        "structs": structs,
        "enums": enums,
        "nirdosha_declarations": declarations,
    }))
}

/// Every `/// nirdosha:<kind> {...}` doc-comment line attached to an
/// item, paired with the item's own name — the same declaration shape
/// `docs/nirdosha-v2-comment-layer.md` §4 defines (one JSON object per
/// line; a real cross-referencing scanner does not exist here yet, so
/// this is reporting, not validating).
fn nirdosha_doc_comments(attrs: &[syn::Attribute], owner: &str) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    for attr in attrs {
        if !attr.path().is_ident("doc") {
            continue;
        }
        let syn::Meta::NameValue(nv) = &attr.meta else { continue };
        let syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(s), .. }) = &nv.value else { continue };
        let text = s.value();
        let trimmed = text.trim();
        if let Some(rest) = trimmed.strip_prefix("nirdosha:") {
            if let Some((kind, payload)) = rest.split_once(char::is_whitespace) {
                out.push(serde_json::json!({ "owner": owner, "kind": kind, "payload": payload.trim() }));
            }
        }
    }
    out
}

