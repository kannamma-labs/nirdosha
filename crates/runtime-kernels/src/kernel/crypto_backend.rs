//! The `fips` Cargo feature's swap point (2026-09) -- `SECURITY.md`/
//! `ROADMAP.md`'s disclosed gap, for this crate's own two real crypto
//! surfaces: `jsonwebtoken` (`nir_oidc_validate_token`/`nir_dpop_verify`,
//! defaults to `ring` internally) and `sha2` (`nir_dpop_verify`'s JWK
//! thumbprint, and -- as of this same session -- `nir_sha256_hex`
//! itself, see that function's own doc comment for why it wasn't
//! routed through a real crypto crate at all before this). Neither is a
//! NIST CMVP-validated module.
//!
//! Same shape as `crates/compiler/src/crypto_backend.rs`: off by
//! default (plain `jsonwebtoken` + `sha2`), swapped to `aws-lc-rs`
//! (directly for hashing, via the `jsonwebtoken-aws-lc` fork for JWT)
//! under `cargo build --no-default-features --features fips`. Confirmed
//! with real scratch builds in this environment before this file
//! existed: `jsonwebtoken-aws-lc` is a drop-in for `jsonwebtoken` 9.x
//! (same `encode`/`decode`/`Header`/`Algorithm`/`DecodingKey` API, no
//! call-site changes needed once this re-export points at it), and
//! `aws_lc_rs::digest::{Context, SHA256}` mirrors `sha2::Sha256`'s
//! update/finish shape closely enough that `sha256_concat` below is the
//! only place that needs to know which backend is active.

#[cfg(not(feature = "fips"))]
pub use jsonwebtoken;

#[cfg(feature = "fips")]
pub use jsonwebtoken_aws_lc as jsonwebtoken;

/// Hashes `a` followed by `b` as one continuous message -- `b` empty is
/// the 1-arg `sha256_hex(s)` case, `b` non-empty is the 2-arg
/// `sha256_hex(prev_hash, payload)` chained form (`nir_sha256_hex`'s own
/// doc comment). The one function `sha256()` (`lib.rs`) delegates to,
/// so the two backends below are the only place that needs to change if
/// a third one is ever added.
pub fn sha256_concat(a: &[u8], b: &[u8]) -> [u8; 32] {
    backend::sha256_concat(a, b)
}

#[cfg(not(feature = "fips"))]
mod backend {
    use sha2::{Digest, Sha256};

    pub fn sha256_concat(a: &[u8], b: &[u8]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(a);
        hasher.update(b);
        hasher.finalize().into()
    }
}

#[cfg(feature = "fips")]
mod backend {
    use aws_lc_rs::digest::{Context, SHA256};

    pub fn sha256_concat(a: &[u8], b: &[u8]) -> [u8; 32] {
        let mut ctx = Context::new(&SHA256);
        ctx.update(a);
        ctx.update(b);
        let digest = ctx.finish();
        let mut out = [0u8; 32];
        out.copy_from_slice(digest.as_ref());
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact test vector `lib.rs`'s own `sha256_hex` tests already
    /// pin against the empty string -- SHA-256("") is a standard,
    /// widely-published value, checked here directly against whichever
    /// backend this build has active so a backend swap can never
    /// silently produce a different digest for the same input.
    #[test]
    fn sha256_concat_matches_the_standard_empty_string_vector() {
        let digest = sha256_concat(b"", b"");
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    }

    /// Streaming `a` then `b` separately must be bit-identical to
    /// hashing the concatenation in one call -- the property
    /// `nir_sha256_hex`'s two-arg chained form depends on.
    #[test]
    fn sha256_concat_of_two_parts_matches_hashing_the_concatenation() {
        let a = b"hello, ";
        let b = b"world!";
        let mut concatenated = Vec::new();
        concatenated.extend_from_slice(a);
        concatenated.extend_from_slice(b);
        assert_eq!(sha256_concat(a, b), sha256_concat(&concatenated, b""));
    }
}
