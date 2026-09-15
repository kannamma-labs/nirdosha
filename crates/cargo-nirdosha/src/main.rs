//! `cargo nirdosha` — the Nirdosha compiler CLI, Stage 1.
//!
//! Usage (as a cargo subcommand):
//!
//! ```text
//! cargo nirdosha build      # verify contracts, then delegate to `cargo build`
//! cargo nirdosha check
//! cargo nirdosha run
//! cargo nirdosha test
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
//! 4. Otherwise delegate to the real cargo with the remaining args, and
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
        "verify" => {
            let workspace = rest.iter().any(|a| a == "--workspace" || a == "--all");
            if workspace {
                return verify_ws_cli();
            }
            let summary = match verify_cwd() {
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
            // `cargo nirdosha check --workspace` gates every in-dialect
            // crate (strict) before delegating the workspace-wide cargo.
            if rest.iter().any(|a| a == "--workspace" || a == "--all") {
                let exit = verify_ws_cli();
                if exit != ExitCode::SUCCESS {
                    eprintln!(
                        "nirdosha: refusing to {s} the workspace — in-dialect crates have violations"
                    );
                    return exit;
                }
                return delegate(s, rest);
            }
            let summary = match verify_cwd() {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("nirdosha: {e}");
                    return ExitCode::FAILURE;
                }
            };
            if summary.violations().is_empty() {
                report(&summary, &[]);
                delegate(s, rest)
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

fn usage() {
    eprintln!(
        "cargo-nirdosha — the Nirdosha compiler (Stage 1.5)\n\
         \n\
         usage: cargo nirdosha <build|check|run|test|doc|verify|bench> [cargo args] [--workspace]\n\
         \n\
         Same source, two compilers:\n\
         - plain `cargo build`            compiles and runs Nirdosha code like any Rust program\n\
         - `cargo nirdosha build`         verifies contract claims first; a lie refuses the build\n\
         - `cargo nirdosha verify --workspace`  strict gate over every in-dialect crate + certificate\n\
         - `cargo nirdosha bench`          nfr(latency_ms) CI gate: your test suite is the workload"
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

fn verify_cwd() -> Result<ScanSummary, String> {
    let loc = locate()?;
    let strict = std::env::var_os("NIRDOSHA_STRICT").is_some_and(|v| !v.is_empty());
    let summary = verify_package(&loc.manifest_dir, strict)?;
    let report_path = summary
        .write_report(&loc.target_dir)
        .map_err(|e| format!("cannot write certificate: {e}"))?;
    eprintln!(
        "nirdosha: certificate → {}",
        report_path.display()
    );
    Ok(summary)
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
        "nirdosha: {} — {} files, {} contracts verified, {} violations",
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
fn verify_ws_cli() -> ExitCode {
    match cargo_nirdosha::verify_workspace(true) {
        Ok(ws) => {
            let target_dir = match workspace_target_dir() {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("nirdosha: {e}");
                    return ExitCode::FAILURE;
                }
            };
            match ws.write_reports(&target_dir) {
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
                "nirdosha: workspace — {} in-dialect package(s) verified, {} contracts, {} violations",
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
    let summary = match verify_cwd() {
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