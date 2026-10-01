//! Closes two of this crate's own disclosed gaps (see `src/lib.rs`'s
//! module doc): "the capability carries no cryptographic signature" and
//! "the policy bundle's `approved_by`/`signed_by` are checked for
//! presence... not verified as a real Ed25519 signature." Both are real
//! Ed25519 signing/verification (via `nirdosha_audit::signing`, the same
//! primitive the native compiler's certificate signing and
//! `nirdosha-hi`'s domain-pack signing already use -- not a hand-rolled
//! second implementation), **strictly additive and opt-in**: every
//! existing `CapabilityIssuer::new`/`GatewayCore::new` call site (the
//! whole `transfer_create`/`funds_reserve` slice, every generated app) is
//! unaffected -- a capability/bundle with no signing key attached is
//! checked exactly as before. A caller opts in via
//! `CapabilityIssuer::with_signing_key` and `GatewayCore` automatically
//! then requires and verifies a signature on every capability it consumes.
//!
//! **What this still does not close**, disclosed rather than implied:
//! *production key custody*. `AuthorityRegistry` is an in-process map from
//! `authority_id` to trusted public keys, loaded from data -- it says
//! nothing about how those public keys themselves are minted, rotated, or
//! protected (an HSM/KMS-backed signing ceremony, real multi-party
//! authorization to add a new trusted authority, etc.). That is
//! organizational/infrastructure process this repository cannot honestly
//! claim to provide from inside a single checked-out crate.

use std::collections::HashMap;

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorityRegistryError {
    Parse(String),
}

impl std::fmt::Display for AuthorityRegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthorityRegistryError::Parse(e) => write!(f, "authority registry is invalid: {e}"),
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawAuthorityRegistryFile {
    #[serde(default)]
    authority: HashMap<String, RawAuthority>,
}

#[derive(Debug, Deserialize)]
struct RawAuthority {
    public_key_b64: String,
}

/// Which Ed25519 public key(s) this deployment trusts for each
/// `authority_id`. An `authority_id` may list more than one currently
/// trusted key (key rotation overlap window).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AuthorityRegistry {
    trusted_keys: HashMap<String, Vec<String>>,
}

impl AuthorityRegistry {
    pub fn from_toml_str(src: &str) -> Result<Self, AuthorityRegistryError> {
        let raw: RawAuthorityRegistryFile = toml::from_str(src).map_err(|e| AuthorityRegistryError::Parse(e.to_string()))?;
        let mut trusted_keys: HashMap<String, Vec<String>> = HashMap::new();
        for (authority_id, entry) in raw.authority {
            trusted_keys.entry(authority_id).or_default().push(entry.public_key_b64);
        }
        Ok(Self { trusted_keys })
    }

    pub fn is_trusted(&self, authority_id: &str, public_key_b64: &str) -> bool {
        self.trusted_keys.get(authority_id).is_some_and(|keys| keys.iter().any(|k| k == public_key_b64))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignedBundleError {
    UntrustedSigningKey { authority_id: String },
    SignatureMismatch,
    SignatureCheckFailed(String),
}

impl std::fmt::Display for SignedBundleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SignedBundleError::UntrustedSigningKey { authority_id } => write!(f, "signing key is not a trusted key for authority `{authority_id}`"),
            SignedBundleError::SignatureMismatch => write!(f, "bundle signature does not match its content"),
            SignedBundleError::SignatureCheckFailed(e) => write!(f, "bundle signature could not be checked: {e}"),
        }
    }
}

/// A `PolicyBundle` plus a *detached* Ed25519 signature over the exact
/// bundle TOML source bytes it was parsed from -- detached (sidecar), not
/// embedded, the same reasoning `nirdosha-screening-list-fixture`'s
/// pinned `.sha256` sidecar uses: a signature field embedded inside the
/// document it signs is self-referential (the bytes to hash/sign would
/// have to exclude the very field carrying the result).
pub struct SignedPolicyBundle {
    pub bundle: crate::bundle::PolicyBundle,
    pub signing_authority_id: String,
    pub public_key_b64: String,
    signature_b64: String,
}

impl SignedPolicyBundle {
    /// Parses `src` as a normal [`crate::bundle::PolicyBundle`] and pairs
    /// it with a detached signature to verify later. Does not itself
    /// check the signature -- call [`Self::verify`] with the trust
    /// registry.
    pub fn new(src: &str, public_key_b64: &str, signature_b64: &str) -> Result<Self, crate::bundle::BundleError> {
        let bundle = crate::bundle::PolicyBundle::from_toml_str(src)?;
        Ok(Self { signing_authority_id: bundle.authority_id.clone(), bundle, public_key_b64: public_key_b64.to_string(), signature_b64: signature_b64.to_string() })
    }

