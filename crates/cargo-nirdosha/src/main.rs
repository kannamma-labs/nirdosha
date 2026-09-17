//! `cargo nirdosha` — the Nirdosha compiler CLI.
//!
//! Usage (as a cargo subcommand):
//!
//! ```text
//! cargo nirdosha build      # Stage 1 scan + Stage 2 (MIR effect lattice), then `cargo build`
//! cargo nirdosha check
//! cargo nirdosha run
//! cargo nirdosha test
//! cargo nirdosha build --fast   # Stage 1 only: sub-second, no nightly/rustc-dev needed
//! cargo nirdosha verify     # verify + emit certificate, no cargo delegation
//! cargo nirdosha fmt        # pass-through: no contract semantics involved
//! ```
//!
//! Behavior:
//! 1. Locate the package the way cargo does (via `cargo metadata`).
//! 2. Scan its `src/` tree: parse every contract (attribute or doc form),
//!    verify `effects(pure)` claims against bodies, enforce dialect
//!    restrictions.
//! 3. Any violation → the build is REFUSED (plain `cargo build` would
//!    have accepted the same code; that difference is the product).
//! 4. Unless `--fast`/`--shallow` was given, the Stage-2 rustc driver
//!    (MIR-level interprocedural effects) rides along as
//!    `RUSTC_WORKSPACE_WRAPPER` — a pure claim that is only locally
//!    clean but reaches an impure call through the call graph is
//!    caught here, not just body-local lies (issue #74: this is the
//!    certifying default now; Stage 1 alone is an opt-in, IDE-speed
//!    hint).
//! 5. Otherwise delegate to the real cargo with the remaining args, and
//!    emit the certificate next to the artifacts:
//!    `<target>/nirdosha/contract-report-<package>.json`.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use cargo_nirdosha::{verify_package, ScanSummary, Severity};

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    // cargo invokes subcommand binaries as `cargo-nirdosha nirdosha <sub>`
    // — the subcommand name rides through as argv[1] (same convention
    // cargo-clippy handles). Strip it; direct invocation is also fine.
    if args.first().map(String::as_str) == Some("nirdosha") {
        args.remove(0);
    }
    let Some(sub) = args.first().cloned() else {
        usage();
        return ExitCode::FAILURE;
    };
    let rest = if args.len() > 1 { &args[1..] } else { &[][..] };

    match sub.as_str() {
        "check-certificate" => check_certificate_cli(rest),
        "verify" => {
            let workspace = rest.iter().any(|a| a == "--workspace" || a == "--all");
            // `--provenance` (G6): additionally binds the resolved
            // dependency closure (Cargo.lock) and toolchain into the
            // certificate. Mechanical binding only, not issuer
            // authentication.
            let bind_provenance = rest.iter().any(|a| a == "--provenance");
            if workspace {
                return verify_ws_cli(bind_provenance);
            }
            if rest.iter().any(|a| a == "--audit") {
                return audit_cli(rest.iter().find(|a| !a.starts_with("--")).cloned());
            }
            let summary = match verify_cwd(bind_provenance) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("nirdosha: {e}");
                    return ExitCode::FAILURE;
                }
            };
            report(&summary, &[]);
            if summary.violations().is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        "bench" => bench_cli(rest),
        s if matches!(s, "build" | "check" | "run" | "test" | "bench" | "doc") => {
            // Stage 2 (the rustc driver, MIR-level interprocedural
            // effects) is the certifying default — see issue #74: a
            // body-local source scan cannot see a pure claim that
            // reaches an impure call three hops down the call graph,
            // and the certificate's headline claim should not be
            // weaker than what the driver can already prove.
            // `--fast`/`--shallow` opts back into Stage-1-only, for
            // sub-second IDE-time feedback with no nightly/rustc-dev
            // requirement. `--deep` is still accepted (and now a
            // no-op) so existing invocations keep working.
            let shallow = rest.iter().any(|a| a == "--fast" || a == "--shallow");
            let deep = !shallow;
            let cargo_args: Vec<String> = rest
                .iter()
                .filter(|a| !matches!(a.as_str(), "--deep" | "--fast" | "--shallow"))
                .cloned()
                .collect();
            // `cargo nirdosha check --workspace` gates every in-dialect
            // crate (strict) before delegating the workspace-wide cargo.
            if cargo_args.iter().any(|a| a == "--workspace" || a == "--all") {
                let exit = verify_ws_cli(false);
                if exit != ExitCode::SUCCESS {
                    eprintln!(
                        "nirdosha: refusing to {s} the workspace — in-dialect crates have violations"
                    );
                    return exit;
                }
                return delegate_with(s, &cargo_args, deep);
            }
            let summary = match verify_cwd(false) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("nirdosha: {e}");
                    return ExitCode::FAILURE;
                }
            };
            if summary.violations().is_empty() {
                report(&summary, &[]);
                delegate_with(s, &cargo_args, deep)
            } else {
                report(&summary, &[s]);
                ExitCode::FAILURE
            }
        }
        s => {
            eprintln!("nirdosha: `{s}` runs as unverified pass-through (no contract semantics)");
            delegate(s, rest)
        }
    }
}

