//! RFC 9449 (DPoP) proof verification — a plain-Rust port of
//! `runtime-kernels`'s own `dpop_verify_inner`/`dpop_jwk_thumbprint`
//! (that crate's own doc comment has the full RFC-citation-level
//! reasoning for every check below; unchanged here), not an FFI call
//! into that crate: `nirdosha-rt` is plain Rust with no compiled-
//! artifact ABI boundary to cross, so there's nothing the FFI wrapping
//! bought that's worth keeping. `Router::with_sender_constrained_tokens`
//! is this module's one caller.
//!
//! **Scope, disclosed, same as the kernel this was ported from:**
//! P-256/ES256 only (RFC 9449's own baseline profile) — RSA/PS256 DPoP
//! proofs are real, undone follow-up work. No FIPS-140-3 crypto-backend
//! swap point either (`runtime-kernels::kernel::crypto_backend`'s own
//! `fips` feature has no counterpart here) — `jsonwebtoken`'s default
//! `ring` backend is what this module actually uses, and that's not a
//! NIST CMVP-validated module.

use jsonwebtoken::jwk::{AlgorithmParameters, EllipticCurve};
use jsonwebtoken::{Algorithm, DecodingKey, Validation};

pub struct DpopVerified {
    /// The verified proof's own key thumbprint (RFC 7638) — handed back
    /// unconditionally on success so a caller can both check it against
    /// an expected binding *and* record it (e.g. at token-issuance
    /// time, to bind a freshly minted access token's own `cnf.jkt` to
    /// whatever key the client just proved it holds). `web.rs`'s one
    /// caller (`check_dpop`) doesn't read it today — it already passed
    /// `expected_jkt` in, so the binding was already checked — but this
    /// is real, public API surface for exactly the token-minting use
    /// just described, not dead weight.
    #[allow(dead_code)]
    pub jkt: String,
    /// The proof's own `jti` — a caller's own replay cache (stateful,
    /// therefore not this function's job — this function stays a pure
    /// function of its inputs) uses this to reject a repeat.
    pub jti: String,
}

/// RFC 7638 canonical JWK thumbprint, EC-only. The exact member set
/// `{crv, kty, x, y}`, no others, each already alphabetically ordered
/// by name, `serde_json`'s own compact (no whitespace) rendering — RFC
/// 7638 requires *lexicographic member ordering with no insignificant
/// whitespace*, which is exactly what building the literal object in
/// this field order and serializing it compactly gives, with no
/// separate canonicalization pass needed.
fn jwk_thumbprint(ec: &jsonwebtoken::jwk::EllipticCurveKeyParameters) -> Result<String, String> {
    use base64::Engine as _;
    use sha2::Digest;
    let crv = match ec.curve {
        EllipticCurve::P256 => "P-256",
        _ => return Err("unsupported DPoP JWK curve (only P-256/ES256 is supported)".to_string()),
    };
    let canonical = serde_json::json!({ "crv": crv, "kty": "EC", "x": ec.x, "y": ec.y });
    let bytes = serde_json::to_vec(&canonical).map_err(|e| format!("failed to canonicalize DPoP JWK: {e}"))?;
    let digest = sha2::Sha256::digest(&bytes);
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest))
}

