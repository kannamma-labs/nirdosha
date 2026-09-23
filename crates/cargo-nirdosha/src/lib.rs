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

pub mod generate;

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
    /// Whether the fn is `pub` — the guarantee bundle's `gated_exports`/
    /// `public_exports` (issue #75 item 1) only describe the exposed
    /// surface, not every internal helper that happens to carry a
    /// contract.
    pub is_pub: bool,
}

pub struct ScanSummary {
    pub package: String,
    /// The package root — the base the certificate's source paths are
    /// relative to.
    pub package_dir: PathBuf,
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
    ///
    /// `nirdosha.certificate/v1`: the report payload rides inside a
    /// hash-bound envelope — every verified file's SHA-256, the tool
    /// identity, and a binding over the whole content. Deterministic by
    /// construction (no timestamps), so re-verification is a byte-diff.
    ///
    /// `bind_provenance`: additionally binds the resolved dependency
    /// closure (`Cargo.lock`) and toolchain (G6) — mechanical binding
    /// only, not issuer authentication. See `nirdosha_contract_core::provenance`.
    pub fn write_report(&self, target_dir: &Path, bind_provenance: bool) -> std::io::Result<PathBuf> {
        let certificate = self.build_certificate(bind_provenance)?;
        self.write_certificate(target_dir, &certificate)
    }

    /// Same as [`Self::write_report`], additionally Ed25519-signing the
    /// certificate's `binding` with the PKCS#8 private key at
    /// `key_path` (issue #75 item 2 — `signature` was "reserved for
    /// the signed-plugin trust chain" and unimplemented; this wires it
    /// up via `nirdosha-audit`, the same Ed25519/SHA-256 backend the
    /// native `.nir` compiler's certificate/pack signing already uses,
    /// FIPS-swappable via the `fips` feature). Signing the binding
    /// rather than the raw certificate bytes means the signature
    /// travels correctly even across re-serializations: the binding is
    /// already a canonical, deterministic digest of every bound field.
    pub fn write_report_signed(
        &self,
        target_dir: &Path,
        bind_provenance: bool,
        key_path: &str,
    ) -> std::io::Result<PathBuf> {
        let mut certificate = self.build_certificate(bind_provenance)?;
        let (public_key, signature) =
            nirdosha_audit::signing::sign_bytes(certificate.binding.as_bytes(), key_path)
                .map_err(std::io::Error::other)?;
        certificate.signature = Some(cc::certificate::Signature {
            algorithm: "ed25519".into(),
            key_id: public_key,
            value: signature,
        });
        self.write_certificate(target_dir, &certificate)
    }

    fn build_certificate(&self, bind_provenance: bool) -> std::io::Result<cc::certificate::Certificate> {
        let provenance = if bind_provenance {
            Some(cc::provenance::Provenance {
                cargo_lock_sha256: cc::provenance::hash_cargo_lock(&self.package_dir)
                    .map_err(std::io::Error::other)?,
                toolchain: rustc_version()?,
            })
        } else {
            None
        };
        let coverage = match &provenance {
            Some(_) => cc::evidence::Coverage::source_scan_with_provenance(self.violations().is_empty(), true),
            None => cc::evidence::Coverage::source_scan(self.violations().is_empty()),
        };
        let mut payload = serde_json::json!({
            "tool": "cargo-nirdosha",
            "dialect": "nirdosha-rt",
            "stage": 1,
            "package": self.package,
            "files_scanned": self.files.len(),
            "contracts": self.contracts,
            "findings": self.findings,
            "violations": self.violations().len(),
            "coverage": coverage,
        });
        if let Some(p) = &provenance {
            payload["provenance"] = serde_json::to_value(p)?;
        }
        Ok(cc::certificate::Certificate::new(
            cc::certificate::Subject {
                package: self.package.clone(),
                version: package_version(&self.package_dir),
            },
            cc::certificate::Tool {
                name: "cargo-nirdosha".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                mode: cc::certificate::Mode::SourceScan,
                // None is honest when unbound: the source scan itself
                // never invokes rustc. Set when provenance is bound.
                toolchain: provenance.as_ref().map(|p| p.toolchain.clone()),
            },
            cc::certificate::scan_sources(&self.package_dir, &self.files)?,
            payload,
        ))
    }