/// Explicit consumer gate, deliberately independent of Cargo metadata.
fn check_certificate_cli(args: &[String]) -> ExitCode {
    let result = (|| -> Result<bool, String> {
        let mut path = None;
        let mut root = None;
        let mut required = Vec::new();
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--root" if root.is_none() => root = Some(PathBuf::from(args.next().ok_or("--root needs a directory")?)),
                "--require" => required.push(args.next().ok_or("--require needs a guarantee")?.clone()),
                value if !value.starts_with('-') && path.is_none() => path = Some(PathBuf::from(value)),
                _ => return Err(format!("unexpected argument `{arg}`")),
            }
        }
        let path = path.ok_or("check-certificate needs a certificate path")?;
        let root = root.ok_or("check-certificate needs --root <package directory>")?;
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        let cert: nirdosha_contract_core::certificate::Certificate =
            serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        nirdosha_contract_core::evidence::check_policy(&cert, &required)?;
        // Refuse paths that could read outside the caller's intended root.
        let root = root.canonicalize().map_err(|e| e.to_string())?;
        for source in &cert.sources {
            let relative = Path::new(&source.path);
            if relative.components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
                return Err(format!("invalid source path `{}`", source.path));
            }
            let resolved = root.join(relative).canonicalize().map_err(|e| e.to_string())?;
            if !resolved.starts_with(&root) { return Err("source escapes package root".into()); }
        }
        if !cert.check_sources(&root).ok() { return Err("listed source files changed or disappeared".into()); }
        // G6: if build_provenance was required, independently re-derive
        // the dependency-closure hash at root and compare — never trust
        // the certificate's own claim without re-checking against the
        // filesystem, same discipline as the source re-hash above.
        let provenance_checked = required.iter().any(|r| r == "build_provenance");
        if provenance_checked {
            nirdosha_contract_core::evidence::check_provenance(&cert, &root)?;
        }
        Ok(provenance_checked)
    })();
    match result {
        Ok(provenance_checked) => {
            if provenance_checked {
                eprintln!("nirdosha: certificate policy passed; listed sources and dependency closure match. Issuer authentication is not established.");
            } else {
                eprintln!("nirdosha: certificate policy passed; listed sources match. Issuer authentication and build provenance are not established.");
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("nirdosha: certificate policy refused: {error}");
            ExitCode::FAILURE
        }
    }
}

fn usage() {
    eprintln!(
        "cargo-nirdosha — the Nirdosha compiler\n\
         \n\
         usage: cargo nirdosha <build|check|run|test|doc|verify|bench> [cargo args] [--workspace] [--fast]\n\
         \n\
         Same source, two compilers:\n\
         - plain `cargo build`            compiles and runs Nirdosha code like any Rust program\n\
         - `cargo nirdosha build`         Stage 1 scan + Stage 2 MIR effect lattice; a lie refuses the build\n\
         - `cargo nirdosha build --fast`  Stage 1 only (source scan): sub-second, no nightly/rustc-dev needed\n\
         - `cargo nirdosha verify --workspace`  strict gate over every in-dialect crate + certificate\n\
         - `cargo nirdosha verify --provenance`  also binds Cargo.lock + toolchain into the certificate\n\
         - `cargo nirdosha bench`          nfr(latency_ms) CI gate: your test suite is the workload\n\
         - `cargo nirdosha check-certificate <path> --root <package> --require <guarantee>`"
    );
}

struct Location {
    manifest_dir: PathBuf,
    target_dir: PathBuf,
}

