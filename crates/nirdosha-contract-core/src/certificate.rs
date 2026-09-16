//! `nirdosha.certificate/v1` — the artifact that makes a verification
//! claim checkable by anyone (#6 in the deferred-work register).
//!
//! A certificate binds **what was verified** to **exactly what it was
//! verified against**:
//!
//! - `sources` — every file examined, each with its SHA-256. Paths are
//!   relative to the package root, so the certificate is portable.
//! - `tool` — which compiler surface produced it (`source-scan` today,
//!   `mir-driver` for Stage 2; the proprietary `.nir` compiler adopts
//!   this same schema with its own mode).
//! - `verification` — the tier-specific report payload (contracts,
//!   findings, violations) embedded verbatim.
//! - `binding` — SHA-256 over all of the above. Change one byte of
//!   source, or one claim, and the binding changes.
//!
//! Three properties, deliberate:
//!
//! 1. **Determinism.** No timestamps, no paths outside the package, no
//!    ambient state: the same sources verified by the same tool yield a
//!    *byte-identical* certificate (`docs/goal.md` Row 10's
//!    reproducible-build requirement — "the compiler is a deterministic
//!    function of source + flags" — extends to its attestations).
//!    Re-verify and diff; zero diff means zero drift.
//! 2. **Two reserved fields, one future each.** `proofs` carries
//!    Stage 2.5's Z3 discharge objects (empty establishes no proof;
//!    see verification.coverage). `signature` is entry #13's envelope (always
//!    `null` in v1; a signature covers the `binding`, never lives
//!    inside what it signs).
//! 3. **One schema across tiers.** The free dialect and the
//!    proprietary compiler emit the same envelope, so a consumer never
//!    learns which tier produced a certificate — only what it attests.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// The schema discriminator written into every certificate.
pub const SCHEMA: &str = "nirdosha.certificate/v1";

/// Which compiler surface produced the certificate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Stage 1: source-scan verification (rustc-independent).
    SourceScan,
    /// Stage 2: the rustc driver over MIR (nightly + rustc-dev).
    MirDriver,
}

/// One examined source file, hashed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceFile {
    /// Relative to the package root (portable, reproducible).
    pub path: String,
    /// Hex SHA-256 of the file's bytes at verification time.
    pub sha256: String,
}

/// What was verified.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subject {
    /// Package name (or `"workspace"` for the aggregate certificate).
    pub package: String,
    /// Package version; `None` for the workspace aggregate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// What verified it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    /// e.g. `"cargo-nirdosha"`; the proprietary tier names itself here.
    pub name: String,
    /// Tool version.
    pub version: String,
    /// Which surface ran (see [`Mode`]).
    pub mode: Mode,
    /// The toolchain the verification actually depends on. `None` is
    /// *honest* for `source-scan` (the scan never invokes rustc); the
    /// MIR driver pins its exact nightly here.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub toolchain: Option<String>,
}

/// Entry #13's signature envelope — reserved, always `None` in v1. A
/// signature attests the `binding`; it is computed over it, not stored
/// inside what it covers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signature {
    pub algorithm: String,
    pub key_id: String,
    pub value: String,
}

/// The certificate. Every field except `binding` and `signature` is
/// covered by the binding hash.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Certificate {
    pub schema: String,
    pub subject: Subject,
    pub tool: Tool,
    pub sources: Vec<SourceFile>,
    /// Stage 2.5 Z3 discharge objects. Empty in v1: claims here are
    /// described by verification.coverage, not inferred to be proven.
    #[serde(default)]
    pub proofs: Vec<Value>,
    /// The tier-specific report (contracts, findings, violations),
    /// embedded verbatim.
    pub verification: Value,
    /// Entry #13: signed-plugin trust chain. `None` in v1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Signature>,
    /// SHA-256 hex over {schema, subject, tool, sources, proofs,
    /// verification}.
    pub binding: String,
}

impl Certificate {
    /// Build a certificate, computing its binding. `sources` must use
    /// package-root-relative paths — see [`scan_sources`].
    pub fn new(
        subject: Subject,
        tool: Tool,
        sources: Vec<SourceFile>,
        verification: Value,
    ) -> Self {
        let mut cert = Self {
            schema: SCHEMA.to_string(),
            subject,
            tool,
            sources,
            proofs: Vec::new(),
            verification,
            signature: None,
            binding: String::new(),
        };
        cert.binding = cert.compute_binding();
        cert
    }

    /// The binding payload: every bound field. `serde_json::Value` maps
    /// are BTree-ordered, so serialization is canonical — identical
    /// content produces identical bytes, which is what the hash rides
    /// on.
    fn binding_payload(&self) -> Value {
        serde_json::json!({
            "schema": self.schema,
            "subject": self.subject,
            "tool": self.tool,
            "sources": self.sources,
            "proofs": self.proofs,
            "verification": self.verification,
        })
    }

    /// SHA-256 hex over the binding payload.
    fn compute_binding(&self) -> String {
        let bytes = serde_json::to_vec(&self.binding_payload())
            .expect("certificate payload is serializable");
        hex(&Sha256::digest(&bytes))
    }

