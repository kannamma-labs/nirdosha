//! Ed25519 signing/verification of arbitrary byte strings -- the
//! primitive behind the native compiler's certificate signing
//! (`crates/compiler/src/verify_pipeline.rs::sign_certificate`) and
//! `crates/nirdosha-hi`'s domain-pack signing (`hi_plugin::sign_pack`).
//! One implementation, not a hand-rolled copy per signer -- extracted
//! here (2026-09-16) so neither crate has to depend on the other just
//! to share it.

/// Ed25519-signs arbitrary `bytes` with the PKCS#8 private key at
/// `key_path` (raw DER, as `nirdosha keygen` writes) -- the primitive
/// behind both `sign_certificate` above and `hi_plugin`'s pack-signing
/// layer (RFC 0016 Phase 4, Sigstore-*pattern* trust for packs): one
/// Ed25519 signing implementation in this crate, not a hand-rolled copy
/// per signer. Returns `(public_key_base64, signature_base64)`.
pub fn sign_bytes(bytes: &[u8], key_path: &str) -> Result<(String, String), String> {
    use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
    use base64::Engine;
    use crate::crypto_backend::signature::KeyPair;

    let pkcs8 = std::fs::read(key_path).map_err(|e| format!("reading private key {key_path}: {e}"))?;
    let keypair = crate::crypto_backend::signature::Ed25519KeyPair::from_pkcs8(&pkcs8).map_err(|e| format!("{key_path} is not a valid Ed25519 PKCS#8 private key: {e}"))?;
    let signature = keypair.sign(bytes);
    Ok((BASE64_STANDARD.encode(keypair.public_key().as_ref()), BASE64_STANDARD.encode(signature.as_ref())))
}

/// The verify-side mirror of [`sign_bytes`]: checks `signature_b64`
/// against `bytes` under `public_key_b64`, both base64 exactly as
/// `sign_bytes`/`nirdosha keygen` produce them. Returns `Ok(false)`,
/// not `Err`, for a well-formed but non-matching signature -- "checked,
/// and it didn't match" is a real, distinct outcome from "couldn't even
/// attempt the check" (malformed base64/key bytes), which stays `Err`.
/// Says nothing about whether `public_key_b64` is a key the *caller*
/// should trust -- pinning acceptable keys is the caller's own
/// operational policy (`SignedCertificate`'s own doc comment on this
/// same point; `hi_plugin`'s trust-anchor list is that policy for
/// packs).
pub fn verify_bytes(bytes: &[u8], public_key_b64: &str, signature_b64: &str) -> Result<bool, String> {
    use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
    use base64::Engine;

    let signature_bytes = BASE64_STANDARD.decode(signature_b64).map_err(|e| format!("signature is not valid base64: {e}"))?;
    let public_key_bytes = BASE64_STANDARD.decode(public_key_b64).map_err(|e| format!("public_key is not valid base64: {e}"))?;
    let public_key = crate::crypto_backend::signature::UnparsedPublicKey::new(&crate::crypto_backend::signature::ED25519, &public_key_bytes);
    Ok(public_key.verify(bytes, &signature_bytes).is_ok())
}