    fn write_certificate(&self, target_dir: &Path, certificate: &cc::certificate::Certificate) -> std::io::Result<PathBuf> {
        let dir = target_dir.join("nirdosha");
        fs::create_dir_all(&dir)?;
        let path = dir.join(format!("contract-report-{}.json", self.package));
        fs::write(&path, serde_json::to_vec_pretty(certificate)?)?;
        Ok(path)
    }

    /// The emitted guarantee bundle (issue #75 item 1 — the dialect's
    /// counterpart to the native `.nir` compiler's RFC 0017 bundle,
    /// `crates/compiler/src/guarantee_manifest.rs::build_bundle`, same
    /// field names/shape where the data maps directly). Real minimal
    /// version: built entirely from what `ScanSummary` already
    /// computes, no new analysis — `inferred_effects` per function,
    /// `gated_exports`/`public_exports` for the exposed surface,
    /// `nfr_tracked` for declared NFRs, and a provenance link back to
    /// the source certificate via `source_hash` (the certificate's own
    /// `binding` — a package has many source files, so there is no
    /// single-file hash the way a `.nir` module has one; the binding is
    /// the honest multi-file analog: it changes if any bound fact does).
    pub fn guarantee_bundle(&self, certificate_path: &Path, certificate_binding: &str) -> serde_json::Value {
        let mut inferred_effects = serde_json::Map::new();
        let mut gated_exports = serde_json::Map::new();
        let mut public_exports: Vec<&str> = Vec::new();
        let mut nfr_tracked = serde_json::Map::new();
        for c in &self.contracts {
            if let Some(effects) = &c.contract.effects {
                inferred_effects.insert(c.function.clone(), serde_json::json!(effects));
            }
            if let Some(requires) = &c.contract.requires {
                let describe = match (&requires.role, &requires.expr, &requires.claim) {
                    (Some(role), _, _) => format!("role:{role}"),
                    (None, Some(expr), _) => format!("expr:{expr}"),
                    (None, None, Some((name, value))) => format!("claim:{name}={value}"),
                    (None, None, None) => "unknown".to_string(),
                };
                gated_exports.insert(c.function.clone(), serde_json::json!(describe));
            } else if c.is_pub {
                public_exports.push(&c.function);
            }
            if let Some(nfr) = &c.contract.nfr {
                nfr_tracked.insert(
                    c.function.clone(),
                    serde_json::to_value(nfr).expect("Nfr always serializes"),
                );
            }
        }
        serde_json::json!({
            "bundle_version": "1",
            "package": self.package,
            "certificate": certificate_path,
            "source_hash": certificate_binding,
            "inferred_effects": inferred_effects,
            "gated_exports": gated_exports,
            "public_exports": public_exports,
            "nfr_tracked": nfr_tracked,
        })
    }

    /// Writes the guarantee bundle next to the certificate:
    /// `<target>/nirdosha/guarantees-<package>.json` — the artifact
    /// that travels with the build, per issue #75 item 1's "nothing
    /// travels with the artifact" gap.
    pub fn write_guarantee_bundle(
        &self,
        target_dir: &Path,
        certificate_path: &Path,
        certificate_binding: &str,
    ) -> std::io::Result<PathBuf> {
        let dir = target_dir.join("nirdosha");
        fs::create_dir_all(&dir)?;
        let path = dir.join(format!("guarantees-{}.json", self.package));
        let bundle = self.guarantee_bundle(certificate_path, certificate_binding);
        fs::write(&path, serde_json::to_vec_pretty(&bundle)?)?;
        Ok(path)
    }
}