    /// Does the stored binding match the certificate's content? An edit
    /// without recomputing the hash fails. This is not authentication:
    /// anyone can recompute an unsigned certificate's binding.
    pub fn binding_valid(&self) -> bool {
        self.schema == SCHEMA && self.binding == self.compute_binding()
    }

    /// Re-hash the listed sources against `root` (the package root the
    /// paths are relative to). Answers "did any verified file change or
    /// disappear since this certificate was minted?"
    pub fn check_sources(&self, root: &Path) -> SourceAudit {
        let mut audit = SourceAudit::default();
        for source in &self.sources {
            let path = root.join(&source.path);
            match fs_read_hash(&path) {
                Some(sha) if sha == source.sha256 => audit.matched += 1,
                Some(_) => audit.changed.push(source.path.clone()),
                None => audit.missing.push(source.path.clone()),
            }
        }
        audit
    }
}

/// What a source re-check found.
#[derive(Debug, Default)]
pub struct SourceAudit {
    pub matched: usize,
    pub changed: Vec<String>,
    pub missing: Vec<String>,
}

impl SourceAudit {
    pub fn ok(&self) -> bool {
        self.changed.is_empty() && self.missing.is_empty()
    }
}

/// SHA-256 hex of a file's bytes; `None` if unreadable.
fn fs_read_hash(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    Some(hex(&Sha256::digest(&bytes)))
}

/// Hash files into package-relative [`SourceFile`]s. Paths that do not
/// live under `root` (external fixtures) are recorded under their file
/// name only — hashed all the same, so nothing verified escapes the
/// binding.
pub fn scan_sources(root: &Path, files: &[std::path::PathBuf]) -> std::io::Result<Vec<SourceFile>> {
    files
        .iter()
        .map(|file| {
            let sha256 = fs_read_hash(file).ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("cannot hash {}", file.display()),
                )
            })?;
            let path = file
                .strip_prefix(root)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| file.file_name().unwrap_or_default().to_string_lossy().into_owned());
            Ok(SourceFile { path, sha256 })
        })
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subject() -> Subject {
        Subject { package: "demo".into(), version: Some("0.1.0".into()) }
    }

    fn tool() -> Tool {
        Tool { name: "cargo-nirdosha".into(), version: "0.1.0".into(), mode: Mode::SourceScan, toolchain: None }
    }

    #[test]
    fn binding_is_deterministic() {
        let sources = vec![SourceFile { path: "src/main.rs".into(), sha256: "deadbeef".into() }];
        let verification = serde_json::json!({ "violations": 0, "contracts": 3 });
        let a = Certificate::new(subject(), tool(), sources.clone(), verification.clone());
        let b = Certificate::new(subject(), tool(), sources, verification);
        assert_eq!(a.binding, b.binding);
        assert!(a.binding_valid());
    }

    #[test]
    fn content_change_changes_the_binding() {
        let sources = vec![SourceFile { path: "src/main.rs".into(), sha256: "deadbeef".into() }];
        let a = Certificate::new(subject(), tool(), sources.clone(), serde_json::json!({ "violations": 0 }));
        let b = Certificate::new(
            subject(),
            tool(),
            sources,
            serde_json::json!({ "violations": 1 }),
        );
        assert_ne!(a.binding, b.binding);
        // A certificate whose stored binding does not match its content
        // fails the audit — the edited-claim case.
        let mut forged = a.clone();
        forged.verification = serde_json::json!({ "violations": 1 });
        assert!(!forged.binding_valid());
    }

    #[test]
    fn source_audit_detects_change_and_disappearance() {
        let dir = std::env::temp_dir().join(format!("nir-cert-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("main.rs");
        std::fs::write(&file, "fn main() {}").unwrap();

        let sources = scan_sources(&dir, &[file.clone()]).unwrap();
        let cert =
            Certificate::new(subject(), tool(), sources, serde_json::json!({ "violations": 0 }));
        assert!(cert.check_sources(&dir).ok());

        // Tamper: same file, new content.
        std::fs::write(&file, "fn main() { /* lied */ }").unwrap();
        let audit = cert.check_sources(&dir);
        assert_eq!(audit.matched, 0);
        assert_eq!(audit.changed, vec!["main.rs".to_string()]);

        // Remove: the verified file no longer exists.
        std::fs::remove_file(&file).unwrap();
        let audit = cert.check_sources(&dir);
        assert_eq!(audit.missing, vec!["main.rs".to_string()]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn round_trip_through_json_preserves_the_binding() {
        let sources = vec![SourceFile { path: "src/main.rs".into(), sha256: "deadbeef".into() }];
        let cert = Certificate::new(
            subject(),
            tool(),
            sources,
            serde_json::json!({ "contracts": 1, "violations": 0 }),
        );
        let json = serde_json::to_string(&cert).unwrap();
        let back: Certificate = serde_json::from_str(&json).unwrap();
        assert_eq!(back.binding, cert.binding);
        assert!(back.binding_valid());
    }
}