fn locate() -> Result<Location, String> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let out = Command::new("cargo")
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
    for package in packages {
        let Some(manifest) = package.get("manifest_path").and_then(|v| v.as_str()) else {
            continue;
        };
        let manifest = Path::new(manifest);
        if manifest.parent() == Some(cwd.as_path()) {
            return Ok(Location {
                manifest_dir: manifest.parent().unwrap().to_path_buf(),
                target_dir,
            });
        }
    }
    Err(format!(
        "cwd {} is not a package root — run cargo-nirdosha from inside the package you want verified",
        cwd.display()
    ))
}

fn verify_cwd(bind_provenance: bool) -> Result<ScanSummary, String> {
    let loc = locate()?;
    let strict = std::env::var_os("NIRDOSHA_STRICT").is_some_and(|v| !v.is_empty());
    let summary = verify_package(&loc.manifest_dir, strict)?;
    let report_path = summary
        .write_report(&loc.target_dir, bind_provenance)
        .map_err(|e| format!("cannot write certificate: {e}"))?;
    eprintln!(
        "nirdosha: certificate → {}",
        report_path.display()
    );
    Ok(summary)
}

/// `cargo nirdosha verify --audit [path]` — re-check a certificate
/// against the sources it attests to: is the binding intact (nobody
/// edited the claims), and do the source hashes still match (nobody
/// edited the code since it was verified)? Exit 0 only when both hold.
fn audit_cli(path: Option<String>) -> ExitCode {
    let loc = match locate() {
        Ok(loc) => loc,
        Err(e) => {
            eprintln!("nirdosha: {e}");
            return ExitCode::FAILURE;
        }
    };
    let path = path.unwrap_or_else(|| {
        loc.target_dir
            .join("nirdosha")
            .join("contract-report-<unknown-package>.json")
            .to_string_lossy()
            .into_owned()
    });
    let path = if path.contains("<unknown-package>") {
        // no explicit path: <target>/nirdosha/contract-report-<pkg>.json,
        // pkg from the manifest in the cwd
        let package = std::fs::read_to_string(loc.manifest_dir.join("Cargo.toml"))
            .ok()
            .and_then(|t| t.lines().find_map(|l| {
                let l = l.trim();
                l.starts_with("name")
                    .then(|| l.split('"').nth(1).map(str::to_string))
                    .flatten()
            }))
            .unwrap_or_default();
        loc.target_dir
            .join("nirdosha")
            .join(format!("contract-report-{package}.json"))
    } else {
        std::path::PathBuf::from(path)
    };
    let cert: nirdosha_contract_core::certificate::Certificate =
        match std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()) {
            Some(cert) => cert,
            None => {
                eprintln!(
                    "nirdosha: cannot read certificate {} — run `cargo nirdosha verify` first",
                    path.display()
                );
                return ExitCode::FAILURE;
            }
        };
    let mut ok = true;
    if cert.binding_valid() {
        eprintln!("nirdosha: audit binding OK — content matches its stored hash ({})", cert.binding);
    } else {
        eprintln!("nirdosha: audit FAILED — binding mismatch; this certificate was edited");
        ok = false;
    }
    let audit = cert.check_sources(&loc.manifest_dir);
    if audit.ok() {
        eprintln!(
            "nirdosha: audit sources OK — {} file(s) match their verified hashes",
            audit.matched
        );
    } else {
        for path in &audit.changed {
            eprintln!("nirdosha: audit FAILED — {path} changed since it was verified");
        }
        for path in &audit.missing {
            eprintln!("nirdosha: audit FAILED — {path} is missing since it was verified");
        }
        ok = false;
    }
    if ok {
        eprintln!("nirdosha: listed source hashes and certificate binding match; this does not authenticate the issuer or establish build provenance");
        ExitCode::SUCCESS
    } else {
        eprintln!(
            "nirdosha: refusing to trust this certificate — plain `cargo build` would have accepted this code; that difference is the product"
        );
        ExitCode::FAILURE
    }
}