/// `rustc --version`, trimmed. The toolchain a provenance-bound
/// certificate records — recorded, not re-verified at consuming time.
fn rustc_version() -> std::io::Result<String> {
    let out = std::process::Command::new("rustc").arg("--version").output()?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The package's declared version, for the certificate subject.
fn package_version(manifest_dir: &Path) -> Option<String> {
    let text = fs::read_to_string(manifest_dir.join("Cargo.toml")).ok()?;
    let mut in_package = false;
    for line in text.lines() {
        let line = line.trim();
        if line == "[package]" {
            in_package = true;
            continue;
        }
        if !in_package {
            continue;
        }
        if line.starts_with('[') {
            // a new section: past [package]; version not declared here
            // (workspace-inherited versions read as None — the honest
            // answer for a scan that never resolves workspace metadata)
            break;
        }
        if let Some(rest) = line.strip_prefix("version") {
            if let Some(v) = rest.trim_start().strip_prefix('=') {
                return Some(v.trim().trim_matches('"').to_string());
            }
        }
    }
    None
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
    collect_dialect_files(&src, &mut files);
    files.sort();
    Ok(verify_sources(&package, manifest_dir, &files, strict))
}

/// Verify a fixed list of `.rs` files (the unit of testing).
pub fn verify_sources(
    package: &str,
    package_dir: &Path,
    files: &[PathBuf],
    strict: bool,
) -> ScanSummary {
    let mut summary = ScanSummary {
        package: package.to_string(),
        package_dir: package_dir.to_path_buf(),
        files: files.to_vec(),
        contracts: Vec::new(),
        findings: Vec::new(),
    };
    for file in files {
        verify_file(file, strict, &mut summary);
    }
    check_pack_invariants(package_dir, files, &mut summary);
    summary
}

/// Issue #76: enforce a signed, active domain pack's `mandatory_fns`/
/// `protected_structs` invariants for real — until this, that same
/// on-disk, install-time-signature-verified state
/// (`nirdosha-contract-core::pack`) only ever fed `nirdosha-hi`'s
/// LLM-generation guidance loop, never a build that actually refuses
/// on violation. A no-op — no re-parsing, no findings — when no active
/// pack declares either (the overwhelmingly common case: no pack
/// installed at all).
fn check_pack_invariants(package_dir: &Path, files: &[PathBuf], out: &mut ScanSummary) {
    let cargo_toml = package_dir.join("Cargo.toml");
    let mandatory_fns = match cc::pack::active_mandatory_fns(package_dir) {
        Ok(m) => m,
        Err(e) => {
            out.findings.push(error(&cargo_toml, 0, None, format!("cannot read active domain packs: {e}")));
            return;
        }
    };
    let protected_structs = match cc::pack::active_protected_structs(package_dir) {
        Ok(p) => p,
        Err(e) => {
            out.findings.push(error(&cargo_toml, 0, None, format!("cannot read active domain packs: {e}")));
            return;
        }
    };
    if mandatory_fns.is_empty() && protected_structs.is_empty() {
        return;
    }
    let mut parsed: Vec<(&PathBuf, syn::File)> = Vec::new();
    for file in files {
        // A file that doesn't read/parse was already reported by
        // `verify_file` above; skip it here rather than double-report.
        let Ok(text) = fs::read_to_string(file) else { continue };
        let Ok(ast) = syn::parse_file(&text) else { continue };
        parsed.push((file, ast));
    }
    let all_files: Vec<syn::File> = parsed.iter().map(|(_, f)| f.clone()).collect();
    for name in cc::pack_check::missing_mandatory_call_sites(&all_files, &mandatory_fns) {
        out.findings.push(error(
            &cargo_toml,
            0,
            None,
            format!(
                "an active domain pack marks `{name}` a mandatory certified primitive, but no fn in this package calls it anywhere — the pack's rule must be wired, not merely installed"
            ),
        ));
    }
    for (path, file) in &parsed {
        for violation in cc::pack_check::check_primitive_exclusivity(file, &protected_structs, &mandatory_fns) {
            out.findings.push(error(
                path,
                violation.line,
                Some(&violation.fn_name),
                format!(
                    "constructs `{}` directly — an active domain pack marks `{}` a protected type; only a mandatory certified primitive may construct it",
                    violation.struct_name, violation.struct_name
                ),
            ));
        }
    }
}

/// Collect the dialect's source files: `.rs` and `.nir` alike — a v2
/// `.nir` file is valid Rust that plain cargo already builds (the v2
/// corpus proved it), so it carries contracts the same way.
fn collect_dialect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_dialect_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs" || e == "nir") {
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

    // Option 1 data-plane guard: screen files must not read or write
    // GuardedEntity rows through the unguarded `system_scan` /
    // `raw_driver_seed` bypasses. Those paths are legitimate in bridge
    // and test-fixture code, but a screen must go through the policy
    // plane (`guarded_snapshot` / `guarded_get` / `guarded_search` /
    // `guarded_insert_checked` / `guarded_update`).
    let path_str = path.to_string_lossy();
    let is_screen_file = path_str.contains("/src/screens/") || path_str.contains("\\src\\screens\\");
    if is_screen_file {
        for bypass in cc::scan::screen_guard_bypasses(&file) {
            out.findings.push(Finding {
                severity: Severity::Error,
                file: path.to_path_buf(),
                line: bypass.line,
                function: None,
                message: format!(
                    "screen must use GuardedTable policy reads/writes: {} ({})",
                    bypass.what, bypass.why
                ),
            });
        }
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
        is_pub: f.is_pub,
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
    /// `bind_provenance` — see `ScanSummary::write_report`. Not consumable
    /// via `check-certificate` today (workspace certificates are refused
    /// there; evaluate the individual package certificates), but bound
    /// here anyway for consistency and future workspace-level policy.
    pub fn write_reports(&self, target_dir: &Path, bind_provenance: bool) -> std::io::Result<PathBuf> {
        for summary in &self.packages {
            let report_path = summary.write_report(target_dir, bind_provenance)?;
            // Issue #75 item 1: every in-dialect workspace member gets
            // its own guarantee bundle too, not just the standalone
            // per-package `verify` path.
            let cert_bytes = fs::read(&report_path)?;
            let cert: cc::certificate::Certificate = serde_json::from_slice(&cert_bytes)?;
            summary.write_guarantee_bundle(target_dir, &report_path, &cert.binding)?;
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
        let provenance = if bind_provenance {
            Some(cc::provenance::Provenance {
                cargo_lock_sha256: cc::provenance::hash_cargo_lock(target_dir)
                    .map_err(std::io::Error::other)?,
                toolchain: rustc_version()?,
            })
        } else {
            None
        };
        let coverage = match &provenance {
            Some(_) => cc::evidence::Coverage::source_scan_with_provenance(self.violations() == 0, true),
            None => cc::evidence::Coverage::source_scan(self.violations() == 0),
        };
        let mut payload = serde_json::json!({
            "tool": "cargo-nirdosha",
            "dialect": "nirdosha-rt",
            "stage": 1.5,
            "mode": "workspace",
            "in_dialect_packages": self.all_packages,
            "contracts": contracts,
            "findings": findings,
            "violations": self.violations(),
            "coverage": coverage,
        });
        if let Some(p) = &provenance {
            payload["provenance"] = serde_json::to_value(p)?;
        }
        // Aggregate certificate: every in-dialect package's sources,
        // prefixed `<package>/<path>` so paths stay unambiguous.
        let mut sources = Vec::new();
        for summary in &self.packages {
            for source in cc::certificate::scan_sources(&summary.package_dir, &summary.files)? {
                sources.push(cc::certificate::SourceFile {
                    path: format!("{}/{}", summary.package, source.path),
                    sha256: source.sha256,
                });
            }
        }
        let certificate = cc::certificate::Certificate::new(
            cc::certificate::Subject { package: "workspace".into(), version: None },
            cc::certificate::Tool {
                name: "cargo-nirdosha".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                mode: cc::certificate::Mode::SourceScan,
                toolchain: provenance.as_ref().map(|p| p.toolchain.clone()),
            },
            sources,
            payload,
        );
        fs::write(&path, serde_json::to_vec_pretty(&certificate)?)?;
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
