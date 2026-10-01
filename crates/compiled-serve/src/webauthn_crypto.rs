//! The crypto/protocol half of WebAuthn support — the vendor-specific
//! port `crate::webauthn_store::PasskeyCredentialStore` deliberately
//! doesn't cover. This is the trait the "any vendor, just an adapter"
//! requirement is really about: parsing a CBOR attestation object at
//! registration, and verifying a signed assertion at login, against
//! whichever concrete crypto backend an implementation chooses.
//!
//! **v1 scope, stated plainly**: `none` attestation only (no
//! authenticator vendor trust-chain verification — standard practice;
//! most real Relying Parties never touch the FIDO metadata service
//! problem at all), ES256 (P-256 ECDSA) only, one credential per
//! subject, no resident/discoverable keys, no extensions (an
//! authenticator setting the extension-data flag is refused, not
//! silently ignored).
//!
//! Every method below is annotated `#[nirdosha_rt::contract(effects(pure))]`
//! where the operation genuinely has no I/O — bytes in, a verified
//! result out — so a future adapter swapping in a different CBOR/crypto
//! backend is independently checked by `cargo nirdosha build`'s real
//! interprocedural effect analysis, not just trusted by convention. This
//! is genuinely novel usage in this codebase (no prior `.nir`/v2 file
//! applies `#[contract(...)]` inside an `impl Trait for Type` block) —
//! proven safe first via a disposable scratch probe (both the honest and
//! a deliberately-lying version, under plain `cargo build`) before any
//! of this file was written.

use crate::webauthn_cbor::CborValue;
use nirdosha_rt::contract;

#[derive(Debug, Clone)]
pub struct AttestationResult {
    pub credential_id: Vec<u8>,
    pub public_key_x: [u8; 32],
    pub public_key_y: [u8; 32],
    pub sign_count: u32,
}

/// Storage/protocol port for the crypto/protocol half of WebAuthn.
/// `Send + Sync`: shared across connection-handling threads behind an
/// `Arc`, the same way `ServeConfig`'s other adapter fields are.
pub trait PasskeyCryptoAdapter: Send + Sync {
    /// Parses `attestation_object` (CBOR) and verifies `client_data_json`
    /// against `expected_challenge`/`expected_origin` for a *registration*
    /// ceremony (`clientDataJSON.type == "webauthn.create"`), and checks
    /// `authenticatorData`'s own `rpIdHash` against `SHA256(expected_rp_id)`
    /// — the check that stops a credential minted for a different site
    /// from being accepted here at all, WebAuthn's own first line of
    /// defense, not an optional extra. Returns the new credential's ID
    /// and public key on success.
    fn parse_and_verify_attestation(&self, attestation_object: &[u8], client_data_json: &[u8], expected_challenge: &str, expected_origin: &str, expected_rp_id: &str) -> Result<AttestationResult, String>;

    /// Verifies a *login* ceremony's assertion: `client_data_json`
    /// against `expected_challenge`/`expected_origin`
    /// (`clientDataJSON.type == "webauthn.get"`), `authenticatorData`'s
    /// `rpIdHash` against `SHA256(expected_rp_id)` (same check as
    /// registration), and `signature` over
    /// `authenticator_data || SHA256(client_data_json)` against the
    /// stored `public_key_{x,y}`. Returns the assertion's own sign
    /// counter on success — the caller (business logic, not this
    /// adapter) is responsible for checking it actually advanced past
    /// the stored value and persisting the new one; this method has no
    /// storage access at all, by design.
    fn verify_assertion(&self, authenticator_data: &[u8], client_data_json: &[u8], signature: &[u8], expected_challenge: &str, expected_origin: &str, expected_rp_id: &str, public_key_x: &[u8; 32], public_key_y: &[u8; 32]) -> Result<u32, String>;
}

/// Reference implementation: hand-rolled CBOR/COSE parsing
/// (`crate::webauthn_cbor`, scoped exactly to what this needs) plus
/// `ring` for raw P-256 ECDSA-over-SHA-256 verification. A future
/// adapter wrapping a dedicated WebAuthn/CBOR crate, or a different
/// signature algorithm, implements the same trait — no change to
/// anything that calls it.
pub struct Es256CborAdapter;

impl Es256CborAdapter {
    pub fn new() -> Self {
        Es256CborAdapter
    }
}

