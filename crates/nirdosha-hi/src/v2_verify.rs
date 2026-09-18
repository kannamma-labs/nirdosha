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

/// Runs `cargo build` (optionally `--release`) over `candidate_path`
/// inside `dir`, then `cargo-nirdosha`'s in-process contract scanner --
/// the shared two-reader core both [`verify_v2_source`] (a throwaway
/// scratch check) and [`build_project`] (a real, keepable project
/// binary) reduce to; only the package directory, the release flag, and
/// what happens to the resulting binary differ between the two.
fn build_and_scan(dir: &Path, candidate_path: &Path, release: bool) -> Result<V2Verdict, String> {
    let mut args = vec!["build", "--quiet"];
    if release {
        args.push("--release");
    }
    let output = Command::new("cargo")
        .args(&args)
        .current_dir(dir)
        .output()
        .map_err(|e| format!("failed to invoke cargo on the v2 candidate: {e}"))?;
    let builds = output.status.success();
    let build_diagnostic = if builds { None } else { Some(String::from_utf8_lossy(&output.stderr).into_owned()) };

    let summary = cargo_nirdosha::verify_sources("candidate", dir, &[candidate_path.to_path_buf()], false);
    let violations = summary
        .violations()
        .into_iter()
        .map(|f| format!("{}:{} {}", f.file.display(), f.line, f.message))
        .collect();

    Ok(V2Verdict { builds, build_diagnostic, contracts_found: summary.contracts.len(), violations })
}

/// Verify a v2 (Rust + comment-layer) source string against both
/// readers. `source` should be a complete, runnable program (a `fn
/// main`), the same convention native `.nir` candidates already follow.
pub fn verify_v2_source(source: &str) -> Result<V2Verdict, String> {
    let _guard = SCRATCH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = ensure_scratch_package()?;
    let candidate_path = dir.join("src/candidate.nir");
    std::fs::write(&candidate_path, source).map_err(|e| format!("failed to stage v2 candidate: {e}"))?;
    build_and_scan(&dir, &candidate_path, false)
}

/// A per-project (not the shared, throwaway `nirdosha-hi-v2-scratch`
/// candidate check's) persistent scratch package, kept under a real
/// project's own `.nir/generated/v2-build/` -- so `:publish`/`:preview`
/// produce a real, keepable `--release` binary with its own warm
/// incremental cache, isolated from every other project's.
fn project_build_dir(root: &Path) -> PathBuf {
    root.join(".nir").join("generated").join("v2-build")
}

