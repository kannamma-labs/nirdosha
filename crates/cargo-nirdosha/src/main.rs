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
        s if matches!(s, "build" | "check" | "run" | "test" | "bench" | "doc") => {
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
        "cargo-nirdosha — the Nirdosha compiler (Stage 1)\n\
         \n\
         usage: cargo nirdosha <build|check|run|test|bench|doc|verify> [cargo args]\n\
         \n\
         Same source, two compilers:\n\
         - plain `cargo build`     compiles and runs Nirdosha code like any Rust program\n\
         - `cargo nirdosha build`  additionally verifies contract claims first, and\n\
                                   refuses to build if a claim is a lie"
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