fn report(summary: &ScanSummary, refused_for: &[&str]) {
    for finding in &summary.findings {
        let severity = match finding.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        let at = finding
            .function
            .as_ref()
            .map(|f| format!(" fn `{f}`"))
            .unwrap_or_default();
        eprintln!(
            "nirdosha: {severity} {}:{}{at} — {}",
            finding.file.display(),
            finding.line,
            finding.message
        );
    }
    eprintln!(
        "nirdosha: {} — source_scan: {} files, {} contracts inspected, {} violations",
        summary.package,
        summary.files.len(),
        summary.contracts.len(),
        summary.violations().len()
    );
    if !summary.violations().is_empty() && !refused_for.is_empty() {
        eprintln!(
            "nirdosha: refusing to {} — plain `cargo {}` would have accepted this code; that difference is the product",
            refused_for[0],
            refused_for[0]
        );
    }
}

/// The workspace's target directory (works from the root, unlike
/// `locate`, which demands a package cwd).
fn workspace_target_dir() -> Result<PathBuf, String> {
    let out = Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .output()
        .map_err(|e| format!("cannot run cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err("cargo metadata failed — is this a cargo workspace?".into());
    }
    let meta: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
    meta.get("target_directory")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .ok_or_else(|| "cargo metadata has no target_directory".to_string())
}

/// `cargo nirdosha verify --workspace`: verify every in-dialect crate,
/// strict by default, plus the aggregate certificate.
fn verify_ws_cli(bind_provenance: bool) -> ExitCode {
    match cargo_nirdosha::verify_workspace(true) {
        Ok(ws) => {
            let target_dir = match workspace_target_dir() {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("nirdosha: {e}");
                    return ExitCode::FAILURE;
                }
            };
            match ws.write_reports(&target_dir, bind_provenance) {
                Ok(path) => eprintln!("nirdosha: workspace certificate → {}", path.display()),
                Err(e) => {
                    eprintln!("nirdosha: cannot write certificate: {e}");
                    return ExitCode::FAILURE;
                }
            }
            for summary in &ws.packages {
                for finding in &summary.findings {
                    let severity = match finding.severity {
                        Severity::Error => "error",
                        Severity::Warning => "warning",
                    };
                    eprintln!(
                        "nirdosha: {severity} [{}] {}:{} — {}",
                        summary.package,
                        finding.file.display(),
                        finding.line,
                        finding.message
                    );
                }
            }
            eprintln!(
                "nirdosha: workspace source_scan — {} in-dialect package(s) scanned, {} contracts, {} violations",
                ws.packages.len(),
                ws.contracts(),
                ws.violations()
            );
            if ws.violations() == 0 {
                ExitCode::SUCCESS
            } else {
                eprintln!(
                    "nirdosha: plain `cargo build` would have accepted all of it; that difference is the product"
                );
                ExitCode::FAILURE
            }
        }
        Err(e) => {
            eprintln!("nirdosha: {e}");
            ExitCode::FAILURE
        }
    }
}