fn ensure_project_build_package(root: &Path) -> Result<PathBuf, String> {
    let dir = project_build_dir(root);
    let src = dir.join("src");
    std::fs::create_dir_all(&src).map_err(|e| format!("failed to create v2 project build package: {e}"))?;

    let manifest = format!(
        "[workspace]\n\n\
         [package]\n\
         name = \"hi-v2-project-build\"\n\
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
    if std::fs::read_to_string(&manifest_path).ok().as_deref() != Some(manifest.as_str()) {
        std::fs::write(&manifest_path, &manifest).map_err(|e| format!("failed to write v2 project build Cargo.toml: {e}"))?;
    }
    Ok(dir)
}

/// One project's build lock -- distinct from [`SCRATCH_LOCK`] (that one
/// guards the single shared throwaway candidate check every caller
/// contends over; this one only needs to keep two concurrent `:generate`/
/// `:publish`/`:preview` calls against the *same* project from racing
/// each other's `Cargo.toml`/binary writes). One process per project
/// (`hi_window::open`'s own "one process, one project" shape) makes a
/// single global lock here exactly as safe as a per-project map would
/// be, for far less bookkeeping.
static PROJECT_BUILD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Builds `root`'s current generated source (`hi_llm::generated_source_
/// path`) as a real, `--release` binary -- the "does this project
/// actually build" question `:publish`/`:preview` both start from.
/// Returns the verdict alongside the built binary's path (valid only
/// when `verdict.passed()`).
pub fn build_project(root: &Path, generated_source_path: &Path) -> Result<(V2Verdict, PathBuf), String> {
    let _guard = PROJECT_BUILD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let source = std::fs::read_to_string(generated_source_path).map_err(|e| format!("reading {}: {e}", generated_source_path.display()))?;
    let dir = ensure_project_build_package(root)?;
    let candidate_path = dir.join("src/candidate.nir");
    std::fs::write(&candidate_path, &source).map_err(|e| format!("failed to stage {}'s generated source for build: {e}", root.display()))?;
    let verdict = build_and_scan(&dir, &candidate_path, true)?;
    let binary_name = if cfg!(windows) { "candidate.exe" } else { "candidate" };
    let binary_path = dir.join("target").join("release").join(binary_name);
    Ok((verdict, binary_path))
}

/// The result of a successful `:publish`: where the real binary and its
/// certificate ended up, alongside the verdict that cleared it.
pub struct PublishResult {
    pub binary_path: PathBuf,
    pub certificate_path: PathBuf,
    pub verdict: V2Verdict,
}

/// Builds `root`'s generated source for real and, only if it passes its
/// own check, copies the binary to `.nir/generated/hi_build` and writes
/// a certificate alongside it (the same `nirdosha.certificate/v2-
/// source-scan` shape [`certificate_json`] gives `certify_code`) --
/// "publish" here means exactly what it does for the native pipeline:
/// produce the final compiled artifact this project's confirmed graph
/// currently describes, refusing rather than shipping something that
/// doesn't even build.
pub fn publish_project(root: &Path, generated_source_path: &Path) -> Result<PublishResult, String> {
    let (verdict, binary_path) = build_project(root, generated_source_path)?;
    if !verdict.passed() {
        return Err(format!(
            "publish refused -- the v2 candidate does not pass its own check (builds: {}, violations: {:?}){}",
            verdict.builds,
            verdict.violations,
            verdict.build_diagnostic.as_deref().map(|d| format!("\n{d}")).unwrap_or_default()
        ));
    }
    let out_path = root.join(".nir").join("generated").join("hi_build");
    std::fs::copy(&binary_path, &out_path).map_err(|e| format!("copying {} to {}: {e}", binary_path.display(), out_path.display()))?;

    let source = std::fs::read_to_string(generated_source_path).map_err(|e| format!("reading {}: {e}", generated_source_path.display()))?;
    let certificate = certificate_json(&source, &verdict);
    let cert_path = out_path.with_extension("certificate.json");
    std::fs::write(&cert_path, serde_json::to_string_pretty(&certificate).expect("this JSON value always serializes")).map_err(|e| format!("writing {}: {e}", cert_path.display()))?;

    Ok(PublishResult { binary_path: out_path, certificate_path: cert_path, verdict })
}

/// The `nirdosha.certificate/v2-source-scan` shape -- shared by
/// `mcp_tools::certify_code` (over an inline source string) and
/// [`publish_project`] (over a real project's generated source), so the
/// two can never quietly disagree about what a v2 certificate contains.
pub fn certificate_json(source: &str, verdict: &V2Verdict) -> serde_json::Value {
    serde_json::json!({
        "certificate_version": "nirdosha.certificate/v2-source-scan",
        "source_hash": crate::hi_graph::sha256_hex(source.as_bytes()),
        "toolchain_version": env!("CARGO_PKG_VERSION"),
        "evidence_tier": "source_scan",
        "verdict": verdict.verdict(),
        "builds": verdict.builds,
        "build_diagnostic": verdict.build_diagnostic,
        "contracts_found": verdict.contracts_found,
        "violations": verdict.violations,
    })
}

/// Discovers the port a v2 app's generated `main()` hardcodes in its
/// own `.serve(<port>)` call in `main`. V2 has no runtime port override.
fn discover_serve_port(source: &str) -> Option<u16> {
    let file = syn::parse_file(source).ok()?;
    struct Finder(Option<u16>);
    impl<'ast> syn::visit::Visit<'ast> for Finder {
        fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
            if self.0.is_none() && node.method == "serve" {
                if let Some(syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Int(n), .. })) = node.args.first() {
                    self.0 = n.base10_parse::<u16>().ok();
                }
            }
            syn::visit::visit_expr_method_call(self, node);
        }
    }
    let mut finder = Finder(None);
    let main = file.items.iter().find_map(|item| match item {
        syn::Item::Fn(f) if f.sig.ident == "main" => Some(&f.block),
        _ => None,
    })?;
    syn::visit::Visit::visit_block(&mut finder, main);
    finder.0
}