/// Verifies `proof` (a raw `DPoP` header value) against this specific
/// request's `expected_method`/`expected_url` and, when non-empty,
/// `expected_ath` (RFC 9449 §4.3's access-token hash) and
/// `expected_jkt` (the access token's own `cnf.jkt` binding). `now` is
/// the real wall clock, read by the caller (never inside this
/// function, which stays a pure function of its inputs — the same
/// reasoning `nir_oidc_validate_token`'s own doc comment gives for the
/// identical split).
pub fn verify_proof(proof: &str, expected_method: &str, expected_url: &str, expected_ath: &str, expected_jkt: &str, max_age_secs: i64, now: i64) -> Result<DpopVerified, String> {
    let header = jsonwebtoken::decode_header(proof).map_err(|e| format!("malformed DPoP proof: {e}"))?;
    if header.typ.as_deref() != Some("dpop+jwt") {
        return Err(format!("DPoP proof header `typ` must be `dpop+jwt`, got {:?}", header.typ));
    }
    let jwk = header.jwk.ok_or("DPoP proof header is missing the required embedded `jwk`")?;
    let AlgorithmParameters::EllipticCurve(ec) = &jwk.algorithm else {
        return Err("DPoP proof's embedded key must be EC/P-256 (ES256) -- no other algorithm is supported yet".to_string());
    };
    if ec.curve != EllipticCurve::P256 {
        return Err("DPoP proof's embedded key must use the P-256 curve -- no other curve is supported yet".to_string());
    }
    let decoding_key = DecodingKey::from_ec_components(&ec.x, &ec.y).map_err(|e| format!("invalid DPoP proof key material: {e}"))?;

    let mut validation = Validation::new(Algorithm::ES256);
    validation.required_spec_claims.clear();
    validation.validate_exp = false;
    let data = jsonwebtoken::decode::<serde_json::Value>(proof, &decoding_key, &validation).map_err(|e| format!("DPoP proof signature verification failed: {e}"))?;
    let claims = data.claims;

    let htm = claims.get("htm").and_then(|v| v.as_str()).ok_or("DPoP proof is missing the required `htm` claim")?;
    if !htm.eq_ignore_ascii_case(expected_method) {
        return Err(format!("DPoP proof's `htm` claim ({htm:?}) does not match this request's method ({expected_method:?})"));
    }
    let htu = claims.get("htu").and_then(|v| v.as_str()).ok_or("DPoP proof is missing the required `htu` claim")?;
    let strip_query_fragment = |u: &str| u.split(['?', '#']).next().unwrap_or(u).to_string();
    if strip_query_fragment(htu) != strip_query_fragment(expected_url) {
        return Err(format!("DPoP proof's `htu` claim ({htu:?}) does not match this request's URL ({expected_url:?})"));
    }
    let iat = claims.get("iat").and_then(|v| v.as_i64()).ok_or("DPoP proof is missing the required `iat` claim")?;
    if iat < now - max_age_secs || iat > now + 60 {
        return Err(format!("DPoP proof's `iat` ({iat}) is outside the freshness window (now={now}, max_age={max_age_secs}s)"));
    }
    let jti = claims.get("jti").and_then(|v| v.as_str()).ok_or("DPoP proof is missing the required `jti` claim")?.to_string();

    let jkt = jwk_thumbprint(ec)?;
    if !expected_jkt.is_empty() && jkt != expected_jkt {
        return Err("DPoP proof's key does not match the access token's own `cnf.jkt` binding".to_string());
    }
    if !expected_ath.is_empty() {
        let ath = claims.get("ath").and_then(|v| v.as_str()).ok_or("DPoP proof is missing the required `ath` claim (an access token is bound to this request)")?;
        if ath != expected_ath {
            return Err("DPoP proof's `ath` claim does not match this request's access token".to_string());
        }
    }
    Ok(DpopVerified { jkt, jti })
}

