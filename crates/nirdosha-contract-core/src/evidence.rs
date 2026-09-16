//! Coverage for source-scan reports, and conservative consuming policy.
//!
//! Integrity is not authentication. Callers must trust the producer separately.
use crate::certificate::{Certificate, Mode};
use crate::provenance::{self, Provenance};
use serde::{Deserialize, Serialize};

pub const PROFILE: &str = "nirdosha.source-scan/v1";
pub const GUARANTEES: &[&str] = &[
    "source_scan",
    "effects_pure",
    "deadlock_freedom",
    "deterministic_execution",
    "authenticated_identity",
    "durable_transactions",
    "process_isolation",
    "build_provenance",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Passed,
    Failed,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Claim {
    pub guarantee: String,
    pub outcome: Outcome,
    pub method: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Coverage {
    pub profile: String,
    pub scope: String,
    pub claims: Vec<Claim>,
    pub limitations: Vec<String>,
}

impl Coverage {
    pub fn source_scan(passed: bool) -> Self {
        Self {
            profile: PROFILE.into(),
            scope: "listed source files only; not a resolved Cargo build".into(),
            claims: GUARANTEES
                .iter()
                .map(|name| Claim {
                    guarantee: (*name).into(),
                    outcome: if *name == "source_scan" {
                        if passed {
                            Outcome::Passed
                        } else {
                            Outcome::Failed
                        }
                    } else {
                        Outcome::Unsupported
                    },
                    method: if *name == "source_scan" {
                        "syntactic_source_scan"
                    } else {
                        "none"
                    }
                    .into(),
                })
                .collect(),
            limitations: [
                "no macro expansion or complete Cargo target/cfg coverage",
                "no resolved transitive effects, dependency or destructor analysis",
                "no production identity, durability, isolation or progress evidence",
                "no workload measurements or formal proof discharge",
                "no executable, toolchain/configuration or dependency closure binding",
                "unsigned: binding integrity does not authenticate the issuer",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        }
    }

    /// Extends a source-scan coverage claim set with a bound dependency
    /// closure (`Cargo.lock`) and toolchain. Still not issuer
    /// authentication — see `provenance` module docs.
    pub fn source_scan_with_provenance(source_scan_passed: bool, provenance_passed: bool) -> Self {
        let mut cov = Self::source_scan(source_scan_passed);
        cov.scope = "listed source files, plus dependency-closure (Cargo.lock) and toolchain binding; not a resolved Cargo build".into();
        for claim in &mut cov.claims {
            if claim.guarantee == "build_provenance" {
                claim.outcome = if provenance_passed { Outcome::Passed } else { Outcome::Failed };
                claim.method = BUILD_PROVENANCE_METHOD.into();
            }
        }
        cov.limitations.retain(|l| l != "no executable, toolchain/configuration or dependency closure binding");
        cov.limitations.push(
            "dependency closure = Cargo.lock bytes only; does not bind the built executable, cfg flags/features, or authenticate the issuer".into(),
        );
        cov
    }
}

pub const BUILD_PROVENANCE_METHOD: &str = "dependency_closure_and_toolchain_binding";

/// Evaluate required guarantees. Source freshness is checked separately by
/// the CLI using the caller-provided package root. Older reports fail closed.
pub fn check_policy(cert: &Certificate, required: &[String]) -> Result<(), String> {
    if required.is_empty() {
        return Err("at least one required guarantee is necessary".into());
    }
    if !cert.binding_valid() {
        return Err("certificate binding is invalid".into());
    }
    if cert.tool.mode != Mode::SourceScan {
        return Err("unsupported certificate analysis mode".into());
    }
    if cert.subject.package == "workspace" {
        return Err(
            "evaluate package certificates individually; workspace source roots are not resolved"
                .into(),
        );
    }
    if cert.sources.is_empty() {
        return Err("certificate has no source coverage".into());
    }
    let violations = cert
        .verification
        .get("violations")
        .and_then(|v| v.as_u64())
        .ok_or("missing violation count")?;
    if violations != 0 {
        return Err("certificate records verification violations".into());
    }
    let coverage: Coverage = serde_json::from_value(
        cert.verification
            .get("coverage")
            .cloned()
            .ok_or("certificate has no coverage evidence")?,
    )
    .map_err(|e| format!("invalid coverage evidence: {e}"))?;
    // A source scanner cannot promote its result into stronger evidence.
    // A new producer/profile needs a deliberate policy implementation.
    let has_provenance = cert.verification.get("provenance").is_some();
    let expected = if has_provenance {
        Coverage::source_scan_with_provenance(true, true)
    } else {
        Coverage::source_scan(true)
    };
    if coverage != expected {
        return Err("unsupported or inconsistent source-scan coverage".into());
    }
    for requirement in required {
        let claim = coverage
            .claims
            .iter()
            .find(|c| &c.guarantee == requirement)
            .ok_or_else(|| format!("unknown required guarantee `{requirement}`"))?;
        if claim.outcome != Outcome::Passed {
            return Err(format!(
                "required guarantee `{requirement}` is unsupported by source_scan"
            ));
        }
    }
    Ok(())
}

/// Re-derives the dependency-closure hash at `root` independently (never
/// trusting a certificate-supplied path — the same discipline
/// `Certificate::check_sources` uses) and compares it to what the
/// certificate bound. Toolchain is recorded on the certificate but not
/// re-verified here: the checking machine may run a different rustc than
/// the one that produced the certificate.
pub fn check_provenance(cert: &Certificate, root: &std::path::Path) -> Result<(), String> {
    let bound: Provenance = serde_json::from_value(
        cert.verification
            .get("provenance")
            .cloned()
            .ok_or("certificate has no provenance evidence")?,
    )
    .map_err(|e| format!("invalid provenance evidence: {e}"))?;
    let current = provenance::hash_cargo_lock(root)?;
    if current != bound.cargo_lock_sha256 {
        return Err(
            "dependency closure (Cargo.lock) has changed since this certificate was minted".into(),
        );
    }
    Ok(())
}