/// The first route a generated UI macro mounts is the preview landing
/// page. UI macros encode their route as a top-level `path: "..."`
/// entry; nested field/action paths do not count.
fn discover_preview_path(source: &str) -> Option<String> {
    let file = syn::parse_file(source).ok()?;
    for item in &file.items {
        let syn::Item::Macro(item) = item else { continue };
        if crate::hi_graph::screen_name_from_macro(item).is_none() { continue; }
        let mut tokens = item.mac.tokens.clone().into_iter();
        while let Some(token) = tokens.next() {
            let proc_macro2::TokenTree::Ident(key) = token else { continue };
            if key != "path" { continue; }
            if !matches!(tokens.next(), Some(proc_macro2::TokenTree::Punct(colon)) if colon.as_char() == ':') { continue; }
            let Some(proc_macro2::TokenTree::Literal(value)) = tokens.next() else { continue };
            let path = syn::parse2::<syn::LitStr>(proc_macro2::TokenTree::Literal(value).into()).ok()?.value();
            if path.starts_with('/') && path != "/" { return Some(path); }
        }
    }
    None
}

#[cfg(test)]
mod preview_port_tests {
    use super::{discover_serve_port, discover_preview_path};

    #[test]
    fn preview_only_accepts_a_literal_serve_in_main() {
        assert_eq!(discover_serve_port("fn main() { println!(\"done\"); }"), None);
        assert_eq!(discover_serve_port("fn unused() { router.serve(8080); } fn main() {}"), None);
        assert_eq!(discover_serve_port("fn main() { router.serve(8096); }"), Some(8096));
    }

    #[test]
    fn preview_path_uses_the_first_real_ui_route() {
        let source = "nirdosha_rt::crud_screens! { mount: mount_TaskListScreen, entity: Task, store: task_store, path: \"/tasks\", fields: [ title: String ] }\nnirdosha_rt::dashboard! { mount: mount_DashboardScreen, path: \"/dashboard\", widgets {} }\nfn main() { router.serve(8080); }";
        assert_eq!(discover_preview_path(source).as_deref(), Some("/tasks"));
        assert_eq!(discover_preview_path("fn main() {}"), None);
    }
}

/// Builds `root`'s generated source for real and, if it passes, starts
/// (or restarts) it on the literal port named by `main`'s `.serve(N)`.
pub fn preview_start(root: &Path, generated_source_path: &Path) -> Result<(u16, String), String> {
    let source = std::fs::read_to_string(generated_source_path).map_err(|e| format!("reading {}: {e}", generated_source_path.display()))?;
    let port = discover_serve_port(&source).ok_or("preview requires main() to call .serve(<literal port>); the generated program exits without starting an HTTP server")?;
    let path = discover_preview_path(&source).unwrap_or_else(|| "/".to_string());
    let (verdict, binary_path) = build_project(root, generated_source_path)?;
    if !verdict.passed() {
        return Err(format!(
            "preview refused -- the v2 candidate does not pass its own check (builds: {}, violations: {:?}){}",
            verdict.builds,
            verdict.violations,
            verdict.build_diagnostic.as_deref().map(|d| format!("\n{d}")).unwrap_or_default()
        ));
    }
    crate::hi_preview::restart(&binary_path, port, &path)?;
    Ok((port, path))
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
