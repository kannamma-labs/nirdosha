//! The Nirdosha compiler, Stage 1: a source-scanning verifier that runs
//! **in front of** plain cargo.
//!
//! The story this crate implements:
//!
//! > To the external world, a Nirdosha program is just another Rust
//! > program — it compiles and runs under stock `cargo` with zero
//! > Nirdosha tooling. Hand the same source to the Nirdosha compiler
//! > and the contract claims stop being comments: `nirdosha:contract`
//! > docs are parsed and cross-checked, `effects(pure)` claims are
//! > verified against the body, dialect restrictions (`unsafe`, raw
//! > threads) are enforced, and a certificate is emitted. Lying is a
//! > build error — the binary refuses to delegate to cargo.
//!
//! Stage 2 replaces the path-based scans with a rustc driver (HIR/MIR
//! passes, interprocedural effects, Z3). The CLI surface and the
//! certificate artifact stay identical, so adoption is not a rewrite.

use std::fs;
use std::path::{Path, PathBuf};

use nirdosha_contract_core as cc;
use serde::Serialize;
use syn::{Expr, ImplItem, Item, TraitItem};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub severity: Severity,
    pub file: PathBuf,
    pub line: u32,
    pub function: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContractForm {
    Attribute,
    Doc,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContractInfo {
    pub file: PathBuf,
    pub line: u32,
    pub function: String,
    pub form: ContractForm,
    pub contract: cc::model::Contract,
}

pub struct ScanSummary {
    pub package: String,
    pub files: Vec<PathBuf>,
    pub contracts: Vec<ContractInfo>,
    pub findings: Vec<Finding>,
}

impl ScanSummary {
    pub fn violations(&self) -> Vec<&Finding> {
        self.findings
            .iter()
            .filter(|f| matches!(f.severity, Severity::Error))
            .collect()
    }

    /// Write the Stage-1 certificate next to the build artifacts:
    /// `<target>/nirdosha/contract-report-<package>.json`.
    pub fn write_report(&self, target_dir: &Path) -> std::io::Result<PathBuf> {
        let dir = target_dir.join("nirdosha");
        fs::create_dir_all(&dir)?;
        let path = dir.join(format!("contract-report-{}.json", self.package));
        let payload = serde_json::json!({
            "tool": "cargo-nirdosha",
            "dialect": "nirdosha-rt",
            "stage": 1,
            "package": self.package,
            "files_scanned": self.files.len(),
            "contracts": self.contracts,
            "findings": self.findings,
            "violations": self.violations().len(),
        });
        fs::write(&path, serde_json::to_vec_pretty(&payload)?)?;
        Ok(path)
    }
}

/// Verify the package rooted at `manifest_dir` (its `src/` tree,
/// recursively).
pub fn verify_package(manifest_dir: &Path, strict: bool) -> Result<ScanSummary, String> {
    let package = package_name(&manifest_dir.join("Cargo.toml"))
        .unwrap_or_else(|| "<unknown-package>".to_string());
    let src = manifest_dir.join("src");
    if !src.is_dir() {
        return Err(format!(
            "no src/ directory under {} — run from inside a package",
            manifest_dir.display()
        ));
    }
    let mut files = Vec::new();
    collect_rs_files(&src, &mut files);
    files.sort();
    Ok(verify_sources(&package, &files, strict))
}

/// Verify a fixed list of `.rs` files (the unit of testing).
pub fn verify_sources(package: &str, files: &[PathBuf], strict: bool) -> ScanSummary {
    let mut summary = ScanSummary {
        package: package.to_string(),
        files: files.to_vec(),
        contracts: Vec::new(),
        findings: Vec::new(),
    };
    for file in files {
        verify_file(file, strict, &mut summary);
    }
    summary
}

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

struct FnInfo<'a> {
    attrs: &'a [syn::Attribute],
    ident: String,
    is_pub: bool,
    block: Option<&'a syn::Block>,
    line: u32,
}

fn fn_info<'a>(
    attrs: &'a [syn::Attribute],
    sig: &'a syn::Signature,
    is_pub: bool,
    block: Option<&'a syn::Block>,
) -> FnInfo<'a> {
    FnInfo {
        attrs,
        ident: sig.ident.to_string(),
        is_pub,
        block,
        line: sig.fn_token.span.start().line as u32,
    }
}