impl Default for Es256CborAdapter {
    fn default() -> Self {
        Self::new()
    }
}

/// `authenticatorData`'s flag byte, bit 6 (0x40): "attested credential
/// data included" — set on a registration's own authenticatorData,
/// clear on a login assertion's.
const FLAG_ATTESTED_CREDENTIAL_DATA: u8 = 0x40;
/// Bit 7 (0x80): "extension data included" — refused outright (v1 scope
/// has no extension support), not silently skipped.
const FLAG_EXTENSION_DATA: u8 = 0x80;

/// Parsed `authenticatorData` — the raw byte layout WebAuthn defines
/// (rpIdHash || flags || signCount || optional attestedCredentialData),
/// not CBOR at this level (only the trailing credential public key,
/// when present, is itself CBOR).
#[derive(Debug)]
struct AuthenticatorData {
    sign_count: u32,
    attested_credential: Option<(Vec<u8>, [u8; 32], [u8; 32])>,
}

#[contract(effects(pure))]
fn parse_authenticator_data(data: &[u8], expected_rp_id: &str) -> Result<AuthenticatorData, String> {
    if data.len() < 37 {
        return Err(format!("authenticatorData is {} bytes, too short for rpIdHash+flags+signCount (need at least 37)", data.len()));
    }
    let expected_rp_id_hash = ring::digest::digest(&ring::digest::SHA256, expected_rp_id.as_bytes());
    if data[0..32] != *expected_rp_id_hash.as_ref() {
        return Err(format!("authenticatorData's rpIdHash does not match SHA-256({expected_rp_id:?}) -- this credential was not created for this relying party"));
    }
    let flags = data[32];
    if flags & FLAG_EXTENSION_DATA != 0 {
        return Err("authenticatorData sets the extension-data flag -- extensions are out of v1 scope, refusing rather than silently ignoring them".to_string());
    }
    let sign_count_bytes: [u8; 4] = data[33..37].try_into().map_err(|_| "authenticatorData: sign count slice is not 4 bytes".to_string())?;
    let sign_count = u32::from_be_bytes(sign_count_bytes);

    if flags & FLAG_ATTESTED_CREDENTIAL_DATA == 0 {
        return Ok(AuthenticatorData { sign_count, attested_credential: None });
    }

    // aaguid (16 bytes, unused) + credentialIdLength (2 bytes, big-endian).
    let cred_id_len_offset = 37 + 16;
    let cred_id_start = cred_id_len_offset + 2;
    if data.len() < cred_id_start {
        return Err("authenticatorData's attested credential data is truncated before the credential ID length field".to_string());
    }
    let cred_id_len_bytes: [u8; 2] = data[cred_id_len_offset..cred_id_start].try_into().map_err(|_| "authenticatorData: credential ID length slice is not 2 bytes".to_string())?;
    let cred_id_len = u16::from_be_bytes(cred_id_len_bytes) as usize;
    let cred_id_end = cred_id_start.checked_add(cred_id_len).ok_or_else(|| "authenticatorData: credential ID length overflow".to_string())?;
    let credential_id = data.get(cred_id_start..cred_id_end).ok_or_else(|| "authenticatorData's credential ID runs past the end of the buffer".to_string())?.to_vec();

    let (public_key_x, public_key_y) = parse_cose_ec2_public_key(data.get(cred_id_end..).ok_or_else(|| "authenticatorData has no bytes left for the credential public key".to_string())?)?;

    Ok(AuthenticatorData { sign_count, attested_credential: Some((credential_id, public_key_x, public_key_y)) })
}

