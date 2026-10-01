//! The `fips` Cargo feature's swap point (2026-09) -- `SECURITY.md`/
//! `ROADMAP.md`'s disclosed gap: `ring` (this crate's default
//! Ed25519-signing backend, `hi_plugin.rs`'s pack manifests and
//! `mcp_tools.rs`'s certificates) is standard RustCrypto-adjacent
//! software, not a NIST CMVP-validated cryptographic module, so a
//! government/regulated deployment requiring FIPS-140-3-validated
//! crypto can't use it as-is.
//!
//! **The fix is a swap, not a rewrite, because the API is close enough
//! to be a pure re-export.** `aws-lc-rs` was built specifically to
//! mirror `ring`'s own `rand`/`signature` surface (same
//! `SystemRandom`, `Ed25519KeyPair::{generate_pkcs8,from_pkcs8,sign}`,
//! `UnparsedPublicKey::{new,verify}`, `ED25519` shapes) so every real
//! call site in this crate (`hi_plugin.rs:1603-1604`,
//! `mcp_tools.rs:1195-1220`) needs only its `ring::` prefix changed to
//! `crate::crypto_backend::` -- confirmed against a real scratch build
//! in this environment before this file existed: `Ed25519KeyPair::
//! generate_pkcs8`/`sign`/`UnparsedPublicKey::verify` all work
//! identically under `aws-lc-rs`'s `fips` feature (which itself needs
//! `cmake`/`go`/`clang` at build time -- checked present, not assumed).
//!
//! Off by default (this module re-exports plain `ring` unless the
//! `fips` feature is on) -- a normal build, and every existing test, is
//! completely unaffected. `cargo build --features fips` opts a
//! regulated deployment into the CMVP-validatable backend instead.
//!
//! **What this does not yet cover, disclosed rather than implied by
//! silence:** `hi_graph::sha256_hex` (pack/content hashing) still goes
//! through the plain `sha2` crate either way -- SHA-256 the *algorithm*
//! doesn't change, but a strict "every cryptographic operation through
//! a validated module" reading of FIPS-140-3 would want that routed
//! through `aws-lc-rs`'s digest API under this same feature too. Real,
//! separate follow-up, not silently declared "done" here.

#[cfg(not(feature = "fips"))]
pub use ring::{rand, signature};

#[cfg(feature = "fips")]
pub use aws_lc_rs::{rand, signature};

/// `hi_graph::sha256_hex` (pack/content integrity hashing) and
/// `audit_chain.rs` (this session's hash-chained audit log) both hash
/// through this, so the `fips` feature covers every real cryptographic
/// operation in this crate, not just Ed25519 signing. Same shape as
/// `crates/runtime-kernels`'s identically-named function.
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    backend::sha256(bytes)
}

#[cfg(not(feature = "fips"))]
mod backend {
    use sha2::{Digest, Sha256};

    pub fn sha256(bytes: &[u8]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        hasher.finalize().into()
    }
}

#[cfg(feature = "fips")]
mod backend {
    use aws_lc_rs::digest::{digest, SHA256};

    pub fn sha256(bytes: &[u8]) -> [u8; 32] {
        let d = digest(&SHA256, bytes);
        let mut out = [0u8; 32];
        out.copy_from_slice(d.as_ref());
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_the_standard_empty_string_vector() {
        let digest = sha256(b"");
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    }
}
