//! Mechanical build-provenance binding: the resolved dependency closure
//! (`Cargo.lock`) and the toolchain that produced a verification.
//!
//! This is NOT issuer authentication. Anyone can recompute this binding
//! from the same lockfile and toolchain — it proves reproducibility
//! (what a build depended on), not who produced it. Authenticated issuer
//! identity is entry #13's signed-plugin trust chain, deferred.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    /// SHA-256 of `Cargo.lock`'s bytes — the resolved dependency closure.
    pub cargo_lock_sha256: String,
    /// `rustc --version` output. Recorded, not re-verified at consuming
    /// time: the checking machine may run a different rustc than the one
    /// that produced the certificate.
    pub toolchain: String,
}

/// Walks upward from `start` for the nearest `Cargo.lock` — the same
/// directory Cargo itself resolves for a package rooted at `start`.
pub fn find_cargo_lock(start: &Path) -> Option<PathBuf> {
    let mut dir = Some(start);
    while let Some(d) = dir {
        let candidate = d.join("Cargo.lock");
        if candidate.is_file() {
            return Some(candidate);
        }
        dir = d.parent();
    }
    None
}

/// Independently re-derives the dependency-closure hash from `start` —
/// never trusts a certificate-supplied path, the same discipline
/// `certificate::check_sources` uses for source files.
pub fn hash_cargo_lock(start: &Path) -> Result<String, String> {
    let lock = find_cargo_lock(start)
        .ok_or("no Cargo.lock found under or above this directory")?;
    let bytes = std::fs::read(&lock).map_err(|e| e.to_string())?;
    Ok(hex(&Sha256::digest(&bytes)))
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

    #[test]
    fn finds_cargo_lock_from_a_nested_package_dir() {
        let dir = std::env::temp_dir().join(format!("nir-prov-{}", std::process::id()));
        let pkg = dir.join("crates/pkg");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(dir.join("Cargo.lock"), b"lockfile bytes").unwrap();

        let found = find_cargo_lock(&pkg).unwrap();
        assert_eq!(found, dir.join("Cargo.lock"));
        assert!(hash_cargo_lock(&pkg).is_ok());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_lockfile_is_an_explicit_error() {
        // /tmp has no Cargo.lock anywhere in its ancestry.
        let dir = std::env::temp_dir().join(format!("nir-prov-missing-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(find_cargo_lock(&dir).is_none());
        assert!(hash_cargo_lock(&dir).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn same_bytes_hash_identically() {
        let dir = std::env::temp_dir().join(format!("nir-prov-det-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Cargo.lock"), b"same bytes").unwrap();
        let a = hash_cargo_lock(&dir).unwrap();
        let b = hash_cargo_lock(&dir).unwrap();
        assert_eq!(a, b);
        std::fs::remove_dir_all(&dir).ok();
    }
}