/// `cargo nirdosha bench [test args…]`: the nfr(latency_ms) CI gate.
/// Verifies first, then runs the package's own test suite as the load
/// generator, with the flight recorder's file sink pointed at
/// target/nirdosha/. The p95 per guarded function is compared against
/// the declared limit. Exit nonzero on any breach — or on any verify
/// failure, so a lying program never gets a bench verdict.
fn bench_cli(rest: &[String]) -> ExitCode {
    let summary = match verify_cwd(false) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("nirdosha: {e}");
            return ExitCode::FAILURE;
        }
    };
    if !summary.violations().is_empty() {
        report(&summary, &["bench"]);
        return ExitCode::FAILURE;
    }
    let loc = match locate() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("nirdosha: {e}");
            return ExitCode::FAILURE;
        }
    };
    let dir = loc.target_dir.join("nirdosha");
    if std::fs::create_dir_all(&dir).is_err() {
        eprintln!("nirdosha: cannot create {}", dir.display());
        return ExitCode::FAILURE;
    }
    let sink = dir.join(format!("nfr-events-{}.ndjson", summary.package));
    let _ = std::fs::remove_file(&sink);

    eprintln!(
        "nirdosha: bench — running `cargo test` as the workload (flight recorder → {})",
        sink.display()
    );
    let status = Command::new("cargo")
        .arg("test")
        .args(rest)
        .env("NIRDOSHA_NFR_LOG_FILE", &sink)
        .status();
    let status = match status {
        Ok(s) => s,
        Err(e) => {
            eprintln!("nirdosha: cannot run cargo test: {e}");
            return ExitCode::FAILURE;
        }
    };
    if !status.success() {
        return ExitCode::from(status.code().unwrap_or(1) as u8);
    }

    let events = cargo_nirdosha::read_bench_events(&sink);
    let verdicts = cargo_nirdosha::evaluate_bench(&events, &summary.contracts);
    let mut breaches = 0;
    let mut unmeasured = 0;
    for v in &verdicts {
        let Some(limit) = v.limit_ms else { continue };
        let verdict = if v.ok { "PASS" } else { "FAIL" };
        if !v.ok {
            breaches += 1;
        }
        eprintln!(
            "nirdosha: bench {verdict} fn `{}` — {} call(s), p50 {:.3}ms, p95 {:.3}ms, max {:.3}ms (limit {}ms)",
            v.function, v.calls, v.p50_ms, v.p95_ms, v.max_ms, limit
        );
    }
    for contract in &summary.contracts {
        if contract.contract.nfr.is_some_and(|n| n.latency_ms.is_some())
            && verdicts
                .iter()
                .find(|v| v.function == contract.function)
                .is_some_and(|v| v.calls == 0)
        {
            unmeasured += 1;
            eprintln!(
                "nirdosha: bench WARN fn `{}` declares nfr(latency_ms) but the workload never called it",
                contract.function
            );
        }
    }
    let report = serde_json::json!({
        "tool": "cargo-nirdosha",
        "dialect": "nirdosha-rt",
        "mode": "bench",
        "package": summary.package,
        "events": events.len(),
        "verdicts": verdicts,
    });
    let path = dir.join(format!("bench-report-{}.json", summary.package));
    if let Ok(json) = serde_json::to_vec_pretty(&report) {
        let _ = std::fs::write(&path, json);
        eprintln!("nirdosha: bench report → {}", path.display());
    }
    if breaches > 0 {
        eprintln!("nirdosha: bench — {breaches} SLA breach(es); refusing");
        ExitCode::FAILURE
    } else {
        eprintln!(
            "nirdosha: bench — {} SLA(s) held, {unmeasured} unmeasured",
            verdicts
                .iter()
                .filter(|v| v.limit_ms.is_some())
                .count()
        );
        if unmeasured > 0 {
            ExitCode::FAILURE
        } else {
            ExitCode::SUCCESS
        }
    }
}

/// The Stage-2 driver binary, expected next to this executable (same
/// target dir). Build it with `cargo build -p nirdosha-driver`.
fn driver_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.join("nirdosha-driver");
    dir.is_file().then_some(dir)
}

/// Delegate to cargo, optionally with the Stage-2 rustc driver attached
/// as RUSTC_WORKSPACE_WRAPPER. `deep` is true unless `--fast`/`--shallow`
/// was given (issue #74: Stage 2 is the certifying default).
fn delegate_with(sub: &str, rest: &[String], deep: bool) -> ExitCode {
    let mut command = Command::new("cargo");
    command.arg(sub).args(rest);
    if deep {
        let Some(driver) = driver_path() else {
            eprintln!(
                "nirdosha: the Stage-2 driver is not built next to cargo-nirdosha \
                 — run `cargo build -p nirdosha-driver` first (nightly + rustc-dev), \
                 or pass `--fast`/`--shallow` for the weaker, Stage-1-only source scan"
            );
            return ExitCode::FAILURE;
        };
        eprintln!(
            "nirdosha: deep verification active — MIR effects lattice via {}",
            driver.display()
        );
        command.env("NIRDOSHA_DRIVER", "1");
        command.env("RUSTC_WORKSPACE_WRAPPER", &driver);
    }
    match command.status() {
        Ok(status) => status
            .code()
            .map(|code| ExitCode::from(code as u8))
            .unwrap_or(ExitCode::SUCCESS),
        Err(e) => {
            eprintln!("nirdosha: cannot run cargo {sub}: {e}");
            ExitCode::FAILURE
        }
    }
}

fn delegate(sub: &str, rest: &[String]) -> ExitCode {
    match Command::new("cargo").arg(sub).args(rest).status() {
        Ok(status) => status
            .code()
            .map(|code| ExitCode::from(code as u8))
            .unwrap_or(ExitCode::SUCCESS),
        Err(e) => {
            eprintln!("nirdosha: cannot run cargo {sub}: {e}");
            ExitCode::FAILURE
        }
    }
}