fn walk_items<'a>(items: &'a [Item], out: &mut Vec<FnInfo<'a>>) {
    for item in items {
        match item {
            Item::Fn(f) => out.push(fn_info(&f.attrs, &f.sig, matches!(f.vis, syn::Visibility::Public(_)), Some(&f.block))),
            Item::Mod(syn::ItemMod {
                content: Some((_, inner)),
                ..
            }) => walk_items(inner, out),
            Item::Impl(imp) => {
                for m in &imp.items {
                    if let ImplItem::Fn(f) = m {
                        out.push(fn_info(&f.attrs, &f.sig, matches!(f.vis, syn::Visibility::Public(_)), Some(&f.block)));
                    }
                }
            }
            Item::Trait(t) => {
                for m in &t.items {
                    if let TraitItem::Fn(f) = m {
                        out.push(fn_info(&f.attrs, &f.sig, true, f.default.as_ref()));
                    }
                }
            }
            _ => {}
        }
    }
}

fn verify_file(path: &Path, strict: bool, out: &mut ScanSummary) {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            out.findings.push(Finding {
                severity: Severity::Error,
                file: path.to_path_buf(),
                line: 0,
                function: None,
                message: format!("cannot read file: {e}"),
            });
            return;
        }
    };
    let file = match syn::parse_file(&text) {
        Ok(f) => f,
        Err(e) => {
            out.findings.push(Finding {
                severity: Severity::Error,
                file: path.to_path_buf(),
                line: e.span().start().line as u32,
                function: None,
                message: format!("file does not parse: {e}"),
            });
            return;
        }
    };

    // Dialect-wide restrictions, contract or no contract.
    for deny in cc::scan::dialect_denies(&file) {
        out.findings.push(Finding {
            severity: Severity::Error,
            file: path.to_path_buf(),
            line: deny.line,
            function: None,
            message: format!("dialect restriction violated: {} ({})", deny.what, deny.why),
        });
    }

    let mut fns = Vec::new();
    walk_items(&file.items, &mut fns);
    for f in fns {
        check_fn(&f, path, strict, out);
    }
}

fn error(path: &Path, line: u32, function: Option<&str>, message: impl Into<String>) -> Finding {
    Finding {
        severity: Severity::Error,
        file: path.to_path_buf(),
        line,
        function: function.map(str::to_string),
        message: message.into(),
    }
}

fn check_fn(f: &FnInfo, path: &Path, strict: bool, out: &mut ScanSummary) {
    let mut attr_contract: Option<cc::model::Contract> = None;
    let mut doc_contract: Option<cc::model::Contract> = None;

    for attr in f.attrs {
        let last = attr
            .path()
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .unwrap_or_default();
        if last == "contract" {
            match &attr.meta {
                syn::Meta::List(list) => match cc::parse::parse_contract(list.tokens.clone()) {
                    Ok(c) => attr_contract = Some(c),
                    Err(e) => out.findings.push(error(
                        path,
                        f.line,
                        Some(&f.ident),
                        format!("malformed #[contract] attribute: {e}"),
                    )),
                },
                _other => out.findings.push(error(
                    path,
                    f.line,
                    Some(&f.ident),
                    format!("#[contract] must take a parenthesized clause list, found something else"),
                )),
            }
        } else if last == "doc" {
            if let syn::Meta::NameValue(nv) = &attr.meta {
                if let Expr::Lit(el) = &nv.value {
                    if let syn::Lit::Str(ls) = &el.lit {
                        match cc::docparse::parse_doc(&ls.value()) {
                            Ok(Some(c)) => {
                                if doc_contract.replace(c).is_some() {
                                    out.findings.push(error(
                                        path,
                                        f.line,
                                        Some(&f.ident),
                                        "two nirdosha:contract docs on one function — keep exactly one",
                                    ));
                                }
                            }
                            Ok(None) => {}
                            Err(msg) => out.findings.push(error(path, f.line, Some(&f.ident), msg)),
                        }
                    }
                }
            }
        }
    }

    let (contract, form) = match (attr_contract, doc_contract) {
        (Some(_), Some(_)) => {
            out.findings.push(error(
                path,
                f.line,
                Some(&f.ident),
                "fn carries both #[contract(..)] and a nirdosha:contract doc — \
                 pick one (the attribute macro emits the doc encoding for you)",
            ));
            return;
        }
        (Some(c), None) => (c, ContractForm::Attribute),
        (None, Some(c)) => (c, ContractForm::Doc),
        (None, None) => {
            if strict && f.is_pub {
                out.findings.push(error(
                    path,
                    f.line,
                    Some(&f.ident),
                    "strict mode: pub fn has no contract — \
                     add #[contract(..)] or a nirdosha:contract doc",
                ));
            }
            return;
        }
    };

    for issue in contract.validate() {
        out.findings.push(error(
            path,
            f.line,
            Some(&f.ident),
            format!("invalid contract: {issue}"),
        ));
    }

    // Honesty: a pure claim is checked against the body.
    if contract.claims_pure() {
        if let Some(block) = f.block {
            for hit in cc::scan::impure_calls_in_block(block) {
                out.findings.push(error(
                    path,
                    hit.line,
                    Some(&f.ident),
                    format!(
                        "claims effects(pure) but the body performs {} ({}) — \
                         a contract is a checked declaration, not a comment",
                        hit.what, hit.why
                    ),
                ));
            }
        } else {
            out.findings.push(Finding {
                severity: Severity::Warning,
                file: path.to_path_buf(),
                line: f.line,
                function: Some(f.ident.clone()),
                message: "pure claim on a body-less declaration — the obligation \
                          moves to implementing blocks (checked like any other fn)"
                    .into(),
            });
        }
    }

    out.contracts.push(ContractInfo {
        file: path.to_path_buf(),
        line: f.line,
        function: f.ident.clone(),
        form,
        contract,
    });
}