/// Decodes a COSE `EC2` public key (RFC 9053 §7.1.1) and checks it's
/// exactly the one shape this v1 adapter supports: `kty=EC2` (2),
/// `alg=ES256` (-7), `crv=P-256` (1), 32-byte `x`/`y` coordinates. Any
/// other shape is refused by name, not coerced.
#[contract(effects(pure))]
fn parse_cose_ec2_public_key(bytes: &[u8]) -> Result<([u8; 32], [u8; 32]), String> {
    let (key, _consumed) = crate::webauthn_cbor::decode_one(bytes)?;

    let kty = key.get_int_key(1).and_then(CborValue::as_int).ok_or_else(|| "COSE key missing kty (label 1)".to_string())?;
    if kty != 2 {
        return Err(format!("COSE key kty={kty}, only EC2 (2) is supported"));
    }
    let alg = key.get_int_key(3).and_then(CborValue::as_int).ok_or_else(|| "COSE key missing alg (label 3)".to_string())?;
    if alg != -7 {
        return Err(format!("COSE key alg={alg}, only ES256 (-7) is supported"));
    }
    let crv = key.get_int_key(-1).and_then(CborValue::as_int).ok_or_else(|| "COSE key missing crv (label -1)".to_string())?;
    if crv != 1 {
        return Err(format!("COSE key crv={crv}, only P-256 (1) is supported"));
    }
    let x = key.get_int_key(-2).and_then(CborValue::as_bytes).ok_or_else(|| "COSE key missing x coordinate (label -2)".to_string())?;
    let y = key.get_int_key(-3).and_then(CborValue::as_bytes).ok_or_else(|| "COSE key missing y coordinate (label -3)".to_string())?;
    let x: [u8; 32] = x.try_into().map_err(|_| format!("COSE key x coordinate is {} bytes, expected 32", x.len()))?;
    let y: [u8; 32] = y.try_into().map_err(|_| format!("COSE key y coordinate is {} bytes, expected 32", y.len()))?;
    Ok((x, y))
}

/// Checks `client_data_json`'s `type`/`challenge`/`origin` fields
/// against what this ceremony step expects. `expected_type` is
/// `"webauthn.create"` for registration, `"webauthn.get"` for login —
/// the one field that differs between the two call sites above.
#[contract(effects(pure))]
fn verify_client_data(client_data_json: &[u8], expected_type: &str, expected_challenge: &str, expected_origin: &str) -> Result<(), String> {
    let parsed: serde_json::Value = serde_json::from_slice(client_data_json).map_err(|e| format!("clientDataJSON is not valid JSON: {e}"))?;
    let actual_type = parsed.get("type").and_then(serde_json::Value::as_str).ok_or_else(|| "clientDataJSON has no \"type\" field".to_string())?;
    if actual_type != expected_type {
        return Err(format!("clientDataJSON type={actual_type:?}, expected {expected_type:?}"));
    }
    let actual_challenge = parsed.get("challenge").and_then(serde_json::Value::as_str).ok_or_else(|| "clientDataJSON has no \"challenge\" field".to_string())?;
    if actual_challenge != expected_challenge {
        return Err("clientDataJSON challenge does not match the challenge this server minted".to_string());
    }
    let actual_origin = parsed.get("origin").and_then(serde_json::Value::as_str).ok_or_else(|| "clientDataJSON has no \"origin\" field".to_string())?;
    if actual_origin != expected_origin {
        return Err(format!("clientDataJSON origin={actual_origin:?}, expected {expected_origin:?}"));
    }
    Ok(())
}

/// Verifies a raw P-256 ECDSA signature (DER/ASN.1-encoded, the wire
/// format WebAuthn signatures actually use — not the fixed-width `r||s`
/// form) over `signed_bytes`, against the given uncompressed public key
/// coordinates. This is the one place `ring` is called directly, rather
/// than through `identity.rs`'s existing JWT-shaped verification (a
/// WebAuthn assertion signs raw bytes, never a JWT).
#[contract(effects(pure))]
fn verify_es256_signature(signed_bytes: &[u8], signature_der: &[u8], public_key_x: &[u8; 32], public_key_y: &[u8; 32]) -> Result<(), String> {
    let mut uncompressed_point = Vec::with_capacity(65);
    uncompressed_point.push(0x04); // SEC1 uncompressed-point tag.
    uncompressed_point.extend_from_slice(public_key_x);
    uncompressed_point.extend_from_slice(public_key_y);

    let public_key = ring::signature::UnparsedPublicKey::new(&ring::signature::ECDSA_P256_SHA256_ASN1, &uncompressed_point);
    public_key.verify(signed_bytes, signature_der).map_err(|_| "ECDSA P-256 signature verification failed".to_string())
}