    /// `src` must be the exact same bytes [`Self::new`] was built from --
    /// a caller re-fetching a bundle from storage passes the same source
    /// text back in, it is not re-derived from `self.bundle`'s own parsed
    /// fields (which deliberately drop the exact original formatting).
    pub fn verify(&self, src: &str, registry: &AuthorityRegistry) -> Result<(), SignedBundleError> {
        if !registry.is_trusted(&self.signing_authority_id, &self.public_key_b64) {
            return Err(SignedBundleError::UntrustedSigningKey { authority_id: self.signing_authority_id.clone() });
        }
        match nirdosha_audit::signing::verify_bytes(src.as_bytes(), &self.public_key_b64, &self.signature_b64) {
            Ok(true) => Ok(()),
            Ok(false) => Err(SignedBundleError::SignatureMismatch),
            Err(e) => Err(SignedBundleError::SignatureCheckFailed(e)),
        }
    }
}

/// Test/dev convenience: a fresh Ed25519 PKCS#8 keypair, `(pkcs8_der,
/// public_key_b64)`. A real deployment generates and custodies this key
/// the way `nirdosha keygen` and its own signing ceremony do -- not part
/// of this crate's scope (see module doc).
pub fn generate_ed25519_keypair() -> (Vec<u8>, String) {
    use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
    use base64::Engine;
    use nirdosha_audit::crypto_backend::{rand::SystemRandom, signature::{Ed25519KeyPair, KeyPair}};

    let rng = SystemRandom::new();
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).expect("Ed25519 key generation must succeed");
    let keypair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).expect("freshly generated pkcs8 must parse");
    let public_key_b64 = BASE64_STANDARD.encode(keypair.public_key().as_ref());
    (pkcs8.as_ref().to_vec(), public_key_b64)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUNDLE_TOML: &str = r#"
[bundle]
authority_id = "ctms-domain-authority"
policy_owner = "risk-platform-team"
jurisdiction = "IN"
approved_by = "ctms-domain-authority"
signed_by = "ctms-domain-authority"
effective_from = "2026-01-01T00:00:00Z"
expires_at = "2027-01-01T00:00:00Z"
"#;

    /// Writes a freshly generated PKCS#8 key to a temp file and signs
    /// `bytes` through the real, shared `nirdosha_audit::signing::sign_bytes`
    /// (which reads its key from a file path, same as production callers)
    /// -- not a second, test-only signing implementation.
    fn generate_key_and_sign(bytes: &[u8]) -> (String, String) {
        let (pkcs8, _public_key_b64) = generate_ed25519_keypair();
        let key_path = std::env::temp_dir().join(format!("ctms_authority_test_key_{}_{}.pk8", std::process::id(), rand_suffix()));
        std::fs::write(&key_path, &pkcs8).unwrap();
        let result = nirdosha_audit::signing::sign_bytes(bytes, key_path.to_str().unwrap()).unwrap();
        let _ = std::fs::remove_file(&key_path);
        result
    }

    fn rand_suffix() -> u64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos() as u64
    }

    #[test]
    fn a_bundle_signed_by_a_registered_trusted_key_verifies() {
        let (public_key_b64, signature_b64) = generate_key_and_sign(BUNDLE_TOML.as_bytes());
        let signed = SignedPolicyBundle::new(BUNDLE_TOML, &public_key_b64, &signature_b64).unwrap();
        let registry = AuthorityRegistry::from_toml_str(&format!("[authority.ctms-domain-authority]\npublic_key_b64 = \"{public_key_b64}\"\n")).unwrap();
        assert_eq!(signed.verify(BUNDLE_TOML, &registry), Ok(()));
    }

    #[test]
    fn a_signature_from_a_key_the_registry_does_not_trust_is_rejected() {
        let (public_key_b64, signature_b64) = generate_key_and_sign(BUNDLE_TOML.as_bytes());
        let signed = SignedPolicyBundle::new(BUNDLE_TOML, &public_key_b64, &signature_b64).unwrap();
        let empty_registry = AuthorityRegistry::default();
        assert_eq!(
            signed.verify(BUNDLE_TOML, &empty_registry),
            Err(SignedBundleError::UntrustedSigningKey { authority_id: "ctms-domain-authority".to_string() })
        );
    }

    #[test]
    fn a_bundle_whose_content_was_tampered_with_after_signing_fails_verification() {
        let (public_key_b64, signature_b64) = generate_key_and_sign(BUNDLE_TOML.as_bytes());
        let signed = SignedPolicyBundle::new(BUNDLE_TOML, &public_key_b64, &signature_b64).unwrap();
        let registry = AuthorityRegistry::from_toml_str(&format!("[authority.ctms-domain-authority]\npublic_key_b64 = \"{public_key_b64}\"\n")).unwrap();
        let tampered = BUNDLE_TOML.replace("risk-platform-team", "attacker-controlled-team");
        assert_eq!(signed.verify(&tampered, &registry), Err(SignedBundleError::SignatureMismatch));
    }

    #[test]
    fn registry_trusts_only_the_exact_registered_key_for_that_authority() {
        let registry = AuthorityRegistry::from_toml_str(
            r#"
[authority.ctms-domain-authority]
public_key_b64 = "abc123"
"#,
        )
        .unwrap();
        assert!(registry.is_trusted("ctms-domain-authority", "abc123"));
        assert!(!registry.is_trusted("ctms-domain-authority", "wrong-key"));
        assert!(!registry.is_trusted("some-other-authority", "abc123"));
    }
}