/// The `ath` claim's own required value (RFC 9449 §4.3):
/// `base64url(no padding, SHA-256(access_token))` — the access token's
/// *string form exactly as it appears in the `Authorization` header*,
/// not its decoded claims.
pub fn access_token_hash(token: &str) -> String {
    use base64::Engine as _;
    use sha2::Digest;
    let digest = sha2::Sha256::digest(token.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

#[cfg(test)]
mod tests {
    use super::*;

    // A fixed, checked-in P-256 test keypair -- test-only, never used
    // for anything real. Identical to `runtime-kernels`'s own DPoP test
    // fixture (that crate's own comment: `openssl ecparam -genkey -name
    // prime256v1`, converted to PKCS#8) -- reused verbatim here rather
    // than generating a fresh one, since it's already independently
    // ground-truthed against Python's own `hashlib`/`base64` there.
    const TEST_PRIVATE_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgzvUAj7DFAlncvF+5\nKh1PaOnplTGaH4VKUbad2SJZRc2hRANCAAT4Fgvuc92G/Tlx3tdInAnryMO+cPO4\nZ77MvnaJskfNgdVa75Dkb9ta42OPIVpSYfDdWMIEg01aGaWsmssB/6vQ\n-----END PRIVATE KEY-----\n";
    const TEST_X: &str = "-BYL7nPdhv05cd7XSJwJ68jDvnDzuGe-zL52ibJHzYE";
    const TEST_Y: &str = "1VrvkORv21rjY48hWlJh8N1YwgSDTVoZpayaywH_q9A";
    const TEST_JKT: &str = "m4TkNpMqi-3VybpMJQzhinaLaT3W4Gt9I7JNoKjF9Y8";

    fn make_proof(claims: serde_json::Value) -> String {
        let mut header = jsonwebtoken::Header::new(Algorithm::ES256);
        header.typ = Some("dpop+jwt".to_string());
        header.jwk = Some(jsonwebtoken::jwk::Jwk {
            common: jsonwebtoken::jwk::CommonParameters::default(),
            algorithm: AlgorithmParameters::EllipticCurve(jsonwebtoken::jwk::EllipticCurveKeyParameters {
                key_type: jsonwebtoken::jwk::EllipticCurveKeyType::EC,
                curve: EllipticCurve::P256,
                x: TEST_X.to_string(),
                y: TEST_Y.to_string(),
            }),
        });
        let encoding_key = jsonwebtoken::EncodingKey::from_ec_pem(TEST_PRIVATE_KEY_PEM.as_bytes()).expect("fixed test key must parse");
        jsonwebtoken::encode(&header, &claims, &encoding_key).expect("signing a well-formed proof must succeed")
    }

    fn claims(htm: &str, htu: &str, iat: i64, jti: &str) -> serde_json::Value {
        serde_json::json!({ "htm": htm, "htu": htu, "iat": iat, "jti": jti })
    }

    #[test]
    fn valid_proof_verifies_and_reports_the_right_jkt() {
        let proof = make_proof(claims("POST", "https://api.example.com/api/transfer", 1_000_000, "proof-1"));
        let verified = verify_proof(&proof, "POST", "https://api.example.com/api/transfer", "", "", 300, 1_000_010).expect("a fresh, matching proof must verify");
        assert_eq!(verified.jkt, TEST_JKT, "the computed thumbprint must match the independently-computed ground truth");
        assert_eq!(verified.jti, "proof-1");
    }

    #[test]
    fn method_is_case_insensitive_but_url_is_exact() {
        let proof = make_proof(claims("post", "https://api.example.com/api/transfer", 1_000_000, "proof-2"));
        assert!(verify_proof(&proof, "POST", "https://api.example.com/api/transfer", "", "", 300, 1_000_010).is_ok());
        assert!(verify_proof(&proof, "POST", "https://api.example.com/api/OTHER", "", "", 300, 1_000_010).is_err());
    }

    #[test]
    fn htu_ignores_query_and_fragment_on_both_sides() {
        let proof = make_proof(claims("GET", "https://api.example.com/api/read?x=1", 1_000_000, "proof-3"));
        assert!(verify_proof(&proof, "GET", "https://api.example.com/api/read#frag", "", "", 300, 1_000_010).is_ok());
    }

    #[test]
    fn stale_proof_is_rejected() {
        let proof = make_proof(claims("GET", "https://api.example.com/x", 1_000_000, "proof-4"));
        assert!(verify_proof(&proof, "GET", "https://api.example.com/x", "", "", 300, 1_000_000 + 301).is_err());
    }

    #[test]
    fn key_mismatch_against_expected_jkt_is_rejected() {
        let proof = make_proof(claims("GET", "https://api.example.com/x", 1_000_000, "proof-5"));
        assert!(verify_proof(&proof, "GET", "https://api.example.com/x", "", "not-the-right-jkt", 300, 1_000_010).is_err());
        assert!(verify_proof(&proof, "GET", "https://api.example.com/x", "", TEST_JKT, 300, 1_000_010).is_ok());
    }

    #[test]
    fn tampered_signature_is_rejected() {
        let mut proof = make_proof(claims("GET", "https://api.example.com/x", 1_000_000, "proof-6"));
        proof.pop();
        proof.push('x');
        assert!(verify_proof(&proof, "GET", "https://api.example.com/x", "", "", 300, 1_000_010).is_err());
    }

    #[test]
    fn ath_binding_is_checked_only_when_expected() {
        let claims = serde_json::json!({ "htm": "GET", "htu": "https://api.example.com/x", "iat": 1_000_000, "jti": "proof-7", "ath": "expected-ath-hash" });
        let proof = make_proof(claims);
        assert!(verify_proof(&proof, "GET", "https://api.example.com/x", "", "", 300, 1_000_010).is_ok(), "no ath expected -> not checked");
        assert!(verify_proof(&proof, "GET", "https://api.example.com/x", "expected-ath-hash", "", 300, 1_000_010).is_ok());
        assert!(verify_proof(&proof, "GET", "https://api.example.com/x", "wrong-ath-hash", "", 300, 1_000_010).is_err());
    }
}