impl PasskeyCryptoAdapter for Es256CborAdapter {
    fn parse_and_verify_attestation(&self, attestation_object: &[u8], client_data_json: &[u8], expected_challenge: &str, expected_origin: &str, expected_rp_id: &str) -> Result<AttestationResult, String> {
        verify_client_data(client_data_json, "webauthn.create", expected_challenge, expected_origin)?;

        let (attestation, _) = crate::webauthn_cbor::decode_one(attestation_object)?;
        let fmt = attestation.get_text_key("fmt").and_then(CborValue::as_text).ok_or_else(|| "attestationObject missing \"fmt\"".to_string())?;
        if fmt != "none" {
            return Err(format!("attestationObject fmt={fmt:?}, only \"none\" attestation is supported in v1"));
        }
        let auth_data_bytes = attestation.get_text_key("authData").and_then(CborValue::as_bytes).ok_or_else(|| "attestationObject missing \"authData\"".to_string())?;

        let parsed = parse_authenticator_data(auth_data_bytes, expected_rp_id)?;
        let (credential_id, public_key_x, public_key_y) = parsed.attested_credential.ok_or_else(|| "authenticatorData has no attested credential data -- required for a registration ceremony".to_string())?;

        Ok(AttestationResult { credential_id, public_key_x, public_key_y, sign_count: parsed.sign_count })
    }