fn package_name(manifest: &Path) -> Option<String> {
    let text = fs::read_to_string(manifest).ok()?;
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("name") {
            let rest = rest.trim().strip_prefix('=')?.trim();
            let rest = rest.strip_prefix('"')?;
            let name = rest.strip_suffix('"')?;
            return Some(name.to_string());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Workspace mode (Stage 1.5): verify every *in-dialect* crate in the
// workspace, strict by default.
// ---------------------------------------------------------------------------

/// A crate is in-dialect when it opts in by any of:
/// - depending on `nirdosha-rt` (the runtime is the dialect's own crate),
/// - declaring `[package.metadata.nirdosha-rt] dialect = true` (the
///   zero-dependency form — see `examples/rt-payroll-lying`),
/// and is never in-dialect when it declares
/// `[package.metadata.nirdosha-rt] toolchain = true` (the dialect's own
/// implementation crates — the verifier doesn't lint itself).
///
/// Note this deliberately does NOT look for contract strings inside
/// sources: the toolchain's own crates mention `nirdosha:contract` in
/// their docs and tests, and the `.nir` plugin crates already use a
/// colliding `[package.metadata.nirdosha]` section for plugin discovery.
/// Opt-in is explicit, adoption is per crate.
pub fn in_dialect(manifest_dir: &Path) -> bool {
    let Ok(manifest) = fs::read_to_string(manifest_dir.join("Cargo.toml")) else {
        return false;
    };
    if manifest.contains("[package.metadata.nirdosha-rt]") {
        if manifest.contains("toolchain = true") {
            return false;
        }
        if manifest.contains("dialect = true") {
            return true;
        }
    }
    // A real dependency entry (not a package *name* containing the
    // substring — the pattern `nirdosha-rt =` only matches dep lines).
    manifest.contains("nirdosha-rt =")
}

pub struct WorkspaceSummary {
    /// In-dialect packages that were verified (in metadata order).
    pub packages: Vec<ScanSummary>,
    /// In-dialect packages, in metadata order, for reporting.
    pub all_packages: Vec<String>,
}

impl WorkspaceSummary {
    pub fn violations(&self) -> usize {
        self.packages
            .iter()
            .map(|s| s.violations().len())
            .sum()
    }

    pub fn contracts(&self) -> usize {
        self.packages.iter().map(|s| s.contracts.len()).sum()
    }

    /// Write each package's report plus the aggregate workspace certificate.
    pub fn write_reports(&self, target_dir: &Path) -> std::io::Result<PathBuf> {
        for summary in &self.packages {
            summary.write_report(target_dir)?;
        }
        let dir = target_dir.join("nirdosha");
        fs::create_dir_all(&dir)?;
        let path = dir.join("contract-report-workspace.json");
        let mut contracts = Vec::new();
        let mut findings = Vec::new();
        for summary in &self.packages {
            for c in &summary.contracts {
                contracts.push(serde_json::json!({
                    "package": summary.package,
                    "file": c.file,
                    "line": c.line,
                    "function": c.function,
                    "form": c.form,
                    "contract": c.contract,
                }));
            }
            findings.extend(summary.findings.iter().cloned());
        }
        let payload = serde_json::json!({
            "tool": "cargo-nirdosha",
            "dialect": "nirdosha-rt",
            "stage": 1.5,
            "mode": "workspace",
            "in_dialect_packages": self.all_packages,
            "contracts": contracts,
            "findings": findings,
            "violations": self.violations(),
        });
        fs::write(&path, serde_json::to_vec_pretty(&payload)?)?;
        Ok(path)
    }
}

/// Verify every in-dialect crate in the current workspace. `strict` is
/// on by default in workspace mode: the workspace gate is the strict
/// one; per-crate `verify` stays lenient unless NIRDOSHA_STRICT is set.
pub fn verify_workspace(strict: bool) -> Result<WorkspaceSummary, String> {
    let out = std::process::Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .output()
        .map_err(|e| format!("cannot run cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err("cargo metadata failed — is this a cargo workspace?".into());
    }
    let meta: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
    let target_dir = meta
        .get("target_directory")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .ok_or("cargo metadata has no target_directory")?;
    let packages = meta
        .get("packages")
        .and_then(|v| v.as_array())
        .ok_or("cargo metadata has no packages")?;
    let mut all = Vec::new();
    let mut summaries = Vec::new();
    for package in packages {
        let Some(manifest) = package.get("manifest_path").and_then(|v| v.as_str()) else {
            continue;
        };
        let manifest_dir = PathBuf::from(manifest)
            .parent()
            .ok_or("manifest has no parent")?
            .to_path_buf();
        if !in_dialect(&manifest_dir) {
            continue;
        }
        all.push(package_name(&manifest_dir.join("Cargo.toml")).unwrap_or_default());
        summaries.push(verify_package(&manifest_dir, strict)?);
    }
    let _ = target_dir; // located again by the CLI for report writing
    Ok(WorkspaceSummary {
        packages: summaries,
        all_packages: all,
    })
}

// ---------------------------------------------------------------------------
// Bench gates (Stage 1.5): nfr(latency_ms) becomes a CI gate driven by
// the real workload (the package's own test run), not a synthetic one.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Deserialize)]
pub struct BenchEvent {
    pub function: String,
    pub latency_ms: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct BenchVerdict {
    pub function: String,
    pub limit_ms: Option<f64>,
    pub calls: usize,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
    pub ok: bool,
}

/// Compare observed flight-recorder events against every declared
/// `nfr(latency_ms)`. The p95 is the verdict; a pure-fn's tests are its
/// load generator — honest, measured, never claimed as proven.
pub fn evaluate_bench(events: &[BenchEvent], contracts: &[ContractInfo]) -> Vec<BenchVerdict> {
    let mut verdicts: Vec<BenchVerdict> = Vec::new();
    for contract in contracts {
        let Some(nfr) = contract.contract.nfr else { continue };
        let mut latencies: Vec<f64> = events
            .iter()
            .filter(|e| e.function == contract.function)
            .map(|e| e.latency_ms)
            .collect();
        latencies.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let (calls, p50, p95, max) = if latencies.is_empty() {
            (0, 0.0, 0.0, 0.0)
        } else {
            let pick = |q: f64| {
                let idx = ((latencies.len() as f64 - 1.0) * q).round() as usize;
                latencies[idx.min(latencies.len() - 1)]
            };
            (
                latencies.len(),
                pick(0.50),
                pick(0.95),
                latencies[latencies.len() - 1],
            )
        };
        let ok = match nfr.latency_ms {
            Some(limit) => calls > 0 && p95 <= limit,
            None => true,
        };
        verdicts.push(BenchVerdict {
            function: contract.function.clone(),
            limit_ms: nfr.latency_ms,
            calls,
            p50_ms: p50,
            p95_ms: p95,
            max_ms: max,
            ok,
        });
    }
    verdicts
}

/// Read a flight-recorder ndjson sink back into events.
pub fn read_bench_events(path: &Path) -> Vec<BenchEvent> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    text.lines()
        .filter_map(|line| serde_json::from_str::<BenchEvent>(line).ok())
        .collect()
}