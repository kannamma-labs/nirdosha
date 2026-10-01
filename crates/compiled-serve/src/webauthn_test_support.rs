//! Test-only fixture builders for WebAuthn ceremonies -- shared between
//! `webauthn_crypto`'s own unit tests and `tests.rs`'s HTTP-level
//! integration test, so the CBOR/authenticatorData construction logic
//! lives in exactly one place rather than being copied. `#[cfg(test)]`
//! only; never compiled into a real binary.
//!
//! Every byte layout here is built to spec (WebAuthn Level 2 §6.1
//! authenticatorData, RFC 9053 COSE_Key), checked against the RFCs, not
//! against whatever `webauthn_crypto`'s own adapter happens to produce --
//! and not captured from a live browser/authenticator (this session had
//! no way to script one), disclosed here rather than implied.

use ring::signature::KeyPair as _;

pub(crate) struct TestKey {
    pub(crate) key_pair: ring::signature::EcdsaKeyPair,
    pub(crate) x: [u8; 32],
    pub(crate) y: [u8; 32],
}

pub(crate) fn generate_test_key() -> TestKey {
    let rng = ring::rand::SystemRandom::new();
    let pkcs8 = ring::signature::EcdsaKeyPair::generate_pkcs8(&ring::signature::ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
    let key_pair = ring::signature::EcdsaKeyPair::from_pkcs8(&ring::signature::ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng).unwrap();
    let public_key_bytes = key_pair.public_key().as_ref();
    // SEC1 uncompressed point: 0x04 || x (32 bytes) || y (32 bytes).
    assert_eq!(public_key_bytes.len(), 65);
    assert_eq!(public_key_bytes[0], 0x04);
    let x: [u8; 32] = public_key_bytes[1..33].try_into().unwrap();
    let y: [u8; 32] = public_key_bytes[33..65].try_into().unwrap();
    TestKey { key_pair, x, y }
}

pub(crate) fn cbor_ec2_public_key(x: &[u8; 32], y: &[u8; 32]) -> Vec<u8> {
    // {1: 2, 3: -7, -1: 1, -2: bytes(x), -3: bytes(y)} -- a real,
    // spec-shaped COSE_Key map.
    let mut out = vec![0xA5]; // map(5)
    out.extend_from_slice(&[0x01, 0x02]); // 1: 2 (kty: EC2)
    out.extend_from_slice(&[0x03, 0x26]); // 3: -7 (alg: ES256)
    out.extend_from_slice(&[0x20, 0x01]); // -1: 1 (crv: P-256)
    out.push(0x21); // -2 (x)
    out.push(0x58); // byte string, 1-byte length follows
    out.push(32);
    out.extend_from_slice(x);
    out.push(0x22); // -3 (y)
    out.push(0x58);
    out.push(32);
    out.extend_from_slice(y);
    out
}

/// The one relying-party ID every test fixture in this suite agrees on
/// -- real WebAuthn requires `authenticatorData`'s `rpIdHash` to equal
/// `SHA256(rp.id)`, so every caller building a fixture must use the same
/// `rp_id` the adapter is told to expect (`Es256CborAdapter::parse_and_verify_attestation`/
/// `verify_assertion`'s own `expected_rp_id` parameter).
pub(crate) const TEST_RP_ID: &str = "example.test";

/// `FLAG_ATTESTED_CREDENTIAL_DATA` inlined (0x40) -- `webauthn_crypto`'s
/// own constant is private to that module; duplicating one named bit
/// flag here is simpler than widening its visibility for a single value.
pub(crate) fn authenticator_data(sign_count: u32, attested_credential: Option<(&[u8], &[u8; 32], &[u8; 32])>) -> Vec<u8> {
    const FLAG_ATTESTED_CREDENTIAL_DATA: u8 = 0x40;
    let rp_id_hash = ring::digest::digest(&ring::digest::SHA256, TEST_RP_ID.as_bytes());
    let mut data = rp_id_hash.as_ref().to_vec();
    let flags: u8 = if attested_credential.is_some() { FLAG_ATTESTED_CREDENTIAL_DATA } else { 0 };
    data.push(flags);
    data.extend_from_slice(&sign_count.to_be_bytes());
    if let Some((cred_id, x, y)) = attested_credential {
        data.extend_from_slice(&[0u8; 16]); // aaguid, unchecked.
        data.extend_from_slice(&(cred_id.len() as u16).to_be_bytes());
        data.extend_from_slice(cred_id);
        data.extend_from_slice(&cbor_ec2_public_key(x, y));
    }
    data
}

pub(crate) fn client_data_json(ty: &str, challenge: &str, origin: &str) -> Vec<u8> {
    serde_json::json!({ "type": ty, "challenge": challenge, "origin": origin, "crossOrigin": false }).to_string().into_bytes()
}

pub(crate) fn attestation_object(auth_data: &[u8]) -> Vec<u8> {
    // {"fmt": "none", "attStmt": {}, "authData": bytes(auth_data)}
    let mut out = vec![0xA3]; // map(3)
    out.extend_from_slice(&[0x63, b'f', b'm', b't']);
    out.extend_from_slice(&[0x64, b'n', b'o', b'n', b'e']);
    out.extend_from_slice(&[0x67, b'a', b't', b't', b'S', b't', b'm', b't']);
    out.push(0xA0); // map(0), empty attStmt
    out.extend_from_slice(&[0x68, b'a', b'u', b't', b'h', b'D', b'a', b't', b'a']);
    // Byte string header for auth_data. `attestation_object`'s only
    // callers pass authenticatorData well under 256 bytes (no attStmt
    // content, a short credential ID); a longer one would need the
    // 2-byte-length CBOR form instead of this 1-byte one.
    assert!(auth_data.len() < 256, "attestation_object's 1-byte CBOR length header can't encode {} bytes", auth_data.len());
    out.push(0x58);
    out.push(auth_data.len() as u8);
    out.extend_from_slice(auth_data);
    out
}