    fn verify_assertion(&self, authenticator_data: &[u8], client_data_json: &[u8], signature: &[u8], expected_challenge: &str, expected_origin: &str, expected_rp_id: &str, public_key_x: &[u8; 32], public_key_y: &[u8; 32]) -> Result<u32, String> {
        verify_client_data(client_data_json, "webauthn.get", expected_challenge, expected_origin)?;

        let parsed = parse_authenticator_data(authenticator_data, expected_rp_id)?;
        if parsed.attested_credential.is_some() {
            return Err("authenticatorData carries attested credential data -- not expected on a login assertion".to_string());
        }

        let client_data_hash = ring::digest::digest(&ring::digest::SHA256, client_data_json);
        let mut signed_bytes = Vec::with_capacity(authenticator_data.len() + 32);
        signed_bytes.extend_from_slice(authenticator_data);
        signed_bytes.extend_from_slice(client_data_hash.as_ref());

        verify_es256_signature(&signed_bytes, signature, public_key_x, public_key_y)?;
        Ok(parsed.sign_count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::webauthn_test_support::{attestation_object, authenticator_data, client_data_json, generate_test_key};

    #[test]
    fn a_real_registration_ceremony_round_trips() {
        let key = generate_test_key();
        let adapter = Es256CborAdapter::new();
        let auth_data = authenticator_data(0, Some((b"cred-1", &key.x, &key.y)));
        let att_obj = attestation_object(&auth_data);
        let client_data = client_data_json("webauthn.create", "chal-abc", "https://example.test");

        let result = adapter.parse_and_verify_attestation(&att_obj, &client_data, "chal-abc", "https://example.test", crate::webauthn_test_support::TEST_RP_ID).unwrap();
        assert_eq!(result.credential_id, b"cred-1");
        assert_eq!(result.public_key_x, key.x);
        assert_eq!(result.public_key_y, key.y);
        assert_eq!(result.sign_count, 0);
    }

    #[test]
    fn registration_rejects_a_credential_minted_for_a_different_relying_party() {
        let key = generate_test_key();
        let adapter = Es256CborAdapter::new();
        let auth_data = authenticator_data(0, Some((b"cred-1", &key.x, &key.y)));
        let att_obj = attestation_object(&auth_data);
        let client_data = client_data_json("webauthn.create", "chal-abc", "https://example.test");

        // `auth_data` was built against `TEST_RP_ID` ("example.test");
        // asking the adapter to check it against a different rp_id must
        // fail on the rpIdHash mismatch, not silently accept a
        // credential minted for another site.
        let err = adapter.parse_and_verify_attestation(&att_obj, &client_data, "chal-abc", "https://example.test", "attacker.example").unwrap_err();
        assert!(err.contains("rpIdHash"), "error should name the real cause: {err}");
    }

    #[test]
    fn registration_rejects_a_challenge_mismatch() {
        let key = generate_test_key();
        let adapter = Es256CborAdapter::new();
        let auth_data = authenticator_data(0, Some((b"cred-1", &key.x, &key.y)));
        let att_obj = attestation_object(&auth_data);
        let client_data = client_data_json("webauthn.create", "chal-abc", "https://example.test");

        let err = adapter.parse_and_verify_attestation(&att_obj, &client_data, "different-challenge", "https://example.test", crate::webauthn_test_support::TEST_RP_ID).unwrap_err();
        assert!(err.contains("challenge"), "error should name the actual mismatch: {err}");
    }

    #[test]
    fn a_real_login_assertion_verifies_against_a_real_signature() {
        let key = generate_test_key();
        let adapter = Es256CborAdapter::new();
        let auth_data = authenticator_data(1, None);
        let client_data = client_data_json("webauthn.get", "chal-xyz", "https://example.test");

        let client_data_hash = ring::digest::digest(&ring::digest::SHA256, &client_data);
        let mut signed_bytes = auth_data.clone();
        signed_bytes.extend_from_slice(client_data_hash.as_ref());
        let rng = ring::rand::SystemRandom::new();
        let signature = key.key_pair.sign(&rng, &signed_bytes).unwrap();

        let new_count = adapter.verify_assertion(&auth_data, &client_data, signature.as_ref(), "chal-xyz", "https://example.test", crate::webauthn_test_support::TEST_RP_ID, &key.x, &key.y).unwrap();
        assert_eq!(new_count, 1);
    }

    #[test]
    fn login_rejects_a_signature_from_the_wrong_key() {
        let key = generate_test_key();
        let wrong_key = generate_test_key();
        let adapter = Es256CborAdapter::new();
        let auth_data = authenticator_data(1, None);
        let client_data = client_data_json("webauthn.get", "chal-xyz", "https://example.test");

        let client_data_hash = ring::digest::digest(&ring::digest::SHA256, &client_data);
        let mut signed_bytes = auth_data.clone();
        signed_bytes.extend_from_slice(client_data_hash.as_ref());
        let rng = ring::rand::SystemRandom::new();
        // Signed by `wrong_key`, verified against `key`'s public coordinates.
        let signature = wrong_key.key_pair.sign(&rng, &signed_bytes).unwrap();

        let err = adapter.verify_assertion(&auth_data, &client_data, signature.as_ref(), "chal-xyz", "https://example.test", crate::webauthn_test_support::TEST_RP_ID, &key.x, &key.y).unwrap_err();
        assert!(err.contains("signature"), "error should name the real cause: {err}");
    }

    #[test]
    fn login_rejects_a_tampered_authenticator_data() {
        let key = generate_test_key();
        let adapter = Es256CborAdapter::new();
        let auth_data = authenticator_data(1, None);
        let client_data = client_data_json("webauthn.get", "chal-xyz", "https://example.test");

        let client_data_hash = ring::digest::digest(&ring::digest::SHA256, &client_data);
        let mut signed_bytes = auth_data.clone();
        signed_bytes.extend_from_slice(client_data_hash.as_ref());
        let rng = ring::rand::SystemRandom::new();
        let signature = key.key_pair.sign(&rng, &signed_bytes).unwrap();

        let mut tampered_auth_data = auth_data.clone();
        tampered_auth_data[33] ^= 0xFF; // flip a bit in the sign-count field
        let err = adapter.verify_assertion(&tampered_auth_data, &client_data, signature.as_ref(), "chal-xyz", "https://example.test", crate::webauthn_test_support::TEST_RP_ID, &key.x, &key.y).unwrap_err();
        assert!(err.contains("signature"), "tampering with the signed bytes must fail signature verification: {err}");
    }

    #[test]
    fn extension_data_flag_is_refused_not_silently_ignored() {
        let mut auth_data = authenticator_data(1, None);
        auth_data[32] |= FLAG_EXTENSION_DATA;
        let err = parse_authenticator_data(&auth_data, crate::webauthn_test_support::TEST_RP_ID).unwrap_err();
        assert!(err.contains("extension"), "must name the real reason: {err}");
    }

    #[test]
    fn an_unsupported_cose_algorithm_is_refused_by_name() {
        // Same map shape but alg = -257 (RS256) instead of -7 (ES256).
        let mut key = vec![0xA5];
        key.extend_from_slice(&[0x01, 0x02]);
        key.extend_from_slice(&[0x03, 0x39, 0x01, 0x00]); // 3: -257 (negative int, 2-byte form)
        key.extend_from_slice(&[0x20, 0x01]);
        key.push(0x21);
        key.push(0x58);
        key.push(32);
        key.extend_from_slice(&[0u8; 32]);
        key.push(0x22);
        key.push(0x58);
        key.push(32);
        key.extend_from_slice(&[0u8; 32]);

        let err = parse_cose_ec2_public_key(&key).unwrap_err();
        assert!(err.contains("alg"), "must name the actual unsupported field: {err}");
    }
}
